//! Root frame, Guide rail, modal overlays, header, and footer layout.

use super::super::*;

pub(in super::super) fn draw_lab_frame(
    frame: &mut Frame<'_>,
    cli: &Cli,
    _backend: &BackendClient,
    app: &LabApp,
) {
    let root = frame.area();
    if app.startup_intro_is_open() {
        draw_startup_intro(frame, cli, root, app);
        return;
    }
    let frame_layout = lab_frame_layout(root, cli, app);
    let header_area = frame_layout.header_area;
    let body_area = frame_layout.body_area;
    let guide_area = frame_layout.guide_area;
    let footer_area = frame_layout.footer_area;

    fill_tui_area(frame, cli, root, TUI_BACKGROUND);

    draw_header(frame, cli, header_area, app);

    if app.screen == LabScreen::Terms {
        draw_terms_screen(frame, cli, body_area, app);
        if let Some(guide_area) = guide_area {
            draw_guide_panel(frame, cli, guide_area, app);
        }
        let missing_lines =
            focused_missing_lines_below(cli, app, body_area, None, frame_layout.activity_area);
        let help =
            Paragraph::new(footer_lines_for_app(cli, app, missing_lines)).wrap(Wrap { trim: true });
        frame.render_widget(help, footer_area);
        draw_lab_modal_overlays(frame, cli, lab_modal_root(root, cli, app), app);
        draw_terminal_size_warning(frame, cli, root, app);
        return;
    }

    let body_layout = lab_page_layout(app.screen, frame_layout);
    if let Some(market_area) = body_layout.market_area {
        draw_dish_list(frame, cli, market_area, app);
    }
    draw_selected_screen(frame, cli, body_layout.selected_area, app);

    if app.screen == LabScreen::Chain && body_layout.activity_area.height > 0 {
        draw_trade_panel(frame, cli, body_layout.activity_area, app);
    }
    if let Some(guide_area) = guide_area {
        draw_guide_panel(frame, cli, guide_area, app);
    }

    let missing_lines = focused_missing_lines_below(
        cli,
        app,
        body_layout.selected_area,
        body_layout.market_area,
        body_layout.activity_area,
    );
    let help =
        Paragraph::new(footer_lines_for_app(cli, app, missing_lines)).wrap(Wrap { trim: true });
    frame.render_widget(help, footer_area);

    draw_lab_modal_overlays(frame, cli, lab_modal_root(root, cli, app), app);
    draw_terminal_size_warning(frame, cli, root, app);
}

pub(in super::super) fn guide_panel_requested(app: &LabApp) -> bool {
    app.guide.composing
        || app.guide.loading
        || app.focus == LabFocus::Guide
        || !app.guide.messages.is_empty()
        || app.guide.action_preview.is_some()
        || !app.guide.suggested_actions.is_empty()
        || !app.guide.highlighted_targets.is_empty()
        || !app.guide.comparison_targets.is_empty()
}

pub(in super::super) fn terminal_size_hides_essential_content(
    root: Rect,
    cli: &Cli,
    app: &LabApp,
) -> bool {
    if root.width == 0 || root.height == 0 {
        return true;
    }

    let frame_layout = lab_frame_layout(root, cli, app);
    if frame_layout.footer_area.height == 0
        || panel_inner_rect(frame_layout.header_area).is_none()
        || (header_nav_visible(app)
            && header_nav_button_rects(cli, frame_layout.header_area, app).is_none())
    {
        return true;
    }

    let page_layout = lab_page_layout(app.screen, frame_layout);
    if panel_inner_rect(page_layout.selected_area).is_none()
        || page_layout
            .market_area
            .is_some_and(|area| panel_inner_rect(area).is_none())
    {
        return true;
    }

    if app.screen == LabScreen::Help && app.home_help_topic == HomeHelpTopic::Overview {
        let help_layout = gitbook_help_layout(page_layout.selected_area);
        if panel_inner_rect(help_layout.navigation_area).is_none()
            || panel_inner_rect(help_layout.article_area).is_none()
        {
            return true;
        }
    }

    if guide_panel_requested(app) && frame_layout.guide_area.and_then(panel_inner_rect).is_none() {
        return true;
    }

    false
}

pub(in super::super) fn terminal_size_warning_border_color(
    started_tick: Option<usize>,
    current_tick: usize,
    reduced_motion: bool,
) -> Color {
    if reduced_motion {
        return Color::LightRed;
    }
    let Some(started_tick) = started_tick else {
        return Color::Red;
    };
    let elapsed = current_tick.saturating_sub(started_tick);
    if elapsed >= TERMINAL_SIZE_WARNING_ATTENTION_TICKS {
        Color::Red
    } else if (elapsed / 2).is_multiple_of(2) {
        Color::LightRed
    } else {
        Color::Red
    }
}

