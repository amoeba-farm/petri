//! Terminal lifecycle, event polling, draw scheduling, and the stable Lab launch contract.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalPointerShape {
    Reset,
    Default,
    Pointer,
}

pub(super) fn terminal_pointer_shape_escape(
    shape: TerminalPointerShape,
    terminal_program: Option<&str>,
) -> Option<&'static str> {
    if terminal_program == Some("Apple_Terminal") {
        return None;
    }
    Some(match shape {
        TerminalPointerShape::Reset => "\x1b]22;\x1b\\",
        TerminalPointerShape::Default => "\x1b]22;default\x1b\\",
        TerminalPointerShape::Pointer => "\x1b]22;pointer\x1b\\",
    })
}

pub(super) fn write_terminal_pointer_shape(shape: TerminalPointerShape) {
    let mut stdout = io::stdout();
    let terminal_program = env::var("TERM_PROGRAM").ok();
    if let Some(sequence) = terminal_pointer_shape_escape(shape, terminal_program.as_deref()) {
        let _ = stdout.write_all(sequence.as_bytes());
        let _ = stdout.flush();
    }
}

pub(super) fn set_terminal_pointer_shape(
    current: &mut TerminalPointerShape,
    next: TerminalPointerShape,
) {
    if *current == next {
        return;
    }
    write_terminal_pointer_shape(next);
    *current = next;
}

pub(super) struct TerminalCleanup;

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        write_terminal_pointer_shape(TerminalPointerShape::Reset);
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableMouseCapture,
            DisableFocusChange,
            LeaveAlternateScreen
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LabExitAction {
    Quit,
    RunUpdate,
    RunRebuild,
}

pub fn run_lab_bench(
    cli: &Cli,
    backend: &BackendClient,
    initial_dish: Option<&str>,
    update_check_requested: bool,
) -> Result<LabExitAction, CliError> {
    run_lab_bench_with_chart(cli, backend, initial_dish, update_check_requested, None)
}

pub fn run_lab_chart_bench(
    cli: &Cli,
    backend: &BackendClient,
    options: &ChartArgs,
    update_check_requested: bool,
) -> Result<LabExitAction, CliError> {
    run_lab_bench_with_chart(
        cli,
        backend,
        Some(&options.market),
        update_check_requested,
        Some(options),
    )
}

