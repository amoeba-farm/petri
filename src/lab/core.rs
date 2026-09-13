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
        let help = help_state::HelpState::new(gitbook::bundled_index());
        let cache = read_cache::TuiCache::new(
            &onchain_config.backend_url,
            &onchain_config.network,
            help.index
                .first_page()
                .and_then(gitbook::bundled_page)
                .map(|page| (help.selected_page_id.clone(), page)),
        );
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
            trading: trade_state::TradeState::new(dishes, selected, pending_initial_dish),
            home_selected: 0,
            home_help_topic: HomeHelpTopic::Overview,
            help,
            mcp_connection_state: McpConnectionState::Disabled,
            mcp_managed_entry_enabled: false,
            mcp_connection_issue: None,
            mcp_repair_failed: false,
            guide,
            oracle: oracle_state::OracleState::new(
                oracle_submission_store_path,
                oracle_submissions,
                oracle_submission_issue,
            ),
            updates: updates::UpdateState::new(update_check_enabled),
            focus: default_focus_for_screen(screen),
            read_panel: None,
            read_panel_request: 0,
            action_panel: None,
            action_panel_request: 0,
            left_mouse_down: false,
            confirmation_mouse_press: None,
            ledger: None,
            ledger_view: LedgerView::Account,
            ledger_pane: LedgerPane::Tabs,
            ledger_account_action_selected: 0,
            ledger_position_selected: 0,
            ledger_writer_selected: 0,
            ledger_history_selected: 0,
            liquidity_preview_form: None,
            liquidity_preview_result: None,
            writers: writer_state::WriterState::new(),
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
            cache,
            panel_scrolls: HashMap::new(),
            ledger_request: 0,
            liquidity_preview_request: 0,
            liquidity_preview_inflight: None,
            staking_status_request: 0,
            staking_action_request: 0,
            staking_action_inflight: None,
            loading_ledger: false,
            loading_staking: false,
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
        self.trading
            .dishes
            .get(self.trading.selected)
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
        self.writers.clear_action_mask_check();
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
        self.writers.action_result = None;
        self.staking_status = None;
        self.staking_issue = None;
        self.staking_form = None;
        self.staking_confirmation = None;
        self.staking_action_result = None;
        self.oracle.rewards = None;
        self.oracle.reward_issue = None;
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
        self.trading
            .detail
            .as_ref()
            .and_then(|detail| detail.option_quotes.get(self.trading.selected_option))
    }

    pub(super) fn selected_chart_expiry(&self) -> Option<&ExpirySummary> {
        self.trading.detail.as_ref().and_then(|detail| {
            detail
                .expiries
                .get(self.trading.chart_expiry)
                .or_else(|| detail.expiries.first())
        })
    }

    pub(super) fn chart_args(&self) -> ChartArgs {
        let launch = self.trading.chart_launch_options.as_ref();
        ChartArgs {
            market: self.selected_id(),
            expiry: self.selected_chart_expiry().map(|expiry| expiry.id.clone()),
            range: self.trading.chart_range,
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
            && self.trading.detail.is_some()
            && !self.trading.loading_chart
            && self
                .trading
                .chart_last_refresh_at
                .is_some_and(|last_refresh| {
                    now.saturating_duration_since(last_refresh)
                        >= Duration::from_secs(refresh_seconds)
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
        if let Some(ticks_remaining) = self.help.hover_grace_ticks.take() {
            if ticks_remaining <= 1 {
                self.help.clear_hover_preview();
                self.help.glossary_hover = None;
            } else {
                self.help.hover_grace_ticks = Some(ticks_remaining.saturating_sub(1));
            }
        }
        self.trading.tick_field_flash();
        self.oracle.tick_flashes();
        if self
            .help
            .transition_tick
            .is_some_and(|started| self.spinner_tick.saturating_sub(started) > 8)
        {
            self.help.transition_tick = None;
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
        self.trading.loading_list
            || self.trading.loading_detail
            || self.trading.loading_settlement
            || self.trading.loading_chart
            || self.loading_ledger
            || self.loading_staking
            || self.oracle.loading_tree
            || self.oracle.loading_live
            || self.help.loading_index
            || self.help.loading_page
            || self.help.loading_preview_page
            || self.updates.is_loading()
            || self.guide.loading
            || self.trading.submit_is_running()
            || self.staking_action_is_running()
            || self.writers.action_is_running()
            || self.writers.action_mask_is_loading()
    }

    pub(super) fn spinner(&self) -> &'static str {
        loading_spinner(self.spinner_tick)
    }

    pub(super) fn request_update_check(&mut self, fetch_tx: &Sender<LabFetchResult>, force: bool) {
        let request = self.updates.begin(force);
        if let Some(status) = request.status {
            self.status = status;
        }
        if let Some(request_id) = request.request_id {
            spawn_update_check(fetch_tx.clone(), request_id);
        }
    }
}
