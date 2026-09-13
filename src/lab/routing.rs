//! Cross-feature screen entry, chart launch, MCP state, and top-level routing.

use super::*;

impl LabApp {
    pub(super) fn open_chart(&mut self, backend_url: &str, fetch_tx: &Sender<LabFetchResult>) {
        self.set_screen(LabScreen::Chart);
        self.request_chart(backend_url, fetch_tx, false);
    }

    pub(super) fn prepare_initial_chart(&mut self, options: &ChartArgs) {
        self.trading.chart_launch_options = Some(options.clone());
        self.trading.chart_range = options.range;
        self.trading.initial_chart_expiry = options.expiry.clone();
        self.trading.chart_last_refresh_at = None;
        if self.screen == LabScreen::Terms {
            self.trading.pending_initial_chart = true;
            return;
        }
        self.trading.pending_initial_chart = false;
        self.enter_initial_chart();
    }

    pub(super) fn enter_initial_chart(&mut self) {
        let (market, range) = self
            .trading
            .chart_launch_options
            .as_ref()
            .map(|options| (options.market.to_uppercase(), options.range))
            .unwrap_or_else(|| (self.selected_id().to_uppercase(), self.trading.chart_range));
        self.set_screen(LabScreen::Chart);
        self.status = format!("Loading {} {} chart...", market, range.label());
    }

    pub(super) fn resume_pending_initial_chart(&mut self) -> bool {
        if !self.trading.pending_initial_chart || !self.wallet_terms.accepted {
            return false;
        }
        self.trading.pending_initial_chart = false;
        self.enter_initial_chart();
        true
    }

    pub(super) fn apply_initial_chart_expiry(&mut self) {
        let Some(expiry_id) = self.trading.initial_chart_expiry.take() else {
            return;
        };
        if let Some(index) = self.trading.detail.as_ref().and_then(|detail| {
            detail
                .expiries
                .iter()
                .position(|expiry| expiry.id.eq_ignore_ascii_case(&expiry_id))
        }) {
            self.trading.chart_expiry = index;
        }
    }

    pub(super) fn open_home(&mut self) {
        self.oracle.search_editing = false;
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
        self.oracle.locked_flash = None;
        self.set_screen(LabScreen::Home);
        self.status = "Home".to_string();
    }

    pub(super) fn open_home_help(
        &mut self,
        topic: HomeHelpTopic,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.help.preview = None;
        self.home_help_topic = topic;
        if topic == HomeHelpTopic::Overview {
            self.prepare_gitbook_help();
        }
        if topic == HomeHelpTopic::Agents {
            self.refresh_mcp_connection();
        }
        self.set_screen(LabScreen::Help);
        self.status = match topic {
            HomeHelpTopic::Overview => "Amoeba Farm help".to_string(),
            HomeHelpTopic::Agents => "AI agent connection".to_string(),
        };
        if topic == HomeHelpTopic::Overview {
            self.request_help_index(fetch_tx, false);
        }
    }

    pub(super) fn refresh_mcp_connection(&mut self) {
        match mcp_setup::status() {
            Ok(status) => {
                self.mcp_connection_state = McpConnectionState::from_setup_status(&status);
                self.mcp_managed_entry_enabled = status.has_enabled_managed_entry();
                self.mcp_connection_issue = None;
                self.mcp_repair_failed = false;
            }
            Err(error) => {
                self.mcp_connection_state = McpConnectionState::NeedsRepair;
                self.mcp_managed_entry_enabled = false;
                self.mcp_connection_issue = Some(format!(
                    "Petri could not safely read your agent settings: {error}"
                ));
                self.mcp_repair_failed = false;
            }
        }
    }

    pub(super) fn mcp_connection_action(&self) -> McpConnectionAction {
        match self.mcp_connection_state {
            McpConnectionState::Disabled => McpConnectionAction::Enable,
            McpConnectionState::Enabled => McpConnectionAction::Disable,
            McpConnectionState::NeedsRepair => McpConnectionAction::Repair,
            McpConnectionState::Conflict if self.mcp_managed_entry_enabled => {
                McpConnectionAction::Disable
            }
            McpConnectionState::Conflict => McpConnectionAction::Blocked,
        }
    }

