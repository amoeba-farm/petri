//! Core Lab state initialization, wallet lifecycle, ticks, and update state.

use super::*;

impl LabApp {
    pub(super) fn new_with_update_check(
        initial_dish: Option<&str>,
        wallet: AttachedWallet,
        onchain_config: OnchainConfig,
        update_check_requested: bool,
    ) -> Self {
        let pending_initial_dish = initial_dish
            .map(str::trim)
            .filter(|market_id| !market_id.is_empty())
            .map(str::to_ascii_lowercase);
        let selected_label = pending_initial_dish
            .as_deref()
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| "market".to_string());
        let dishes = Vec::new();
        let selected = 0;
        let issues = Vec::new();
        let wallet_terms = wallet_terms::status_for_wallet(wallet.pubkey.as_deref());
        let terms_required = wallet.is_attached() && !wallet_terms.accepted;
        let oracle_submission_store_path = oracle_submissions::default_path();
        let (oracle_submissions, oracle_submission_issue) =
            load_oracle_submission_records(oracle_submission_store_path.as_ref());
        let oracle_tree = None;
        let help_index = gitbook::bundled_index();
        let help_selected_page_id = help_index
            .first_page()
            .map(|page| page.id.clone())
            .unwrap_or_default();
        let mut help_pages = crate::cache::DisplayCache::new(read_cache::HELP_PAGES);
        if let Some(page) = help_index.first_page().and_then(gitbook::bundled_page) {
            help_pages.insert(help_selected_page_id.clone(), page);
        }
        let read_cache_scope =
            read_cache::ReadScope::new(&onchain_config.backend_url, &onchain_config.network);
        let help_expanded_categories = vec![true; help_index.categories.len()];
        let update_check_enabled = update_check_requested && env_update_check_enabled();
        let guide = GuidePanelState::new(guide::GuideConfig::load());
        let screen = if terms_required {
            LabScreen::Terms
        } else {
            LabScreen::Home
        };
        let status = if terms_required {
            "Accept Terms to use this wallet in Petri.".to_string()
        } else {
            format!("Opening Petri. Loading live {selected_label} data...")
        };
        Self {
            dishes,
            selected,
            pending_initial_dish,
            home_selected: 0,
            home_help_topic: HomeHelpTopic::Overview,
            help_index,
            help_pages,
            help_page_revision: 1,
            help_render_cache: RefCell::new(None),
            help_selected_page_id,
            help_selected_nav: 1,
            help_expanded_categories,
            help_pane: HelpPane::Navigation,
            help_nav_scroll: 0,
            help_article_scroll: 0,
            help_preview: None,
            help_glossary_hover: None,
            help_hover_grace_ticks: None,
            help_index_request: 0,
            help_page_request: 0,
            help_page_request_id: None,
            help_preview_page_request: 0,
            help_preview_page_request_id: None,
            loading_help_index: false,
            loading_help_page: false,
            loading_help_preview_page: false,
            help_preview_failed_page_id: None,
            help_issue: None,
            help_transition_tick: None,
            help_last_checked_at: None,
            mcp_connection_state: McpConnectionState::Disabled,
            mcp_managed_entry_enabled: false,
            mcp_connection_issue: None,
            mcp_repair_failed: false,
            guide,
            oracle_intro_selected: 0,
            oracle_view: OracleView::Advanced,
            oracle_earn_selected: 0,
            oracle_selected: 0,
            oracle_node_selected: DEFAULT_ORACLE_NODE_INDEX,
            oracle_search_input: String::new(),
            oracle_search_editing: false,
            oracle_form: None,
            oracle_form_field_flash: None,
            oracle_locked_flash: None,
            oracle_tree,
            oracle_tree_issue: None,
            oracle_tree_retry_after_tick: None,
            oracle_submissions,
            oracle_submission_store_path,
            oracle_submission_issue,
            oracle_live: None,
            oracle_live_issue: None,
            oracle_rewards: None,
            oracle_reward_issue: None,
            update_check_enabled,
            update_check_request: 0,
            update_report: None,
            update_issue: None,
            update_check_forced: false,
            loading_update_check: false,
            update_mouse_requested: false,
            selected_option: 0,
            active_option_kind: OptionKind::Call,
            chain_focus: ChainFocus::Markets,
            focus: default_focus_for_screen(screen),
            market_series_open: false,
            chart_expiry: 0,
            initial_chart_expiry: None,
            chart_range: ChartRangeValue::TwentyFourHours,
            chart_launch_options: None,
            chart_last_refresh_at: None,
            pending_initial_chart: false,
            trade_action: TradeAction::Buy,
            trade_ticket: None,
            read_panel: None,
            read_panel_request: 0,
            action_panel: None,
            action_panel_request: 0,
            trade_review_scroll: 0,
            trade_ticket_field_flash: None,
            trade_result_modal: None,
            suppress_trade_result_escape_repeat: false,
            left_mouse_down: false,
            confirmation_mouse_press: None,
            detail: None,
            detail_view: DetailView::Overview,
            settlement_bundle: None,
            settlement_issue: None,
            chart: None,
            ledger: None,
            ledger_view: LedgerView::Account,
            ledger_pane: LedgerPane::Tabs,
            ledger_account_action_selected: 0,
            ledger_position_selected: 0,
            ledger_writer_selected: 0,
            ledger_history_selected: 0,
            liquidity_preview_form: None,
            liquidity_preview_result: None,
            writer_action_selected: 0,
            writer_form: None,
            writer_confirmation: None,
            writer_action_result: None,
            staking_status: None,
            staking_issue: None,
            staking_selected: 0,
            staking_form: None,
            staking_confirmation: None,
            staking_action_result: None,
            wallet,
            wallet_terms,
            wallet_switch_input: String::new(),
            wallet_switch_editing: false,
            onchain_config,
            screen,
            startup_intro_started_at: None,
            screen_history: Vec::new(),
            status,
            issues,
            read_cache_scope,
            detail_cache: crate::cache::DisplayCache::new(read_cache::MARKET_DETAILS),
            chart_cache: crate::cache::DisplayCache::new(read_cache::CHARTS),
            settlement_cache: crate::cache::DisplayCache::new(read_cache::SETTLEMENTS),
            panel_scrolls: HashMap::new(),
            list_request: 0,
            detail_request: 0,
            settlement_request: 0,
            chart_request: 0,
            ledger_request: 0,
            liquidity_preview_request: 0,
            liquidity_preview_inflight: None,
            writer_action_request: 0,
            writer_action_inflight: None,
            writer_action_mask_request: 0,
            writer_action_mask_inflight: None,
            pending_writer_review: None,
            staking_status_request: 0,
            staking_action_request: 0,
            staking_action_inflight: None,
            trade_submit_request: 0,
            trade_submit_inflight: None,
            oracle_tree_request: 0,
            oracle_live_request: 0,
            oracle_reward_request: 0,
            loading_list: false,
            loading_detail: false,
            loading_settlement: false,
            loading_chart: false,
            loading_ledger: false,
            loading_staking: false,
            loading_oracle_tree: false,
            loading_oracle_live: false,
            loading_oracle_rewards: false,
            spinner_tick: 0,
            terminal_size_warning_started_tick: None,
        }
    }

    pub(super) fn show_startup_intro_at(&mut self, now: Instant) {
        self.startup_intro_started_at = Some(now);
    }

    pub(super) fn dismiss_startup_intro(&mut self) {
        self.startup_intro_started_at = None;
    }

    pub(super) fn startup_intro_is_open(&self) -> bool {
        self.startup_intro_started_at.is_some()
    }

    pub(super) fn startup_intro_frame_index_at(&self, now: Instant) -> usize {
        self.startup_intro_started_at
            .map(|started_at| startup_intro_frame_index(now.saturating_duration_since(started_at)))
            .unwrap_or(0)
    }

    pub(super) fn startup_intro_next_frame_timeout_at(&self, now: Instant) -> Duration {
        self.startup_intro_started_at
            .map(|started_at| {
                startup_intro_next_frame_timeout(now.saturating_duration_since(started_at))
            })
            .unwrap_or(STARTUP_INTRO_FRAME_INTERVAL)
    }

    pub(super) fn selected_id(&self) -> String {
        self.dishes
            .get(self.selected)
            .map(|dish| dish.id.clone())
            .unwrap_or_else(|| "ramx".to_string())
    }

    pub(super) fn accept_wallet_terms(&mut self) {
        let Some(pubkey) = self.wallet.pubkey.as_deref() else {
            self.wallet_terms.accepted = true;
            self.screen = LabScreen::Home;
            self.focus = default_focus_for_screen(self.screen);
            self.status = "No attached wallet. Terms gate skipped.".to_string();
            return;
        };
        match wallet_terms::accept_wallet_terms(pubkey) {
            Ok(status) => {
                self.wallet_terms = status;
                self.screen = LabScreen::Home;
                self.focus = default_focus_for_screen(self.screen);
                self.status = "Terms accepted for this wallet.".to_string();
            }
            Err(error) => {
                self.status = format!("Terms acceptance could not be saved: {error}");
                self.wallet_terms.issue = Some(error.to_string());
            }
        }
    }

    pub(super) fn open_terms_page(&mut self) {
        match wallet_terms::open_terms_url(&self.wallet_terms.terms_url) {
            Ok(()) => {
                self.status = "Opened Terms page in your browser.".to_string();
            }
            Err(error) => {
                self.status = error.to_string();
                self.wallet_terms.issue = Some(error.to_string());
            }
        }
    }

    pub(super) fn begin_wallet_switch(&mut self) {
        self.wallet_switch_editing = true;
        self.wallet_switch_input.clear();
        self.status = "Type a keypair path, then press Enter.".to_string();
    }

    pub(super) fn cancel_wallet_switch(&mut self) {
        self.wallet_switch_editing = false;
        self.wallet_switch_input.clear();
        self.status = "Wallet switch cancelled.".to_string();
    }

    pub(super) fn push_wallet_switch_char(&mut self, character: char) {
        if !character.is_control() {
            self.wallet_switch_input.push(character);
        }
    }

    pub(super) fn backspace_wallet_switch_input(&mut self) {
        self.wallet_switch_input.pop();
    }

    pub(super) fn switch_wallet_from_input(&mut self) {
        self.clear_writer_action_mask_check();
        let return_to_ledger = self.screen == LabScreen::Ledger;
        let keypair_path = normalize_keypair_path_input(&self.wallet_switch_input);
        if keypair_path.is_empty() {
            self.status = "Type a keypair path before pressing Enter.".to_string();
            return;
        }

        let wallet = attached_wallet::inspect_keypair_path(keypair_path);
        self.onchain_config.keypair_path = Some(wallet.keypair_path.clone());
        self.wallet_terms = wallet_terms::status_for_wallet(wallet.pubkey.as_deref());
        self.wallet = wallet;
        self.ledger = None;
        self.ledger_position_selected = 0;
        self.ledger_history_selected = 0;
        self.liquidity_preview_form = None;
        self.liquidity_preview_result = None;
        self.liquidity_preview_request = self.liquidity_preview_request.wrapping_add(1);
        self.liquidity_preview_inflight = None;
        self.writer_action_result = None;
        self.staking_status = None;
        self.staking_issue = None;
        self.staking_form = None;
        self.staking_confirmation = None;
        self.staking_action_result = None;
        self.oracle_rewards = None;
        self.oracle_reward_issue = None;
        self.wallet_switch_editing = false;
        self.wallet_switch_input.clear();

        if self.wallet.issue.is_some() {
            self.status = "Wallet could not be attached from that local signing source. Check that it exists, is readable, and is supported."
                .to_string();
            return;
        }
        if let Some(pubkey) = self.wallet.pubkey.as_deref() {
            if self.wallet_terms.accepted {
                self.screen = if return_to_ledger {
                    LabScreen::Ledger
                } else {
                    LabScreen::Home
                };
                self.focus = default_focus_for_screen(self.screen);
                self.status = format!("Wallet switched to {}.", short_pubkey(pubkey));
            } else {
                self.screen = LabScreen::Terms;
                self.focus = default_focus_for_screen(self.screen);
                self.status = format!(
                    "Wallet switched to {}. Review Terms to continue.",
                    short_pubkey(pubkey)
                );
            }
        } else {
            self.status = "Wallet could not be attached.".to_string();
        }
    }

    pub(super) fn selected_quote(&self) -> Option<&OptionQuote> {
        self.detail
            .as_ref()
            .and_then(|detail| detail.option_quotes.get(self.selected_option))
    }

    pub(super) fn selected_chart_expiry(&self) -> Option<&ExpirySummary> {
        self.detail.as_ref().and_then(|detail| {
            detail
                .expiries
                .get(self.chart_expiry)
                .or_else(|| detail.expiries.first())
        })
    }

    pub(super) fn chart_args(&self) -> ChartArgs {
        let launch = self.chart_launch_options.as_ref();
        ChartArgs {
            market: self.selected_id(),
            expiry: self.selected_chart_expiry().map(|expiry| expiry.id.clone()),
            range: self.chart_range,
            refresh_seconds: launch.map(|options| options.refresh_seconds).unwrap_or(0),
            static_view: true,
            points: launch.map(|options| options.points).unwrap_or(240),
            height: launch.map(|options| options.height).unwrap_or(14),
        }
    }

    pub(super) fn chart_auto_refresh_due_at(&self, now: Instant) -> bool {
        let refresh_seconds = self.chart_args().refresh_seconds;
        self.screen == LabScreen::Chart
            && refresh_seconds > 0
            && self.detail.is_some()
            && !self.loading_chart
            && self.chart_last_refresh_at.is_some_and(|last_refresh| {
                now.saturating_duration_since(last_refresh) >= Duration::from_secs(refresh_seconds)
            })
    }

    pub(super) fn refresh_chart_if_due_at(
        &mut self,
        now: Instant,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.chart_auto_refresh_due_at(now) {
            self.request_chart(backend_url, fetch_tx, true);
        }
    }

    pub(super) fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        if let Some(ticks_remaining) = self.help_hover_grace_ticks.take() {
            if ticks_remaining <= 1 {
                self.clear_help_hover_preview();
                self.help_glossary_hover = None;
            } else {
                self.help_hover_grace_ticks = Some(ticks_remaining.saturating_sub(1));
            }
        }
        if let Some(flash) = self.trade_ticket_field_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.trade_ticket_field_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.trade_ticket_field_flash = None;
                }
            }
        }
        if let Some(flash) = self.oracle_form_field_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.oracle_form_field_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.oracle_form_field_flash = None;
                }
            }
        }
        if let Some(flash) = self.oracle_locked_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.oracle_locked_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.oracle_locked_flash = None;
                }
            }
        }
        if self
            .help_transition_tick
            .is_some_and(|started| self.spinner_tick.saturating_sub(started) > 8)
        {
            self.help_transition_tick = None;
        }
    }

    pub(super) fn update_terminal_size_warning(&mut self, cli: &Cli, root: Rect) {
        if terminal_size_hides_essential_content(root, cli, self) {
            if self.terminal_size_warning_started_tick.is_none() {
                self.terminal_size_warning_started_tick = Some(self.spinner_tick);
            }
        } else {
            self.terminal_size_warning_started_tick = None;
        }
    }

    pub(super) fn is_loading(&self) -> bool {
        self.loading_list
            || self.loading_detail
            || self.loading_settlement
            || self.loading_chart
            || self.loading_ledger
            || self.loading_staking
            || self.loading_oracle_tree
            || self.loading_oracle_live
            || self.loading_help_index
            || self.loading_help_page
            || self.loading_help_preview_page
            || self.loading_update_check
            || self.guide.loading
            || self.trade_submit_is_running()
            || self.staking_action_is_running()
            || self.writer_action_is_running()
            || self.writer_action_mask_is_loading()
    }

    pub(super) fn spinner(&self) -> &'static str {
        loading_spinner(self.spinner_tick)
    }

    pub(super) fn trade_submit_is_running(&self) -> bool {
        self.trade_submit_inflight.is_some()
    }

    pub(super) fn trade_confirmation_is_open(&self) -> bool {
        self.trade_ticket
            .as_ref()
            .is_some_and(|ticket| ticket.confirmation.is_some())
    }

    pub(super) fn trade_result_modal_is_open(&self) -> bool {
        self.trade_result_modal.is_some()
    }

    pub(super) fn dismiss_trade_result_modal(&mut self) {
        self.trade_result_modal = None;
    }

    pub(super) fn expire_trade_result_modal_at(&mut self, now: Instant) {
        if self
            .trade_result_modal
            .as_ref()
            .is_some_and(|modal| modal.is_expired_at(now))
        {
            self.trade_result_modal = None;
        }
    }

    pub(super) fn handle_trade_result_modal_key(&mut self, key: &KeyEvent) -> bool {
        if self.trade_result_modal_is_open() {
            if key.code == KeyCode::Esc && key.kind != KeyEventKind::Release {
                self.dismiss_trade_result_modal();
                self.suppress_trade_result_escape_repeat = true;
            }
            return true;
        }

        if !self.suppress_trade_result_escape_repeat {
            return false;
        }
        if key.code != KeyCode::Esc {
            self.suppress_trade_result_escape_repeat = false;
            return false;
        }
        match key.kind {
            KeyEventKind::Repeat => true,
            KeyEventKind::Release => {
                self.suppress_trade_result_escape_repeat = false;
                true
            }
            KeyEventKind::Press => {
                self.suppress_trade_result_escape_repeat = false;
                false
            }
        }
    }

    pub(super) fn update_exit_action(&self) -> Option<LabExitAction> {
        let report = self.update_report.as_ref()?;
        if report.blocked {
            return None;
        }
        match report.status {
            workspace_update::UpdateStatus::UpdateAvailable => Some(LabExitAction::RunUpdate),
            workspace_update::UpdateStatus::RebuildAvailable => Some(LabExitAction::RunRebuild),
            workspace_update::UpdateStatus::Current | workspace_update::UpdateStatus::Blocked => {
                None
            }
        }
    }

    pub(super) fn request_update_check(&mut self, fetch_tx: &Sender<LabFetchResult>, force: bool) {
        if !self.update_check_enabled {
            if force {
                self.status = format!(
                    "Petri update checks are off. Unset {PETRI_UPDATE_CHECK_ENV} or run `petri update check`."
                );
            }
            return;
        }
        if self.loading_update_check {
            if force {
                self.status = "Petri is already checking for updates...".to_string();
            }
            return;
        }
        if !force && (self.update_report.is_some() || self.update_issue.is_some()) {
            return;
        }

        self.update_check_request = self.update_check_request.wrapping_add(1);
        self.loading_update_check = true;
        self.update_check_forced = force;
        self.update_issue = None;
        if force {
            self.status = "Checking for Petri updates...".to_string();
        }
        spawn_update_check(fetch_tx.clone(), self.update_check_request);
    }
}