pub(in super::super) fn terminal_size_warning_rect(root: Rect) -> Rect {
    let width = root.width.saturating_sub(2).min(62);
    let height = root.height.saturating_sub(2).min(9);
    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: root.y + root.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(in super::super) fn draw_terminal_size_warning(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    if !terminal_size_hides_essential_content(root, cli, app) {
        return;
    }

    let warning = terminal_size_warning_rect(root);
    if warning.width == 0 || warning.height == 0 {
        return;
    }

    dim_tui_for_modal(frame, cli, root);
    frame.render_widget(Clear, warning);
    let color = terminal_size_warning_border_color(
        app.terminal_size_warning_started_tick,
        app.spinner_tick,
        gitbook_reduced_motion(),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(style(cli, color).add_modifier(Modifier::BOLD))
        .style(tui_alt_panel_style(cli))
        .title(Span::styled(
            " ! TERMINAL TOO SMALL ! ",
            style(cli, color).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(warning);
    frame.render_widget(block, warning);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = if inner.width >= 34 && inner.height >= 5 {
        vec![
            Line::from(Span::styled(
                "Some Petri content is hidden.",
                style(cli, Color::LightRed).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!(
                "Current terminal: {} columns × {} rows.",
                root.width, root.height
            )),
            Line::from("Widen the window or make it taller."),
            Line::from("The full layout returns automatically after resize."),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                "Content hidden.",
                style(cli, Color::LightRed).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "Resize wider/taller ({}×{}).",
                root.width, root.height
            )),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true }),
        inner,
    );
}

pub(in super::super) fn lab_modal_root(root: Rect, cli: &Cli, app: &LabApp) -> Rect {
    modal_root_around_guide(root, lab_frame_layout(root, cli, app).guide_area)
}

pub(in super::super) fn modal_root_around_guide(root: Rect, guide_area: Option<Rect>) -> Rect {
    let Some(guide) = guide_area else {
        return root;
    };
    if guide.x > root.x {
        return Rect {
            width: guide.x.saturating_sub(root.x),
            ..root
        };
    }
    if guide.y > root.y {
        return Rect {
            height: guide.y.saturating_sub(root.y),
            ..root
        };
    }
    root
}

pub(in super::super) fn draw_guide_panel(
    frame: &mut Frame<'_>,
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focused = app.focus == LabFocus::Guide;
    let title = app.guide.provider_status.title();
    let color = match app.guide.provider_status {
        guide::GuideProviderStatus::Connected(_) => Color::LightGreen,
        guide::GuideProviderStatus::Choose { .. } => Color::LightCyan,
        guide::GuideProviderStatus::SetupRequired { .. } => Color::Yellow,
        guide::GuideProviderStatus::Off => Color::DarkGray,
        guide::GuideProviderStatus::Checking => Color::Cyan,
    };
    let block = panel_block(cli, &title, color, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if inner.height <= 2 {
        let text = if app.guide.composing {
            guide_input_text(app, usize::from(inner.width))
        } else if app.guide.loading {
            format!(
                "{} Guide {}...",
                app.spinner(),
                app.guide.progress.as_deref().unwrap_or("thinking")
            )
        } else {
            "[g] ask about this screen".to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text,
                style(cli, if focused { Color::Yellow } else { Color::Gray }),
            ))),
            inner,
        );
        return;
    }

    let input_height = if inner.height >= 4 { 2 } else { 1 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(input_height.min(inner.height)),
        ])
        .split(inner);
    let message_area = rows[0];
    let input_area = rows[1];
    let lines = guide_panel_lines(cli, app, usize::from(message_area.width.max(1)));
    let visible_height = usize::from(message_area.height);
    let max_scroll = lines
        .len()
        .saturating_sub(visible_height.saturating_sub(1).max(1));
    let scroll = max_scroll.saturating_sub(app.guide.scroll.min(max_scroll));
    let visible = scroll_lines_to_height(lines, visible_height, cli, scroll, focused);
    frame.render_widget(
        Paragraph::new(visible).wrap(Wrap { trim: false }),
        message_area,
    );

    let input = Paragraph::new(Line::from(Span::styled(
        guide_input_text(app, usize::from(input_area.width)),
        style(
            cli,
            if app.guide.composing {
                Color::White
            } else {
                Color::Gray
            },
        )
        .bg(if cli.no_color {
            Color::Reset
        } else if app.guide.composing {
            TUI_FIELD_ACTIVE_BACKGROUND
        } else {
            TUI_FIELD_BACKGROUND
        }),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(style(cli, if focused { Color::Yellow } else { color })),
    );
    frame.render_widget(input, input_area);
}

pub(in super::super) fn guide_input_text(app: &LabApp, width: usize) -> String {
    if let guide::GuideProviderStatus::Choose { providers } = &app.guide.provider_status {
        let count = providers.len();
        let index = app.guide.selected_provider.min(count.saturating_sub(1));
        let name = providers
            .get(index)
            .map(|connection| connection.kind.display_name())
            .unwrap_or("No provider");
        if app.focus != LabFocus::Guide {
            return fit_text_to_width(
                &format!(
                    " [g] choose Guide | {name} ({}/{})",
                    index.saturating_add(1),
                    count
                ),
                width,
            );
        }
        fit_text_to_width(
            &format!(
                " {name} ({}/{}) | [←/→] choose | [Enter] use | [Esc] leave",
                index.saturating_add(1),
                count
            ),
            width,
        )
    } else if app.guide.composing {
        if app.guide.input.is_empty() {
            fit_text_to_width(" Ask a question_", width)
        } else {
            let available = width.saturating_sub(2);
            let tail = app
                .guide
                .input
                .chars()
                .rev()
                .take(available)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!(" {tail}_")
        }
    } else if app.guide.loading {
        format!(
            " {} Guide {}...",
            app.spinner(),
            app.guide.progress.as_deref().unwrap_or("thinking")
        )
    } else {
        fit_text_to_width(" [g] ask [Enter] send [esc] exit", width)
    }
}

pub(in super::super) fn guide_panel_lines(
    cli: &Cli,
    app: &LabApp,
    width: usize,
) -> Vec<Line<'static>> {
    let width = width.max(8);
    let mut lines = Vec::new();
    if app.guide.messages.is_empty() {
        for line in wrap_gitbook_text(
            &app.guide.provider_status.panel_message(),
            width,
            "Guide · ",
            "        ",
        ) {
            lines.push(Line::from(Span::styled(line, style(cli, Color::Gray))));
        }
    } else {
        for message in &app.guide.messages {
            let (prefix, continuation, color) = match message.role {
                guide::GuideConversationRole::User => ("You · ", "      ", Color::Cyan),
                guide::GuideConversationRole::Assistant => ("Guide · ", "        ", Color::White),
            };
            for line in wrap_gitbook_text(&message.text, width, prefix, continuation) {
                lines.push(Line::from(Span::styled(line, style(cli, color))));
            }
            lines.push(Line::from(""));
        }
    }
    if app.guide.loading {
        lines.push(Line::from(Span::styled(
            format!(
                "{} {}...",
                app.spinner(),
                app.guide.progress.as_deref().unwrap_or("thinking")
            ),
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        )));
    }
    if app.guide.comparison_targets.len() == 2 {
        let first = guide_target_label(app, &app.guide.comparison_targets[0]);
        let second = guide_target_label(app, &app.guide.comparison_targets[1]);
        for line in wrap_gitbook_text(
            &format!("Comparing {first} with {second}."),
            width,
            "Compare · ",
            "          ",
        ) {
            lines.push(Line::from(Span::styled(
                line,
                style(cli, Color::LightMagenta).add_modifier(Modifier::BOLD),
            )));
        }
    }
    if let Some(target_id) = app.guide.focused_control.as_deref() {
        let label = guide_target_label(app, target_id);
        for line in wrap_gitbook_text(
            &format!("{label} is ready. Review it, then press or click it yourself."),
            width,
            "Your click · ",
            "             ",
        ) {
            lines.push(Line::from(Span::styled(
                line,
                style(cli, Color::LightYellow).add_modifier(Modifier::BOLD),
            )));
        }
    }
    if let Some(preview) = &app.guide.action_preview {
        for line in wrap_gitbook_text(
            &format!(
                "{} — {} Confirmation stays in Petri's normal review flow.",
                preview.title, preview.summary
            ),
            width,
            "Preview · ",
            "          ",
        ) {
            lines.push(Line::from(Span::styled(line, style(cli, Color::Yellow))));
        }
    }
    for suggestion in app.guide.suggested_actions.iter().take(4) {
        for line in wrap_gitbook_text(suggestion, width, "Try · ", "      ") {
            lines.push(Line::from(Span::styled(line, style(cli, Color::LightCyan))));
        }
    }
    lines
}