pub(super) fn run_lab_bench_with_chart(
    cli: &Cli,
    backend: &BackendClient,
    initial_dish: Option<&str>,
    update_check_requested: bool,
    initial_chart: Option<&ChartArgs>,
) -> Result<LabExitAction, CliError> {
    if !io::stdout().is_terminal() {
        let payload = dish_list_payload(backend)?;
        if !cli.quiet {
            println!("{}", render_lab_overview(cli, backend, &payload));
        }
        return Ok(LabExitAction::Quit);
    }

    // Finish immutable asset initialization before terminal setup and the intro
    // clock, so cold decompression cannot skip the first authored frames.
    if initial_chart.is_none() {
        std::sync::LazyLock::force(&STARTUP_INTRO_TERMINAL_FRAMES);
    }
    let attached_wallet = attached_wallet::inspect_attached_wallet(cli);
    let onchain_config = build_lab_onchain_config(cli)?;

    enable_raw_mode()
        .map_err(|error| CliError::new(format!("failed to enter raw mode: {error}")))?;
    let mut stdout = io::stdout();
    let enter_result = execute!(
        stdout,
        EnterAlternateScreen,
        EnableFocusChange,
        EnableMouseCapture
    );
    enter_result
        .map_err(|error| CliError::new(format!("failed to enter alternate screen: {error}")))?;
    let mut terminal_pointer_shape = TerminalPointerShape::Reset;
    set_terminal_pointer_shape(&mut terminal_pointer_shape, TerminalPointerShape::Default);
    let cleanup = TerminalCleanup;
    // Crossterm emits several small writes for each changed colored cell. Buffer the
    // immutable intro's frame delta into one OS write so large terminals keep cadence.
    let backend_terminal = CrosstermBackend::new(io::BufWriter::with_capacity(64 * 1024, stdout));
    let mut terminal = Terminal::new(backend_terminal)
        .map_err(|error| CliError::new(format!("failed to initialize terminal: {error}")))?;

    let backend_url = backend.base_url().to_string();
    let (fetch_tx, fetch_rx) = mpsc::channel::<LabFetchResult>();
    let mut app = LabApp::new_with_update_check(
        initial_dish,
        attached_wallet,
        onchain_config,
        update_check_requested,
    );
    if initial_chart.is_none() {
        app.show_startup_intro_at(Instant::now());
    }
    app.request_market_list(&backend_url, &fetch_tx);
    if let Some(options) = initial_chart {
        app.prepare_initial_chart(options);
    }
    app.request_update_check(&fetch_tx, false);
    app.request_guide_probe(&fetch_tx);
    let mut last_visual_tick = Instant::now();
    let mut last_intro_draw = None;
    let mut last_terminal_draw_size = None;
    let mut full_redraw_requested = false;
    let mut exit_action = LabExitAction::Quit;

    loop {
        app.apply_completed_fetches(&backend_url, &fetch_tx, &fetch_rx);
        if app.resume_pending_initial_chart() && !app.trading.loading_detail {
            app.request_selected_detail(&backend_url, &fetch_tx, false);
        }
        let now = Instant::now();
        app.refresh_chart_if_due_at(now, &backend_url, &fetch_tx);
        app.trading.expire_result_modal_at(now);
        advance_visual_ticks(&mut app, &mut last_visual_tick, now);
        app.request_visible_help_preview_if_due(&fetch_tx);
        app.retry_oracle_tree_if_due(&backend_url, &fetch_tx);
        app.refresh_gitbook_help_if_due(&fetch_tx);
        let (width, height) = crossterm::terminal::size()
            .map_err(|error| CliError::new(format!("failed to read terminal size: {error}")))?;
        app.update_terminal_size_warning(cli, Rect::new(0, 0, width, height));

        let intro_open = app.startup_intro_is_open();
        let intro_animation_visible = intro_open
            && !gitbook_reduced_motion()
            && startup_intro_animation_rect(Rect::new(0, 0, width, height)).width > 0;
        let intro_frame = if intro_animation_visible {
            app.startup_intro_frame_index_at(now)
        } else {
            0
        };
        let intro_draw = intro_open.then_some((intro_frame, width, height));
        let terminal_draw_size = (width, height);
        let terminal_geometry_changed = last_terminal_draw_size
            .is_some_and(|previous_size| previous_size != terminal_draw_size);
        if full_redraw_requested
            || terminal_geometry_changed
            || startup_intro_draw_required(last_intro_draw, intro_draw)
        {
            let can_draw_animation_delta = !full_redraw_requested
                && !terminal_geometry_changed
                && matches!(
                    (last_intro_draw, intro_draw),
                    (
                        Some((_, previous_width, previous_height)),
                        Some((_, current_width, current_height))
                    ) if previous_width == current_width && previous_height == current_height
                )
                && intro_animation_visible;
            let synchronize_full_redraw = !can_draw_animation_delta
                && (terminal_geometry_changed || last_intro_draw.is_some());
            let animation_rect = startup_intro_animation_rect(Rect::new(0, 0, width, height));
            let native_animation_delta = can_draw_animation_delta
                && startup_intro_native_animation_available(animation_rect);
            let synchronized_update = intro_open || synchronize_full_redraw;
            if synchronized_update {
                queue!(terminal.backend_mut(), BeginSynchronizedUpdate).map_err(|error| {
                    CliError::new(format!(
                        "failed to begin synchronized terminal frame: {error}"
                    ))
                })?;
                if native_animation_delta {
                    terminal.backend_mut().flush().map_err(|error| {
                        CliError::new(format!(
                            "failed to begin native synchronized terminal frame: {error}"
                        ))
                    })?;
                }
            }
            let draw_result = if can_draw_animation_delta {
                let (previous_frame, _, _) = last_intro_draw.expect("checked previous intro frame");
                let native_drawn = if native_animation_delta {
                    draw_startup_intro_animation_native(animation_rect, previous_frame, intro_frame)
                        .map_err(|error| {
                            CliError::new(format!(
                                "failed to draw native startup animation frame: {error}"
                            ))
                        })?
                } else {
                    false
                };
                if native_drawn {
                    Ok(())
                } else {
                    draw_startup_intro_animation_delta(
                        terminal.backend_mut(),
                        cli,
                        animation_rect,
                        previous_frame,
                        intro_frame,
                    )
                }
            } else {
                terminal
                    .draw(|frame| draw_lab_frame(frame, cli, backend, &app))
                    .map(|_| ())
            };
            let synchronized_end_result = if synchronized_update {
                execute!(terminal.backend_mut(), EndSynchronizedUpdate)
            } else {
                Ok(())
            };
            draw_result
                .map_err(|error| CliError::new(format!("failed to draw Lab Bench: {error}")))?;
            synchronized_end_result.map_err(|error| {
                CliError::new(format!(
                    "failed to present synchronized terminal frame: {error}"
                ))
            })?;
            last_intro_draw = intro_draw;
            last_terminal_draw_size = Some(terminal_draw_size);
            full_redraw_requested = false;
        }

        let poll_timeout = lab_event_poll_timeout(&app, last_visual_tick, Instant::now());
        if event::poll(poll_timeout)
            .map_err(|error| CliError::new(format!("failed to poll terminal input: {error}")))?
        {
            let event = event::read().map_err(|error| {
                CliError::new(format!("failed to read terminal input: {error}"))
            })?;
            if matches!(
                &event,
                Event::FocusGained | Event::FocusLost | Event::Resize(_, _)
            ) {
                app.help.clear_hover_preview();
                app.help.glossary_hover = None;
                app.help.hover_grace_ticks = None;
            }
            if terminal_event_requires_full_redraw(&event) {
                full_redraw_requested = !matches!(
                    event,
                    Event::Resize(event_width, event_height)
                        if last_terminal_draw_size == Some((event_width, event_height))
                );
                set_terminal_pointer_shape(
                    &mut terminal_pointer_shape,
                    TerminalPointerShape::Default,
                );
                continue;
            }
            if let Event::Mouse(mouse) = event {
                if app.read_panel.is_some() || app.action_panel.is_some() {
                    continue;
                }
                let (width, height) = crossterm::terminal::size().map_err(|error| {
                    CliError::new(format!("failed to read terminal size: {error}"))
                })?;
                let root = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                set_terminal_pointer_shape(
                    &mut terminal_pointer_shape,
                    app.mouse_pointer_shape_at(cli, root, mouse.column, mouse.row),
                );
                app.handle_mouse_event(mouse, cli, root, &backend_url, &fetch_tx);
                if app.updates.take_mouse_request()
                    && let Some(action) = handle_update_key(&mut app, &fetch_tx)
                {
                    exit_action = action;
                    break;
                }
                continue;
            }
            let Event::Key(key) = event else {
                continue;
            };
            if !is_actionable_key_event(key.kind) {
                continue;
            }
            if !is_deliberate_enter(&key) {
                continue;
            }
            if app.startup_intro_is_open() {
                match startup_intro_key_action(&key.code) {
                    StartupIntroKeyAction::Open => app.dismiss_startup_intro(),
                    StartupIntroKeyAction::Quit => break,
                    StartupIntroKeyAction::Ignore => {}
                }
                continue;
            }
            if app.handle_action_panel_key(&key, &fetch_tx) {
                continue;
            }
            if key.code == KeyCode::F(9)
                && app.screen != LabScreen::Terms
                && !app.trading.confirmation_is_open()
                && app.writers.confirmation.is_none()
                && app.staking_confirmation.is_none()
                && app.read_panel.is_none()
            {
                if let Some(value) = app
                    .liquidity_preview_result
                    .as_mut()
                    .and_then(|r| r.payload.take())
                {
                    app.review_action_payload(value);
                } else {
                    app.open_action_panel();
                }
                continue;
            }
            if app.handle_read_panel_key(&key, &backend_url, &fetch_tx) {
                continue;
            }
            if key.code == KeyCode::F(8)
                && app.screen != LabScreen::Terms
                && !app.trading.confirmation_is_open()
                && app.writers.confirmation.is_none()
                && app.staking_confirmation.is_none()
            {
                app.open_operations_panel(&fetch_tx);
                continue;
            }
            if is_universal_guide_shortcut(&key) {
                app.begin_guide_input(&backend_url, &fetch_tx);
                continue;
            }
            if app.handle_guide_input_key(&key, &fetch_tx) {
                continue;
            }
            if app.trading.handle_result_modal_key(&key) {
                continue;
            }
            if app.screen == LabScreen::Ledger && app.writers.confirmation.is_some() {
                match key.code {
                    KeyCode::Char('q' | 'Q') => {
                        if app.can_quit_lab() {
                            break;
                        }
                    }
                    KeyCode::Esc => app.cancel_writer_confirmation(),
                    KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                        app.writers.move_confirmation_choice(-1)
                    }
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                        app.writers.move_confirmation_choice(1)
                    }
                    KeyCode::Enter if key.kind == KeyEventKind::Press => {
                        app.activate_writer_confirmation(cli, &backend_url, &fetch_tx)
                    }
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Ledger && app.wallet_switch_editing {
                match key.code {
                    KeyCode::Esc => app.cancel_wallet_switch(),
                    KeyCode::Enter => app.switch_wallet_from_input(),
                    KeyCode::Backspace => app.backspace_wallet_switch_input(),
                    KeyCode::Char(character) => app.push_wallet_switch_char(character),
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Terms {
                if app.wallet_switch_editing {
                    match key.code {
                        KeyCode::Esc => app.cancel_wallet_switch(),
                        KeyCode::Enter => app.switch_wallet_from_input(),
                        KeyCode::Backspace => app.backspace_wallet_switch_input(),
                        KeyCode::Char(character) => app.push_wallet_switch_char(character),
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q' | 'Q') | KeyCode::Esc => break,
                        KeyCode::Enter => {
                            app.accept_wallet_terms();
                        }
                        KeyCode::Char('u' | 'U') => {
                            if let Some(action) = handle_update_key(&mut app, &fetch_tx) {
                                exit_action = action;
                                break;
                            }
                        }
                        KeyCode::Char('t') | KeyCode::Char('T') => {
                            app.open_terms_page();
                        }
                        KeyCode::Char('w') | KeyCode::Char('W') => {
                            app.begin_wallet_switch();
                        }
                        KeyCode::Char('g') | KeyCode::Char('G') => {
                            app.begin_guide_input(&backend_url, &fetch_tx);
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if app.trading.confirmation_is_open() {
                match key.code {
                    KeyCode::PageUp => {
                        app.trading.review_scroll = app.trading.review_scroll.saturating_sub(5)
                    }
                    KeyCode::PageDown => {
                        app.trading.review_scroll =
                            app.trading.review_scroll.saturating_add(5).min(24)
                    }
                    KeyCode::Char('q' | 'Q') => break,
                    KeyCode::Esc => app.cancel_trade_confirmation(),
                    KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                        app.trading.move_confirmation_choice(-1)
                    }
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                        app.trading.move_confirmation_choice(1)
                    }
                    KeyCode::Enter if key.kind == KeyEventKind::Press => {
                        app.activate_trade_confirmation(cli, &fetch_tx)
                    }
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Staking && app.staking_confirmation.is_some() {
                match key.code {
                    KeyCode::Char('q' | 'Q') => {
                        if app.can_quit_lab() {
                            break;
                        }
                    }
                    KeyCode::Esc => app.cancel_staking_confirmation(),
                    KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                        app.move_staking_confirmation_choice(-1)
                    }
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                        app.move_staking_confirmation_choice(1)
                    }
                    KeyCode::Enter if key.kind == KeyEventKind::Press => {
                        app.activate_staking_confirmation(&fetch_tx)
                    }
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Ledger && app.writers.form.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_writer_form(),
                    KeyCode::Enter => app.review_or_run_writer_form(cli, &backend_url, &fetch_tx),
                    KeyCode::Up | KeyCode::BackTab => app.move_writer_form_field(-1),
                    KeyCode::Down | KeyCode::Tab => app.move_writer_form_field(1),
                    KeyCode::Backspace => app.writers.backspace_form_input(),
                    KeyCode::Char(character) => app.writers.push_form_char(character),
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Ledger && app.liquidity_preview_form.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_liquidity_preview_form(),
                    KeyCode::Enter => app.request_liquidity_preview(&backend_url, &fetch_tx),
                    KeyCode::Up | KeyCode::BackTab => app.move_liquidity_preview_field(-1),
                    KeyCode::Down | KeyCode::Tab => app.move_liquidity_preview_field(1),
                    KeyCode::Left => app.cycle_liquidity_preview_action(-1),
                    KeyCode::Right => app.cycle_liquidity_preview_action(1),
                    KeyCode::Backspace => app.backspace_liquidity_preview_input(),
                    KeyCode::Char(character) => app.push_liquidity_preview_char(character),
                    _ => {}
                }
                continue;
            }
            let oracle_text_editing = app.screen == LabScreen::Oracle
                && (app.oracle.search_editing || app.oracle.form.is_some());
            let trade_ticket_editing =
                app.screen == LabScreen::Chain && app.trading.ticket.is_some();
            let staking_form_editing =
                app.screen == LabScreen::Staking && app.staking_form.is_some();
            let ledger_form_editing = app.screen == LabScreen::Ledger
                && (app.writers.form.is_some()
                    || app.writers.confirmation.is_some()
                    || app.liquidity_preview_form.is_some()
                    || app.wallet_switch_editing);
            if is_home_shortcut(
                &key,
                !oracle_text_editing
                    && !trade_ticket_editing
                    && !staking_form_editing
                    && !ledger_form_editing,
            ) {
                app.open_home();
                continue;
            }
            if app.screen == LabScreen::Help && app.home_help_topic == HomeHelpTopic::Overview {
                let (width, height) = crossterm::terminal::size().map_err(|error| {
                    CliError::new(format!("failed to read terminal size: {error}"))
                })?;
                let root = Rect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
                if app.handle_gitbook_help_key(&key, cli, root, &fetch_tx) {
                    continue;
                }
            }
            match key.code {
                KeyCode::PageUp => {
                    app.scroll_focused_panel(-1);
                    continue;
                }
                KeyCode::PageDown => {
                    app.scroll_focused_panel(1);
                    continue;
                }
                _ => {}
            }
            if app.screen == LabScreen::OracleHelp
                && matches!(key.code, KeyCode::Backspace | KeyCode::Left)
            {
                app.open_oracle_intro();
                continue;
            }
            if app.screen == LabScreen::Help
                && app.home_help_topic == HomeHelpTopic::Agents
                && matches!(key.code, KeyCode::Backspace | KeyCode::Left)
            {
                app.open_home();
                continue;
            }
            if app.screen == LabScreen::Oracle && app.oracle.search_editing {
                match key.code {
                    KeyCode::Esc => app.cancel_oracle_search(),
                    KeyCode::Enter => app.apply_oracle_search(),
                    KeyCode::Backspace => app.backspace_oracle_search_input(),
                    KeyCode::Char(character) => app.push_oracle_search_char(character),
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Oracle && app.oracle.form.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_oracle_form(),
                    KeyCode::Enter => app.submit_oracle_form(),
                    KeyCode::Up | KeyCode::BackTab => app.move_oracle_form_field(-1),
                    KeyCode::Down | KeyCode::Tab => app.move_oracle_form_field(1),
                    KeyCode::Backspace => app.backspace_oracle_form_input(),
                    KeyCode::Char(character) => app.push_oracle_form_char(character),
                    _ => {}
                }
                continue;
            }
            if app.screen == LabScreen::Chain && app.trading.ticket.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_trade_ticket(),
                    KeyCode::Enter => app.review_or_submit_trade_ticket(cli, &fetch_tx),
                    KeyCode::Tab => app.trading.move_ticket_field(1),
                    KeyCode::BackTab => app.trading.move_ticket_field(-1),
                    KeyCode::Backspace => app.trading.backspace_ticket_input(),
                    KeyCode::Char(character) if trade_ticket_input_character(character) => {
                        app.trading.push_ticket_char(character)
                    }
                    _ => {}
                }
                if trade_ticket_consumes_key(&key.code) {
                    continue;
                }
            }
            if app.screen == LabScreen::Staking && app.staking_form.is_some() {
                match key.code {
                    KeyCode::Esc => app.cancel_staking_form(),
                    KeyCode::Enter => app.review_staking_form(),
                    KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                        app.move_staking_form_field()
                    }
                    KeyCode::Backspace => app.backspace_staking_form_input(),
                    KeyCode::Char(character) if staking_form_input_character(character) => {
                        app.push_staking_form_char(character)
                    }
                    _ => {}
                }
                if staking_form_consumes_key(&key.code) {
                    continue;
                }
            }
            if matches!(key.code, KeyCode::Char('g' | 'G')) {
                app.begin_guide_input(&backend_url, &fetch_tx);
                continue;
            }
            match key.code {
                KeyCode::Char('q' | 'Q') | KeyCode::Esc => {
                    if app.can_quit_lab() {
                        break;
                    }
                }
                KeyCode::Char('u' | 'U') => {
                    if let Some(action) = handle_update_key(&mut app, &fetch_tx) {
                        exit_action = action;
                        break;
                    }
                }
                KeyCode::Char('/') => {
                    if app.screen == LabScreen::Oracle && app.oracle.view == OracleView::Advanced {
                        app.begin_oracle_search();
                    }
                }
                KeyCode::Backspace => {
                    if app.screen == LabScreen::Oracle && app.oracle.view == OracleView::Advanced {
                        app.open_oracle_parent();
                    }
                }
                KeyCode::Left => {
                    if app.screen == LabScreen::Ledger {
                        if app.ledger_pane == LedgerPane::Tabs {
                            app.move_ledger_view(-1, &backend_url, &fetch_tx);
                        } else {
                            app.move_ledger_pane(-1);
                        }
                    } else if app.screen == LabScreen::Detail {
                        app.move_detail_view(-1, &backend_url, &fetch_tx);
                    } else {
                        app.move_focus_left();
                    }
                }
                KeyCode::Right => {
                    if app.screen == LabScreen::Ledger {
                        if app.ledger_pane == LedgerPane::Tabs {
                            app.move_ledger_view(1, &backend_url, &fetch_tx);
                        } else {
                            app.move_ledger_pane(1);
                        }
                    } else if app.screen == LabScreen::Detail {
                        app.move_detail_view(1, &backend_url, &fetch_tx);
                    } else {
                        app.move_focus_right();
                    }
                }
                KeyCode::Char('[') => {
                    if app.screen == LabScreen::Oracle
                        && app.oracle.view == OracleView::Advanced
                        && app.focus == LabFocus::OracleTasks
                    {
                        app.select_prev_oracle_pin();
                    }
                }
                KeyCode::Char(']') => {
                    if app.screen == LabScreen::Oracle
                        && app.oracle.view == OracleView::Advanced
                        && app.focus == LabFocus::OracleTasks
                    {
                        app.select_next_oracle_pin();
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if app.screen == LabScreen::Staking {
                        app.move_staking_action(-1);
                    } else if app.screen == LabScreen::Ledger {
                        app.move_ledger_selection(-1);
                    } else {
                        app.move_focus_up(&backend_url, &fetch_tx);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.screen == LabScreen::Staking {
                        app.move_staking_action(1);
                    } else if app.screen == LabScreen::Ledger {
                        app.move_ledger_selection(1);
                    } else {
                        app.move_focus_down(&backend_url, &fetch_tx);
                    }
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    if app.screen == LabScreen::Ledger {
                        app.move_ledger_pane(if key.code == KeyCode::BackTab { -1 } else { 1 });
                    } else if key.code == KeyCode::BackTab {
                        app.move_focus_prev_panel();
                    } else {
                        app.move_focus_next_panel();
                    }
                }
                KeyCode::Enter => {
                    if app.screen == LabScreen::Ledger {
                        app.activate_ledger_control(cli, &backend_url, &fetch_tx);
                    } else {
                        app.activate_focused_panel(&backend_url, &fetch_tx);
                    }
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    if app.screen == LabScreen::Chart {
                        app.request_chart(&backend_url, &fetch_tx, true);
                    } else if app.screen == LabScreen::Ledger {
                        app.request_ledger(&backend_url, &fetch_tx, true);
                    } else if app.screen == LabScreen::Detail
                        && app.trading.detail_view == DetailView::Settlement
                    {
                        app.request_settlement(&backend_url, &fetch_tx, true);
                    } else if app.screen == LabScreen::Staking {
                        app.request_staking_status(&fetch_tx, true);
                    } else if app.screen == LabScreen::Oracle {
                        app.request_oracle_tree(&backend_url, &fetch_tx, true);
                        app.request_oracle_live(&backend_url, &fetch_tx, true);
                        app.request_oracle_rewards(&backend_url, &fetch_tx, true);
                    } else {
                        app.request_market_refresh(&backend_url, &fetch_tx)
                    }
                }
                KeyCode::Char('1') if app.screen == LabScreen::Chart => {
                    app.set_chart_range(&backend_url, &fetch_tx, ChartRangeValue::OneHour);
                }
                KeyCode::Char('2') if app.screen == LabScreen::Chart => {
                    app.set_chart_range(&backend_url, &fetch_tx, ChartRangeValue::TwentyFourHours);
                }
                KeyCode::Char('3') if app.screen == LabScreen::Chart => {
                    app.set_chart_range(&backend_url, &fetch_tx, ChartRangeValue::SevenDays);
                }
                KeyCode::Char('4') if app.screen == LabScreen::Chart => {
                    app.set_chart_range(&backend_url, &fetch_tx, ChartRangeValue::ThirtyDays);
                }
                KeyCode::Char('5') if app.screen == LabScreen::Chart => {
                    app.set_chart_range(&backend_url, &fetch_tx, ChartRangeValue::All);
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    if matches!(app.screen, LabScreen::Chart | LabScreen::Chain) {
                        app.open_activity();
                    }
                }
                KeyCode::Char('b') | KeyCode::Char('B') => {
                    let opened_chain = app.screen != LabScreen::Chain;
                    app.select_trade_action(TradeAction::Buy);
                    if opened_chain {
                        app.request_oracle_live(&backend_url, &fetch_tx, false);
                    }
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    let opened_chain = app.screen != LabScreen::Chain;
                    app.select_trade_action(TradeAction::Sell);
                    if opened_chain {
                        app.request_oracle_live(&backend_url, &fetch_tx, false);
                    }
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    app.open_chart(&backend_url, &fetch_tx);
                }
                KeyCode::Char('v') | KeyCode::Char('V') => match (app.screen, app.oracle.view) {
                    (LabScreen::Oracle, OracleView::Earn) => {
                        app.open_oracle(&backend_url, &fetch_tx)
                    }
                    (LabScreen::Oracle, OracleView::Advanced) => {
                        app.open_oracle_earn(&backend_url, &fetch_tx)
                    }
                    _ => app.open_oracle_intro(),
                },
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    app.open_ledger(&backend_url, &fetch_tx);
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    app.open_chain();
                }
                _ => {}
            }
        }
    }

    drop(terminal);
    drop(cleanup);

    Ok(exit_action)
}

pub(super) fn advance_visual_ticks(app: &mut LabApp, last_tick: &mut Instant, now: Instant) {
    let interval = Duration::from_millis(LAB_VISUAL_TICK_INTERVAL_MS);
    let elapsed = now.saturating_duration_since(*last_tick);
    if elapsed < interval {
        return;
    }

    let tick_count =
        (elapsed.as_millis() / interval.as_millis()).min(LAB_MAX_VISUAL_TICKS_PER_FRAME);
    for _ in 0..tick_count {
        app.tick();
    }
    *last_tick = now;
}

pub(super) fn lab_event_poll_timeout(app: &LabApp, last_tick: Instant, now: Instant) -> Duration {
    let input_timeout = if app.is_loading() {
        Duration::from_millis(80)
    } else {
        Duration::from_millis(200)
    };
    let interval = Duration::from_millis(LAB_VISUAL_TICK_INTERVAL_MS);
    let elapsed = now.saturating_duration_since(last_tick);
    let visual_timeout = interval.saturating_sub(elapsed);
    let timeout = input_timeout.min(visual_timeout);
    let timeout = if app.startup_intro_is_open() && !gitbook_reduced_motion() {
        timeout.min(app.startup_intro_next_frame_timeout_at(now))
    } else {
        timeout
    };
    terminal_poll_timeout(timeout)
}

pub(super) fn terminal_poll_timeout(timeout: Duration) -> Duration {
    #[cfg(windows)]
    {
        // Crossterm 0.28 passes `Duration::as_millis()` to WaitForMultipleObjects,
        // truncating fractional milliseconds. Round up so an early wake cannot spin
        // through duplicate animation redraws before the next frame boundary.
        let whole_millis = timeout.as_millis() as u64;
        if timeout > Duration::from_millis(whole_millis) {
            Duration::from_millis(whole_millis.saturating_add(1))
        } else {
            timeout
        }
    }
    #[cfg(not(windows))]
    {
        timeout
    }
}

pub(super) fn startup_intro_draw_required(
    previous: Option<(usize, u16, u16)>,
    current: Option<(usize, u16, u16)>,
) -> bool {
    current.is_none() || current != previous
}

pub(super) fn terminal_event_requires_full_redraw(event: &Event) -> bool {
    matches!(event, Event::FocusGained | Event::Resize(_, _))
}

pub(super) fn trade_ticket_consumes_key(code: &KeyCode) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab | KeyCode::Backspace => {
            true
        }
        KeyCode::Char(character) => trade_ticket_input_character(*character),
        _ => false,
    }
}

