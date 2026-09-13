//! Screen lifecycle, focus movement, ordered keyboard/mouse dispatch, and hit activation.

use super::*;

impl LabApp {
    pub(super) fn can_go_back(&self) -> bool {
        !self.screen_history.is_empty()
    }

    pub(super) fn can_quit_lab(&mut self) -> bool {
        if self.trading.submit_is_running() {
            self.status =
                "Order submission is still running. Wait for the result before quitting Petri."
                    .to_string();
            false
        } else if self.staking_action_is_running() {
            self.status =
                "A staking transaction is still running. Wait for its result before quitting Petri."
                    .to_string();
            false
        } else if self.writers.action_is_running() {
            self.status =
                "A writer transaction is still running. Wait for its result before quitting Petri."
                    .to_string();
            false
        } else {
            true
        }
    }

    pub(super) fn go_back(&mut self) {
        while let Some(screen) = self.screen_history.pop() {
            if screen == self.screen {
                continue;
            }
            self.set_screen_from_history(screen);
            self.status = format!("Back to {}.", screen_title(screen));
            return;
        }
        self.status = "No previous page yet.".to_string();
    }

    pub(super) fn set_screen(&mut self, screen: LabScreen) {
        self.set_screen_internal(screen, true);
    }

    pub(super) fn set_screen_from_history(&mut self, screen: LabScreen) {
        self.set_screen_internal(screen, false);
    }

    pub(super) fn set_screen_internal(&mut self, screen: LabScreen, record_history: bool) {
        if self.screen != screen {
            self.clear_guide_context_actions();
        }
        if record_history && self.screen != screen && self.screen != LabScreen::Terms {
            if self.screen_history.last().copied() != Some(self.screen) {
                self.screen_history.push(self.screen);
            }
            if self.screen_history.len() > SCREEN_HISTORY_LIMIT {
                self.screen_history.remove(0);
            }
        }
        self.screen = screen;
        if screen != LabScreen::Help {
            self.help.preview = None;
            self.help.glossary_hover = None;
            self.help.hover_grace_ticks = None;
        }
        if screen != LabScreen::Chain && !self.trading.submit_is_running() {
            self.trading.ticket = None;
        }
        if screen != LabScreen::Staking && !self.staking_action_is_running() {
            self.staking_form = None;
            self.staking_confirmation = None;
        }
        if screen != LabScreen::Ledger && !self.writers.action_is_running() {
            self.writers.clear_action_mask_check();
            self.writers.form = None;
            self.writers.confirmation = None;
            if !self.liquidity_preview_is_running() {
                self.liquidity_preview_form = None;
            }
            self.wallet_switch_editing = false;
        }
        self.panel_scrolls.clear();
        self.set_focus(default_focus_for_screen(screen));
    }

    pub(super) fn set_focus(&mut self, focus: LabFocus) {
        match focus {
            LabFocus::Markets => self.focus_markets(),
            LabFocus::MarketSeries => self.focus_market_series(),
            LabFocus::Calls => self.focus_option_side(OptionKind::Call),
            LabFocus::Puts => self.focus_option_side(OptionKind::Put),
            focus => self.focus = focus,
        }
    }

    pub(super) fn focused_panel_scroll(&self, focus: LabFocus) -> usize {
        self.panel_scrolls.get(&focus).copied().unwrap_or(0)
    }

    pub(super) fn reset_panel_scroll(&mut self, focus: LabFocus) {
        self.panel_scrolls.remove(&focus);
    }

    pub(super) fn scroll_focused_panel(&mut self, direction: isize) {
        if direction == 0 {
            return;
        }
        if self.trading.submit_is_running() && self.screen == LabScreen::Chain {
            self.status = "Order submission is running. Contract controls are temporarily locked."
                .to_string();
            return;
        }
        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Overview {
            match self.help.pane {
                HelpPane::Navigation => {
                    self.select_help_nav_offset(direction * PANEL_SCROLL_STEP as isize)
                }
                HelpPane::Article => self.help.scroll_article(direction, PANEL_SCROLL_STEP),
            }
            self.status = "Scroll through GitBook topics or the selected page.".to_string();
            return;
        }
        if self.screen == LabScreen::Oracle
            && self.focus == LabFocus::OracleActions
            && self.oracle.form.is_some()
        {
            self.move_oracle_form_field(direction * PANEL_SCROLL_STEP as isize);
            return;
        }
        if self.focus == LabFocus::Guide {
            self.scroll_guide(direction);
            self.status = format!("{PANEL_SCROLL_HELP} scrolls the Guide conversation.");
            return;
        }

        if self.screen == LabScreen::Home && self.focus == LabFocus::HomeActions {
            for _ in 0..PANEL_SCROLL_STEP {
                let moved = if direction < 0 {
                    self.select_prev_home_action()
                } else {
                    self.select_next_home_action()
                };
                if !moved {
                    break;
                }
            }
            self.status = format!("{PANEL_SCROLL_HELP} moves through Home actions.");
            return;
        }

        if self.screen == LabScreen::Ledger
            && self.focus == LabFocus::Ledger
            && matches!(self.ledger_pane, LedgerPane::List | LedgerPane::Actions)
        {
            self.move_ledger_selection(direction * PANEL_SCROLL_STEP as isize);
            self.status = format!("{PANEL_SCROLL_HELP} moves through the focused Ledger pane.");
            return;
        }

        if matches!(self.focus, LabFocus::Calls | LabFocus::Puts) {
            let target = if self.focus == LabFocus::Calls {
                OptionKind::Call
            } else {
                OptionKind::Put
            };
            self.select_option_side(target);
            for _ in 0..PANEL_SCROLL_STEP {
                let moved = if direction < 0 {
                    self.select_prev_option()
                } else {
                    self.select_next_option()
                };
                if !moved {
                    break;
                }
            }
            self.status = format!("{PANEL_SCROLL_HELP} moves through visible quote rows.");
            return;
        }

        let focus = self.focus;
        let entry = self.panel_scrolls.entry(focus).or_insert(0);
        if direction < 0 {
            *entry = entry.saturating_sub(PANEL_SCROLL_STEP);
        } else {
            *entry = entry.saturating_add(PANEL_SCROLL_STEP);
        }
        self.status = format!("{PANEL_SCROLL_HELP} scrolls the focused panel.");
    }