pub(in super::super) fn guide_suggestion_hit_at(
    cli: &Cli,
    root: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<usize> {
    let guide_area = lab_frame_layout(root, cli, app).guide_area?;
    let inner = panel_inner_rect(guide_area)?;
    if inner.height <= 2 {
        return None;
    }
    let input_height = if inner.height >= 4 { 2 } else { 1 };
    let message_height = inner.height.saturating_sub(input_height);
    let message_area = Rect::new(inner.x, inner.y, inner.width, message_height);
    if !rect_contains(message_area, column, row) {
        return None;
    }

    let width = usize::from(message_area.width.max(1));
    let lines = guide_panel_lines(cli, app, width);
    let visible_height = usize::from(message_area.height);
    let max_scroll = lines.len().saturating_sub(visible_height);
    let scroll = max_scroll.saturating_sub(app.guide.scroll.min(max_scroll));
    let source_row = scroll + usize::from(row.saturating_sub(message_area.y));
    let suggestion_counts = app
        .guide
        .suggested_actions
        .iter()
        .take(4)
        .map(|suggestion| wrap_gitbook_text(suggestion, width, "Try · ", "      ").len())
        .collect::<Vec<_>>();
    let mut suggestion_row = lines
        .len()
        .saturating_sub(suggestion_counts.iter().sum::<usize>());
    for (index, count) in suggestion_counts.into_iter().enumerate() {
        if (suggestion_row..suggestion_row + count).contains(&source_row) {
            return Some(index);
        }
        suggestion_row += count;
    }
    None
}

pub(in super::super) fn guide_target_label(app: &LabApp, target_id: &str) -> String {
    app.guide_available_targets()
        .into_iter()
        .find(|target| target.target_id == target_id)
        .map(|target| target.label)
        .or_else(|| {
            app.guide_available_form_actions()
                .into_iter()
                .find(|target| target.target_id == target_id)
                .map(|target| target.title)
        })
        .or_else(|| {
            app.oracle_tree()
                .and_then(|tree| tree.nodes.iter().find(|node| node.node_id == target_id))
                .map(|node| node.label.clone())
        })
        .unwrap_or_else(|| target_id.to_string())
}

pub(in super::super) fn draw_lab_modal_overlays(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    if app.action_panel.is_some() {
        actions::draw(frame, cli, root, app);
        return;
    }
    if app.read_panel.is_some() {
        read_panel::draw(frame, cli, root, app);
        return;
    }
    if app.trading.result_modal_is_open() {
        draw_trade_result_modal(frame, cli, root, app);
    } else if app.trading.confirmation_is_open() {
        draw_trade_confirmation_modal(frame, cli, root, app);
    } else if app.writers.confirmation.is_some() {
        draw_writer_confirmation_modal(frame, cli, root, app);
    }
}

pub(in super::super) fn trade_result_modal_rect(root: Rect) -> Rect {
    let width = root.width.saturating_sub(4).min(72);
    let height = root.height.saturating_sub(4).min(13);
    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: root.y + root.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(in super::super) fn trade_result_close_rect(root: Rect) -> Option<Rect> {
    let modal = trade_result_modal_rect(root);
    if modal.width < 8 || modal.height < 3 {
        return None;
    }
    Some(Rect {
        x: modal.x + modal.width.saturating_sub(6),
        y: modal.y,
        width: 5,
        height: 1,
    })
}

pub(in super::super) fn draw_trade_result_modal(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    let Some(result) = app.trading.result_modal.as_ref() else {
        return;
    };
    dim_tui_for_modal(frame, cli, root);

    let modal = trade_result_modal_rect(root);
    if modal.width < 4 || modal.height < 4 {
        return;
    }
    let result_color = if result.waiting {
        Color::Yellow
    } else if result.ok {
        Color::Green
    } else {
        Color::Red
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style(cli, result_color).add_modifier(Modifier::BOLD))
        .style(tui_alt_panel_style(cli))
        .title(Span::styled(
            " order result ",
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    if let Some(close) = trade_result_close_rect(root) {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [X] ",
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            )))
            .style(tui_alt_panel_style(cli)),
            close,
        );
    }

    let footer_height = inner.height.min(1);
    let content_height = inner.height.saturating_sub(footer_height);
    if content_height > 0 {
        frame.render_widget(
            Paragraph::new(trade_result_modal_lines(cli, result))
                .style(tui_alt_panel_style(cli))
                .wrap(Wrap { trim: true }),
            Rect {
                x: inner.x.saturating_add(1),
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: content_height,
            },
        );
    }

    if footer_height > 0 {
        let remaining = result.remaining_seconds_at(Instant::now()).max(1);
        let unit = if remaining == 1 { "second" } else { "seconds" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Auto-closing in ", style(cli, Color::DarkGray)),
                Span::styled(
                    remaining.to_string(),
                    style(cli, result_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {unit}  |  Esc closes now  |  Click [X]"),
                    style(cli, Color::DarkGray),
                ),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .style(tui_alt_panel_style(cli)),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            },
        );
    }
}

pub(in super::super) fn trade_result_modal_lines(
    cli: &Cli,
    result: &TradeResultModal,
) -> Vec<Line<'static>> {
    let result_color = if result.ok { Color::Green } else { Color::Red };
    let mut lines = vec![Line::from(Span::styled(
        if result.waiting {
            "ORDER WAITING"
        } else if result.ok {
            "ORDER SUCCESS"
        } else {
            "ORDER FAILURE"
        },
        style(cli, result_color).add_modifier(Modifier::BOLD),
    ))];
    if let Some(summary) = result.summary.as_ref() {
        let strategy = match summary.kind {
            OptionKind::Call => "call spread",
            OptionKind::Put => "put spread",
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{} {} {strategy} {}-{}",
                summary.action.label().to_ascii_uppercase(),
                summary.symbol,
                summary.lower_strike,
                summary.upper_strike
            ),
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("Contracts {}  |  Expiry {}", summary.qty, summary.expiry),
            style(cli, Color::White),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            if result.waiting {
                format!(
                    "{} order is waiting for finalized state.",
                    result.action.label()
                )
            } else if result.ok {
                format!("{} order submission completed.", result.action.label())
            } else {
                format!(
                    "{} order submission did not complete.",
                    result.action.label()
                )
            },
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));
    if result.waiting {
        lines.push(Line::from(Span::styled(
            "Check Operations for the exact status. Do not resubmit an unresolved payment.",
            style(cli, Color::White),
        )));
    } else if result.ok {
        lines.push(Line::from(Span::styled(
            "Check Activity for transaction status and fills.",
            style(cli, Color::White),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            result
                .failure_reason
                .as_ref()
                .map(|reason| format!("Problem: {reason}"))
                .unwrap_or_else(|| "Problem: The order could not be submitted.".to_string()),
            style(cli, Color::White),
        )));
        lines.push(Line::from(Span::styled(
            if result.details_in_ticket {
                "Review the ticket before trying again."
            } else {
                "No automatic retry was made."
            },
            style(cli, Color::DarkGray),
        )));
    }
    lines
}