pub(super) fn trade_ticket_input_character(character: char) -> bool {
    character.is_ascii_digit() || character == '.'
}

pub(super) fn staking_form_input_character(character: char) -> bool {
    character.is_ascii_digit() || character == '.'
}

pub(super) fn staking_form_consumes_key(code: &KeyCode) -> bool {
    match code {
        KeyCode::Esc
        | KeyCode::Enter
        | KeyCode::Tab
        | KeyCode::BackTab
        | KeyCode::Backspace
        | KeyCode::Up
        | KeyCode::Down => true,
        KeyCode::Char(character) => staking_form_input_character(*character),
        _ => false,
    }
}

pub(super) fn validate_staking_decimal(
    label: &str,
    input: &str,
    allow_zero: bool,
) -> Result<(), String> {
    let value = input.trim();
    let valid_shape = !value.is_empty()
        && value.len() <= 40
        && value
            .split_once('.')
            .map(|(whole, fractional)| {
                !whole.is_empty()
                    && !fractional.is_empty()
                    && whole.chars().all(|character| character.is_ascii_digit())
                    && fractional
                        .chars()
                        .all(|character| character.is_ascii_digit())
                    && !fractional.contains('.')
            })
            .unwrap_or_else(|| value.chars().all(|character| character.is_ascii_digit()));
    if !valid_shape {
        return Err(format!(
            "{label} must be a plain token amount, for example 12.5."
        ));
    }
    if !allow_zero
        && !value
            .chars()
            .any(|character| matches!(character, '1'..='9'))
    {
        return Err(format!("{label} must be greater than zero."));
    }
    Ok(())
}