    pub(super) fn handle_mouse_event(
        &mut self,
        mouse: MouseEvent,
        cli: &Cli,
        root: Rect,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let activate_left_click = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.left_mouse_down = true;
                true
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let had_matching_down = self.left_mouse_down;
                self.left_mouse_down = false;
                !had_matching_down
            }
            _ => false,
        };
        if self.startup_intro_is_open() {
            if activate_left_click
                && rect_contains(
                    startup_intro_open_button_rect(root),
                    mouse.column,
                    mouse.row,
                )
            {
                self.dismiss_startup_intro();
            }
            return;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self.screen == LabScreen::Help
            && self.home_help_topic == HomeHelpTopic::Overview
            && self
                .help
                .preview
                .as_ref()
                .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Hover)
        {
            let frame_layout = lab_frame_layout(root, cli, self);
            let body_layout = lab_page_layout(self.screen, frame_layout);
            let help_layout = gitbook_help_layout(body_layout.selected_area);
            let keeps_hover_preview =
                gitbook_help_preview_geometry(body_layout.selected_area, self)
                    .is_some_and(|geometry| rect_contains(geometry.popup, mouse.column, mouse.row))
                    || gitbook_nav_hit_at(
                        cli,
                        help_layout.navigation_area,
                        self,
                        mouse.column,
                        mouse.row,
                    )
                    .is_some();
            if !keeps_hover_preview {
                self.help.clear_hover_preview();
            }
        }
        if self.trading.result_modal_is_open() {
            let modal_root = lab_modal_root(root, cli, self);
            if activate_left_click
                && trade_result_close_rect(modal_root)
                    .is_some_and(|area| rect_contains(area, mouse.column, mouse.row))
            {
                self.trading.dismiss_result_modal();
            }
            return;
        }
        if self.trading.confirmation_is_open() {
            let modal_root = lab_modal_root(root, cli, self);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let choice = trade_confirmation_button_at(modal_root, mouse.column, mouse.row);
                    self.confirmation_mouse_press = choice.map(ConfirmationMousePress::Trade);
                    if let Some(choice) = choice {
                        if let Some(confirmation) = self
                            .trading
                            .ticket
                            .as_mut()
                            .and_then(|ticket| ticket.confirmation.as_mut())
                        {
                            confirmation.choice = choice;
                        }
                        self.status =
                            "Release over the same order button to activate it.".to_string();
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let choice = trade_confirmation_button_at(modal_root, mouse.column, mouse.row);
                    let matches_press = choice.is_some_and(|choice| {
                        self.confirmation_mouse_press.take()
                            == Some(ConfirmationMousePress::Trade(choice))
                    });
                    if matches_press {
                        self.handle_trade_confirmation_mouse_click(
                            cli,
                            modal_root,
                            mouse.column,
                            mouse.row,
                            fetch_tx,
                        );
                    } else {
                        self.confirmation_mouse_press = None;
                        self.status =
                            "Order action not activated; press and release the same button."
                                .to_string();
                    }
                }
                _ => {}
            }
            return;
        }
        if self.writers.confirmation.is_some() {
            let modal_root = lab_modal_root(root, cli, self);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let choice = writer_confirmation_button_at(modal_root, mouse.column, mouse.row);
                    self.confirmation_mouse_press = choice.map(ConfirmationMousePress::Writer);
                    if let Some(choice) = choice {
                        if let Some(confirmation) = self.writers.confirmation.as_mut() {
                            confirmation.choice = choice;
                        }
                        self.status =
                            "Release over the same writer button to activate it.".to_string();
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let choice = writer_confirmation_button_at(modal_root, mouse.column, mouse.row);
                    let matches_press = choice.is_some_and(|choice| {
                        self.confirmation_mouse_press.take()
                            == Some(ConfirmationMousePress::Writer(choice))
                    });
                    if matches_press {
                        self.handle_writer_confirmation_mouse_click(
                            cli,
                            modal_root,
                            mouse.column,
                            mouse.row,
                            backend_url,
                            fetch_tx,
                        );
                    } else {
                        self.confirmation_mouse_press = None;
                        self.status =
                            "Writer action not activated; press and release the same button."
                                .to_string();
                    }
                }
                _ => {}
            }
            return;
        }
        if self.screen == LabScreen::Staking && self.staking_confirmation.is_some() {
            let frame_layout = lab_frame_layout(root, cli, self);
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    let choice = staking_confirmation_button_at(
                        frame_layout.body_area,
                        self,
                        mouse.column,
                        mouse.row,
                    );
                    self.confirmation_mouse_press = choice.map(ConfirmationMousePress::Staking);
                    if let Some(choice) = choice {
                        if let Some(confirmation) = self.staking_confirmation.as_mut() {
                            confirmation.choice = choice;
                        }
                        self.status =
                            "Release over the same staking button to activate it.".to_string();
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let choice = staking_confirmation_button_at(
                        frame_layout.body_area,
                        self,
                        mouse.column,
                        mouse.row,
                    );
                    let matches_press = choice.is_some_and(|choice| {
                        self.confirmation_mouse_press.take()
                            == Some(ConfirmationMousePress::Staking(choice))
                    });
                    if matches_press {
                        self.handle_staking_confirmation_mouse_click(
                            frame_layout.body_area,
                            mouse.column,
                            mouse.row,
                            fetch_tx,
                        );
                    } else {
                        self.confirmation_mouse_press = None;
                        self.status =
                            "Staking action not activated; press and release the same button."
                                .to_string();
                    }
                }
                _ => {}
            }
            return;
        }
        if self.guide.composing && mouse.kind != MouseEventKind::Moved {
            let guide_hit = lab_frame_layout(root, cli, self)
                .guide_area
                .is_some_and(|area| rect_contains(area, mouse.column, mouse.row));
            if !guide_hit {
                self.cancel_guide_input();
                return;
            }
        }
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && gitbook_help_preview_geometry_for_root(cli, root, self)
            .is_some_and(|geometry| rect_contains(geometry.popup, mouse.column, mouse.row))
        {
            let direction = if mouse.kind == MouseEventKind::ScrollUp {
                -1
            } else {
                1
            };
            let max_scroll = gitbook_help_preview_max_scroll_for_root(cli, root, self).unwrap_or(0);
            self.help
                .scroll_preview(direction, PANEL_SCROLL_STEP, max_scroll);
            self.status = "Scrolling the topic preview.".to_string();
            return;
        }
        match mouse.kind {
            MouseEventKind::Moved => {
                self.update_gitbook_help_hover(cli, root, mouse.column, mouse.row);
            }
            MouseEventKind::ScrollUp => {
                self.focus_mouse_target(cli, root, mouse.column, mouse.row);
                self.scroll_focused_panel(-1);
            }
            MouseEventKind::ScrollDown => {
                self.focus_mouse_target(cli, root, mouse.column, mouse.row);
                self.scroll_focused_panel(1);
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                if activate_left_click =>
            {
                self.handle_mouse_click(cli, root, mouse.column, mouse.row, backend_url, fetch_tx);
            }
            _ => {}
        }
    }

    pub(super) fn mouse_pointer_shape_at(
        &self,
        cli: &Cli,
        root: Rect,
        column: u16,
        row: u16,
    ) -> TerminalPointerShape {
        if self.clickable_button_at(cli, root, column, row) {
            TerminalPointerShape::Pointer
        } else {
            TerminalPointerShape::Default
        }
    }

    pub(super) fn clickable_button_at(&self, cli: &Cli, root: Rect, column: u16, row: u16) -> bool {
        if self.startup_intro_is_open() {
            return rect_contains(startup_intro_open_button_rect(root), column, row);
        }
        let modal_root = lab_modal_root(root, cli, self);
        if self.trading.result_modal_is_open() {
            return trade_result_close_rect(modal_root)
                .is_some_and(|area| rect_contains(area, column, row));
        }
        if self.trading.confirmation_is_open() {
            return trade_confirmation_button_at(modal_root, column, row).is_some();
        }
        if self.writers.confirmation.is_some() {
            return writer_confirmation_button_at(modal_root, column, row).is_some();
        }
        if self.screen == LabScreen::Staking && self.staking_confirmation.is_some() {
            let frame_layout = lab_frame_layout(root, cli, self);
            return staking_confirmation_button_at(frame_layout.body_area, self, column, row)
                .is_some();
        }
        let frame_layout = lab_frame_layout(root, cli, self);
        let body_layout = lab_page_layout(self.screen, frame_layout);

        if header_update_hit_at(cli, frame_layout.header_area, self, column, row) {
            return true;
        }

        if external_link_hit_at(cli, root, self, column, row).is_some() {
            return true;
        }

        if self.screen != LabScreen::Terms
            && frame_layout
                .guide_area
                .is_some_and(|area| rect_contains(area, column, row))
        {
            return true;
        }

        if let Some(action) = header_nav_hit_at(cli, frame_layout.header_area, self, column, row) {
            return action == HeaderNavAction::Home || self.can_go_back();
        }

        if self.screen == LabScreen::Terms {
            return !self.wallet_switch_editing
                && terms_action_hit_at(
                    cli,
                    frame_layout.body_area,
                    self,
                    self.focused_panel_scroll(LabFocus::Terms),
                    column,
                    row,
                )
                .is_some();
        }

        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Overview {
            let layout = gitbook_help_layout(body_layout.selected_area);
            if gitbook_help_preview_geometry(body_layout.selected_area, self).is_some_and(
                |geometry| {
                    rect_contains(geometry.popup, column, row)
                        || gitbook_help_preview_close_rect(geometry.popup)
                            .is_some_and(|area| rect_contains(area, column, row))
                },
            ) {
                return true;
            }
            return gitbook_nav_hit_at(cli, layout.navigation_area, self, column, row).is_some();
        }

        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Agents {
            if self.mcp_connection_button_active()
                && agent_connection_button_rect(body_layout.selected_area)
                    .is_some_and(|area| rect_contains(area, column, row))
            {
                return true;
            }
            return false;
        }

        if self.screen == LabScreen::Home {
            return home_action_hit_at(cli, body_layout.selected_area, self, column, row).is_some();
        }

        if self.screen == LabScreen::Chart {
            if let Some(hit) = chart_control_hit_at(body_layout.selected_area, column, row) {
                return !matches!(hit, ChartControlHit::Refresh) || !self.trading.loading_chart;
            }
            return chart_activity_area(body_layout.selected_area, self)
                .is_some_and(|activity_area| rect_contains(activity_area, column, row));
        }

        if self.screen == LabScreen::Detail {
            return detail_tab_hit_at(body_layout.selected_area, column, row).is_some();
        }

        if self.screen == LabScreen::Ledger
            && self.writers.form.is_none()
            && self.liquidity_preview_form.is_none()
            && !self.wallet_switch_editing
        {
            return ledger_tab_hit_at(body_layout.selected_area, column, row).is_some()
                || ledger_list_row_hit_at(cli, body_layout.selected_area, self, column, row)
                    .is_some()
                || ledger_writer_action_hit_at(body_layout.selected_area, self, column, row)
                    .is_some()
                || ledger_liquidity_action_hit_at(body_layout.selected_area, self, column, row);
        }

        if self.screen == LabScreen::OracleIntro {
            return oracle_intro_action_hit_at(cli, body_layout.selected_area, self, column, row)
                .is_some();
        }

        if self.screen == LabScreen::Oracle {
            if oracle_view_tab_hit_at(body_layout.selected_area, column, row).is_some() {
                return true;
            }
            return match self.oracle.view {
                OracleView::Earn => {
                    oracle_earn_action_hit_at(body_layout.selected_area, column, row)
                }
                OracleView::Advanced => {
                    oracle_action_hit_at(cli, body_layout.selected_area, self, column, row)
                        .is_some()
                }
            };
        }

        if self.screen != LabScreen::Chain {
            return false;
        }

        if self.trading.ticket.is_none()
            && body_layout.activity_area.height > 0
            && let Some((buy_rect, sell_rect)) =
                selected_contract_button_rects(body_layout.activity_area)
            && (rect_contains(buy_rect, column, row) || rect_contains(sell_rect, column, row))
        {
            return true;
        }
        if !self.trading.submit_is_running()
            && let Some(order_area) = active_order_ticket_area(cli, body_layout.selected_area, self)
        {
            if trade_ticket_field_hit_at(cli, order_area, self, column, row).is_some() {
                return true;
            }
            if let Some(button_rect) = order_ticket_place_button_rect(order_area) {
                return rect_contains(button_rect, column, row);
            }
        }

        false
    }

    pub(super) fn handle_trade_confirmation_mouse_click(
        &mut self,
        cli: &Cli,
        modal_root: Rect,
        column: u16,
        row: u16,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let Some(choice) = trade_confirmation_button_at(modal_root, column, row) else {
            self.status = "Choose Cancel or Confirm order.".to_string();
            return;
        };
        if let Some(confirmation) = self
            .trading
            .ticket
            .as_mut()
            .and_then(|ticket| ticket.confirmation.as_mut())
        {
            confirmation.choice = choice;
        }
        self.activate_trade_confirmation(cli, fetch_tx);
    }

    pub(super) fn handle_writer_confirmation_mouse_click(
        &mut self,
        cli: &Cli,
        modal_root: Rect,
        column: u16,
        row: u16,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.action_is_running() {
            self.status = "The writer transaction is already running.".to_string();
            return;
        }
        let Some(choice) = writer_confirmation_button_at(modal_root, column, row) else {
            self.status = "Choose Cancel or Confirm & Send.".to_string();
            return;
        };
        if let Some(confirmation) = self.writers.confirmation.as_mut() {
            confirmation.choice = choice;
        }
        self.activate_writer_confirmation(cli, backend_url, fetch_tx);
    }

    pub(super) fn handle_staking_confirmation_mouse_click(
        &mut self,
        area: Rect,
        column: u16,
        row: u16,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.staking_action_is_running() {
            self.status = "The staking transaction is already running.".to_string();
            return;
        }
        let Some(choice) = staking_confirmation_button_at(area, self, column, row) else {
            self.status = "Choose Cancel or Confirm & Sign.".to_string();
            return;
        };
        if let Some(confirmation) = self.staking_confirmation.as_mut() {
            confirmation.choice = choice;
        }
        self.activate_staking_confirmation(fetch_tx);
    }

    pub(super) fn activate_terms_action(&mut self, action: TermsAction) {
        match action {
            TermsAction::OpenTerms => self.open_terms_page(),
            TermsAction::SwitchWallet => self.begin_wallet_switch(),
            TermsAction::Accept => self.accept_wallet_terms(),
        }
    }

    pub(super) fn activate_external_link(&mut self, target: ExternalLinkTarget) {
        let (label, url) = match target {
            ExternalLinkTarget::Docs => ("documentation", docs_url()),
            ExternalLinkTarget::Terms => ("Terms page", self.wallet_terms.terms_url.clone()),
        };
        match wallet_terms::open_external_url(&url) {
            Ok(()) => self.status = format!("Opened {label} in your browser."),
            Err(error) => {
                self.status = format!("Could not open {label}. Visit {url} manually. {error}")
            }
        }
    }

    pub(super) fn handle_mouse_click(
        &mut self,
        cli: &Cli,
        root: Rect,
        column: u16,
        row: u16,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.startup_intro_is_open() {
            if rect_contains(startup_intro_open_button_rect(root), column, row) {
                self.dismiss_startup_intro();
            }
            return;
        }
        let frame_layout = lab_frame_layout(root, cli, self);
        let body_layout = lab_page_layout(self.screen, frame_layout);

        if header_update_hit_at(cli, frame_layout.header_area, self, column, row) {
            self.updates.request_mouse_exit();
            return;
        }

        if let Some(action) = header_nav_hit_at(cli, frame_layout.header_area, self, column, row) {
            match action {
                HeaderNavAction::Back => self.go_back(),
                HeaderNavAction::Home => self.open_home(),
            }
            return;
        }

        if let Some(target) = external_link_hit_at(cli, root, self, column, row) {
            self.activate_external_link(target);
            return;
        }

        if let Some(suggestion_index) = guide_suggestion_hit_at(cli, root, self, column, row) {
            self.submit_guide_suggestion(suggestion_index, fetch_tx);
            return;
        }

        if self.screen != LabScreen::Terms
            && frame_layout
                .guide_area
                .is_some_and(|area| rect_contains(area, column, row))
        {
            self.begin_guide_input(backend_url, fetch_tx);
            return;
        }

        if self.screen == LabScreen::Terms {
            if !self.wallet_switch_editing
                && let Some(action) = terms_action_hit_at(
                    cli,
                    frame_layout.body_area,
                    self,
                    self.focused_panel_scroll(LabFocus::Terms),
                    column,
                    row,
                )
            {
                self.activate_terms_action(action);
                return;
            }
            if rect_contains(frame_layout.body_area, column, row) {
                self.set_focus(LabFocus::Terms);
            }
            return;
        }

        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Overview {
            let layout = gitbook_help_layout(body_layout.selected_area);
            if let Some(geometry) = gitbook_help_preview_geometry(body_layout.selected_area, self) {
                if gitbook_help_preview_close_rect(geometry.popup)
                    .is_some_and(|area| rect_contains(area, column, row))
                {
                    self.help.preview = None;
                    self.status = "Topic preview closed.".to_string();
                    return;
                }
                if rect_contains(geometry.popup, column, row) {
                    if let Some(nav_index) =
                        self.help.preview.as_ref().map(|preview| preview.nav_index)
                    {
                        self.help.selected_nav = nav_index;
                    }
                    self.activate_help_selection(fetch_tx);
                    return;
                }
            }
            if let Some(index) = gitbook_nav_hit_at(cli, layout.navigation_area, self, column, row)
            {
                self.help.pane = HelpPane::Navigation;
                self.help.selected_nav = index;
                self.activate_help_selection(fetch_tx);
            } else if rect_contains(layout.article_area, column, row) {
                self.help.pane = HelpPane::Article;
            }
            return;
        }

        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Agents {
            if agent_connection_button_rect(body_layout.selected_area)
                .is_some_and(|area| rect_contains(area, column, row))
                && self.mcp_connection_button_active()
            {
                self.set_focus(LabFocus::Help);
                self.toggle_mcp_connection();
                return;
            }
            if rect_contains(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::Help);
            }
            return;
        }

        if let Some(market_area) = body_layout.market_area
            && let Some(hit) = market_rail_hit_at(cli, self, market_area, column, row)
        {
            match hit {
                MarketRailHit::Market(index) => {
                    let was_selected = self.trading.selected == index;
                    if self.select_market_index(index, backend_url, fetch_tx) && was_selected {
                        self.activate_market_row(backend_url, fetch_tx);
                    }
                }
                MarketRailHit::Series(index) => {
                    self.set_focus(LabFocus::MarketSeries);
                    if self.select_market_series_index(index) {
                        if self.screen == LabScreen::Chart {
                            self.request_chart(backend_url, fetch_tx, false);
                        } else {
                            self.activate_market_series(backend_url, fetch_tx);
                        }
                    }
                }
            }
            return;
        }

        if self.screen == LabScreen::Home {
            if let Some(index) =
                home_action_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                self.home_selected = index;
                self.set_focus(LabFocus::HomeActions);
                self.activate_home_action(backend_url, fetch_tx);
                return;
            }
            if let Some(focus) = home_focus_at(cli, body_layout.selected_area, self, column, row) {
                self.set_focus(focus);
            }
            return;
        }

        if self.screen == LabScreen::Chart {
            if let Some(hit) = chart_control_hit_at(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::Chart);
                match hit {
                    ChartControlHit::Range(range) => {
                        self.set_chart_range(backend_url, fetch_tx, range)
                    }
                    ChartControlHit::Refresh if !self.trading.loading_chart => {
                        self.request_chart(backend_url, fetch_tx, true)
                    }
                    ChartControlHit::Refresh => {
                        self.status = "The chart is already refreshing.".to_string()
                    }
                }
                return;
            }
            if let Some(activity_area) = chart_activity_area(body_layout.selected_area, self)
                && rect_contains(activity_area, column, row)
            {
                self.open_activity();
                return;
            }
            if rect_contains(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::Chart);
            }
            return;
        }

        if self.screen == LabScreen::Detail {
            if let Some(view) = detail_tab_hit_at(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::Detail);
                self.select_detail_view(view, backend_url, fetch_tx);
            } else if rect_contains(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::Detail);
            }
            return;
        }

        if self.screen == LabScreen::Ledger {
            self.set_focus(LabFocus::Ledger);
            if self.writers.form.is_some()
                || self.liquidity_preview_form.is_some()
                || self.wallet_switch_editing
            {
                return;
            }
            if let Some(view) = ledger_tab_hit_at(body_layout.selected_area, column, row) {
                self.select_ledger_view(view, backend_url, fetch_tx);
                return;
            }
            if let Some(index) =
                ledger_writer_action_hit_at(body_layout.selected_area, self, column, row)
            {
                self.ledger_pane = LedgerPane::Actions;
                self.writers.action_selected = index;
                self.activate_writer_action(cli, backend_url, fetch_tx);
                return;
            }
            if ledger_liquidity_action_hit_at(body_layout.selected_area, self, column, row) {
                self.ledger_pane = LedgerPane::Actions;
                self.open_liquidity_preview_form();
                return;
            }
            if let Some(index) =
                ledger_list_row_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                match self.ledger_view {
                    LedgerView::Account => {
                        self.ledger_pane = LedgerPane::Actions;
                        self.ledger_account_action_selected = index;
                        self.activate_ledger_control(cli, backend_url, fetch_tx);
                    }
                    LedgerView::Positions => {
                        if self.liquidity_preview_is_running() {
                            self.status = "The visible liquidity request is locked until its preview returns."
                                .to_string();
                            return;
                        }
                        self.ledger_position_selected = index;
                        self.liquidity_preview_result = None;
                        self.ledger_pane = LedgerPane::Detail;
                        self.status = "Manager-liquidity position selected.".to_string();
                    }
                    LedgerView::Writers => {
                        if self.writers.interaction_is_locked() {
                            self.status = "The selected writer sleeve is locked until the current action or availability check finishes."
                                .to_string();
                            return;
                        }
                        self.writers.clear_action_mask_check();
                        self.ledger_writer_selected = index;
                        self.ledger_pane = LedgerPane::List;
                        self.writers.action_result = None;
                        self.status = "Collective-writer sleeve selected.".to_string();
                    }
                    LedgerView::History => {
                        self.ledger_history_selected = index;
                        self.ledger_pane = LedgerPane::Detail;
                        self.status = "Wallet activity row selected.".to_string();
                    }
                }
                return;
            }
            let layout = ledger_screen_layout(body_layout.selected_area, self.ledger_view);
            if rect_contains(layout.detail, column, row) {
                self.ledger_pane = LedgerPane::Detail;
            }
            return;
        }

        if self.screen == LabScreen::OracleIntro {
            if let Some(index) =
                oracle_intro_action_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                self.oracle.intro_selected = index;
                self.set_focus(LabFocus::OracleIntro);
                self.activate_oracle_intro_action(backend_url, fetch_tx);
                return;
            }
            if rect_contains(body_layout.selected_area, column, row) {
                self.set_focus(LabFocus::OracleIntro);
            }
            return;
        }

        if self.screen == LabScreen::Oracle {
            if let Some(view) = oracle_view_tab_hit_at(body_layout.selected_area, column, row) {
                self.open_oracle_view(view, backend_url, fetch_tx);
                return;
            }
            if self.oracle.view == OracleView::Earn {
                if let Some(index) =
                    oracle_earn_claim_hit_at(body_layout.selected_area, self, column, row)
                {
                    self.set_focus(LabFocus::OracleEarn);
                    self.select_oracle_earn_reward(index);
                } else if oracle_earn_action_hit_at(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::OracleEarn);
                    self.activate_oracle_earn(backend_url, fetch_tx);
                } else if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::OracleEarn);
                }
                return;
            }
            if let Some(field_index) =
                oracle_form_field_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                self.set_focus(LabFocus::OracleActions);
                self.select_oracle_form_field_index(field_index);
                return;
            }
            if let Some(action) =
                oracle_action_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                self.set_focus(LabFocus::OracleActions);
                self.select_oracle_action(action);
                self.oracle.locked_flash = None;
                self.activate_oracle_action();
                return;
            }
            if let Some(node_index) =
                oracle_tree_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                let was_selected = self.selected_oracle_node_index() == node_index;
                self.set_focus(LabFocus::OracleTasks);
                self.select_oracle_node(node_index);
                if was_selected {
                    self.open_oracle_child();
                }
                return;
            }
            if let Some(node_index) =
                oracle_path_hit_at(cli, body_layout.selected_area, self, column, row)
            {
                self.set_focus(LabFocus::OracleTasks);
                self.select_oracle_node(node_index);
                return;
            }
            if let Some(focus) = oracle_focus_at(cli, body_layout.selected_area, self, column, row)
            {
                self.set_focus(focus);
            }
            return;
        }

        if self.screen != LabScreen::Chain {
            self.focus_mouse_target(cli, root, column, row);
            return;
        }

        if self.trading.ticket.is_none()
            && body_layout.activity_area.height > 0
            && let Some((buy_rect, sell_rect)) =
                selected_contract_button_rects(body_layout.activity_area)
        {
            if rect_contains(buy_rect, column, row) {
                self.select_trade_action(TradeAction::Buy);
                return;
            }
            if rect_contains(sell_rect, column, row) {
                self.select_trade_action(TradeAction::Sell);
                return;
            }
        }
        if let Some(order_area) = active_order_ticket_area(cli, body_layout.selected_area, self) {
            if self.trading.submit_is_running() && rect_contains(order_area, column, row) {
                self.status =
                    "Order submission is running. The ticket will unlock when it finishes."
                        .to_string();
                return;
            }
            if let Some(button_rect) = order_ticket_place_button_rect(order_area)
                && rect_contains(button_rect, column, row)
            {
                if self.trading.submit_is_running() {
                    self.status = "Order submission is already running.".to_string();
                } else {
                    self.review_or_submit_trade_ticket(cli, fetch_tx);
                }
                return;
            }
            if rect_contains(order_area, column, row) {
                if let Some(field) = trade_ticket_field_hit_at(cli, order_area, self, column, row) {
                    self.select_trade_ticket_field(field);
                } else {
                    self.status =
                        "Order ticket focused. Type price or contracts, Enter reviews, Esc cancels."
                            .to_string();
                }
                return;
            }
        }

        if self.trading.submit_is_running() {
            self.status = "Order submission is running. Contract controls are temporarily locked."
                .to_string();
            return;
        }

        if let Some(detail) = &self.trading.detail {
            if let Some(index) = option_quote_hit_at(
                cli,
                body_layout.selected_area,
                detail,
                self,
                OptionKind::Call,
                column,
                row,
            ) {
                self.select_option_index(OptionKind::Call, index);
                return;
            }
            if let Some(index) = option_quote_hit_at(
                cli,
                body_layout.selected_area,
                detail,
                self,
                OptionKind::Put,
                column,
                row,
            ) {
                self.select_option_index(OptionKind::Put, index);
                return;
            }
        }

        if let Some(focus) = chain_focus_at(cli, body_layout.selected_area, self, column, row) {
            self.set_focus(focus);
            match focus {
                LabFocus::Calls => self.status = "Calls focused.".to_string(),
                LabFocus::Puts => self.status = "Puts focused.".to_string(),
                LabFocus::Detail => self.status = "Options summary focused.".to_string(),
                _ => {}
            }
        } else {
            self.request_selected_detail(backend_url, fetch_tx, true);
        }
    }

    pub(super) fn update_gitbook_help_hover(
        &mut self,
        cli: &Cli,
        root: Rect,
        column: u16,
        row: u16,
    ) {
        if self.screen != LabScreen::Help || self.home_help_topic != HomeHelpTopic::Overview {
            if self
                .help
                .preview
                .as_ref()
                .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Hover)
            {
                self.help.preview = None;
            }
            self.help.glossary_hover = None;
            self.help.hover_grace_ticks = None;
            return;
        }
        if self
            .help
            .preview
            .as_ref()
            .is_some_and(|preview| preview.origin == HelpPreviewOrigin::Keyboard)
        {
            return;
        }

        let frame_layout = lab_frame_layout(root, cli, self);
        let body_layout = lab_page_layout(self.screen, frame_layout);
        let layout = gitbook_help_layout(body_layout.selected_area);
        let glossary_geometry = gitbook_glossary_geometry(body_layout.selected_area, self);
        if glossary_geometry.is_some_and(|geometry| rect_contains(geometry.popup, column, row)) {
            self.help.hover_grace_ticks = None;
            return;
        }
        let preview_geometry = gitbook_help_preview_geometry(body_layout.selected_area, self);
        if preview_geometry.is_some_and(|geometry| rect_contains(geometry.popup, column, row)) {
            self.help.hover_grace_ticks = None;
            return;
        }
        if let Some(nav_index) = gitbook_nav_hit_at(cli, layout.navigation_area, self, column, row)
        {
            self.help.glossary_hover = None;
            self.help.hover_grace_ticks = None;
            self.set_help_hover_preview(nav_index);
            return;
        }
        if let Some(hit) = gitbook_glossary_hit_at(cli, layout.article_area, self, column, row) {
            self.help.clear_hover_preview();
            self.help.hover_grace_ticks = None;
            let unchanged =
                self.help.glossary_hover.as_ref().is_some_and(|hover| {
                    hover.term == hit.entry.term && hover.anchor == hit.anchor
                });
            if !unchanged {
                self.help.glossary_hover = Some(GitbookGlossaryHover {
                    term: hit.entry.term,
                    definition: hit.entry.definition,
                    anchor: hit.anchor,
                });
            }
            return;
        }
        if preview_geometry.is_some() || glossary_geometry.is_some() {
            if self.help.hover_grace_ticks.is_none() {
                self.help.hover_grace_ticks = Some(HELP_HOVER_EXIT_GRACE_TICKS);
            }
            return;
        }
        self.help.clear_hover_preview();
        self.help.glossary_hover = None;
        self.help.hover_grace_ticks = None;
    }

    pub(super) fn focus_mouse_target(&mut self, cli: &Cli, root: Rect, column: u16, row: u16) {
        let frame_layout = lab_frame_layout(root, cli, self);
        let body_layout = lab_page_layout(self.screen, frame_layout);

        if self.screen != LabScreen::Terms
            && frame_layout
                .guide_area
                .is_some_and(|area| rect_contains(area, column, row))
        {
            self.set_focus(LabFocus::Guide);
            return;
        }

        if let Some(market_area) = body_layout.market_area
            && rect_contains(market_area, column, row)
        {
            let focus = match market_rail_hit_at(cli, self, market_area, column, row) {
                Some(MarketRailHit::Series(_)) => LabFocus::MarketSeries,
                _ => LabFocus::Markets,
            };
            self.set_focus(focus);
            return;
        }

        match self.screen {
            LabScreen::Home => {
                if let Some(focus) =
                    home_focus_at(cli, body_layout.selected_area, self, column, row)
                {
                    self.set_focus(focus);
                }
            }
            LabScreen::Chain => {
                if let Some(focus) =
                    chain_focus_at(cli, body_layout.selected_area, self, column, row)
                {
                    self.set_focus(focus);
                }
            }
            LabScreen::Chart => {
                if chart_control_hit_at(body_layout.selected_area, column, row).is_some() {
                    self.set_focus(LabFocus::Chart);
                } else if let Some(activity_area) =
                    chart_activity_area(body_layout.selected_area, self)
                    && rect_contains(activity_area, column, row)
                {
                    self.set_focus(LabFocus::Activity);
                } else if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Chart);
                }
            }
            LabScreen::OracleIntro => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::OracleIntro);
                }
            }
            LabScreen::OracleHelp => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::OracleHelp);
                }
            }
            LabScreen::Help => {
                if self.home_help_topic == HomeHelpTopic::Overview {
                    let layout = gitbook_help_layout(body_layout.selected_area);
                    if rect_contains(layout.navigation_area, column, row) {
                        self.help.pane = HelpPane::Navigation;
                    } else if rect_contains(layout.article_area, column, row) {
                        self.help.pane = HelpPane::Article;
                    }
                }
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Help);
                }
            }
            LabScreen::Oracle => {
                if self.oracle.view == OracleView::Earn {
                    if rect_contains(body_layout.selected_area, column, row) {
                        self.set_focus(LabFocus::OracleEarn);
                    }
                } else if let Some(focus) =
                    oracle_focus_at(cli, body_layout.selected_area, self, column, row)
                {
                    self.set_focus(focus);
                }
            }
            LabScreen::Detail => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Detail);
                }
            }
            LabScreen::Activity => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Activity);
                }
            }
            LabScreen::Ledger => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Ledger);
                    let layout = ledger_screen_layout(body_layout.selected_area, self.ledger_view);
                    self.ledger_pane = if rect_contains(layout.tabs, column, row) {
                        LedgerPane::Tabs
                    } else if layout
                        .actions
                        .is_some_and(|area| rect_contains(area, column, row))
                        && (self.ledger_view == LedgerView::Account
                            || self.ledger_view == LedgerView::Positions
                            || self.ledger_view == LedgerView::Writers)
                    {
                        LedgerPane::Actions
                    } else if rect_contains(layout.list, column, row) {
                        LedgerPane::List
                    } else {
                        LedgerPane::Detail
                    };
                }
            }
            LabScreen::Staking => {
                if rect_contains(body_layout.selected_area, column, row) {
                    self.set_focus(LabFocus::Staking);
                }
            }
            LabScreen::Terms => {
                if rect_contains(frame_layout.body_area, column, row) {
                    self.set_focus(LabFocus::Terms);
                }
            }
        }
    }

    pub(super) fn move_focus_left(&mut self) {
        self.move_focus(FocusDirection::Left, None, None);
    }

    pub(super) fn move_focus_right(&mut self) {
        self.move_focus(FocusDirection::Right, None, None);
    }

    pub(super) fn move_focus_up(&mut self, backend_url: &str, fetch_tx: &Sender<LabFetchResult>) {
        self.move_focus(FocusDirection::Up, Some(backend_url), Some(fetch_tx));
    }

    pub(super) fn move_focus_down(&mut self, backend_url: &str, fetch_tx: &Sender<LabFetchResult>) {
        self.move_focus(FocusDirection::Down, Some(backend_url), Some(fetch_tx));
    }

    pub(super) fn move_focus_next_panel(&mut self) {
        self.move_focus_in_cycle(1);
    }

    pub(super) fn move_focus_prev_panel(&mut self) {
        self.move_focus_in_cycle(-1);
    }

    pub(super) fn move_focus_in_cycle(&mut self, offset: isize) {
        if self.trading.submit_is_running() && self.screen == LabScreen::Chain {
            self.status = "Order submission is running. Contract controls are temporarily locked."
                .to_string();
            return;
        }
        let order = if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Earn {
            &[LabFocus::Markets, LabFocus::OracleEarn][..]
        } else {
            focus_cycle_order(self.screen)
        };
        if order.is_empty() {
            return;
        }
        let current = if self.focus == LabFocus::MarketSeries {
            LabFocus::Markets
        } else {
            self.focus
        };
        let current_position = order
            .iter()
            .position(|focus| *focus == current)
            .unwrap_or(0);
        let len = order.len() as isize;
        let next_position = (current_position as isize + offset).rem_euclid(len) as usize;
        self.set_focus(order[next_position]);
    }

    pub(super) fn move_focus(
        &mut self,
        direction: FocusDirection,
        backend_url: Option<&str>,
        fetch_tx: Option<&Sender<LabFetchResult>>,
    ) {
        if self.trading.submit_is_running() && self.screen == LabScreen::Chain {
            self.status = "Order submission is running. Contract controls are temporarily locked."
                .to_string();
            return;
        }
        if self.handle_focused_list_navigation(direction, backend_url, fetch_tx) {
            return;
        }
        if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Earn {
            match (self.focus, direction) {
                (LabFocus::Markets | LabFocus::MarketSeries, FocusDirection::Right) => {
                    self.set_focus(LabFocus::OracleEarn);
                }
                (LabFocus::OracleEarn, FocusDirection::Left) => {
                    self.set_focus(LabFocus::Markets);
                }
                _ => {}
            }
            return;
        }
        if let Some(next) = focus_neighbor(self.screen, self.focus, direction) {
            self.set_focus(next);
        }
    }

    pub(super) fn handle_focused_list_navigation(
        &mut self,
        direction: FocusDirection,
        backend_url: Option<&str>,
        fetch_tx: Option<&Sender<LabFetchResult>>,
    ) -> bool {
        match (self.focus, direction) {
            (LabFocus::Guide, FocusDirection::Up) => {
                self.scroll_guide(-1);
                true
            }
            (LabFocus::Guide, FocusDirection::Down) => {
                self.scroll_guide(1);
                true
            }
            (LabFocus::Markets, FocusDirection::Up) => {
                let (Some(backend_url), Some(fetch_tx)) = (backend_url, fetch_tx) else {
                    return false;
                };
                self.select_prev(backend_url, fetch_tx);
                true
            }
            (LabFocus::Markets, FocusDirection::Down) => {
                let (Some(backend_url), Some(fetch_tx)) = (backend_url, fetch_tx) else {
                    return false;
                };
                if self.trading.market_series_open && self.market_series_count() > 0 {
                    self.set_focus(LabFocus::MarketSeries);
                } else {
                    self.select_next(backend_url, fetch_tx);
                }
                true
            }
            (LabFocus::MarketSeries, FocusDirection::Up) => {
                let (Some(backend_url), Some(fetch_tx)) = (backend_url, fetch_tx) else {
                    return false;
                };
                if !self.select_prev_market_series(backend_url, fetch_tx) {
                    self.set_focus(LabFocus::Markets);
                }
                true
            }
            (LabFocus::MarketSeries, FocusDirection::Down) => {
                let (Some(backend_url), Some(fetch_tx)) = (backend_url, fetch_tx) else {
                    return false;
                };
                if !self.select_next_market_series(backend_url, fetch_tx) {
                    self.select_next_after_market_series(backend_url, fetch_tx);
                }
                true
            }
            (LabFocus::HomeActions, FocusDirection::Up) => self.select_prev_home_action(),
            (LabFocus::HomeActions, FocusDirection::Down) => self.select_next_home_action(),
            (LabFocus::OracleIntro, FocusDirection::Up) => self.oracle.select_prev_intro_action(),
            (LabFocus::OracleIntro, FocusDirection::Down) => self.oracle.select_next_intro_action(),
            (LabFocus::OracleEarn, FocusDirection::Up) => {
                self.select_prev_oracle_earn_reward();
                true
            }
            (LabFocus::OracleEarn, FocusDirection::Down) => {
                self.select_next_oracle_earn_reward();
                true
            }
            (LabFocus::OracleTasks, FocusDirection::Up) => {
                self.select_oracle_node_by_offset(-1);
                true
            }
            (LabFocus::OracleTasks, FocusDirection::Down) => {
                self.select_oracle_node_by_offset(1);
                true
            }
            (LabFocus::OracleTasks, FocusDirection::Left) => self.open_oracle_parent(),
            (LabFocus::OracleTasks, FocusDirection::Right) => {
                self.open_oracle_child();
                true
            }
            (LabFocus::Calls, FocusDirection::Up) => {
                self.select_option_side(OptionKind::Call);
                self.select_prev_option();
                true
            }
            (LabFocus::Calls, FocusDirection::Down) => {
                self.select_option_side(OptionKind::Call);
                self.select_next_option();
                true
            }
            (LabFocus::Puts, FocusDirection::Up) => {
                self.select_option_side(OptionKind::Put);
                self.select_prev_option();
                true
            }
            (LabFocus::Puts, FocusDirection::Down) => {
                self.select_option_side(OptionKind::Put);
                self.select_next_option();
                true
            }
            (LabFocus::OracleActions, FocusDirection::Up) => self.select_prev_oracle_action(),
            (LabFocus::OracleActions, FocusDirection::Down) => self.select_next_oracle_action(),
            _ => false,
        }
    }

    pub(super) fn activate_focused_panel(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        match (self.screen, self.focus) {
            (_, LabFocus::Guide) => self.begin_guide_input(backend_url, fetch_tx),
            (_, LabFocus::Markets) => self.activate_market_row(backend_url, fetch_tx),
            (_, LabFocus::MarketSeries) => self.activate_market_series(backend_url, fetch_tx),
            (LabScreen::Home, LabFocus::HomeActions) => {
                self.activate_home_action(backend_url, fetch_tx);
            }
            (LabScreen::Home, LabFocus::HomePreview) => {
                self.activate_home_action(backend_url, fetch_tx);
            }
            (LabScreen::Home, LabFocus::HomeSummary) => self.open_detail(),
            (LabScreen::Staking, LabFocus::Staking) => self.activate_staking_action(fetch_tx),
            (LabScreen::OracleIntro, LabFocus::OracleIntro) => {
                self.activate_oracle_intro_action(backend_url, fetch_tx);
            }
            (LabScreen::OracleHelp, LabFocus::OracleHelp) => {
                self.open_oracle_intro();
            }
            (LabScreen::Help, LabFocus::Help) if self.home_help_topic == HomeHelpTopic::Agents => {
                self.toggle_mcp_connection();
            }
            (LabScreen::Help, _) => {}
            (LabScreen::Chart, _) => self.request_chart(backend_url, fetch_tx, true),
            (LabScreen::Ledger, _) => self.request_ledger(backend_url, fetch_tx, true),
            (LabScreen::Oracle, LabFocus::OracleEarn) => {
                self.activate_oracle_earn(backend_url, fetch_tx);
            }
            (LabScreen::Oracle, LabFocus::OracleOverview) => {
                self.set_focus(LabFocus::OracleActions);
            }
            (LabScreen::Oracle, LabFocus::OracleActions) => self.activate_oracle_action(),
            (LabScreen::Oracle, LabFocus::OracleTasks) => {
                self.open_oracle_child();
            }
            _ => self.request_selected_detail(backend_url, fetch_tx, true),
        }
    }
}