pub(in super::super) fn trade_confirmation_modal_rect(root: Rect) -> Rect {
    let width = root.width.saturating_sub(4).min(72);
    let height = root.height.saturating_sub(4).min(24);
    Rect {
        x: root.x + root.width.saturating_sub(width) / 2,
        y: root.y + root.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(in super::super) fn trade_confirmation_button_rects(root: Rect) -> Option<(Rect, Rect)> {
    let modal = trade_confirmation_modal_rect(root);
    let inner = panel_inner_rect(modal)?;
    if inner.width < 24 || inner.height < 5 {
        return None;
    }
    let margin = 1;
    let gap = if inner.width >= 50 { 3 } else { 1 };
    let available = inner.width.saturating_sub(margin * 2).saturating_sub(gap);
    let cancel_width = available / 2;
    let confirm_width = available.saturating_sub(cancel_width);
    if cancel_width < 8 || confirm_width < 8 {
        return None;
    }
    let y = inner.y + inner.height.saturating_sub(3);
    let cancel = Rect {
        x: inner.x + margin,
        y,
        width: cancel_width,
        height: 3,
    };
    let confirm = Rect {
        x: cancel.x + cancel.width + gap,
        y,
        width: confirm_width,
        height: 3,
    };
    Some((cancel, confirm))
}

pub(in super::super) fn trade_confirmation_button_at(
    root: Rect,
    column: u16,
    row: u16,
) -> Option<TradeConfirmationChoice> {
    let (cancel, confirm) = trade_confirmation_button_rects(root)?;
    if rect_contains(cancel, column, row) {
        Some(TradeConfirmationChoice::Cancel)
    } else if rect_contains(confirm, column, row) {
        Some(TradeConfirmationChoice::Confirm)
    } else {
        None
    }
}

pub(in super::super) fn draw_trade_confirmation_modal(
    frame: &mut Frame<'_>,
    cli: &Cli,
    root: Rect,
    app: &LabApp,
) {
    let Some(confirmation) = app
        .trading
        .ticket
        .as_ref()
        .and_then(|ticket| ticket.confirmation.as_ref())
    else {
        return;
    };
    dim_tui_for_modal(frame, cli, root);

    let modal = trade_confirmation_modal_rect(root);
    if modal.width < 4 || modal.height < 4 {
        return;
    }
    frame.render_widget(Clear, modal);
    let title = format!(
        " confirm {} order ",
        confirmation
            .prepared
            .summary
            .action
            .label()
            .to_ascii_lowercase()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            style(
                cli,
                trade_action_color(confirmation.prepared.summary.action),
            )
            .add_modifier(Modifier::BOLD),
        )
        .style(tui_alt_panel_style(cli))
        .title(Span::styled(
            title,
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let content_height = inner.height.saturating_sub(4);
    if content_height > 0 {
        let mut reviewed_lines = Vec::new();
        if let Some(result) = app.trading.ticket.as_ref().and_then(|t| t.result.as_ref()) {
            reviewed_lines.extend(
                result
                    .message
                    .lines()
                    .map(|s| Line::from(crate::backend::terminal_safe_text(s))),
            );
        } else {
            reviewed_lines.extend(trade_confirmation_lines(
                cli,
                &confirmation.prepared.summary,
            ));
        }
        let content = Paragraph::new(reviewed_lines)
            .style(tui_alt_panel_style(cli))
            .scroll((app.trading.review_scroll, 0))
            .wrap(Wrap { trim: true });
        frame.render_widget(
            content,
            Rect {
                x: inner.x.saturating_add(1),
                y: inner.y,
                width: inner.width.saturating_sub(2),
                height: content_height,
            },
        );
    }

    let hint_y = inner.y + inner.height.saturating_sub(4);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tab choose | PgUp/PgDn scroll | Enter approve | Esc cancel",
            style(cli, Color::DarkGray),
        )))
        .alignment(ratatui::layout::Alignment::Center)
        .style(tui_alt_panel_style(cli)),
        Rect {
            x: inner.x,
            y: hint_y,
            width: inner.width,
            height: 1,
        },
    );

    if let Some((cancel_area, confirm_area)) = trade_confirmation_button_rects(root) {
        let cancel_selected = confirmation.choice == TradeConfirmationChoice::Cancel;
        let confirm_selected = confirmation.choice == TradeConfirmationChoice::Confirm;
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                "CANCEL",
                Color::Blue,
                cancel_selected,
                cancel_area.width,
                cancel_area.height,
            )),
            cancel_area,
        );
        frame.render_widget(
            Paragraph::new(raised_button_lines(
                cli,
                "CONFIRM ORDER",
                if app.guide.focused_control.as_deref() == Some("control:trade:confirm") {
                    Color::LightYellow
                } else {
                    trade_action_color(confirmation.prepared.summary.action)
                },
                confirm_selected,
                confirm_area.width,
                confirm_area.height,
            )),
            confirm_area,
        );
    }
}

pub(in super::super) fn dim_tui_for_modal(frame: &mut Frame<'_>, cli: &Cli, root: Rect) {
    let dim_style = Style::default()
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::BOLD);
    let buffer = frame.buffer_mut();
    for row in root.y..root.y.saturating_add(root.height) {
        for column in root.x..root.x.saturating_add(root.width) {
            let cell = &mut buffer[(column, row)];
            if !cli.no_color {
                cell.set_fg(Color::Rgb(73, 84, 102)).set_bg(TUI_BACKGROUND);
            }
            cell.set_style(dim_style);
        }
    }
}