pub(super) fn is_home_shortcut(key: &KeyEvent, allow_plain_m: bool) -> bool {
    matches!(key.code, KeyCode::Home)
        || (matches!(key.code, KeyCode::Char('h' | 'H'))
            && key.modifiers.contains(KeyModifiers::ALT))
        || (allow_plain_m
            && matches!(key.code, KeyCode::Char('m' | 'M'))
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::CONTROL))
}

pub(super) fn is_universal_guide_shortcut(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::F(2))
        || (matches!(key.code, KeyCode::Char('g' | 'G'))
            && key.modifiers.contains(KeyModifiers::CONTROL))
}

pub(super) fn is_deliberate_enter(key: &KeyEvent) -> bool {
    !matches!(key.code, KeyCode::Enter) || key.kind == KeyEventKind::Press
}

pub(super) fn handle_update_key(
    app: &mut LabApp,
    fetch_tx: &Sender<LabFetchResult>,
) -> Option<LabExitAction> {
    if app.trading.submit_is_running() {
        app.status =
            "Order submission is still running. Wait for the result before updating Petri."
                .to_string();
        return None;
    }
    if let Some(action) = app.updates.exit_action() {
        return Some(action);
    }
    app.request_update_check(fetch_tx, true);
    None
}

pub(super) fn build_lab_onchain_config(cli: &Cli) -> Result<OnchainConfig, CliError> {
    crate::app_context::build_onchain_config(cli)
}