    pub(super) fn toggle_mcp_connection(&mut self) {
        let action = self.mcp_connection_action();
        if action == McpConnectionAction::Blocked {
            self.status = "Petri left the existing connection unchanged.".to_string();
            return;
        }
        let result = match action {
            McpConnectionAction::Enable => mcp_setup::enable().map(|status| {
                (
                    status,
                    "Petri MCP enabled. Restart or reload an AI agent that is already open."
                        .to_string(),
                )
            }),
            McpConnectionAction::Disable => mcp_setup::disable().map(|status| {
                let message = if status.has_conflict() {
                    "Petri disabled the connection it created. Another connection was left unchanged."
                } else {
                    "Petri MCP disabled. Restart or reload an AI agent that is already open."
                };
                (status, message.to_string())
            }),
            McpConnectionAction::Repair => mcp_setup::repair().map(|repair| {
                let message = match repair.outcome {
                    mcp_setup::McpRepairOutcome::AlreadyHealthy => {
                        "Connection check complete. Petri MCP is healthy; no settings were changed."
                    }
                    mcp_setup::McpRepairOutcome::Repaired => {
                        "Repair complete. Petri MCP is healthy. Restart or reload an AI agent that is already open. Wallet execution requires explicit approval."
                    }
                };
                (repair.status, message.to_string())
            }),
            McpConnectionAction::Blocked => unreachable!("blocked action returned above"),
        };
        match result {
            Ok((status, message)) => {
                self.mcp_connection_state = McpConnectionState::from_setup_status(&status);
                self.mcp_managed_entry_enabled = status.has_enabled_managed_entry();
                self.mcp_connection_issue = None;
                self.mcp_repair_failed = false;
                self.status = message;
            }
            Err(error) => {
                self.mcp_repair_failed = action == McpConnectionAction::Repair;
                self.mcp_connection_issue = Some(if self.mcp_repair_failed {
                    format!("Repair could not finish safely: {error}")
                } else {
                    format!("Petri could not finish safely: {error}")
                });
                self.status = if self.mcp_repair_failed {
                    "Repair stopped safely. Petri did not disconnect the existing connection."
                        .to_string()
                } else {
                    "Petri left the existing agent settings unchanged.".to_string()
                };
            }
        }
    }

    pub(super) fn mcp_connection_button_active(&self) -> bool {
        self.mcp_connection_action() != McpConnectionAction::Blocked
    }

    pub(super) fn open_oracle_intro(&mut self) {
        self.oracle.search_editing = false;
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
        self.oracle.locked_flash = None;
        self.set_screen(LabScreen::OracleIntro);
        self.status = "Oracle evidence entry".to_string();
    }

    pub(super) fn open_oracle_help(&mut self) {
        self.oracle.search_editing = false;
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
        self.oracle.locked_flash = None;
        self.set_screen(LabScreen::OracleHelp);
        self.status = "Oracle help".to_string();
    }

    pub(super) fn open_oracle(&mut self, backend_url: &str, fetch_tx: &Sender<LabFetchResult>) {
        self.open_oracle_view(OracleView::Advanced, backend_url, fetch_tx);
    }

    pub(super) fn open_oracle_earn(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.open_oracle_view(OracleView::Earn, backend_url, fetch_tx);
    }

    pub(super) fn open_oracle_view(
        &mut self,
        view: OracleView,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.oracle.form.is_some() && self.oracle.view != view {
            self.status =
                "Finish or cancel the open Oracle form before switching views.".to_string();
            return;
        }
        if self.oracle.view != view {
            self.clear_guide_context_actions();
        }
        self.oracle.view = view;
        self.set_screen(LabScreen::Oracle);
        self.set_focus(match view {
            OracleView::Earn => LabFocus::OracleEarn,
            OracleView::Advanced => LabFocus::OracleTasks,
        });
        self.clamp_oracle_selection();
        self.request_oracle_tree(backend_url, fetch_tx, false);
        self.request_oracle_live(backend_url, fetch_tx, false);
        self.request_oracle_rewards(backend_url, fetch_tx, false);
        if !self.oracle.loading_tree {
            self.status = match view {
                OracleView::Earn => {
                    "Earn shows funded rewards for the selected market and month.".to_string()
                }
                OracleView::Advanced => format!(
                    "{} oracle work. Locked actions are skipped.",
                    self.oracle_phase().label()
                ),
            };
        }
    }
}