pub(in super::super) fn trade_confirmation_lines(
    cli: &Cli,
    summary: &TradeConfirmationSummary,
) -> Vec<Line<'static>> {
    if summary.action == TradeAction::Sell {
        return vec![
            Line::from(format!("SELL OWNED OPTIONS | {}", summary.expiry)),
            Line::from(format!(
                "{} contracts | Minimum price {} USDC each",
                summary.qty, summary.price
            )),
            Line::from(format!(
                "Account {} | No new short exposure or collateral requirement",
                summary.account
            )),
            Line::from("Historical cost and realized P&L are not inferred from this sale."),
        ];
    }
    let strategy = match summary.kind {
        OptionKind::Call => "call spread",
        OptionKind::Put => "put spread",
    };
    let entry_label = match summary.action {
        TradeAction::Buy => "Maximum debit if filled",
        TradeAction::Sell => "Premium credit if filled",
    };
    let payout_label = match summary.action {
        TradeAction::Buy => "Gross payout",
        TradeAction::Sell => "Owned options released; no new short",
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!("{} ", summary.action.label().to_ascii_uppercase()),
                trade_action_style(cli, summary.action).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} {strategy}", summary.symbol),
                style(cli, option_kind_color(summary.kind)).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                "Range {}-{}  |  Expiry {}  |  Account {}",
                summary.lower_strike, summary.upper_strike, summary.expiry, summary.account
            ),
            style(cli, Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                format!("{} / contract ", summary.action.price_label()),
                style(cli, Color::DarkGray),
            ),
            Span::styled(
                format_usd(summary.price),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  |  Contracts ", style(cli, Color::DarkGray)),
            Span::styled(
                summary.qty.to_string(),
                style(cli, Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("{entry_label} "), style(cli, Color::DarkGray)),
            Span::styled(
                format_usd(summary.entry_total),
                trade_action_style(cli, summary.action).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Maximum loss ", style(cli, Color::DarkGray)),
            Span::styled(
                format_usd(summary.total_max_loss),
                style(cli, Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  |  Maximum gain ", style(cli, Color::DarkGray)),
            Span::styled(
                format_usd(summary.total_max_gain),
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("{payout_label} "), style(cli, Color::DarkGray)),
            Span::styled(
                format_usd(summary.total_max_payout),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  |  Liquidation ", style(cli, Color::DarkGray)),
            Span::styled(
                "none",
                style(cli, Color::Green).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    lines.extend(trade_expiry_probability_lines(cli, summary));
    lines.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Nothing is submitted until you confirm this order.",
            style(cli, Color::White).add_modifier(Modifier::BOLD),
        )),
    ]);
    lines
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) struct TradeExpiryProbabilityPercentages {
    pub(in super::super) itm: u8,
    pub(in super::super) otm: u8,
    pub(in super::super) partial: Option<u8>,
    pub(in super::super) max_payout: Option<u8>,
}

pub(in super::super) fn trade_expiry_probability_percentages(
    probability_itm: Option<f64>,
    probability_cap_hit: Option<f64>,
) -> Option<TradeExpiryProbabilityPercentages> {
    let itm_probability = probability_itm.and_then(normalize_probability)?;
    let itm = ((itm_probability * 100.0).round() as u8).min(100);
    let otm = 100u8.saturating_sub(itm);
    let max_payout = probability_cap_hit
        .and_then(normalize_probability)
        .map(|probability| ((probability * 100.0).round() as u8).min(itm));
    let partial = max_payout.map(|max_payout| itm.saturating_sub(max_payout));
    Some(TradeExpiryProbabilityPercentages {
        itm,
        otm,
        partial,
        max_payout,
    })
}

pub(in super::super) fn trade_expiry_probability_lines(
    cli: &Cli,
    summary: &TradeConfirmationSummary,
) -> Vec<Line<'static>> {
    let Some(probabilities) =
        trade_expiry_probability_percentages(summary.probability_itm, summary.probability_cap_hit)
    else {
        return vec![
            Line::from(vec![
                Span::styled("Estimated expiry  ", style(cli, Color::DarkGray)),
                Span::styled("Contract ITM n/a", style(cli, Color::DarkGray)),
                Span::styled(" | ", style(cli, Color::DarkGray)),
                Span::styled("Contract OTM n/a", style(cli, Color::DarkGray)),
            ]),
            Line::from(Span::styled(
                "Model probability unavailable for this contract.",
                style(cli, Color::DarkGray),
            )),
        ];
    };

    let mut lines = vec![Line::from(vec![
        Span::styled(
            "Estimated expiry  Contract ITM ",
            style(cli, Color::DarkGray),
        ),
        Span::styled(
            format!("{}%", probabilities.itm),
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | Contract OTM ", style(cli, Color::DarkGray)),
        Span::styled(
            format!("{}%", probabilities.otm),
            style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
        ),
    ])];
    if let (Some(partial), Some(max_payout)) = (probabilities.partial, probabilities.max_payout) {
        lines.push(Line::from(vec![
            Span::styled("Outcome split  OTM ", style(cli, Color::DarkGray)),
            Span::styled(
                format!("{}%", probabilities.otm),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | Partial payout ", style(cli, Color::DarkGray)),
            Span::styled(
                format!("{partial}%"),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" | Max payout ", style(cli, Color::DarkGray)),
            Span::styled(
                format!("{max_payout}%"),
                style(cli, Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "Outcome split  Partial/max payout detail unavailable.",
            style(cli, Color::DarkGray),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Model estimate; gross payout before premium/fees, not profit odds.",
        style(cli, Color::DarkGray),
    )));
    lines
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct LabFrameLayout {
    pub(in super::super) header_area: Rect,
    pub(in super::super) body_area: Rect,
    pub(in super::super) activity_area: Rect,
    pub(in super::super) guide_area: Option<Rect>,
    pub(in super::super) footer_area: Rect,
}

pub(in super::super) fn lab_frame_layout(root: Rect, cli: &Cli, app: &LabApp) -> LabFrameLayout {
    let ascii_lines = terminal_brand::ascii_header_lines_for_width(root.width as usize);
    let account_line_count = wallet_header_lines(cli, app).len() as u16;
    let footer_height = root.height.min(3);
    let chart_mode = app.screen == LabScreen::Chart;
    let chain_ticket_open = app.screen == LabScreen::Chain && app.trading.ticket.is_some();
    let full_header_preferred =
        ascii_lines.len() as u16 + account_line_count + HEADER_TICKER_LINES + 2;
    let compact_header_preferred = account_line_count
        .min(HEADER_COMPACT_WALLET_LINES_MAX)
        .saturating_add(HEADER_TICKER_LINES)
        .saturating_add(2);
    let full_brand_width = ascii_lines
        .iter()
        .map(|line| text_width(line) as u16)
        .max()
        .unwrap_or_default();
    let brand_min_body = if usize::from(root.width) < terminal_brand::WIDE_BANNER_MIN_COLUMNS {
        HEADER_MIN_BODY_WITH_COMPACT_BRAND
    } else {
        HEADER_MIN_BODY_WITH_BRAND
    };
    let compact_brand_can_yield_space = usize::from(root.width)
        < terminal_brand::WIDE_BANNER_MIN_COLUMNS
        && (chain_ticket_open || matches!(app.screen, LabScreen::Help | LabScreen::OracleHelp));
    let full_brand_fits = root.width.saturating_sub(2) >= full_brand_width
        && !compact_brand_can_yield_space
        && root.height
            >= full_header_preferred
                .saturating_add(footer_height)
                .saturating_add(brand_min_body);
    let header_preferred = if full_brand_fits {
        full_header_preferred
    } else {
        compact_header_preferred
    };
    let min_body_height = if app.screen == LabScreen::Help && root.height >= 24 {
        15
    } else if root.height >= 24 {
        8
    } else {
        5
    };
    let header_height = header_preferred
        .min(root.height.saturating_sub(footer_height + min_body_height))
        .max(3)
        .min(root.height.saturating_sub(footer_height));
    let preferred_activity_height = if chart_mode
        || chain_ticket_open
        || matches!(
            app.screen,
            LabScreen::Terms
                | LabScreen::Home
                | LabScreen::Staking
                | LabScreen::Help
                | LabScreen::OracleIntro
                | LabScreen::Oracle
                | LabScreen::OracleHelp
                | LabScreen::Ledger
        ) {
        0
    } else if root.height >= 32 {
        9
    } else if root.height >= 24 {
        7
    } else {
        5
    };
    let remaining = root.height.saturating_sub(header_height + footer_height);
    let max_activity_height = remaining.saturating_sub(min_body_height);
    let activity_height = if chart_mode || preferred_activity_height == 0 {
        0
    } else {
        preferred_activity_height
            .min(max_activity_height)
            .max(if max_activity_height > 0 { 3 } else { 0 })
    };
    let mut body_height = remaining.saturating_sub(activity_height);
    let mut content_width = root.width;
    let mut guide_area = None;
    let guide_right_width = (root.width / 4).clamp(34, 42);
    let guide_expanded = guide_panel_requested(app);
    let guide_on_right = app.screen != LabScreen::Terms
        && root.width >= 146
        && root.width.saturating_sub(guide_right_width) >= 104
        && remaining >= 8;
    if guide_on_right {
        content_width = root.width.saturating_sub(guide_right_width);
        guide_area = Some(Rect {
            x: root.x.saturating_add(content_width),
            y: root.y.saturating_add(header_height),
            width: guide_right_width,
            height: remaining,
        });
    } else if guide_expanded && root.width >= 12 && remaining >= 4 {
        let preferred_guide_height = if app.screen == LabScreen::Terms && remaining >= 12 {
            7
        } else if remaining >= 24 {
            10
        } else if remaining >= 16 {
            8
        } else if remaining >= 10 {
            5
        } else {
            2
        };
        let guide_height = preferred_guide_height.min(body_height.saturating_sub(3));
        if guide_height > 0 {
            body_height = body_height.saturating_sub(guide_height);
            guide_area = Some(Rect {
                x: root.x,
                y: root
                    .y
                    .saturating_add(header_height)
                    .saturating_add(body_height)
                    .saturating_add(activity_height),
                width: root.width,
                height: guide_height,
            });
        }
    }
    let header_area = Rect {
        x: root.x,
        y: root.y,
        width: root.width,
        height: header_height,
    };
    let body_area = Rect {
        x: root.x,
        y: root.y + header_height,
        width: content_width,
        height: body_height,
    };
    let activity_area = Rect {
        x: root.x,
        y: root.y + header_height + body_height,
        width: content_width,
        height: activity_height,
    };
    let footer_area = Rect {
        x: root.x,
        y: root
            .y
            .saturating_add(root.height.saturating_sub(footer_height)),
        width: root.width,
        height: footer_height,
    };

    LabFrameLayout {
        header_area,
        body_area,
        activity_area,
        guide_area,
        footer_area,
    }
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct LabBodyLayout {
    pub(in super::super) market_area: Option<Rect>,
    pub(in super::super) selected_area: Rect,
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct LabPageLayout {
    pub(in super::super) market_area: Option<Rect>,
    pub(in super::super) selected_area: Rect,
    pub(in super::super) activity_area: Rect,
}

pub(in super::super) fn lab_page_layout(
    screen: LabScreen,
    frame_layout: LabFrameLayout,
) -> LabPageLayout {
    let content_area = if frame_layout.activity_area.height > 0 {
        Rect {
            height: frame_layout
                .body_area
                .height
                .saturating_add(frame_layout.activity_area.height),
            ..frame_layout.body_area
        }
    } else {
        frame_layout.body_area
    };
    let content_layout = lab_body_layout(screen, content_area);
    let selected_height = if frame_layout.activity_area.height > 0 {
        frame_layout.body_area.height
    } else {
        content_layout.selected_area.height
    };
    let selected_area = Rect {
        height: selected_height.min(content_layout.selected_area.height),
        ..content_layout.selected_area
    };
    let activity_area = if frame_layout.activity_area.height > 0 {
        Rect {
            x: content_layout.selected_area.x,
            y: frame_layout.activity_area.y,
            width: content_layout.selected_area.width,
            height: frame_layout.activity_area.height,
        }
    } else {
        frame_layout.activity_area
    };

    LabPageLayout {
        market_area: content_layout.market_area,
        selected_area,
        activity_area,
    }
}

pub(in super::super) fn lab_body_layout(screen: LabScreen, body_area: Rect) -> LabBodyLayout {
    if matches!(
        screen,
        LabScreen::Terms | LabScreen::Staking | LabScreen::Help | LabScreen::Ledger
    ) {
        return LabBodyLayout {
            market_area: None,
            selected_area: body_area,
        };
    }

    let chart_mode = screen == LabScreen::Chart;
    let body = if chart_mode {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
            .split(body_area)
    } else if matches!(
        screen,
        LabScreen::OracleIntro | LabScreen::Oracle | LabScreen::OracleHelp
    ) && body_area.width >= 100
        && body_area.height >= 12
    {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(50)])
            .split(body_area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
            .split(body_area)
    };

    LabBodyLayout {
        market_area: Some(body[0]),
        selected_area: body[1],
    }
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct HeaderContentPlan {
    pub(in super::super) ticker_line_count: usize,
    pub(in super::super) ascii_line_count: usize,
    pub(in super::super) wallet_line_count: usize,
}

pub(in super::super) fn header_content_plan(
    inner: Rect,
    wallet_line_count: usize,
    terminal_width: usize,
) -> HeaderContentPlan {
    let ticker_line_count = usize::from(inner.width > 0 && inner.height > 0);
    let remaining_after_ticker = (inner.height as usize).saturating_sub(ticker_line_count);
    let wallet_line_count = wallet_line_count.min(remaining_after_ticker);
    let remaining_for_brand = remaining_after_ticker.saturating_sub(wallet_line_count);
    let ascii_lines = terminal_brand::ascii_header_lines_for_width(terminal_width);
    let brand_fits_width = ascii_lines
        .iter()
        .all(|line| text_width(line) <= inner.width as usize);
    let ascii_line_count = if brand_fits_width && remaining_for_brand >= ascii_lines.len() {
        ascii_lines.len()
    } else {
        0
    };

    HeaderContentPlan {
        ticker_line_count,
        ascii_line_count,
        wallet_line_count,
    }
}

pub(in super::super) fn draw_header(frame: &mut Frame<'_>, cli: &Cli, area: Rect, app: &LabApp) {
    let wallet_lines = wallet_header_lines(cli, app);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(panel_border_style(cli, false))
        .style(tui_surface_style(cli, TUI_BACKGROUND));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let plan = header_content_plan(inner, wallet_lines.len(), area.width as usize);
    let brand_color_enabled = terminal_brand::color_enabled(cli.no_color, true);
    let mut header_lines =
        Vec::with_capacity(plan.ticker_line_count + plan.ascii_line_count + plan.wallet_line_count);
    header_lines.extend(
        terminal_brand::terminal_banner_lines_for_width(area.width as usize)
            .into_iter()
            .take(plan.ascii_line_count)
            .map(|line| {
                Line::from(
                    line.into_iter()
                        .map(|span| tui_brand_span(span, brand_color_enabled))
                        .collect::<Vec<_>>(),
                )
            }),
    );
    if plan.ticker_line_count > 0 {
        header_lines.push(ticker_tape_line(cli, app, inner.width));
    }
    header_lines.extend(wallet_lines.into_iter().take(plan.wallet_line_count));
    frame.render_widget(Paragraph::new(header_lines), inner);
}

pub(in super::super) fn tui_brand_span(
    span: terminal_brand::BrandSpan,
    color_enabled: bool,
) -> Span<'static> {
    let span_style = tui_brand_span_style(&span, color_enabled);
    let text = if color_enabled && span.fill {
        " ".repeat(span.text.chars().count())
    } else {
        span.text
    };
    Span::styled(text, span_style)
}

pub(in super::super) fn tui_brand_span_style(
    span: &terminal_brand::BrandSpan,
    color_enabled: bool,
) -> Style {
    let color = match span.tone {
        terminal_brand::BrandTone::Title => Color::Rgb(78, 165, 202),
        terminal_brand::BrandTone::Outline => Color::Rgb(229, 229, 229),
        terminal_brand::BrandTone::Body => Color::Rgb(231, 231, 231),
        terminal_brand::BrandTone::Plain => Color::Reset,
    };
    if color_enabled && span.tone != terminal_brand::BrandTone::Plain {
        let style = Style::default().fg(color);
        if span.fill { style.bg(color) } else { style }
    } else {
        Style::default()
    }
}

pub(in super::super) fn ticker_tape_line(cli: &Cli, app: &LabApp, width: u16) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::from("");
    }

    let prefix = fit_text_to_width(" TAPE ", width);
    let gutter = fit_text_to_width(
        HEADER_TICKER_GUTTER,
        width.saturating_sub(text_width(&prefix)),
    );
    let tape_width = width
        .saturating_sub(text_width(&prefix))
        .saturating_sub(text_width(&gutter));
    let tape = ticker_tape_window(
        &ticker_tape_text(app),
        tape_width,
        app.spinner_tick / TICKER_SCROLL_TICKS_PER_COLUMN,
    );

    Line::from(vec![
        Span::styled(
            prefix,
            terminal_button_key_style(cli, Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw(gutter),
        Span::styled(tape, style(cli, Color::Yellow)),
    ])
    .style(tui_surface_style(cli, TUI_TICKER_BACKGROUND))
}

pub(in super::super) fn ticker_tape_text(app: &LabApp) -> String {
    ticker_tape_items(app).join(TICKER_SEPARATOR)
}

pub(in super::super) fn ticker_tape_items(app: &LabApp) -> Vec<String> {
    let mut items = Vec::new();

    if let Some(detail) = &app.trading.detail {
        let symbol = ticker_symbol(&detail.symbol, &detail.id);
        if is_known_value(&detail.current_print) {
            items.push(format!("{symbol} print {}", detail.current_print));
        } else {
            items.push(format!("{symbol} live print unavailable"));
            if is_known_value(&detail.base) {
                items.push(format!("{symbol} reference level {}", detail.base));
            }
        }

        if is_known_value(&detail.expiry_label) {
            let mut month = format!("{symbol} {}", detail.expiry_label);
            let settle_label = time_to_settle_label(&detail.days);
            if is_known_value(&settle_label) {
                month.push_str(&format!(" settles in {settle_label}"));
            }
            items.push(month);
        }

        if is_known_value(&detail.cap_width) {
            items.push(format!("contract cap width {}", detail.cap_width));
        } else {
            items.push("max loss previewed before trade".to_string());
        }
        items.push("no liquidation".to_string());

        if detail.rows != "0" && is_known_value(&detail.rows) {
            items.push(format!("{} contracts listed", detail.rows));
        }
        if !detail.issues.is_empty() {
            items.push("live contract data unavailable".to_string());
        }

        return items;
    }

    if app.trading.loading_list {
        items.push("loading live markets".to_string());
    }
    for dish in app.trading.dishes.iter().take(4) {
        let symbol = ticker_symbol(&dish.symbol, &dish.id);
        if is_known_value(&dish.expiry_count) {
            items.push(format!("{symbol} {} monthly contracts", dish.expiry_count));
        } else {
            items.push(format!("{symbol} monthly markets"));
        }
    }
    items.push("max loss previewed before trade".to_string());
    items.push("no liquidation".to_string());
    items.push("transparent settlement index".to_string());
    items
}

pub(in super::super) fn ticker_symbol(symbol: &str, id: &str) -> String {
    match symbol.trim() {
        "" => id.to_ascii_uppercase(),
        symbol => symbol.to_string(),
    }
}

pub(in super::super) fn ticker_tape_window(text: &str, width: usize, offset: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let tape = text.trim();
    let source = if tape.is_empty() {
        "max loss previewed before trade"
    } else {
        tape
    };
    let cycle = format!("{source}{TICKER_SEPARATOR}");
    let chars = cycle.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return " ".repeat(width);
    }

    let start = offset % chars.len();
    (0..width)
        .map(|index| chars[(start + index) % chars.len()])
        .collect()
}

pub(in super::super) fn fit_text_to_width(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

pub(in super::super) fn text_width(text: &str) -> usize {
    text.chars().count()
}

pub(in super::super) fn wallet_header_lines(cli: &Cli, app: &LabApp) -> Vec<Line<'static>> {
    let wallet = &app.wallet;
    let account = wallet
        .pubkey
        .as_deref()
        .map(short_pubkey)
        .unwrap_or_else(|| "not attached".to_string());
    let status_color = if wallet.is_attached() {
        Color::Green
    } else {
        Color::Yellow
    };
    let mut lines = vec![Line::from(vec![
        Span::styled("Account: ", style(cli, Color::DarkGray)),
        Span::styled(
            account,
            style(cli, status_color).add_modifier(Modifier::BOLD),
        ),
        divider_span(cli),
        Span::styled(
            format!("signer: {}", wallet.path_source.label()),
            style(cli, Color::Gray),
        ),
    ])];
    if header_nav_visible(app) {
        lines.push(header_nav_line(cli, app));
    }
    if let Some(line) = wallet_balance_line(cli, app) {
        lines.push(line);
    }
    if let Some(line) = update_header_line(cli, app) {
        lines.push(line);
    }
    lines
}

pub(in super::super) fn update_header_line(cli: &Cli, app: &LabApp) -> Option<Line<'static>> {
    if app.updates.is_loading() {
        return Some(Line::from(vec![
            Span::styled("Update: ", style(cli, Color::DarkGray)),
            Span::styled("checking", style(cli, Color::Yellow)),
        ]));
    }

    let Some(report) = app.updates.report() else {
        return None;
    };
    if report.action == "release_update_check" && report.blocked {
        return Some(Line::from(vec![
            Span::styled("Update: ", style(cli, Color::DarkGray)),
            Span::styled(
                report
                    .blocked_reason
                    .clone()
                    .unwrap_or_else(|| "unavailable".into()),
                style(cli, Color::Yellow),
            ),
        ]));
    }
    if report.blocked && (report.update_available || report.rebuild_required) {
        let blocked_hint = if report.dirty {
            " | commit or stash local changes"
        } else {
            " | merge or rebase the local branch"
        };
        return Some(Line::from(vec![
            Span::styled("Update: ", style(cli, Color::DarkGray)),
            Span::styled("available, blocked", style(cli, Color::Yellow)),
            Span::styled(blocked_hint, style(cli, Color::Gray)),
        ]));
    }
    let update_key_color = if app.guide.focused_control.as_deref() == Some("control:update") {
        Color::LightYellow
    } else {
        Color::Green
    };
    if report.update_available {
        return Some(Line::from(vec![
            Span::styled("Update: ", style(cli, Color::DarkGray)),
            Span::styled("available", style(cli, Color::Green)),
            Span::styled(" | press ", style(cli, Color::Gray)),
            help_key_with_color(cli, "U", update_key_color),
            Span::styled(" to exit and run petri update", style(cli, Color::Gray)),
        ]));
    }
    if report.rebuild_required {
        return Some(Line::from(vec![
            Span::styled("Update: ", style(cli, Color::DarkGray)),
            Span::styled("rebuild available", style(cli, Color::Yellow)),
            Span::styled(" | press ", style(cli, Color::Gray)),
            help_key_with_color(cli, "U", update_key_color),
            Span::styled(" to exit and rebuild", style(cli, Color::Gray)),
        ]));
    }
    None
}

pub(in super::super) fn header_update_rect(cli: &Cli, area: Rect, app: &LabApp) -> Option<Rect> {
    let report = app.updates.report()?;
    if app.updates.is_loading()
        || report.blocked
        || (!report.update_available && !report.rebuild_required)
    {
        return None;
    }
    let wallet_lines = wallet_header_lines(cli, app);
    update_header_line(cli, app)?;
    let wallet_line = wallet_lines.len().checked_sub(1)?;
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let plan = header_content_plan(inner, wallet_lines.len(), area.width as usize);
    if wallet_line >= plan.wallet_line_count {
        return None;
    }
    let preceding = plan
        .ascii_line_count
        .saturating_add(plan.ticker_line_count)
        .saturating_add(wallet_line);
    Some(Rect {
        x: inner.x,
        y: inner
            .y
            .saturating_add(preceding.try_into().unwrap_or(u16::MAX)),
        width: inner.width,
        height: 1,
    })
}

pub(in super::super) fn header_update_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> bool {
    header_update_rect(cli, area, app).is_some_and(|rect| rect_contains(rect, column, row))
}

pub(in super::super) fn header_nav_visible(app: &LabApp) -> bool {
    app.screen != LabScreen::Terms
}

pub(in super::super) fn header_nav_line(cli: &Cli, app: &LabApp) -> Line<'static> {
    let back_style = terminal_button_surface_style(cli, TUI_NAV_BUTTON_BROWN, app.can_go_back());
    let home_style = terminal_button_surface_style(cli, TUI_NAV_BUTTON_BROWN, true);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(HEADER_BACK_BUTTON_LABEL, back_style),
        Span::raw(" "),
        Span::styled(HEADER_HOME_BUTTON_LABEL, home_style),
    ])
}

#[derive(Clone, Copy, Debug)]
pub(in super::super) struct HeaderNavRects {
    pub(in super::super) back: Rect,
    pub(in super::super) home: Rect,
}

pub(in super::super) fn header_nav_button_rects(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
) -> Option<HeaderNavRects> {
    if !header_nav_visible(app) {
        return None;
    }
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    let wallet_lines = wallet_header_lines(cli, app);
    let plan = header_content_plan(inner, wallet_lines.len(), area.width as usize);
    let nav_wallet_line = 1usize;
    if plan.wallet_line_count <= nav_wallet_line {
        return None;
    }
    let y = inner.y.saturating_add(
        (plan.ticker_line_count + plan.ascii_line_count + nav_wallet_line)
            .try_into()
            .unwrap_or(u16::MAX),
    );

    let back_width = text_width(HEADER_BACK_BUTTON_LABEL) as u16;
    let home_width = text_width(HEADER_HOME_BUTTON_LABEL) as u16;
    let gap = 1;
    let required_width = 1u16
        .saturating_add(back_width)
        .saturating_add(gap)
        .saturating_add(home_width);
    if inner.width < required_width {
        return None;
    }

    let back_x = inner.x.saturating_add(1);
    let home_x = back_x.saturating_add(back_width).saturating_add(gap);
    Some(HeaderNavRects {
        back: Rect {
            x: back_x,
            y,
            width: back_width,
            height: 1,
        },
        home: Rect {
            x: home_x,
            y,
            width: home_width,
            height: 1,
        },
    })
}

pub(in super::super) fn header_nav_hit_at(
    cli: &Cli,
    area: Rect,
    app: &LabApp,
    column: u16,
    row: u16,
) -> Option<HeaderNavAction> {
    let rects = header_nav_button_rects(cli, area, app)?;
    if rect_contains(rects.back, column, row) {
        Some(HeaderNavAction::Back)
    } else if rect_contains(rects.home, column, row) {
        Some(HeaderNavAction::Home)
    } else {
        None
    }
}

pub(in super::super) fn panel_block(
    cli: &Cli,
    title: &str,
    color: Color,
    focused: bool,
) -> Block<'static> {
    let title_text = if focused {
        format!("{title} *")
    } else {
        title.to_string()
    };
    let title_style = if focused {
        style(cli, Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        style(cli, color)
    };
    let border_style = panel_border_style(cli, focused);
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(tui_panel_style(cli))
        .title(Span::styled(title_text, title_style))
}

pub(in super::super) fn fill_tui_area(frame: &mut Frame<'_>, cli: &Cli, area: Rect, color: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("").style(tui_surface_style(cli, color)),
        area,
    );
}

pub(in super::super) fn tui_surface_style(cli: &Cli, color: Color) -> Style {
    if cli.no_color {
        Style::default()
    } else {
        Style::default().fg(Color::White).bg(color)
    }
}

pub(in super::super) fn tui_panel_style(cli: &Cli) -> Style {
    tui_surface_style(cli, TUI_PANEL_BACKGROUND)
}

pub(in super::super) fn tui_alt_panel_style(cli: &Cli) -> Style {
    tui_surface_style(cli, TUI_PANEL_BACKGROUND_ALT)
}

pub(in super::super) fn panel_border_style(cli: &Cli, focused: bool) -> Style {
    if focused {
        style(cli, TUI_BORDER_BLUE).add_modifier(Modifier::BOLD)
    } else {
        style(cli, TUI_BORDER_MUTED)
    }
}

pub(in super::super) fn content_sized_panel_area(area: Rect, line_count: usize) -> Rect {
    Rect {
        height: line_count_panel_height(line_count).min(area.height),
        ..area
    }
}

pub(in super::super) fn wrapped_content_sized_panel_area(
    area: Rect,
    lines: &[Line<'static>],
    trim: bool,
) -> Rect {
    let wrapped_rows = wrapped_line_count(lines, area.width.saturating_sub(2), trim);
    content_sized_panel_area(area, wrapped_rows)
}

pub(in super::super) fn double_spaced_lines(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let mut spaced = Vec::with_capacity(lines.len().saturating_mul(2).saturating_sub(1));
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            spaced.push(Line::from(""));
        }
        spaced.push(line);
    }
    spaced
}
