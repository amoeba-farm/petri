//! Bounded in-TUI Guide projection, target validation, and staged command application.

use super::*;
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub(super) struct GuidePendingContinuation {
    pub(super) expected_screen: LabScreen,
    pub(super) market_id: Option<String>,
    pub(super) tool_result: guide::GuideToolResult,
}

#[derive(Clone, Debug)]
pub(super) struct GuidePanelState {
    pub(super) config: guide::GuideConfig,
    pub(super) provider_status: guide::GuideProviderStatus,
    pub(super) selected_provider: usize,
    pub(super) input: String,
    pub(super) composing: bool,
    pub(super) context_focus: Option<LabFocus>,
    pub(super) messages: VecDeque<guide::GuideConversationTurn>,
    pub(super) scroll: usize,
    pub(super) request_id: u64,
    pub(super) loading: bool,
    pub(super) progress: Option<String>,
    pub(super) suggested_actions: Vec<String>,
    pub(super) selected_suggestion: Option<usize>,
    pub(super) action_preview: Option<guide::GuideActionPreview>,
    pub(super) highlighted_targets: Vec<String>,
    pub(super) comparison_targets: Vec<String>,
    pub(super) focused_control: Option<String>,
    pub(super) request_target_ids: HashSet<String>,
    pub(super) request_ui_target_ids: HashSet<String>,
    pub(super) request_challenge_ids: HashSet<String>,
    pub(super) active_question: Option<String>,
    pub(super) allow_continuation: bool,
    pub(super) tool_step: u8,
    pub(super) pending_continuation: Option<GuidePendingContinuation>,
}

impl GuidePanelState {
    pub(super) fn new(config: guide::GuideConfig) -> Self {
        let provider_status = if let Some(issue) = config.issue.clone() {
            guide::GuideProviderStatus::SetupRequired { message: issue }
        } else if config.provider == guide::GuideProviderPreference::Off {
            guide::GuideProviderStatus::Off
        } else {
            guide::GuideProviderStatus::Checking
        };
        Self {
            config,
            provider_status,
            selected_provider: 0,
            input: String::new(),
            composing: false,
            context_focus: None,
            messages: VecDeque::new(),
            scroll: 0,
            request_id: 0,
            loading: false,
            progress: None,
            suggested_actions: Vec::new(),
            selected_suggestion: None,
            action_preview: None,
            highlighted_targets: Vec::new(),
            comparison_targets: Vec::new(),
            focused_control: None,
            request_target_ids: HashSet::new(),
            request_ui_target_ids: HashSet::new(),
            request_challenge_ids: HashSet::new(),
            active_question: None,
            allow_continuation: false,
            tool_step: 0,
            pending_continuation: None,
        }
    }

    pub(super) fn push_message(
        &mut self,
        role: guide::GuideConversationRole,
        text: impl Into<String>,
    ) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.messages
            .push_back(guide::GuideConversationTurn { role, text });
        while self.messages.len() > 16 {
            self.messages.pop_front();
        }
        self.scroll = 0;
    }

    pub(super) fn conversation_context(&self) -> Vec<guide::GuideConversationTurn> {
        self.messages
            .iter()
            .rev()
            .take(4)
            .cloned()
            .map(|mut turn| {
                turn.text = turn.text.chars().take(600).collect();
                turn
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

impl LabApp {
    pub(super) fn request_guide_probe(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        if let Some(issue) = self.guide.config.issue.clone() {
            self.guide.provider_status =
                guide::GuideProviderStatus::SetupRequired { message: issue };
            return;
        }
        if self.guide.config.provider == guide::GuideProviderPreference::Off {
            self.guide.provider_status = guide::GuideProviderStatus::Off;
            return;
        }
        self.guide.request_id = self.guide.request_id.wrapping_add(1);
        self.guide.provider_status = guide::GuideProviderStatus::Checking;
        spawn_guide_probe(
            self.guide.config.clone(),
            fetch_tx.clone(),
            self.guide.request_id,
        );
    }

    pub(super) fn begin_guide_input(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if matches!(
            self.guide.provider_status,
            guide::GuideProviderStatus::SetupRequired { .. }
        ) {
            // A newly installed CLI or completed sign-in should not require a
            // TUI restart. The existing request id discards stale probe results.
            self.guide.config = guide::GuideConfig::load();
            self.request_guide_probe(fetch_tx);
        }
        if self.focus != LabFocus::Guide {
            self.guide.context_focus = Some(self.focus);
        }
        self.focus = LabFocus::Guide;
        self.guide.composing = !matches!(
            self.guide.provider_status,
            guide::GuideProviderStatus::Choose { .. }
        );
        if self.screen == LabScreen::Oracle
            && self.oracle_tree().is_none()
            && !self.oracle.loading_tree
        {
            self.request_oracle_tree(backend_url, fetch_tx, false);
        }
        self.status = match &self.guide.provider_status {
            guide::GuideProviderStatus::Connected(_) => {
                "Ask the Guide about this screen. Enter sends; Esc leaves the chat.".to_string()
            }
            status => status.panel_message(),
        };
    }

    pub(super) fn cancel_guide_input(&mut self) {
        self.guide.composing = false;
        self.focus = self
            .guide
            .context_focus
            .take()
            .filter(|focus| *focus != LabFocus::Guide)
            .unwrap_or_else(|| default_focus_for_screen(self.screen));
        self.status = "Guide input closed. Press g to ask another question.".to_string();
    }

    pub(super) fn push_guide_input_char(&mut self, character: char) {
        if character.is_control() || self.guide.input.chars().count() >= 1000 {
            return;
        }
        self.guide.input.push(character);
        self.guide.selected_suggestion = None;
    }

    pub(super) fn backspace_guide_input(&mut self) {
        self.guide.input.pop();
        self.guide.selected_suggestion = None;
    }

    pub(super) fn move_guide_suggestion(&mut self, direction: isize) {
        let count = self.guide.suggested_actions.len();
        if count == 0 {
            return;
        }
        let next = match (self.guide.selected_suggestion, direction < 0) {
            (None, true) => count - 1,
            (None, false) => 0,
            (Some(index), true) => index.saturating_sub(1),
            (Some(index), false) => (index + 1).min(count - 1),
        };
        self.guide.selected_suggestion = Some(next);
        self.guide.input = self.guide.suggested_actions[next].clone();
    }

    pub(super) fn scroll_guide(&mut self, direction: isize) {
        if direction < 0 {
            self.guide.scroll = self.guide.scroll.saturating_add(PANEL_SCROLL_STEP);
        } else if direction > 0 {
            self.guide.scroll = self.guide.scroll.saturating_sub(PANEL_SCROLL_STEP);
        }
    }

    pub(super) fn bind_guide_request_targets(&mut self, snapshot: &guide::TuiStateSnapshot) {
        self.guide.request_target_ids = snapshot
            .navigation_targets
            .iter()
            .map(|target| target.node_id.clone())
            .chain(
                snapshot
                    .breadcrumb_path
                    .iter()
                    .map(|target| target.node_id.clone()),
            )
            .collect();
        self.guide.request_ui_target_ids = snapshot
            .available_targets
            .iter()
            .map(|target| target.target_id.clone())
            .chain(
                snapshot
                    .available_form_actions
                    .iter()
                    .map(|action| action.target_id.clone()),
            )
            .collect();
        if let Some(form) = snapshot.active_form.as_ref() {
            self.guide
                .request_ui_target_ids
                .insert(form.form_id.clone());
            self.guide
                .request_ui_target_ids
                .insert(form.final_control_id.clone());
        }
        self.guide.request_challenge_ids = snapshot
            .active_challenges
            .iter()
            .map(|challenge| challenge.challenge_id.clone())
            .collect();
    }

    pub(super) fn submit_guide_question(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        if self.guide.loading {
            self.status = "The Guide is still answering the previous question.".to_string();
            return;
        }
        let question = self.guide.input.trim().to_string();
        if question.is_empty() {
            self.status = "Type a question for the Guide first.".to_string();
            return;
        }
        let Some(connection) = self.guide.provider_status.connection().cloned() else {
            let message = self.guide.provider_status.panel_message();
            self.guide
                .push_message(guide::GuideConversationRole::Assistant, message.clone());
            self.guide.input.clear();
            self.guide.selected_suggestion = None;
            self.guide.composing = false;
            self.status = message;
            return;
        };

        let snapshot = self.guide_snapshot();
        self.guide.pending_continuation = None;
        self.guide.active_question = Some(question.clone());
        self.guide.allow_continuation = true;
        self.bind_guide_request_targets(&snapshot);
        let state_revision = snapshot.state_revision.clone();
        let conversation = self.guide.conversation_context();
        let request = guide::GuideRequest {
            mode: guide::GuideRequestMode::Answer,
            question: question.clone(),
            snapshot,
            conversation,
            tool_result: None,
        };
        self.guide
            .push_message(guide::GuideConversationRole::User, question);
        self.guide.input.clear();
        self.guide.selected_suggestion = None;
        self.guide.composing = false;
        self.guide.loading = true;
        self.guide.progress = Some("starting".to_string());
        self.guide.suggested_actions.clear();
        self.guide.selected_suggestion = None;
        self.guide.action_preview = None;
        self.guide.tool_step = 0;
        self.guide.request_id = self.guide.request_id.wrapping_add(1);
        self.status = format!(
            "{} is reading this screen...",
            connection.kind.display_name()
        );
        spawn_guide_request(
            connection,
            request,
            None,
            state_revision,
            fetch_tx.clone(),
            self.guide.request_id,
        );
    }

    pub(super) fn submit_guide_suggestion(
        &mut self,
        suggestion_index: usize,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let Some(suggestion) = self.guide.suggested_actions.get(suggestion_index).cloned() else {
            return;
        };
        self.focus = LabFocus::Guide;
        self.guide.selected_suggestion = Some(suggestion_index);
        self.guide.input = suggestion;
        self.submit_guide_question(fetch_tx);
    }

    pub(super) fn take_ready_guide_continuation(&mut self) -> Option<GuidePendingContinuation> {
        let Some(pending) = self.guide.pending_continuation.as_ref() else {
            return None;
        };
        if self.screen != pending.expected_screen || self.guide_destination_is_loading() {
            return None;
        }
        if let Some(market_id) = pending.market_id.as_deref()
            && (!self.selected_id().eq_ignore_ascii_case(market_id)
                || self
                    .trading
                    .detail
                    .as_ref()
                    .is_some_and(|detail| !detail.id.eq_ignore_ascii_case(market_id)))
        {
            return None;
        }
        self.guide.pending_continuation.take()
    }

    pub(super) fn guide_destination_is_loading(&self) -> bool {
        match self.screen {
            LabScreen::Chain | LabScreen::Activity => self.trading.loading_detail,
            LabScreen::Detail => {
                self.trading.loading_detail
                    || (self.trading.detail_view == DetailView::Settlement
                        && self.trading.loading_settlement)
            }
            LabScreen::Chart => self.trading.loading_detail || self.trading.loading_chart,
            LabScreen::Oracle => self.trading.loading_detail || self.oracle.loading_tree,
            LabScreen::Help if self.home_help_topic == HomeHelpTopic::Overview => {
                self.help.loading_index || self.help.loading_page
            }
            LabScreen::Ledger => self.loading_ledger,
            LabScreen::Staking => self.loading_staking || self.staking_action_is_running(),
            LabScreen::Terms
            | LabScreen::Home
            | LabScreen::OracleIntro
            | LabScreen::OracleHelp
            | LabScreen::Help => false,
        }
    }

    pub(super) fn continue_guide_after_navigation(
        &mut self,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        let Some(pending) = self.take_ready_guide_continuation() else {
            return false;
        };
        self.start_guide_tool_followup(pending.tool_result, fetch_tx)
    }

    pub(super) fn start_guide_tool_followup(
        &mut self,
        tool_result: guide::GuideToolResult,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        let Some(connection) = self.guide.provider_status.connection().cloned() else {
            self.guide.active_question = None;
            self.guide.allow_continuation = false;
            let message = if tool_result.ok {
                "The view changed, but the Guide connection ended before it could continue. Press g to ask again."
                    .to_string()
            } else {
                format!(
                    "The Guide connection ended before it could retry. {}",
                    tool_result.message
                )
            };
            self.guide
                .push_message(guide::GuideConversationRole::Assistant, message);
            return true;
        };
        let Some(question) = self.guide.active_question.clone() else {
            return false;
        };
        let snapshot = self.guide_snapshot();
        self.bind_guide_request_targets(&snapshot);
        let state_revision = snapshot.state_revision.clone();
        let request = guide::GuideRequest {
            mode: guide::GuideRequestMode::ToolResult,
            question: question.clone(),
            snapshot,
            conversation: self.guide.conversation_context(),
            tool_result: Some(tool_result.clone()),
        };
        self.guide.active_question = Some(question);
        self.guide.allow_continuation = true;
        self.guide.loading = true;
        self.guide.progress = Some(format!("checking {}", tool_result.command));
        self.guide.request_id = self.guide.request_id.wrapping_add(1);
        self.status = format!(
            "{} is continuing after {}...",
            connection.kind.display_name(),
            tool_result.command
        );
        spawn_guide_request(
            connection,
            request,
            None,
            state_revision,
            fetch_tx.clone(),
            self.guide.request_id,
        );
        true
    }

    pub(super) fn queue_guide_tool_followup(
        &mut self,
        question: String,
        tool_result: guide::GuideToolResult,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        self.guide.active_question = Some(question.clone());
        self.guide.allow_continuation = true;
        if tool_result.ok && self.guide_destination_is_loading() {
            let expected_screen = self.screen;
            let market_id = matches!(
                expected_screen,
                LabScreen::Chain
                    | LabScreen::Chart
                    | LabScreen::Oracle
                    | LabScreen::Detail
                    | LabScreen::Activity
            )
            .then(|| self.selected_id().to_string());
            self.guide.pending_continuation = Some(GuidePendingContinuation {
                expected_screen,
                market_id,
                tool_result,
            });
            true
        } else {
            self.start_guide_tool_followup(tool_result, fetch_tx)
        }
    }

    pub(super) fn handle_guide_input_key(
        &mut self,
        key: &KeyEvent,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.focus != LabFocus::Guide {
            return false;
        }
        if matches!(
            self.guide.provider_status,
            guide::GuideProviderStatus::Choose { .. }
        ) {
            match key.code {
                KeyCode::Esc => self.cancel_guide_input(),
                KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                    self.move_guide_provider_choice(-1)
                }
                KeyCode::Right | KeyCode::Down | KeyCode::Tab => self.move_guide_provider_choice(1),
                KeyCode::Enter => self.activate_guide_provider_choice(),
                KeyCode::Char(character) if character.is_ascii_digit() && character != '0' => {
                    let index = character.to_digit(10).unwrap_or(0) as usize - 1;
                    let count = self.guide_provider_choice_count();
                    if index >= count {
                        return false;
                    }
                    self.guide.selected_provider = index;
                    self.activate_guide_provider_choice();
                }
                _ => return false,
            }
            return true;
        }
        if !self.guide.composing {
            return false;
        }
        match key.code {
            KeyCode::Esc => self.cancel_guide_input(),
            KeyCode::Enter => self.submit_guide_question(fetch_tx),
            KeyCode::Up => self.move_guide_suggestion(-1),
            KeyCode::Down => self.move_guide_suggestion(1),
            KeyCode::Backspace => self.backspace_guide_input(),
            KeyCode::Tab | KeyCode::BackTab => self.cancel_guide_input(),
            KeyCode::Char(character) => self.push_guide_input_char(character),
            _ => return false,
        }
        true
    }

    pub(super) fn guide_provider_choice_count(&self) -> usize {
        match &self.guide.provider_status {
            guide::GuideProviderStatus::Choose { providers } => providers.len(),
            _ => 0,
        }
    }

    pub(super) fn move_guide_provider_choice(&mut self, direction: isize) {
        let count = self.guide_provider_choice_count();
        if count == 0 {
            self.guide.selected_provider = 0;
            return;
        }
        self.guide.selected_provider = if direction < 0 {
            (self.guide.selected_provider + count - 1) % count
        } else {
            (self.guide.selected_provider + 1) % count
        };
        if let guide::GuideProviderStatus::Choose { providers } = &self.guide.provider_status
            && let Some(connection) = providers.get(self.guide.selected_provider)
        {
            self.status = format!(
                "{} selected. Press Enter to use it for Guide questions.",
                connection.kind.display_name()
            );
        }
    }

    pub(super) fn activate_guide_provider_choice(&mut self) {
        let connection = match &self.guide.provider_status {
            guide::GuideProviderStatus::Choose { providers } => providers
                .get(
                    self.guide
                        .selected_provider
                        .min(providers.len().saturating_sub(1)),
                )
                .cloned(),
            _ => None,
        };
        let Some(connection) = connection else {
            self.status = "No signed-in Guide is available right now.".to_string();
            return;
        };
        let name = connection.kind.display_name();
        self.guide.provider_status = guide::GuideProviderStatus::Connected(connection);
        self.guide.composing = true;
        self.status =
            format!("{name} selected. Ask about this screen; Enter sends and Esc leaves the chat.");
    }

    pub(super) fn guide_snapshot(&self) -> guide::TuiStateSnapshot {
        let tree = (self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Advanced)
            .then(|| self.oracle_tree())
            .flatten();
        let selected_index = tree.map(|tree| {
            self.oracle
                .node_selected
                .min(tree.nodes.len().saturating_sub(1))
        });
        let breadcrumb_path = tree
            .zip(selected_index)
            .map(|(tree, index)| {
                oracle_path_indices(tree, index)
                    .into_iter()
                    .filter_map(|index| guide_node_snapshot(tree, index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let highlighted_node = tree
            .zip(selected_index)
            .and_then(|(tree, index)| guide_node_snapshot(tree, index));
        let highlighted_source = highlighted_node
            .as_ref()
            .filter(|node| node.kind == "source")
            .cloned();
        let visible_contracts = if self.screen == LabScreen::Terms {
            Vec::new()
        } else {
            self.guide_visible_contracts()
        };
        let mut navigation_targets = if self.screen == LabScreen::Terms {
            Vec::new()
        } else {
            self.trading
                .dishes
                .iter()
                .map(|dish| guide::GuideNodeSnapshot {
                    node_id: guide_market_target_id(&dish.id),
                    label: format!("{} — {}", dish.symbol, dish.title),
                    kind: "market".to_string(),
                    path: format!("markets / {}", dish.symbol),
                    description: Some(
                        "Open this market's fixed-risk monthly contracts.".to_string(),
                    ),
                    weight_pct: None,
                })
                .collect::<Vec<_>>()
        };
        navigation_targets.extend(visible_contracts.iter().map(|contract| {
            guide::GuideNodeSnapshot {
                node_id: contract.contract_id.clone(),
                label: format!("{} {} {}", contract.month, contract.kind, contract.range),
                kind: "contract".to_string(),
                path: format!(
                    "markets / {} / {} / {} {}",
                    contract.market_id.to_uppercase(),
                    contract.month,
                    contract.kind,
                    contract.range
                ),
                description: Some(if contract.executable {
                    "Quoted on-chain contract with visible depth; selection is inspection only."
                        .to_string()
                } else {
                    "Informational contract; it is not currently executable with visible depth."
                        .to_string()
                }),
                weight_pct: None,
            }
        }));
        if let Some(tree) = tree {
            let breadcrumb_ids = breadcrumb_path
                .iter()
                .map(|node| node.node_id.as_str())
                .collect::<HashSet<_>>();
            navigation_targets.extend(
                guide_oracle_context_indices(
                    tree,
                    selected_index.unwrap_or_default(),
                    &self.oracle.search_input,
                )
                .into_iter()
                .filter_map(|index| guide_node_snapshot(tree, index))
                .filter(|node| !breadcrumb_ids.contains(node.node_id.as_str())),
            );
        }
        navigation_targets.truncate(64);
        let phase = self.oracle_phase();
        let active_phase_index = OraclePhase::ALL
            .iter()
            .position(|candidate| *candidate == phase);
        let visible_phase_timeline =
            if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Advanced {
                OraclePhase::ALL
                    .iter()
                    .enumerate()
                    .map(|(index, phase)| guide::GuidePhaseSnapshot {
                        phase_id: oracle_phase_id(*phase).to_string(),
                        label: guide_phase_label(*phase).to_string(),
                        state: match active_phase_index {
                            Some(active) if index < active => "complete",
                            Some(active) if index == active => "active",
                            _ => "upcoming",
                        }
                        .to_string(),
                        summary: guide_phase_summary(*phase).to_string(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let active_challenges =
            if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Advanced {
                self.guide_active_challenges()
            } else {
                Vec::new()
            };
        let mut available_safe_actions = vec![
            guide_safe_action(
                "explain_current_screen",
                "Explain what is visible now",
                "read",
            ),
            guide_safe_action("show_phase_timeline", "Show the market phase", "navigate"),
            guide_safe_action("show_next_action", "Explain the next safe step", "read"),
        ];
        if !self.trading.dishes.is_empty() {
            available_safe_actions.push(guide_safe_action(
                "open_market_contracts",
                "Open a market and load its contracts",
                "navigate",
            ));
        }
        if !visible_contracts.is_empty() {
            available_safe_actions.push(guide_safe_action(
                "open_contract",
                "Select a contract for inspection",
                "navigate",
            ));
        }
        if tree.is_some() {
            available_safe_actions.extend([
                guide_safe_action("go_to_bucket", "Open a product bucket", "navigate"),
                guide_safe_action("highlight_source", "Highlight a source", "navigate"),
                guide_safe_action("open_source_detail", "Open source detail", "navigate"),
                guide_safe_action("compare_sources", "Compare two sources", "navigate"),
            ]);
        }
        if !active_challenges.is_empty() {
            available_safe_actions.push(guide_safe_action(
                "open_challenge_view",
                "Explain an active challenge",
                "navigate",
            ));
        }
        available_safe_actions.push(guide_safe_action(
            "open_target",
            "Open an offered Petri screen or view",
            "navigate",
        ));
        available_safe_actions.push(guide_safe_action(
            "focus_control",
            "Point to a control the user must activate",
            "navigate",
        ));
        if self.screen == LabScreen::Chain {
            available_safe_actions.push(guide_safe_action(
                "stage_trade",
                "Open and fill a reviewable trade ticket",
                "stage",
            ));
        }
        if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Advanced {
            available_safe_actions.push(guide_safe_action(
                "stage_oracle_form",
                "Open and fill a reviewable evidence form",
                "stage",
            ));
            available_safe_actions.push(guide_safe_action(
                "search_oracle",
                "Search the local source tree",
                "navigate",
            ));
        }
        let writer_form_available = self.screen == LabScreen::Ledger
            && self.ledger_view == LedgerView::Writers
            && self.writers.form.is_none()
            && self.writers.confirmation.is_none()
            && !self.writers.interaction_is_locked()
            && self.selected_writer_sleeve_address().is_some();
        let liquidity_form_available = self.screen == LabScreen::Ledger
            && self.ledger_view == LedgerView::Positions
            && self.wallet.is_attached()
            && self.liquidity_preview_form.is_none()
            && !self.liquidity_preview_is_running();
        if writer_form_available || liquidity_form_available {
            available_safe_actions.push(guide_safe_action(
                "stage_action_form",
                if writer_form_available {
                    "Open and fill a reviewable writer form"
                } else {
                    "Open and fill an unsigned liquidity preview form"
                },
                "stage",
            ));
        }
        if self
            .guide_active_form()
            .is_some_and(|form| form.fields.iter().any(|field| field.editable_by_guide))
        {
            available_safe_actions.push(guide_safe_action(
                "fill_active_form",
                "Fill editable fields in the open form",
                "stage",
            ));
        }
        if self.screen == LabScreen::Terms {
            available_safe_actions.retain(|action| {
                matches!(
                    action.command.as_str(),
                    "explain_current_screen" | "show_next_action" | "focus_control"
                )
            });
        }
        available_safe_actions.truncate(16);

        let selected_market = self.trading.dishes.get(self.trading.selected).map(|dish| {
            let detail = self
                .trading
                .detail
                .as_ref()
                .filter(|detail| detail.id.eq_ignore_ascii_case(&dish.id));
            guide::GuideMarketSnapshot {
                market_id: dish.id.clone(),
                symbol: dish.symbol.clone(),
                title: dish.title.clone(),
                month: detail.map(|detail| detail.expiry_label.clone()),
                settlement: detail.map(|detail| detail.settlement.clone()),
                phase: Some(guide_phase_label(phase).to_string()),
                current_print: detail.and_then(|detail| {
                    is_known_value(&detail.current_print).then(|| detail.current_print.clone())
                }),
                starting_index: detail.map(|detail| detail.base.clone()),
                days_to_settlement: detail.map(|detail| detail.days.clone()),
                cap_width: detail.map(|detail| detail.cap_width.clone()),
                freshness: detail.map(|detail| detail.freshness.clone()),
            }
        });
        let selected_contract =
            matches!(self.screen, LabScreen::Chain | LabScreen::Activity)
                .then(|| {
                    self.trading.detail.as_ref().zip(self.selected_quote()).map(
                        |(detail, quote)| {
                            guide_contract_snapshot(detail, quote, phase == OraclePhase::GameMode)
                        },
                    )
                })
                .flatten();
        let open_trade_ticket = self.trading.ticket.as_ref().map(|ticket| {
            let price = ticket.premium_input.trim().parse::<f64>().ok();
            let contracts = ticket
                .quantity_input
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|contracts| *contracts > 0);
            let risk = self.selected_quote().and_then(|quote| {
                price.and_then(|price| trade_risk_preview_for_price(quote, ticket.action, price))
            });
            guide::GuideTradeTicketSnapshot {
                action: ticket.action.label().to_ascii_lowercase(),
                price,
                contracts,
                maximum_loss: risk
                    .map(|risk| risk.max_loss_per_contract * contracts.unwrap_or(1) as f64),
                maximum_gain: risk
                    .map(|risk| risk.max_gain_per_contract * contracts.unwrap_or(1) as f64),
                maximum_payout: risk
                    .map(|risk| risk.max_payout_per_contract * contracts.unwrap_or(1) as f64),
                ready_for_review: risk.is_some() && contracts.is_some(),
            }
        });
        let available_targets = self.guide_available_targets();
        let available_form_actions = self.guide_available_form_actions();
        let active_form = self.guide_active_form();
        guide::TuiStateSnapshot {
            schema_version: guide::GUIDE_SCHEMA_VERSION.to_string(),
            state_revision: self.guide_state_revision(),
            context_scope: self.guide_context_scope(),
            current_screen: screen_title(self.screen).to_string(),
            current_focus: lab_focus_id(
                self.guide
                    .context_focus
                    .filter(|_| self.focus == LabFocus::Guide)
                    .unwrap_or(self.focus),
            )
            .to_string(),
            overlay: self.guide_overlay(),
            wallet_state: if self.wallet.is_attached() {
                "attached"
            } else {
                "not attached"
            }
            .to_string(),
            chart_range: (self.screen == LabScreen::Chart)
                .then(|| self.trading.chart_range.label().to_string()),
            selected_market,
            selected_contract,
            visible_contracts,
            open_trade_ticket,
            breadcrumb_path,
            highlighted_node,
            highlighted_source,
            visible_phase_timeline,
            active_challenges,
            available_safe_actions,
            available_targets,
            available_form_actions,
            active_form,
            navigation_targets,
            visible_cues: self.guide_visible_cues(),
            interaction_locked: self.guide_navigation_locked(),
        }
    }

    pub(super) fn guide_context_scope(&self) -> String {
        match (self.screen, self.home_help_topic) {
            (LabScreen::Help, HomeHelpTopic::Agents) => "connect_agents".to_string(),
            (LabScreen::Help, HomeHelpTopic::Overview) => "help".to_string(),
            (LabScreen::Ledger, _) => {
                format!("wallet_ledger_{}", guide_ledger_view_id(self.ledger_view))
            }
            (LabScreen::Detail, _) => {
                format!(
                    "market_detail_{}",
                    guide_detail_view_id(self.trading.detail_view)
                )
            }
            _ => screen_title(self.screen).replace(' ', "_"),
        }
    }

    pub(super) fn guide_overlay(&self) -> Option<String> {
        let label = if self.writers.confirmation.is_some() {
            "writer_confirmation"
        } else if self.writers.form.is_some() {
            "writer_action_form"
        } else if self.staking_confirmation.is_some() {
            "staking_confirmation"
        } else if self.staking_form.is_some() {
            "staking_amount_form"
        } else if self.trading.confirmation_is_open() {
            "trade_confirmation"
        } else if self.trading.result_modal_is_open() {
            "trade_result"
        } else if self.trading.ticket.is_some() {
            "trade_ticket"
        } else if self.oracle.form.is_some() {
            "oracle_form"
        } else if self.wallet_switch_editing {
            "wallet_switch"
        } else if self.oracle.search_editing {
            "oracle_search"
        } else if self.help.glossary_hover.is_some() {
            "glossary_definition"
        } else if self.help.preview.is_some() {
            "help_preview"
        } else {
            return None;
        };
        Some(label.to_string())
    }

    pub(super) fn guide_available_targets(&self) -> Vec<guide::GuideTargetSnapshot> {
        let mut targets = if self.screen == LabScreen::Terms {
            Vec::new()
        } else {
            guide_navigation_targets(
                "screen",
                &[
                    (
                        "surface:home",
                        "Home",
                        "Current market, next actions, and account status.",
                    ),
                    (
                        "surface:options",
                        "Options",
                        "Fixed-risk monthly calls and puts.",
                    ),
                    (
                        "surface:chart",
                        "Month chart",
                        "Fair price, movement, volume, and liquidity history.",
                    ),
                    (
                        "surface:oracle-entry",
                        "Oracle evidence entry",
                        "A concise orientation before inspecting or contributing evidence.",
                    ),
                    (
                        "surface:oracle-earn",
                        "Oracle Earn",
                        "A compact funded-reward list scoped to the selected market and month.",
                    ),
                    (
                        "surface:oracle",
                        "Advanced Oracle evidence",
                        "Sources, monthly phases, evidence tasks, challenges, and drafts.",
                    ),
                    (
                        "surface:oracle-help",
                        "Oracle help",
                        "Evidence rules, source-local movement, and contributor tasks.",
                    ),
                    (
                        "surface:help",
                        "Help",
                        "Published product guide and glossary.",
                    ),
                    (
                        "surface:market-detail",
                        "Market detail",
                        "Current print, month, settlement, cap, and freshness.",
                    ),
                    (
                        "surface:contract-trades",
                        "Contract activity",
                        "Recent activity, quote, depth, and volume for the selected contract.",
                    ),
                    (
                        "surface:wallet-ledger",
                        "Wallet ledger",
                        "Balances, wallet activity, Amoeba trades, and claims.",
                    ),
                    (
                        "surface:staking",
                        "Staking",
                        "AMBA available to stake, sAMBA shares, embedded rewards, and unstaking progress.",
                    ),
                    (
                        "surface:connect-agents",
                        "Connect your AI agent",
                        "Review the optional Petri AI agent connection and approval model.",
                    ),
                ],
            )
        };

        if self.screen == LabScreen::Ledger {
            targets.extend(guide_navigation_targets(
                "workspace_tab",
                &[
                    (
                        "surface:ledger:account",
                        "Account tab",
                        "Balances, collateral, and human-only account controls.",
                    ),
                    (
                        "surface:ledger:positions",
                        "Liquidity tab",
                        "Current manager-liquidity positions.",
                    ),
                    (
                        "surface:ledger:writers",
                        "Writers tab",
                        "Global collective-writer sleeves and reviewable actions.",
                    ),
                    (
                        "surface:ledger:history",
                        "History tab",
                        "Combined authoritative wallet and Amoeba activity.",
                    ),
                ],
            ));
        }
        if self.screen == LabScreen::Detail {
            targets.extend(guide_navigation_targets(
                "workspace_tab",
                &[
                    (
                        "surface:detail:overview",
                        "Overview tab",
                        "Current print, listed month, cap, and freshness.",
                    ),
                    (
                        "surface:detail:settlement",
                        "Settlement tab",
                        "Exact-month settlement, readiness, and Oracle evidence.",
                    ),
                ],
            ));
        }

        if self.screen != LabScreen::Terms && self.can_go_back() {
            targets.push(guide_ui_target(
                "surface:back",
                "Back",
                "screen",
                "Return to the previous Petri page.",
                "navigate",
                false,
            ));
        }

        if self.screen != LabScreen::Terms
            && let Some(detail) = self
                .trading
                .detail
                .as_ref()
                .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))
        {
            targets.extend(detail.expiries.iter().map(|expiry| {
                guide_ui_target(
                    &guide_month_target_id(&detail.id, &expiry.id),
                    &format!("{} {}", detail.symbol, expiry.label),
                    "month",
                    &format!("Open the {} monthly contracts.", expiry.label),
                    "navigate",
                    false,
                )
            }));
        }

        if self.screen == LabScreen::Chart {
            for range in [
                ChartRangeValue::OneHour,
                ChartRangeValue::TwentyFourHours,
                ChartRangeValue::SevenDays,
                ChartRangeValue::ThirtyDays,
                ChartRangeValue::All,
            ] {
                targets.push(guide_ui_target(
                    &format!("chart-range:{}", range.label()),
                    &format!("{} chart range", range.label()),
                    "view_control",
                    "Change only the visible chart window.",
                    "navigate",
                    false,
                ));
            }
        }

        if self.screen == LabScreen::Oracle
            && self.oracle.view == OracleView::Advanced
            && let Some(node) = self.selected_oracle_node()
            && let Some(context) = self.selected_oracle_action_context()
        {
            for action in self.visible_oracle_actions() {
                if OracleFormMode::from_action(action).is_some() {
                    continue;
                }
                let availability = match action.availability(context) {
                    OracleActionAvailability::Active => "active",
                    OracleActionAvailability::ChooseSource => "choose a source first",
                    OracleActionAvailability::Locked => "locked in this phase",
                };
                targets.push(guide_ui_target(
                    &guide_oracle_action_target_id(action, &node.node_id),
                    action.label(),
                    "view_control",
                    &format!(
                        "Select this Oracle action row ({availability}). {}",
                        action.contextual_detail(context)
                    ),
                    "navigate",
                    false,
                ));
            }
        }

        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Overview {
            for page in self
                .help
                .index
                .categories
                .iter()
                .flat_map(|category| category.pages.iter())
                .take(24)
            {
                targets.push(guide_ui_target(
                    &guide_help_page_target_id(&page.id),
                    &page.title,
                    "help_page",
                    &page.description,
                    "navigate",
                    false,
                ));
            }
        }

        if self.screen == LabScreen::Terms {
            targets.extend([
                guide_ui_target(
                    "control:terms:open",
                    "Open Terms",
                    "final_control",
                    "Open the Terms page. The Guide never activates it.",
                    "focus",
                    true,
                ),
                guide_ui_target(
                    "control:terms:switch-wallet",
                    "Switch wallet",
                    "final_control",
                    "Choose a wallet yourself; the path is never sent to the Guide.",
                    "focus",
                    true,
                ),
                guide_ui_target(
                    "control:terms:accept",
                    "Accept for this wallet",
                    "final_control",
                    "Accept the Terms yourself after review.",
                    "focus",
                    true,
                ),
            ]);
        }
        if self.trading.ticket.is_some() {
            let control = if self.trading.confirmation_is_open() {
                ("control:trade:confirm", "Confirm order")
            } else {
                ("control:trade:review", "Review / export ticket")
            };
            targets.push(guide_ui_target(
                control.0,
                control.1,
                "final_control",
                "Only the user may prepare and review the exact trade, then explicitly approve signing and submission. The Guide cannot activate either boundary.",
                "focus",
                true,
            ));
        }
        if self.oracle.form.is_some() {
            targets.push(guide_ui_target(
                "control:oracle:queue-draft",
                "Queue local draft",
                "final_control",
                "Review every evidence field, then queue the draft yourself.",
                "focus",
                true,
            ));
        }
        if let Some(form) = self.writers.form.as_ref() {
            let (target_id, label, description) = if self.writers.confirmation.is_some() {
                (
                    "control:writer:confirm",
                    "Unavailable writer action",
                    "This stale confirmation requires a new verified review.",
                )
            } else if form.action.signs_and_submits() {
                (
                    "control:writer:review",
                    "Unavailable writer change",
                    "The current release cannot prepare, sign, or send this wallet change.",
                )
            } else {
                (
                    "control:writer:run",
                    "Run writer read",
                    "Check the exact identifiers, then run this read-only request yourself.",
                )
            };
            targets.push(guide_ui_target(
                target_id,
                label,
                "final_control",
                description,
                "focus",
                true,
            ));
        }
        if self.liquidity_preview_form.is_some() {
            targets.push(guide_ui_target(
                "control:liquidity:preview",
                "Request unsigned preview",
                "final_control",
                "Check every manager-only input, then request the unsigned Lean preview yourself. It never prepares, signs, or submits.",
                "focus",
                true,
            ));
        }
        if self.screen == LabScreen::Help && self.home_help_topic == HomeHelpTopic::Agents {
            targets.push(guide_ui_target(
                "control:mcp:connection",
                "Petri assistant connection",
                "final_control",
                "Enable, repair, or disable only when the user presses the button.",
                "focus",
                true,
            ));
        }
        if self.screen != LabScreen::Terms && self.updates.has_result() {
            targets.push(guide_ui_target(
                "control:update",
                "Update Petri",
                "final_control",
                "The Guide can explain the update but never starts it.",
                "focus",
                true,
            ));
        }
        targets.truncate(48);
        targets
    }

    pub(super) fn guide_available_form_actions(&self) -> Vec<guide::GuideFormActionSnapshot> {
        let mut actions = Vec::new();
        if self.screen == LabScreen::Chain
            && self.trading.ticket.is_none()
            && let Some(detail) = self.trading.detail.as_ref()
            && let Some(quote) = self.selected_quote()
            && self.oracle_phase() == OraclePhase::GameMode
            && quote.prepare_eligible
            && quote
                .depth_usd
                .is_some_and(|depth| depth.is_finite() && depth > 0.0)
        {
            let contract_id = guide_contract_target_id(detail, quote);
            for (action, title) in [
                (TradeAction::Buy, "Buy ticket"),
                (TradeAction::Sell, "Sell/write ticket"),
            ] {
                let Some(route_price) = guide_trade_route_price(quote, action) else {
                    continue;
                };
                let price = Some(format_decimal(route_price, 3));
                actions.push(guide::GuideFormActionSnapshot {
                    target_id: guide_trade_action_target_id(action, &contract_id),
                    title: title.to_string(),
                    purpose: "Stage price and contracts for review; no order is reviewed or sent."
                        .to_string(),
                    fields: guide_trade_fields(action, price, None, true),
                    final_control_label: "Review / export ticket — user presses Enter".to_string(),
                });
            }
        }

        if self.screen == LabScreen::Oracle
            && self.oracle.view == OracleView::Advanced
            && self.oracle.form.is_none()
            && self.oracle_tree().is_some()
            && let Some(node) = self.selected_oracle_node()
            && let Some(context) = self.selected_oracle_action_context()
        {
            for action in self.visible_oracle_actions() {
                if action.availability(context) != OracleActionAvailability::Active {
                    continue;
                }
                let Some(mode) = OracleFormMode::from_action(action) else {
                    continue;
                };
                let Some(form) = self.oracle_form_draft_for_selected(mode) else {
                    continue;
                };
                actions.push(guide::GuideFormActionSnapshot {
                    target_id: guide_oracle_action_target_id(action, &node.node_id),
                    title: mode.title().to_string(),
                    purpose: mode.user_hint().to_string(),
                    fields: form
                        .fields
                        .iter()
                        .map(guide_oracle_field_snapshot)
                        .collect(),
                    final_control_label: "Queue local draft — user presses Enter".to_string(),
                });
            }
        }
        if self.screen == LabScreen::Ledger
            && self.ledger_view == LedgerView::Writers
            && self.writers.form.is_none()
            && self.writers.confirmation.is_none()
            && !self.writers.interaction_is_locked()
            && let Some(sleeve) = self.selected_writer_sleeve()
            && let Some(sleeve_address) = self.selected_writer_sleeve_address()
        {
            for action in WriterAction::ALL {
                let availability = self.writer_action_availability(action);
                if !availability.is_actionable() {
                    continue;
                }
                let form = writers::writer_form_for_action(action, Some(sleeve));
                if form.fields.is_empty() {
                    continue;
                }
                let capability_note = availability
                    .note()
                    .map(|note| format!(" Current capability note: {note}"))
                    .unwrap_or_default();
                actions.push(guide::GuideFormActionSnapshot {
                    target_id: guide_writer_action_target_id(action, &sleeve_address),
                    title: action.label().to_string(),
                    purpose: format!(
                        "{}{} The Guide may stage fields only; {}",
                        action.detail(),
                        capability_note,
                        if action.signs_and_submits() {
                            "the user must open review and explicitly confirm the wallet change."
                        } else {
                            "the user must run the read-only request."
                        }
                    ),
                    fields: form
                        .fields
                        .iter()
                        .map(|field| guide_writer_field_snapshot(field, true))
                        .collect(),
                    final_control_label: if action.signs_and_submits() {
                        "Open wallet review - user presses Enter"
                    } else {
                        "Run read-only request — user presses Enter"
                    }
                    .to_string(),
                });
            }
        }
        if self.screen == LabScreen::Ledger
            && self.ledger_view == LedgerView::Positions
            && self.wallet.is_attached()
            && self.liquidity_preview_form.is_none()
            && !self.liquidity_preview_is_running()
            && let Some(owner) = self.wallet.pubkey.as_deref()
        {
            let form = liquidity::LiquidityPreviewForm::from_position(
                self.liquidity_position_rows()
                    .get(self.ledger_position_selected),
            );
            actions.push(guide::GuideFormActionSnapshot {
                target_id: guide_liquidity_preview_target_id(owner, &form),
                title: "Unsigned manager-liquidity preview".to_string(),
                purpose: "Stage exact manager-only inputs for a bounded Lean preview. Petri does not independently SDK-validate, prepare, sign, or submit it."
                    .to_string(),
                fields: guide_liquidity_field_snapshots(&form, true),
                final_control_label: "Request unsigned preview — user presses Enter".to_string(),
            });
        }
        actions.truncate(12);
        actions
    }

    pub(super) fn guide_active_form(&self) -> Option<guide::GuideFormSnapshot> {
        if let Some(ticket) = self.trading.ticket.as_ref() {
            return Some(guide::GuideFormSnapshot {
                form_id: "form:trade-ticket".to_string(),
                title: format!("{} order ticket", ticket.action.label()),
                purpose: "Editable Buy or owned-option Sell intent. Only the user may request SDK-backed preparation and approve the immutable amount review; the Guide cannot review, sign, or submit."
                    .to_string(),
                fields: guide_trade_fields(
                    ticket.action,
                    (!ticket.premium_input.is_empty()).then(|| ticket.premium_input.clone()),
                    (!ticket.quantity_input.is_empty()).then(|| ticket.quantity_input.clone()),
                    ticket.confirmation.is_none() && !ticket.submitting,
                ),
                final_control_id: if ticket.confirmation.is_some() {
                    "control:trade:confirm"
                } else {
                    "control:trade:review"
                }
                .to_string(),
                final_control_label: if ticket.confirmation.is_some() {
                    "Confirm order"
                } else {
                    "Review / export ticket"
                }
                .to_string(),
                user_must_activate_final_control: true,
            });
        }
        if let Some(form) = self.oracle.form.as_ref() {
            return Some(guide::GuideFormSnapshot {
                form_id: guide_oracle_form_id(form),
                title: form.mode.title().to_string(),
                purpose: form.mode.user_hint().to_string(),
                fields: form
                    .fields
                    .iter()
                    .map(guide_oracle_field_snapshot)
                    .collect(),
                final_control_id: "control:oracle:queue-draft".to_string(),
                final_control_label: "Queue local draft".to_string(),
                user_must_activate_final_control: true,
            });
        }
        if let Some(form) = self.writers.form.as_ref() {
            let in_confirmation = self.writers.confirmation.is_some();
            let editable = !in_confirmation && !self.writers.interaction_is_locked();
            let (final_control_id, final_control_label) = if in_confirmation {
                ("control:writer:confirm", "Wallet change unavailable")
            } else if form.action.signs_and_submits() {
                ("control:writer:review", "Wallet change unavailable")
            } else {
                ("control:writer:run", "Run read-only request")
            };
            return Some(guide::GuideFormSnapshot {
                form_id: guide_writer_form_id(form),
                title: form.action.label().to_string(),
                purpose: format!(
                    "{} The Guide can stage non-secret fields but cannot review, confirm, sign, send, or run the request.",
                    form.action.detail()
                ),
                fields: form
                    .fields
                    .iter()
                    .map(|field| guide_writer_field_snapshot(field, editable))
                    .collect(),
                final_control_id: final_control_id.to_string(),
                final_control_label: final_control_label.to_string(),
                user_must_activate_final_control: true,
            });
        }
        if let Some(form) = self.liquidity_preview_form.as_ref() {
            return Some(guide::GuideFormSnapshot {
                form_id: guide_liquidity_preview_form_id().to_string(),
                title: "Unsigned manager-liquidity preview".to_string(),
                purpose: "Exact manager-only inputs for a Lean preview. The Guide may stage fields but cannot request the preview; Petri never prepares, signs, or submits from this form."
                    .to_string(),
                fields: guide_liquidity_field_snapshots(
                    form,
                    !self.liquidity_preview_is_running(),
                ),
                final_control_id: "control:liquidity:preview".to_string(),
                final_control_label: "Request unsigned preview".to_string(),
                user_must_activate_final_control: true,
            });
        }
        if self.wallet_switch_editing {
            return Some(guide::GuideFormSnapshot {
                form_id: "form:wallet-switch".to_string(),
                title: "Switch wallet".to_string(),
                purpose: "Choose a wallet path privately. Petri never sends it to the Guide."
                    .to_string(),
                fields: vec![guide::GuideFieldSnapshot {
                    field_id: "wallet_path".to_string(),
                    label: "Wallet path".to_string(),
                    value: None,
                    required: true,
                    editable_by_guide: false,
                }],
                final_control_id: "control:terms:switch-wallet".to_string(),
                final_control_label: "Switch wallet".to_string(),
                user_must_activate_final_control: true,
            });
        }
        None
    }

    pub(super) fn guide_visible_contracts(&self) -> Vec<guide::GuideContractSnapshot> {
        if !matches!(self.screen, LabScreen::Chain | LabScreen::Activity) {
            return Vec::new();
        }
        let Some(detail) = self
            .trading
            .detail
            .as_ref()
            .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))
        else {
            return Vec::new();
        };
        detail
            .option_quotes
            .iter()
            .take(64)
            .map(|quote| {
                guide_contract_snapshot(detail, quote, self.oracle_phase() == OraclePhase::GameMode)
            })
            .collect()
    }

    pub(super) fn guide_state_revision(&self) -> String {
        let expiry = self
            .trading
            .detail
            .as_ref()
            .map(|detail| detail.expiry_id.as_str())
            .unwrap_or("-");
        let node_id = self
            .selected_oracle_node()
            .map(|node| node.node_id.as_str())
            .unwrap_or("-");
        let mut interaction = std::collections::hash_map::DefaultHasher::new();
        screen_title(self.screen).hash(&mut interaction);
        lab_focus_id(
            self.guide
                .context_focus
                .filter(|_| self.focus == LabFocus::Guide)
                .unwrap_or(self.focus),
        )
        .hash(&mut interaction);
        self.home_selected.hash(&mut interaction);
        guide_detail_view_id(self.trading.detail_view).hash(&mut interaction);
        guide_ledger_view_id(self.ledger_view).hash(&mut interaction);
        match self.ledger_pane {
            LedgerPane::Tabs => "ledger_tabs",
            LedgerPane::List => "ledger_list",
            LedgerPane::Actions => "ledger_actions",
            LedgerPane::Detail => "ledger_detail",
        }
        .hash(&mut interaction);
        self.ledger_position_selected.hash(&mut interaction);
        self.ledger_writer_selected.hash(&mut interaction);
        self.writers.action_selected.hash(&mut interaction);
        self.ledger_history_selected.hash(&mut interaction);
        self.trading.chart_range.label().hash(&mut interaction);
        self.trading
            .active_option_kind
            .label()
            .hash(&mut interaction);
        self.trading.selected_option.hash(&mut interaction);
        self.help.selected_page_id.hash(&mut interaction);
        match self.home_help_topic {
            HomeHelpTopic::Overview => "help_overview",
            HomeHelpTopic::Agents => "help_agents",
        }
        .hash(&mut interaction);
        match self.help.pane {
            HelpPane::Navigation => "help_navigation",
            HelpPane::Article => "help_article",
        }
        .hash(&mut interaction);
        self.guide_overlay().hash(&mut interaction);
        self.wallet.is_attached().hash(&mut interaction);
        if let Some(ticket) = self.trading.ticket.as_ref() {
            ticket.premium_input.hash(&mut interaction);
            ticket.quantity_input.hash(&mut interaction);
            ticket.action.label().hash(&mut interaction);
            ticket.confirmation.is_some().hash(&mut interaction);
            ticket
                .confirmation
                .as_ref()
                .map(|confirmation| match confirmation.choice {
                    TradeConfirmationChoice::Cancel => "cancel",
                    TradeConfirmationChoice::Confirm => "confirm",
                })
                .hash(&mut interaction);
        }
        if let Some(form) = self.oracle.form.as_ref() {
            guide_oracle_form_mode_id(form.mode).hash(&mut interaction);
            form.node_index.hash(&mut interaction);
            form.field_selected.hash(&mut interaction);
            for field in &form.fields {
                field.label.hash(&mut interaction);
                field.value.hash(&mut interaction);
            }
        }
        if let Some(form) = self.writers.form.as_ref() {
            guide_writer_action_id(form.action).hash(&mut interaction);
            form.selected_field.hash(&mut interaction);
            for field in &form.fields {
                field.key.hash(&mut interaction);
                field.value.hash(&mut interaction);
            }
        }
        if let Some(form) = self.liquidity_preview_form.as_ref() {
            form.action.label().hash(&mut interaction);
            form.market.hash(&mut interaction);
            form.expiry.hash(&mut interaction);
            form.position_nonce.hash(&mut interaction);
            form.entries.hash(&mut interaction);
            form.selected_field.hash(&mut interaction);
        }
        self.liquidity_preview_inflight.hash(&mut interaction);
        if let Some(confirmation) = self.writers.confirmation.as_ref() {
            guide_writer_action_id(confirmation.action).hash(&mut interaction);
            match confirmation.choice {
                UserActionConfirmationChoice::Cancel => "writer_cancel",
                UserActionConfirmationChoice::Confirm => "writer_confirm",
            }
            .hash(&mut interaction);
        }
        self.writers.action_inflight.hash(&mut interaction);
        if let Some(bundle) = self.trading.settlement_bundle.as_ref() {
            bundle.market_id.hash(&mut interaction);
            bundle.expiry_id.hash(&mut interaction);
            bundle.available_endpoint_count().hash(&mut interaction);
            bundle.issue_count().hash(&mut interaction);
        }
        self.trading.loading_settlement.hash(&mut interaction);
        if let Some(detail) = self.trading.detail.as_ref() {
            detail.id.hash(&mut interaction);
            detail.expiry_id.hash(&mut interaction);
            detail.current_print.hash(&mut interaction);
            detail.base.hash(&mut interaction);
            detail.days.hash(&mut interaction);
            detail.cap_width.hash(&mut interaction);
            for quote in &detail.option_quotes {
                quote.kind.label().hash(&mut interaction);
                quote.lower_strike.hash(&mut interaction);
                quote.upper_strike.hash(&mut interaction);
                quote.bid.map(f64::to_bits).hash(&mut interaction);
                quote.ask.map(f64::to_bits).hash(&mut interaction);
                quote.mid.map(f64::to_bits).hash(&mut interaction);
                quote.depth_usd.map(f64::to_bits).hash(&mut interaction);
                quote.volume.map(f64::to_bits).hash(&mut interaction);
                quote.open_interest.map(f64::to_bits).hash(&mut interaction);
                quote.prepare_eligible.hash(&mut interaction);
                quote.status.hash(&mut interaction);
            }
        }
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{:016x}",
            screen_title(self.screen),
            self.selected_id(),
            expiry,
            node_id,
            oracle_phase_id(self.oracle_phase()),
            usize::from(self.guide_navigation_locked()),
            self.trading.selected_option,
            self.trading.detail_request,
            interaction.finish(),
        )
    }

    pub(super) fn guide_navigation_locked(&self) -> bool {
        self.oracle.form.is_some()
            || self.trading.ticket.is_some()
            || self.trading.confirmation_is_open()
            || self.trading.submit_is_running()
            || self.trading.result_modal_is_open()
            || self.wallet_switch_editing
            || self.writers.form.is_some()
            || self.writers.confirmation.is_some()
            || self.writers.interaction_is_locked()
            || self.liquidity_preview_form.is_some()
            || self.liquidity_preview_is_running()
            || self.staking_form.is_some()
            || self.staking_confirmation.is_some()
            || self.staking_action_is_running()
    }

    pub(super) fn guide_active_challenges(&self) -> Vec<guide::GuideChallengeSnapshot> {
        let mut challenges = Vec::new();
        if let Some(live) = self
            .oracle
            .live
            .as_ref()
            .filter(|live| live.market_id.eq_ignore_ascii_case(&self.selected_id()))
        {
            for emergency in &live.emergencies {
                challenges.push(guide::GuideChallengeSnapshot {
                    challenge_id: emergency
                        .challenge_id_hex
                        .clone()
                        .unwrap_or_else(|| emergency.dispute_id_hex.clone()),
                    label: format!("{} review", humanize_guide_label(&emergency.kind)),
                    status: humanize_guide_label(&emergency.status),
                    target_id: Some(emergency.target_id_hex.clone()),
                    reason: None,
                    origin: "live".to_string(),
                });
            }
            for observation in &live.observations {
                if !oracle_source_status_has_unresolved_challenge(&observation.status)
                    || challenges.iter().any(|challenge| {
                        challenge.target_id.as_deref() == Some(&observation.source_id_hex)
                    })
                {
                    continue;
                }
                challenges.push(guide::GuideChallengeSnapshot {
                    challenge_id: observation.source_id_hex.clone(),
                    label: format!("{} source challenge", observation.source),
                    status: humanize_guide_label(&observation.status),
                    target_id: exact_tree_node_id_for_live_source(
                        self.oracle_tree(),
                        &observation.source_id_hex,
                    ),
                    reason: None,
                    origin: "live".to_string(),
                });
            }
        }
        for record in self.oracle.submissions.iter().filter(|record| {
            record.title == "Challenge"
                && record.update_state == Some(OracleUpdateState::Challenged)
        }) {
            let reason = record
                .fields
                .iter()
                .find(|(label, _)| label.to_ascii_lowercase().contains("reason"))
                .map(|(_, value)| value.clone())
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    record
                        .summary
                        .split_once("reason=")
                        .map(|(_, reason)| reason.trim().to_string())
                        .filter(|reason| !reason.is_empty())
                });
            challenges.push(guide::GuideChallengeSnapshot {
                challenge_id: record
                    .stored_id
                    .clone()
                    .unwrap_or_else(|| format!("local-{}", challenges.len() + 1)),
                label: format!("{} challenge preview", record.node_label),
                status: "awaiting review".to_string(),
                target_id: self
                    .oracle_tree()
                    .and_then(|tree| {
                        tree.nodes
                            .iter()
                            .find(|node| node.label == record.node_label)
                    })
                    .map(|node| node.node_id.clone()),
                reason,
                origin: "local_preview".to_string(),
            });
        }
        challenges.truncate(24);
        challenges
    }

    pub(super) fn guide_visible_cues(&self) -> Vec<String> {
        let mut cues = Vec::new();
        match self.screen {
            LabScreen::Terms => {
                cues.push(
                    "Wallet-specific Terms are open. The Guide may explain or point, but only the user can open, switch, or accept."
                        .to_string(),
                );
            }
            LabScreen::Home => {
                if let Some(detail) = self.trading.detail.as_ref() {
                    cues.push(format!(
                        "{} {} | month {} | settles {} | fixed max loss, no liquidation.",
                        detail.symbol,
                        market_price_context(detail),
                        detail.expiry_label,
                        detail.settlement
                    ));
                }
                let action = self.selected_home_action();
                cues.push(format!(
                    "Selected Home action: {} — {}",
                    action.label(),
                    action.detail()
                ));
            }
            LabScreen::Staking => {
                cues.push(
                    "Staking queues available AMBA for seven days, then activates it into transferable sAMBA shares. Queued AMBA earns no rewards or voting power."
                        .to_string(),
                );
                cues.push(
                    "After activation, rewards increase the AMBA redeemable by each sAMBA share; there is no separate staking-reward claim."
                        .to_string(),
                );
                cues.push(
                    "Unstaking fixes the AMBA amount, burns the sAMBA shares, waits seven days, then requires the wallet owner to claim the ready AMBA."
                        .to_string(),
                );
                if let Some(status) = self.staking_status.as_ref()
                    && staking_status_flag(
                        status,
                        &["supplyChangesLocked", "supply_changes_locked"],
                    )
                {
                    cues.push(
                        "Activation and unstaking are paused while an emergency vote is unresolved; queue, cancel, balance reads, and ready claims remain available."
                            .to_string(),
                    );
                }
                cues.push(format!(
                    "Selected staking action: {} — {}",
                    self.selected_staking_action().label(),
                    self.selected_staking_action().detail()
                ));
            }
            LabScreen::Chain => {
                let contracts = self.guide_visible_contracts();
                let has_calls = contracts
                    .iter()
                    .any(|contract| contract.kind == "call spread");
                let has_executable_calls = contracts
                    .iter()
                    .any(|contract| contract.kind == "call spread" && contract.executable);
                if has_calls && !has_executable_calls {
                    cues.push(
                        "The selected month has no call with a positive ask, visible depth, and on-chain availability. Do not select an informational call as executable."
                            .to_string(),
                    );
                }
                cues.push(
                    "A staged ticket is editable only. User Enter prepares an exact SDK-backed operation; a second explicit approval signs and submits. The Guide cannot activate either step."
                        .to_string(),
                );
            }
            LabScreen::Chart => {
                let args = self.chart_args();
                let refresh = if args.refresh_seconds == 0 {
                    "manual refresh only".to_string()
                } else {
                    format!("auto-refresh every {} seconds", args.refresh_seconds)
                };
                cues.push(format!(
                    "Chart sampling: {refresh}; show at most {} points. R requests a refresh now.",
                    args.points
                ));
                if let Some(chart) = self.trading.chart.as_ref() {
                    cues.push(format!(
                        "{} | range {} | {} visible of {} stored points.",
                        chart.title(),
                        chart.range_label(),
                        chart.point_count(),
                        chart.total_point_count()
                    ));
                    cues.extend(chart.guide_facts());
                } else {
                    cues.push(
                        "Chart history is loading or unavailable for the selected month."
                            .to_string(),
                    );
                }
            }
            LabScreen::OracleIntro => {
                let action = self.oracle.selected_intro_action();
                cues.push(format!(
                    "Selected Oracle entry action: {} — {}",
                    action.label(),
                    action.detail()
                ));
                cues.push(
                    "Contributors can propose repeatable public sources, update changed evidence, or challenge bad evidence."
                        .to_string(),
                );
            }
            LabScreen::Oracle => {
                if self.oracle.view == OracleView::Earn {
                    cues.push(
                        "Earn is the simple Oracle view: a funded-reward list for only the selected market and month, with exact reward amounts and claim status."
                            .to_string(),
                    );
                    if let Some(claim) = self.selected_oracle_reward_claim() {
                        cues.push(format!(
                            "Selected reward: {}. The exact funded reward is {} and is ready to review.",
                            claim.label, claim.amount_label
                        ));
                    } else {
                        cues.push(
                            "No funded reward is ready for this wallet, market, and month. Petri does not invent paid work."
                                .to_string(),
                        );
                    }
                } else {
                    if self.oracle.search_editing {
                        cues.push(format!(
                            "Oracle search popup: local query is {}. The Guide may safely apply a source-tree search; it never submits evidence.",
                            if self.oracle.search_input.trim().is_empty() {
                                "empty".to_string()
                            } else {
                                format!("\"{}\"", self.oracle.search_input.trim())
                            }
                        ));
                    }
                    if self.oracle.locked_flash.is_some_and(|flash| flash.visible) {
                        cues.push(
                            "A red flash means the selected action is unavailable in this phase."
                                .to_string(),
                        );
                    }
                    if self
                        .oracle
                        .live
                        .as_ref()
                        .is_some_and(|live| live.active_emergency_count() > 0)
                    {
                        cues.push(
                            "Red source status marks an active challenge or emergency review."
                                .to_string(),
                        );
                    }
                    if let Some(node) = self.selected_oracle_node() {
                        cues.push(format!(
                            "Selected {}: {}. Each source is measured against its own opening value before frozen row weighting.",
                            node.kind.label(), node.label
                        ));
                        if self
                            .oracle_source_action_state(node)
                            .has_unresolved_challenge
                        {
                            cues.push(
                                "The selected source is challenged and is awaiting resolution."
                                    .to_string(),
                            );
                        }
                    }
                    if let Some(context) = self.selected_oracle_action_context() {
                        let action = self.selected_oracle_action();
                        cues.push(format!(
                            "Selected task: {} [{}] — {}",
                            action.label(),
                            action.state(context),
                            action.contextual_detail(context)
                        ));
                    }
                }
            }
            LabScreen::OracleHelp => {
                cues.push(
                    "Oracle help explains monthly source selection, frozen weights, opening values, live updates, challenges, and settlement."
                        .to_string(),
                );
            }
            LabScreen::Help => match self.home_help_topic {
                HomeHelpTopic::Overview => {
                    if let Some(link) = self.help.current_link() {
                        cues.push(format!(
                            "Help article: {} | source {}.",
                            link.title,
                            self.help.index.source.label()
                        ));
                    }
                    if let Some(preview) = self.help.preview.as_ref() {
                        let rows =
                            gitbook::nav_rows(&self.help.index, &self.help.expanded_categories);
                        let row = rows.get(preview.nav_index);
                        let label = row
                            .map(|row| row.label.clone())
                            .unwrap_or_else(|| "selected topic".to_string());
                        let summary = preview
                            .page
                            .as_ref()
                            .and_then(|page| page.blocks.iter().find_map(guide_markdown_block_text))
                            .map(str::to_string)
                            .or_else(|| {
                                let GitbookNavTarget::Page { category, page } = row?.target else {
                                    return None;
                                };
                                self.help
                                    .index
                                    .categories
                                    .get(category)?
                                    .pages
                                    .get(page)
                                    .map(|link| link.description.trim().to_string())
                                    .filter(|description| !description.is_empty())
                            });
                        cues.push(match summary {
                            Some(summary) => format!("Help preview: {label} — {summary}"),
                            None => format!("Help preview open for {label}."),
                        });
                    }
                    if let Some(glossary) = self.help.glossary_hover.as_ref() {
                        cues.push(format!(
                            "Glossary popup: {} — {}",
                            glossary.term, glossary.definition
                        ));
                    }
                }
                HomeHelpTopic::Agents => {
                    cues.push(format!(
                        "Petri assistant connection: {} | validation/parity only; signing and submission are unavailable through Petri MCP.",
                        if self.mcp_managed_entry_enabled { "enabled" } else { "disabled" },
                    ));
                }
            },
            LabScreen::Detail => {
                if self.trading.detail_view == DetailView::Settlement {
                    if self.trading.loading_settlement {
                        cues.push(
                            "The exact-month settlement record, readiness, and Oracle evidence are loading independently."
                                .to_string(),
                        );
                    } else if let Some(bundle) = self.trading.settlement_bundle.as_ref() {
                        cues.push(format!(
                            "Settlement evidence for {}/{}: {} of 3 independent reads available.",
                            bundle.market_id.to_uppercase(),
                            bundle.expiry_id,
                            bundle.available_endpoint_count()
                        ));
                        cues.extend(
                            settlement_data::settlement_bundle_lines(bundle)
                                .into_iter()
                                .filter(|line| !line.trim().is_empty())
                                .take(12),
                        );
                    } else {
                        cues.push(
                            self.trading.settlement_issue
                                .as_deref()
                                .map(crate::backend::terminal_safe_text)
                                .unwrap_or_else(|| {
                                    "Settlement evidence is not loaded for the selected market and month."
                                        .to_string()
                                }),
                        );
                    }
                } else if let Some(detail) = self.trading.detail.as_ref() {
                    cues.push(format!(
                        "{} market detail: {} | {} settles {} | cap width {}.",
                        detail.symbol,
                        market_price_context(detail),
                        detail.expiry_label,
                        detail.settlement,
                        detail.cap_width
                    ));
                }
            }
            LabScreen::Activity => {
                if let Some(contract) = self.trading.detail.as_ref().zip(self.selected_quote()).map(
                    |(detail, quote)| {
                        guide_contract_snapshot(
                            detail,
                            quote,
                            self.oracle_phase() == OraclePhase::GameMode,
                        )
                    },
                ) {
                    cues.push(format!(
                        "Selected {} {} {} | bid {:?} | ask {:?} | depth {:?} | volume {:?}.",
                        contract.month,
                        contract.kind,
                        contract.range,
                        contract.bid,
                        contract.ask,
                        contract.depth_usd,
                        contract.volume
                    ));
                }
            }
            LabScreen::Ledger => {
                cues.push(if self.wallet.pubkey.as_deref().is_some_and(|owner| {
                    self.ledger_matches_owner(owner)
                }) {
                    "Wallet balances and recent wallet/Amoeba activity are loaded; raw technical fields stay out of Guide context."
                        .to_string()
                } else if self.wallet.is_attached() {
                    "Wallet ledger is loading or temporarily unavailable. Refresh remains a user view action."
                        .to_string()
                } else {
                    "No wallet is attached, so account-specific balances and activity are unavailable."
                        .to_string()
                });
                match self.ledger_view {
                    LedgerView::Account => cues.push(
                        "Account view shows only safe connection labels. Wallet switching and chain-connection changes remain human-only."
                            .to_string(),
                    ),
                    LedgerView::Positions => {
                        cues.push(
                            "Liquidity covers manager-owned positions only, not option-token or Flat holdings. Exact add, remove, and close inputs can be sent to Lean for an unsigned preview; execution remains unavailable and the preview is not independently SDK-validated."
                                .to_string(),
                        );
                        if self.liquidity_preview_form.is_some() {
                            cues.push(
                                "Unsigned liquidity preview form open. The Guide may stage fields and focus the final control, but only the user may request the preview; no transaction is prepared, signed, or submitted."
                                    .to_string(),
                            );
                        } else if let Some(result) = self.liquidity_preview_result.as_ref() {
                            cues.push(format!(
                                "Unsigned liquidity preview result: {}",
                                result.message
                            ));
                        }
                    }
                    LedgerView::Writers => {
                        cues.push(
                            "Writer rows are the global sleeve catalog, not proof that the attached wallet owns a sleeve."
                                .to_string(),
                        );
                        cues.push(format!(
                            "Selected writer action: {} — {}",
                            self.writers.selected_action().label(),
                            self.writers.selected_action().detail()
                        ));
                        cues.push(self.writer_close_capability_cue());
                        if let Some(form) = self.writers.form.as_ref() {
                            cues.push(format!(
                                "Writer form open: {}. The Guide may stage non-secret fields only; the user must {}.",
                                form.action.label(),
                                if form.action.signs_and_submits() {
                                    "dismiss this stale form because the current release cannot prepare, sign, or send it"
                                } else {
                                    "run the read-only request"
                                }
                            ));
                        }
                    }
                    LedgerView::History => cues.push(
                        "History is the combined authoritative activity view; typed categories remain unavailable until the service proves them."
                            .to_string(),
                    ),
                }
            }
        }
        if let Some(confirmation) = self.writers.confirmation.as_ref() {
            cues.push(format!(
                "Writer confirmation open for {} with {} selected. Only the user may change the choice or activate it; the Guide cannot review, confirm, sign, or send.",
                confirmation.action.label(),
                match confirmation.choice {
                    UserActionConfirmationChoice::Cancel => "Cancel",
                    UserActionConfirmationChoice::Confirm => "Confirm",
                }
            ));
        }
        if let Some(result) = self.writers.action_result.as_ref() {
            if result.action == WriterAction::CloseStatus {
                if result.ok {
                    if let Some(payload) = result.payload.as_ref() {
                        cues.extend(guide_writer_close_status_cues(
                            payload,
                            self.writer_action_availability(WriterAction::AdvanceClose)
                                .is_actionable(),
                            self.writer_action_availability(WriterAction::CancelClose)
                                .is_actionable(),
                        ));
                    }
                } else {
                    cues.push(
                        "Writer close status is unavailable, so no next stage is proven. Refresh or run Close status again before staging another close action."
                            .to_string(),
                    );
                }
            } else if result.ok
                && matches!(
                    result.action,
                    WriterAction::BeginClose
                        | WriterAction::AdvanceClose
                        | WriterAction::CancelClose
                )
            {
                cues.push(
                    "A historical writer-close result is displayed. Run Close status for current read evidence; the current release cannot stage another wallet change."
                        .to_string(),
                );
            }
        }
        if let Some(confirmation) = self
            .trading
            .ticket
            .as_ref()
            .and_then(|ticket| ticket.confirmation.as_ref())
        {
            let summary = &confirmation.prepared.summary;
            cues.push(format!(
                "Order confirmation popup: {} {} contracts at {} | max loss {} | max payout {}. Only the user can confirm.",
                summary.action.label(),
                summary.qty,
                format_usd(summary.price),
                format_usd(summary.total_max_loss),
                format_usd(summary.total_max_payout)
            ));
        }
        if let Some(result) = self.trading.result_modal.as_ref() {
            if result.waiting {
                cues.push(
                    "Order result popup: waiting for current finalized state; no transaction target was prepared. The Guide cannot retry or send."
                        .to_string(),
                );
            } else if result.ok {
                cues.push(
                    "Order result popup: request accepted. Check Activity for transaction status and fills."
                        .to_string(),
                );
            } else {
                let reason = result
                    .failure_reason
                    .as_deref()
                    .map(user_safe_trade_failure_reason)
                    .unwrap_or("The order could not be submitted.");
                let next = if result.details_in_ticket {
                    "Review the still-open ticket before trying again."
                } else {
                    "Close the popup before trying again."
                };
                cues.push(format!(
                    "Order result popup: request failed — {reason} {next} The Guide cannot retry or send."
                ));
            }
        }
        if cues.is_empty() {
            cues.push(
                "Yellow marks the selected item; red marks a challenge or unavailable action."
                    .to_string(),
            );
        }
        cues.into_iter()
            .map(|cue| cue.chars().take(240).collect())
            .take(12)
            .collect()
    }

    pub(super) fn clear_guide_context_actions(&mut self) {
        self.guide.suggested_actions.clear();
        self.guide.action_preview = None;
        self.guide.highlighted_targets.clear();
        self.guide.comparison_targets.clear();
        self.guide.focused_control = None;
        self.guide.request_target_ids.clear();
        self.guide.request_ui_target_ids.clear();
        self.guide.request_challenge_ids.clear();
        self.guide.pending_continuation = None;
        self.guide.active_question = None;
        self.guide.allow_continuation = false;
    }

    pub(super) fn exact_oracle_node_index(&self, target_id: &str) -> Option<usize> {
        self.oracle_tree()?
            .nodes
            .iter()
            .position(|node| node.node_id == target_id)
    }

    pub(super) fn exact_guide_oracle_node_index(&self, target_id: &str) -> Option<usize> {
        if !self.guide.request_target_ids.contains(target_id) {
            return None;
        }
        self.exact_oracle_node_index(target_id)
    }

    pub(super) fn exact_guide_market_index(&self, target_id: &str) -> Option<usize> {
        if !self.guide.request_target_ids.contains(target_id) {
            return None;
        }
        self.trading
            .dishes
            .iter()
            .position(|dish| guide_market_target_id(&dish.id) == target_id)
    }

    pub(super) fn guide_ui_target_was_offered(&self, target_id: &str) -> bool {
        self.guide.request_ui_target_ids.contains(target_id)
    }

    pub(super) fn exact_guide_trade_action(
        &self,
        target_id: &str,
        requested: guide::GuideTradeAction,
    ) -> Option<(usize, OptionKind, TradeAction)> {
        if self.oracle_phase() != OraclePhase::GameMode {
            return None;
        }
        if !self.guide_ui_target_was_offered(target_id) {
            return None;
        }
        let action = match requested {
            guide::GuideTradeAction::Buy => TradeAction::Buy,
            guide::GuideTradeAction::Sell => TradeAction::Sell,
        };
        let detail = self
            .trading
            .detail
            .as_ref()
            .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))?;
        detail
            .option_quotes
            .iter()
            .enumerate()
            .find(|(_, quote)| {
                let contract_id = guide_contract_target_id(detail, quote);
                guide_trade_action_target_id(action, &contract_id) == target_id
                    && guide_trade_action_is_executable(quote, action)
            })
            .map(|(index, quote)| (index, quote.kind, action))
    }

    pub(super) fn exact_guide_writer_form_action(
        &self,
        target_id: &str,
    ) -> Option<(WriterAction, WriterActionForm)> {
        if !self.guide_ui_target_was_offered(target_id)
            || self.screen != LabScreen::Ledger
            || self.ledger_view != LedgerView::Writers
            || self.writers.form.is_some()
            || self.writers.confirmation.is_some()
            || self.writers.interaction_is_locked()
        {
            return None;
        }
        let sleeve = self.selected_writer_sleeve()?;
        let sleeve_address = self.selected_writer_sleeve_address()?;
        WriterAction::ALL.into_iter().find_map(|action| {
            let form = writers::writer_form_for_action(action, Some(sleeve));
            (!form.fields.is_empty()
                && self.writer_action_availability(action).is_actionable()
                && guide_writer_action_target_id(action, &sleeve_address) == target_id)
                .then_some((action, form))
        })
    }

    pub(super) fn exact_guide_liquidity_form_action(
        &self,
        target_id: &str,
    ) -> Option<liquidity::LiquidityPreviewForm> {
        if !self.guide_ui_target_was_offered(target_id)
            || self.screen != LabScreen::Ledger
            || self.ledger_view != LedgerView::Positions
            || self.liquidity_preview_form.is_some()
            || self.liquidity_preview_is_running()
        {
            return None;
        }
        let owner = self.wallet.pubkey.as_deref()?;
        let form = liquidity::LiquidityPreviewForm::from_position(
            self.liquidity_position_rows()
                .get(self.ledger_position_selected),
        );
        (guide_liquidity_preview_target_id(owner, &form) == target_id).then_some(form)
    }

    pub(super) fn exact_guide_oracle_form_action(
        &self,
        target_id: &str,
    ) -> Option<(OracleAction, OracleFormMode)> {
        if !self.guide_ui_target_was_offered(target_id) {
            return None;
        }
        let node = self.selected_oracle_node()?;
        let context = self.selected_oracle_action_context()?;
        self.visible_oracle_actions()
            .into_iter()
            .find_map(|action| {
                (guide_oracle_action_target_id(action, &node.node_id) == target_id
                    && action.availability(context) == OracleActionAvailability::Active)
                    .then(|| OracleFormMode::from_action(action).map(|mode| (action, mode)))
                    .flatten()
            })
    }

    pub(super) fn exact_guide_oracle_action_target(&self, target_id: &str) -> Option<OracleAction> {
        let node = self.selected_oracle_node()?;
        self.visible_oracle_actions()
            .into_iter()
            .find(|action| guide_oracle_action_target_id(*action, &node.node_id) == target_id)
    }

    pub(super) fn exact_guide_contract_index(
        &self,
        target_id: &str,
    ) -> Option<(usize, OptionKind)> {
        if !self.guide.request_target_ids.contains(target_id) {
            return None;
        }
        let detail = self
            .trading
            .detail
            .as_ref()
            .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))?;
        detail
            .option_quotes
            .iter()
            .enumerate()
            .find(|(_, quote)| guide_contract_target_id(detail, quote) == target_id)
            .map(|(index, quote)| (index, quote.kind))
    }

    pub(super) fn apply_guide_open_target(
        &mut self,
        target_id: &str,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> Result<(), String> {
        if !self.guide_ui_target_was_offered(target_id) || target_id.starts_with("control:") {
            return Err(
                "That Petri view was not offered for this Guide request. Nothing changed."
                    .to_string(),
            );
        }
        if let Some(action) = self.exact_guide_oracle_action_target(target_id) {
            if let Some(index) = self
                .visible_oracle_actions()
                .iter()
                .position(|candidate| *candidate == action)
            {
                self.oracle.selected = index;
            }
            self.set_focus(LabFocus::OracleActions);
            self.guide.focused_control = None;
            self.status = format!(
                "{} selected. Review its phase status and details; press Enter yourself to open it.",
                action.label()
            );
            return Ok(());
        }
        match target_id {
            "surface:back" => self.go_back(),
            "surface:home" => self.open_home(),
            "surface:options" => self.open_chain(),
            "surface:chart" => self.open_chart(backend_url, fetch_tx),
            "surface:oracle-entry" => self.open_oracle_intro(),
            "surface:oracle-earn" => self.open_oracle_earn(backend_url, fetch_tx),
            "surface:oracle" => self.open_oracle(backend_url, fetch_tx),
            "surface:oracle-help" => self.open_oracle_help(),
            "surface:help" => self.open_home_help(HomeHelpTopic::Overview, fetch_tx),
            "surface:market-detail" => self.open_detail(),
            "surface:detail:overview" => {
                self.open_detail();
                self.select_detail_view(DetailView::Overview, backend_url, fetch_tx);
            }
            "surface:detail:settlement" => {
                self.open_detail();
                self.select_detail_view(DetailView::Settlement, backend_url, fetch_tx);
            }
            "surface:contract-trades" => self.open_activity(),
            "surface:wallet-ledger" => self.open_ledger(backend_url, fetch_tx),
            "surface:ledger:account" => {
                self.set_screen(LabScreen::Ledger);
                self.select_ledger_view(LedgerView::Account, backend_url, fetch_tx);
            }
            "surface:ledger:positions" => {
                self.set_screen(LabScreen::Ledger);
                self.select_ledger_view(LedgerView::Positions, backend_url, fetch_tx);
            }
            "surface:ledger:writers" => {
                self.set_screen(LabScreen::Ledger);
                self.select_ledger_view(LedgerView::Writers, backend_url, fetch_tx);
            }
            "surface:ledger:history" => {
                self.set_screen(LabScreen::Ledger);
                self.select_ledger_view(LedgerView::History, backend_url, fetch_tx);
            }
            "surface:staking" => self.open_staking(fetch_tx),
            "surface:connect-agents" => self.open_home_help(HomeHelpTopic::Agents, fetch_tx),
            _ if target_id.starts_with("month:") => {
                let index = self
                    .trading
                    .detail
                    .as_ref()
                    .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))
                    .and_then(|detail| {
                        detail.expiries.iter().position(|expiry| {
                            guide_month_target_id(&detail.id, &expiry.id) == target_id
                        })
                    })
                    .ok_or_else(|| {
                        "That month is no longer listed. Nothing changed.".to_string()
                    })?;
                if !self.select_market_series_index(index) {
                    return Err("That month could not be selected. Nothing changed.".to_string());
                }
                self.activate_market_series(backend_url, fetch_tx);
            }
            _ if target_id.starts_with("chart-range:") => {
                let range = match target_id.trim_start_matches("chart-range:") {
                    "1h" => ChartRangeValue::OneHour,
                    "24h" => ChartRangeValue::TwentyFourHours,
                    "7d" => ChartRangeValue::SevenDays,
                    "30d" => ChartRangeValue::ThirtyDays,
                    "all" => ChartRangeValue::All,
                    _ => {
                        return Err(
                            "That chart range is no longer available. Nothing changed.".to_string()
                        );
                    }
                };
                self.set_chart_range(backend_url, fetch_tx, range);
            }
            _ if target_id.starts_with("help-page:") => {
                let link = self
                    .help
                    .index
                    .categories
                    .iter()
                    .flat_map(|category| category.pages.iter())
                    .find(|page| guide_help_page_target_id(&page.id) == target_id)
                    .cloned()
                    .ok_or_else(|| {
                        "That help topic is no longer available. Nothing changed.".to_string()
                    })?;
                self.open_home_help(HomeHelpTopic::Overview, fetch_tx);
                self.help.selected_page_id = link.id.clone();
                if !self.cache.help_pages().contains_key(&link.id)
                    && let Some(page) = gitbook::bundled_page(&link)
                {
                    self.cache_help_page(link.id.clone(), page);
                }
                self.help.sync_nav_selection();
                self.help.pane = HelpPane::Article;
                self.help.article_scroll = 0;
                self.request_current_help_page(fetch_tx, true);
            }
            _ => {
                return Err(
                    "That Petri target is not a safe navigation view. Nothing changed.".to_string(),
                );
            }
        }
        self.guide.focused_control = None;
        Ok(())
    }

    pub(super) fn apply_guide_stage_trade(
        &mut self,
        target_id: &str,
        requested_action: guide::GuideTradeAction,
        fields: &[guide::GuideFieldValue],
    ) -> Result<(), String> {
        let (price, quantity) = validate_guide_trade_fields(fields)?;
        let (index, kind, action) = self
            .exact_guide_trade_action(target_id, requested_action)
            .ok_or_else(|| {
                "That contract is not currently quoted, liquid, and on-chain, so no ticket was opened."
                    .to_string()
            })?;
        if !self.select_option_index(kind, index) {
            return Err("That contract could not be selected. Nothing changed.".to_string());
        }
        self.select_trade_action(action);
        let Some(ticket) = self.trading.ticket.as_mut() else {
            return Err(
                "Petri could not open that reviewable ticket. Nothing changed.".to_string(),
            );
        };
        if let Some(price) = price {
            ticket.premium_input = price;
        }
        if let Some(quantity) = quantity {
            ticket.quantity_input = quantity;
        }
        ticket.field = TradeTicketField::Quantity;
        ticket.clear_review();
        self.trading.ticket_field_flash = None;
        self.guide.focused_control = Some("control:trade:review".to_string());
        self.status = "Ticket staged. Check price and contracts; press Enter yourself to prepare the exact trade. Review its gross input, net minimum output, and costs before approving."
            .to_string();
        Ok(())
    }

    pub(super) fn apply_guide_stage_oracle_form(
        &mut self,
        target_id: &str,
        fields: &[guide::GuideFieldValue],
    ) -> Result<(), String> {
        let (action, mode) = self
            .exact_guide_oracle_form_action(target_id)
            .ok_or_else(|| {
                "That evidence task is no longer available for the selected source and phase. Nothing changed."
                    .to_string()
            })?;
        let mut form = self.oracle_form_draft_for_selected(mode).ok_or_else(|| {
            "That evidence form is no longer available. Nothing changed.".to_string()
        })?;
        apply_guide_oracle_fields(&mut form, fields)?;
        if let Some(index) = self
            .visible_oracle_actions()
            .iter()
            .position(|candidate| *candidate == action)
        {
            self.oracle.selected = index;
        }
        self.set_focus(LabFocus::OracleActions);
        self.oracle.form = Some(form);
        self.oracle.form_field_flash = None;
        self.guide.focused_control = Some("control:oracle:queue-draft".to_string());
        self.status =
            "Evidence form staged. Check every source and archive field; press Enter to queue the local draft yourself."
                .to_string();
        Ok(())
    }

    pub(super) fn apply_guide_stage_action_form(
        &mut self,
        target_id: &str,
        fields: &[guide::GuideFieldValue],
    ) -> Result<(), String> {
        if let Some(mut form) = self.exact_guide_liquidity_form_action(target_id) {
            apply_guide_liquidity_fields(&mut form, fields)?;
            self.liquidity_preview_form = Some(form);
            self.liquidity_preview_result = None;
            self.ledger_pane = LedgerPane::Detail;
            self.guide.focused_control = Some("control:liquidity:preview".to_string());
            self.status = "Unsigned liquidity preview fields staged. Check the exact position nonce and every bin entry, then press Enter yourself; nothing will be prepared, signed, or submitted."
                .to_string();
            return Ok(());
        }
        let (action, mut form) = self
            .exact_guide_writer_form_action(target_id)
            .ok_or_else(|| {
                "That writer form was not offered for the selected sleeve in this Guide request. Nothing changed."
                    .to_string()
            })?;
        apply_guide_writer_fields(&mut form, fields)?;
        self.writers.action_selected = WriterAction::ALL
            .iter()
            .position(|candidate| *candidate == action)
            .unwrap_or(self.writers.action_selected);
        self.ledger_pane = LedgerPane::Detail;
        self.writers.form = Some(form);
        self.writers.action_result = None;
        let (control, instruction) = if action.signs_and_submits() {
            (
                "control:writer:review",
                "press Enter yourself to open wallet review; the Guide cannot confirm or submit",
            )
        } else {
            (
                "control:writer:run",
                "press Enter yourself to run the read-only request",
            )
        };
        self.guide.focused_control = Some(control.to_string());
        self.status = format!(
            "{} fields staged. Check every address and exact atom amount; {instruction}.",
            action.label()
        );
        Ok(())
    }

    pub(super) fn apply_guide_fill_active_form(
        &mut self,
        target_id: &str,
        fields: &[guide::GuideFieldValue],
    ) -> Result<(), String> {
        if !self.guide_ui_target_was_offered(target_id) {
            return Err(
                "That form was not offered for this Guide request. Nothing changed.".to_string(),
            );
        }
        if target_id == "form:trade-ticket" {
            let (price, quantity) = validate_guide_trade_fields(fields)?;
            let mut ticket = self
                .trading
                .ticket
                .clone()
                .filter(|ticket| ticket.confirmation.is_none() && !ticket.submitting)
                .ok_or_else(|| {
                    "The order is already in review or submission, so its fields were not changed."
                        .to_string()
                })?;
            if let Some(price) = price {
                ticket.premium_input = price;
            }
            if let Some(quantity) = quantity {
                ticket.quantity_input = quantity;
            }
            ticket.field = TradeTicketField::Quantity;
            ticket.clear_review();
            self.trading.ticket = Some(ticket);
            self.guide.focused_control = Some("control:trade:review".to_string());
            self.status = "Ticket fields staged. Press Enter yourself to prepare the exact trade; signing requires your separate confirmation."
                .to_string();
            return Ok(());
        }
        let current_form_id = self.oracle.form.as_ref().map(guide_oracle_form_id);
        if current_form_id.as_deref() == Some(target_id) {
            let mut form = self.oracle.form.clone().expect("form id requires form");
            apply_guide_oracle_fields(&mut form, fields)?;
            self.oracle.form = Some(form);
            self.guide.focused_control = Some("control:oracle:queue-draft".to_string());
            self.status = "Evidence fields staged. Press Enter to queue the local draft yourself."
                .to_string();
            return Ok(());
        }
        let current_writer_form_id = self.writers.form.as_ref().map(guide_writer_form_id);
        if current_writer_form_id.as_deref() == Some(target_id) {
            if self.writers.confirmation.is_some() || self.writers.interaction_is_locked() {
                return Err(
                    "The writer action is already in final review or running, so its fields were not changed."
                        .to_string(),
                );
            }
            let mut form = self
                .writers
                .form
                .clone()
                .expect("form id requires writer form");
            apply_guide_writer_fields(&mut form, fields)?;
            let action = form.action;
            self.writers.form = Some(form);
            let (control, instruction) = if action.signs_and_submits() {
                (
                    "control:writer:review",
                    "This wallet change is unavailable in the current release.",
                )
            } else {
                (
                    "control:writer:run",
                    "Press Enter yourself to run the read-only request.",
                )
            };
            self.guide.focused_control = Some(control.to_string());
            self.status = format!("Writer fields staged. {instruction}");
            return Ok(());
        }
        if target_id == guide_liquidity_preview_form_id() {
            if self.liquidity_preview_is_running() {
                return Err(
                    "The unsigned liquidity preview is already loading, so its fields were not changed."
                        .to_string(),
                );
            }
            let mut form = self.liquidity_preview_form.clone().ok_or_else(|| {
                "The liquidity preview form is no longer open. Nothing changed.".to_string()
            })?;
            apply_guide_liquidity_fields(&mut form, fields)?;
            self.liquidity_preview_form = Some(form);
            self.guide.focused_control = Some("control:liquidity:preview".to_string());
            self.status = "Liquidity preview fields staged. Press Enter yourself to request the unsigned Lean preview; Petri will not prepare, sign, or submit."
                .to_string();
            return Ok(());
        }
        Err("That open form cannot be filled by the Guide. Nothing changed.".to_string())
    }

    pub(super) fn apply_guide_focus_control(&mut self, target_id: &str) -> Result<(), String> {
        if !self.guide_ui_target_was_offered(target_id) {
            return Err(
                "That control was not offered for this Guide request. Nothing changed.".to_string(),
            );
        }
        match target_id {
            "control:trade:review" => {
                let Some(ticket) = self.trading.ticket.as_mut() else {
                    return Err("The trade ticket is no longer open. Nothing changed.".to_string());
                };
                if ticket.confirmation.is_some() || ticket.submitting {
                    return Err("The ticket is already past review. Nothing changed.".to_string());
                }
                ticket.field = TradeTicketField::Quantity;
                self.trading.flash_ticket_field(TradeTicketField::Quantity);
                self.status = "Review is highlighted. Press Enter yourself to prepare the exact trade, then review and explicitly approve its amounts."
                    .to_string();
            }
            "control:trade:confirm" => {
                if !self.trading.confirmation_is_open() {
                    return Err(
                        "Order confirmation is no longer open. Nothing changed.".to_string()
                    );
                }
                return Err(crate::current_release::write_unavailable_error().to_string());
            }
            "control:oracle:queue-draft" => {
                let last_editable = self
                    .oracle
                    .form
                    .as_ref()
                    .and_then(|form| form.fields.iter().rposition(|field| field.editable));
                if let Some(index) = last_editable {
                    self.select_oracle_form_field_index(index);
                } else if self.oracle.form.is_none() {
                    return Err("The Oracle form is no longer open. Nothing changed.".to_string());
                }
                self.status = "Queue local draft is highlighted. Press Enter yourself after checking the evidence."
                    .to_string();
            }
            "control:writer:review" | "control:writer:run" => {
                let Some(form) = self.writers.form.as_mut() else {
                    return Err("The writer form is no longer open. Nothing changed.".to_string());
                };
                if self.writers.confirmation.is_some() || self.writers.action_inflight.is_some() {
                    return Err(
                        "The writer action is already past field review. Nothing changed."
                            .to_string(),
                    );
                }
                let expected = if form.action.signs_and_submits() {
                    "control:writer:review"
                } else {
                    "control:writer:run"
                };
                if target_id != expected {
                    return Err(
                        "That writer control does not match the open form. Nothing changed."
                            .to_string(),
                    );
                }
                if form.action.signs_and_submits() {
                    return Err(crate::current_release::write_unavailable_error().to_string());
                }
                form.selected_field = form.fields.len().saturating_sub(1);
                self.status = if form.action.signs_and_submits() {
                    "Writer review is highlighted. Check every value, then press Enter yourself; the Guide cannot review, confirm, sign, or send."
                        .to_string()
                } else {
                    "Run read-only request is highlighted. Check every identifier, then press Enter yourself."
                        .to_string()
                };
            }
            "control:writer:confirm" => {
                let Some(confirmation) = self.writers.confirmation.as_ref() else {
                    return Err(
                        "The writer confirmation is no longer open. Nothing changed.".to_string(),
                    );
                };
                self.status = format!(
                    "Writer confirmation is highlighted with {} selected. Only you may change the choice or activate it; the Guide cannot confirm, sign, or send.",
                    match confirmation.choice {
                        UserActionConfirmationChoice::Cancel => "Cancel",
                        UserActionConfirmationChoice::Confirm => "Confirm",
                    }
                );
            }
            "control:liquidity:preview" => {
                let Some(form) = self.liquidity_preview_form.as_mut() else {
                    return Err(
                        "The liquidity preview form is no longer open. Nothing changed."
                            .to_string(),
                    );
                };
                if self.liquidity_preview_inflight.is_some() {
                    return Err(
                        "The unsigned liquidity preview is already loading. Nothing changed."
                            .to_string(),
                    );
                }
                form.selected_field = liquidity::LiquidityPreviewField::ALL.len() - 1;
                self.ledger_pane = LedgerPane::Detail;
                self.status = "Unsigned preview is highlighted. Check the exact owner-bound inputs, then press Enter yourself; Petri will not prepare, sign, or submit."
                    .to_string();
            }
            "control:terms:open" | "control:terms:switch-wallet" | "control:terms:accept" => {
                if self.screen != LabScreen::Terms {
                    return Err("Wallet Terms are no longer open. Nothing changed.".to_string());
                }
                self.status = match target_id {
                    "control:terms:open" => "Open Terms is highlighted. Press T yourself.",
                    "control:terms:switch-wallet" => {
                        "Switch wallet is highlighted. Press W and enter the path privately."
                    }
                    _ => "Accept is highlighted. Review the Terms, then press Enter yourself.",
                }
                .to_string();
            }
            "control:mcp:connection" => {
                if self.screen != LabScreen::Help || self.home_help_topic != HomeHelpTopic::Agents {
                    return Err(
                        "The assistant connection page is no longer open. Nothing changed."
                            .to_string(),
                    );
                }
                self.status = "The connection button is highlighted. Only your click changes assistant settings. MCP execution requires your approval of an exact reviewed operation."
                    .to_string();
            }
            "control:update" => {
                self.status = "The update control is highlighted. Press U yourself to start the guarded update."
                    .to_string();
            }
            _ => {
                return Err(
                    "That target is not a user-only Petri control. Nothing changed.".to_string(),
                );
            }
        }
        self.guide.focused_control = Some(target_id.to_string());
        Ok(())
    }

    pub(super) fn apply_guide_response(
        &mut self,
        response: guide::GuideResponse,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.guide.push_message(
            guide::GuideConversationRole::Assistant,
            response.assistant_text,
        );
        if let Some(notice) = response.safety_notice {
            self.guide
                .push_message(guide::GuideConversationRole::Assistant, notice);
        }
        self.guide.suggested_actions = response.suggested_actions;
        self.guide.selected_suggestion = None;
        self.guide.action_preview = response.action_preview;

        if let Some(command) = response.ui_command.as_ref() {
            let Some(question) = self.guide.active_question.clone() else {
                self.status = "Guide answer ready. Press g to ask another question.".to_string();
                return;
            };
            let command_result = self.apply_guide_command(command, backend_url, fetch_tx);
            self.guide.tool_step = self.guide.tool_step.saturating_add(1);
            let tool_result = guide_tool_result(self.guide.tool_step, command, &command_result);
            if self.guide.tool_step >= guide::GUIDE_MAX_TOOL_STEPS {
                if let Err(message) = command_result {
                    self.guide.push_message(
                        guide::GuideConversationRole::Assistant,
                        format!(
                            "I could not finish that navigation after {} safe attempts. {message}",
                            guide::GUIDE_MAX_TOOL_STEPS
                        ),
                    );
                }
                self.guide.active_question = None;
                self.guide.allow_continuation = false;
                self.guide.pending_continuation = None;
                self.status = "Guide stopped at the safe navigation limit.".to_string();
                return;
            }
            self.queue_guide_tool_followup(question, tool_result, fetch_tx);
            return;
        }
        if let Some(target_id) = response.highlight_target
            && response.ui_command.is_none()
            && let Err(message) = self.apply_guide_highlight(&target_id, backend_url, fetch_tx)
        {
            self.guide
                .push_message(guide::GuideConversationRole::Assistant, message.clone());
            self.status = message;
            self.guide.active_question = None;
            self.guide.allow_continuation = false;
            self.guide.pending_continuation = None;
            return;
        }
        if self.continue_guide_after_navigation(fetch_tx) {
            return;
        }
        if self.guide.pending_continuation.is_some() {
            return;
        }
        self.guide.active_question = None;
        self.guide.allow_continuation = false;
        self.status = "Guide answer ready. Press g to ask another question.".to_string();
    }

    pub(super) fn apply_guide_command(
        &mut self,
        command: &guide::GuideCommand,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> Result<(), String> {
        let allowed_on_terms = matches!(
            command,
            guide::GuideCommand::ExplainCurrentScreen | guide::GuideCommand::ShowNextAction
        ) || matches!(
            command,
            guide::GuideCommand::FocusControl { target_id }
                if target_id.starts_with("control:terms:")
        );
        if self.screen == LabScreen::Terms && !allowed_on_terms {
            return Err(
                "Review and accept the Terms yourself before Petri can leave this screen. Nothing changed."
                    .to_string(),
            );
        }
        let can_work_inside_open_review = matches!(
            command,
            guide::GuideCommand::ExplainCurrentScreen
                | guide::GuideCommand::ShowNextAction
                | guide::GuideCommand::FillActiveForm { .. }
                | guide::GuideCommand::FocusControl { .. }
        );
        if self.guide_navigation_locked() && !can_work_inside_open_review {
            return Err(
                "Finish or cancel the open review first. The Guide left your current work unchanged."
                    .to_string(),
            );
        }
        match command {
            guide::GuideCommand::ExplainCurrentScreen | guide::GuideCommand::ShowNextAction => {
                Ok(())
            }
            guide::GuideCommand::OpenTarget { target_id } => {
                self.apply_guide_open_target(target_id, backend_url, fetch_tx)?;
                Ok(())
            }
            guide::GuideCommand::StageTrade {
                target_id,
                action,
                fields,
            } => self.apply_guide_stage_trade(target_id, *action, fields),
            guide::GuideCommand::StageOracleForm { target_id, fields } => {
                self.apply_guide_stage_oracle_form(target_id, fields)
            }
            guide::GuideCommand::StageActionForm { target_id, fields } => {
                self.apply_guide_stage_action_form(target_id, fields)
            }
            guide::GuideCommand::FillActiveForm { target_id, fields } => {
                self.apply_guide_fill_active_form(target_id, fields)
            }
            guide::GuideCommand::FocusControl { target_id } => {
                self.apply_guide_focus_control(target_id)
            }
            guide::GuideCommand::SearchOracle { query } => {
                if self.screen != LabScreen::Oracle
                    || self.oracle.view != OracleView::Advanced
                    || self.oracle_tree().is_none()
                {
                    return Err(
                        "Open Oracle evidence first, then ask the Guide to search the loaded source tree."
                            .to_string(),
                    );
                }
                self.oracle.search_input = query.clone();
                self.oracle.search_editing = false;
                self.apply_oracle_search();
                Ok(())
            }
            guide::GuideCommand::OpenMarketContracts { target_id } => {
                let index = self.exact_guide_market_index(target_id).ok_or_else(|| {
                    "That market was not offered for this Guide request. Nothing changed."
                        .to_string()
                })?;
                let market_id = self.trading.dishes[index].id.clone();
                if !self.select_market_index(index, backend_url, fetch_tx) {
                    return Err("That market is no longer available. Nothing changed.".to_string());
                }
                self.open_chain();
                let detail_is_ready = self
                    .trading
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.id.eq_ignore_ascii_case(&market_id));
                if !detail_is_ready && !self.trading.loading_detail {
                    self.request_selected_detail(backend_url, fetch_tx, false);
                }
                self.status = if detail_is_ready {
                    format!("{} contracts ready", market_id.to_uppercase())
                } else {
                    format!(
                        "Opening {} contracts and loading the live chain...",
                        market_id.to_uppercase()
                    )
                };
                Ok(())
            }
            guide::GuideCommand::OpenContract { target_id } => {
                let (index, kind) =
                    self.exact_guide_contract_index(target_id).ok_or_else(|| {
                        "That contract is no longer available on this screen. Nothing changed."
                            .to_string()
                    })?;
                self.open_chain();
                if !self.select_option_index(kind, index) {
                    return Err(
                        "That contract could not be selected. No ticket or order was created."
                            .to_string(),
                    );
                }
                self.guide.highlighted_targets = vec![target_id.clone()];
                self.guide.comparison_targets.clear();
                Ok(())
            }
            guide::GuideCommand::GoToBucket { target_id } => {
                let index = self
                    .exact_guide_oracle_node_index(target_id)
                    .ok_or_else(|| {
                        "That bucket is no longer available on this screen. Nothing changed."
                            .to_string()
                    })?;
                if self
                    .oracle_tree()
                    .and_then(|tree| tree.node(index))
                    .map(|node| node.kind)
                    != Some(OracleNodeKind::RowBucket)
                {
                    return Err(
                        "The requested target is not a product bucket. Nothing changed."
                            .to_string(),
                    );
                }
                self.open_guide_oracle_target(index, backend_url, fetch_tx, LabFocus::OracleTasks);
                self.guide.highlighted_targets = vec![target_id.clone()];
                self.guide.comparison_targets.clear();
                Ok(())
            }
            guide::GuideCommand::HighlightSource { target_id }
            | guide::GuideCommand::OpenSourceDetail { target_id } => {
                let index = self
                    .exact_guide_oracle_node_index(target_id)
                    .ok_or_else(|| {
                        "That source is no longer available on this screen. Nothing changed."
                            .to_string()
                    })?;
                if self
                    .oracle_tree()
                    .and_then(|tree| tree.node(index))
                    .map(|node| node.kind)
                    != Some(OracleNodeKind::TerminalPin)
                {
                    return Err(
                        "The requested target is not a source. Nothing changed.".to_string()
                    );
                }
                let focus = if matches!(command, guide::GuideCommand::OpenSourceDetail { .. }) {
                    LabFocus::OraclePath
                } else {
                    LabFocus::OracleTasks
                };
                self.open_guide_oracle_target(index, backend_url, fetch_tx, focus);
                self.guide.highlighted_targets = vec![target_id.clone()];
                self.guide.comparison_targets.clear();
                Ok(())
            }
            guide::GuideCommand::CompareSources { target_ids } => {
                let first = self.exact_guide_oracle_node_index(&target_ids[0]);
                let second = self.exact_guide_oracle_node_index(&target_ids[1]);
                let Some((first, second)) = first.zip(second) else {
                    return Err(
                        "One of those sources is no longer available. Nothing changed.".to_string(),
                    );
                };
                let both_sources = self.oracle_tree().is_some_and(|tree| {
                    [first, second].into_iter().all(|index| {
                        tree.node(index).map(|node| node.kind) == Some(OracleNodeKind::TerminalPin)
                    })
                });
                if !both_sources {
                    return Err(
                        "Source comparison needs two source IDs. Nothing changed.".to_string()
                    );
                }
                self.open_guide_oracle_target(first, backend_url, fetch_tx, LabFocus::OraclePath);
                self.guide.highlighted_targets = target_ids.to_vec();
                self.guide.comparison_targets = target_ids.to_vec();
                Ok(())
            }
            guide::GuideCommand::OpenChallengeView { challenge_id } => {
                let challenges = self.guide_active_challenges();
                let challenge = match challenge_id.as_deref() {
                    Some(challenge_id) => challenges
                        .iter()
                        .find(|challenge| challenge.challenge_id == challenge_id),
                    None => challenges.first(),
                }
                .cloned()
                .ok_or_else(|| {
                    "That challenge is no longer visible. Nothing changed.".to_string()
                })?;
                if !self
                    .guide
                    .request_challenge_ids
                    .contains(&challenge.challenge_id)
                {
                    return Err(
                        "That challenge was not part of this Guide request. Nothing changed."
                            .to_string(),
                    );
                }
                if let Some(target_id) = challenge.target_id.as_deref()
                    && let Some(index) = self.exact_oracle_node_index(target_id)
                {
                    self.open_guide_oracle_target(
                        index,
                        backend_url,
                        fetch_tx,
                        LabFocus::OraclePath,
                    );
                    self.guide.highlighted_targets = vec![target_id.to_string()];
                } else {
                    self.open_oracle(backend_url, fetch_tx);
                    self.set_focus(LabFocus::OracleOverview);
                    self.guide.highlighted_targets.clear();
                }
                self.guide.comparison_targets.clear();
                Ok(())
            }
            guide::GuideCommand::ShowPhaseTimeline => {
                self.open_oracle(backend_url, fetch_tx);
                self.set_focus(LabFocus::OracleOverview);
                Ok(())
            }
        }
    }

    pub(super) fn apply_guide_highlight(
        &mut self,
        target_id: &str,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> Result<(), String> {
        if self.guide_navigation_locked() {
            return Err(
                "Finish or cancel the open review first. The Guide left your current work unchanged."
                    .to_string(),
            );
        }
        let index = self
            .exact_guide_oracle_node_index(target_id)
            .ok_or_else(|| {
                "That highlight target is no longer available. Nothing changed.".to_string()
            })?;
        self.open_guide_oracle_target(index, backend_url, fetch_tx, LabFocus::OracleTasks);
        self.guide.highlighted_targets = vec![target_id.to_string()];
        self.guide.comparison_targets.clear();
        Ok(())
    }

    pub(super) fn open_guide_oracle_target(
        &mut self,
        index: usize,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        focus: LabFocus,
    ) {
        self.oracle.search_input.clear();
        self.oracle.search_editing = false;
        self.open_oracle(backend_url, fetch_tx);
        self.select_oracle_node(index);
        self.set_focus(focus);
    }
}

fn guide_ledger_view_id(view: LedgerView) -> &'static str {
    match view {
        LedgerView::Account => "account",
        LedgerView::Positions => "positions",
        LedgerView::Writers => "writers",
        LedgerView::History => "history",
    }
}

fn guide_detail_view_id(view: DetailView) -> &'static str {
    match view {
        DetailView::Overview => "overview",
        DetailView::Settlement => "settlement",
    }
}

fn guide_liquidity_preview_form_id() -> &'static str {
    "form:liquidity-preview"
}

fn guide_liquidity_preview_target_id(
    owner: &str,
    form: &liquidity::LiquidityPreviewForm,
) -> String {
    let market = if form.market.trim().is_empty() {
        "manual"
    } else {
        form.market.trim()
    };
    let expiry = if form.expiry.trim().is_empty() {
        "manual"
    } else {
        form.expiry.trim()
    };
    format!(
        "liquidity-preview:{}:{}:{}",
        guide_target_segment(owner),
        guide_target_segment(market),
        guide_target_segment(expiry)
    )
}

fn guide_liquidity_field_snapshots(
    form: &liquidity::LiquidityPreviewForm,
    editable: bool,
) -> Vec<guide::GuideFieldSnapshot> {
    let snapshot = |field_id: &str, label: &str, value: String| guide::GuideFieldSnapshot {
        field_id: field_id.to_string(),
        label: label.to_string(),
        value: (!value.is_empty()).then_some(value),
        required: true,
        editable_by_guide: editable,
    };
    vec![
        snapshot(
            "action",
            "Action (add, remove, or close-position)",
            form.action.cli_value().to_string(),
        ),
        snapshot("market", "Exact market id", form.market.clone()),
        snapshot("expiry", "Exact expiry id", form.expiry.clone()),
        snapshot(
            "position_nonce",
            "Authoritative position nonce",
            form.position_nonce.clone(),
        ),
        snapshot(
            "entries",
            "Entries (BIN:AMOUNT_A:AMOUNT_B:AMOUNT_C)",
            form.entries.chars().take(1024).collect(),
        ),
    ]
}

fn guide_liquidity_value_is_safe(value: &str, maximum_chars: usize) -> bool {
    value.chars().count() <= maximum_chars
        && !value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{202a}'
                        | '\u{202b}'
                        | '\u{202c}'
                        | '\u{202d}'
                        | '\u{202e}'
                        | '\u{2066}'
                        | '\u{2067}'
                        | '\u{2068}'
                        | '\u{2069}'
                )
        })
}

fn apply_guide_liquidity_fields(
    form: &mut liquidity::LiquidityPreviewForm,
    fields: &[guide::GuideFieldValue],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for patch in fields {
        if !seen.insert(patch.field_id.as_str()) {
            return Err(
                "A liquidity preview field was provided twice. Nothing changed.".to_string(),
            );
        }
        let value = patch.value.trim();
        match patch.field_id.as_str() {
            "action" => {
                if !guide_liquidity_value_is_safe(value, 32) {
                    return Err("Liquidity action is invalid. Nothing changed.".to_string());
                }
                form.action = match value.to_ascii_lowercase().replace('_', "-").as_str() {
                    "add" => liquidity::LiquidityPreviewAction::Add,
                    "remove" => liquidity::LiquidityPreviewAction::Remove,
                    "close" | "close-position" | "close position" => {
                        liquidity::LiquidityPreviewAction::ClosePosition
                    }
                    _ => {
                        return Err(
                            "Liquidity action must be add, remove, or close-position. Nothing changed."
                                .to_string(),
                        );
                    }
                };
                form.selected_field = 0;
            }
            "market" | "expiry" => {
                let maximum_chars = if patch.field_id == "market" { 64 } else { 128 };
                if value.is_empty()
                    || !guide_liquidity_value_is_safe(value, maximum_chars)
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                {
                    return Err(format!(
                        "{} must be an exact current identifier. Nothing changed.",
                        if patch.field_id == "market" {
                            "Market"
                        } else {
                            "Expiry"
                        }
                    ));
                }
                if patch.field_id == "market" {
                    form.market = value.to_string();
                    form.selected_field = 1;
                } else {
                    form.expiry = value.to_string();
                    form.selected_field = 2;
                }
            }
            "position_nonce" => {
                let canonical = !value.is_empty()
                    && (value == "0" || !value.starts_with('0'))
                    && value.bytes().all(|byte| byte.is_ascii_digit())
                    && value.parse::<u64>().is_ok();
                if !canonical {
                    return Err(
                        "Position nonce must be a canonical unsigned 64-bit integer from an authoritative source. Nothing changed."
                            .to_string(),
                    );
                }
                form.position_nonce = value.to_string();
                form.selected_field = 3;
            }
            "entries" => {
                if !guide_liquidity_value_is_safe(value, 4096)
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_digit()
                            || byte.is_ascii_whitespace()
                            || matches!(byte, b':' | b',' | b';')
                    })
                {
                    return Err(
                        "Liquidity entries contain unsupported characters. Nothing changed."
                            .to_string(),
                    );
                }
                let entries = liquidity::liquidity_preview_entries(value)
                    .map_err(|message| format!("{message} Nothing changed."))?;
                if entries.iter().any(|entry| {
                    let parts = entry.split(':').collect::<Vec<_>>();
                    parts.len() != 4
                        || parts.iter().any(|part| {
                            part.is_empty()
                                || (part.len() > 1 && part.starts_with('0'))
                                || !part.bytes().all(|byte| byte.is_ascii_digit())
                        })
                }) {
                    return Err(
                        "Each liquidity entry must contain four canonical unsigned decimals. Nothing changed."
                            .to_string(),
                    );
                }
                form.entries = value.to_string();
                form.selected_field = 4;
            }
            _ => {
                return Err(format!(
                    "{} is not a field in the liquidity preview form. Nothing changed.",
                    patch.field_id
                ));
            }
        }
    }
    Ok(())
}

fn guide_writer_action_id(action: WriterAction) -> &'static str {
    match action {
        WriterAction::Show => "show",
        WriterAction::Policy => "policy",
        WriterAction::Deposit => "deposit",
        WriterAction::Bid => "bid",
        WriterAction::Withdraw => "withdraw",
        WriterAction::Refunds => "refunds",
        WriterAction::Refund => "refund",
        WriterAction::Liquidity => "liquidity",
        WriterAction::LiquidityInitialize => "liquidity-initialize",
        WriterAction::LiquidityAdd => "liquidity-add",
        WriterAction::LiquidityRemove => "liquidity-remove",
        WriterAction::LiquiditySweep => "liquidity-sweep",
        WriterAction::ClosePreview => "close-preview",
        WriterAction::BeginClose => "begin-close",
        WriterAction::AdvanceClose => "advance-close",
        WriterAction::CancelClose => "cancel-close",
        WriterAction::CloseStatus => "close-status",
        WriterAction::ClaimLong => "claim-long",
        WriterAction::ClaimFlat => "claim-flat",
        WriterAction::TransferFlat => "transfer-flat",
        WriterAction::Refresh => "refresh",
    }
}

fn guide_writer_close_status_cues(
    payload: &Value,
    advance_available: bool,
    cancel_available: bool,
) -> Vec<String> {
    let rendered = crate::writer_output::render_writer_close_status(payload);
    let mut cues = Vec::new();
    if let Some(status) = rendered
        .lines()
        .find(|line| line.starts_with("Request status:"))
    {
        cues.push(format!("Writer close read: {status}"));
    }
    if crate::writer_output::validate_current_writer_close_status(payload).is_err() {
        cues.push(
            "Next stage: unavailable. The close-request response is not the exact current DTO, so the Guide cannot verify or stage a continuation. Refresh or run Close status again."
                .to_string(),
        );
        return cues;
    }
    if let Some(next) = rendered.lines().find(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("backend-reported next stage:")
            || line.starts_with("verified next stage:")
            || line.starts_with("next stage:")
            || line.starts_with("available action:")
            || line.starts_with("available actions:")
            || line.starts_with("available forward action:")
            || line.starts_with("cancellation state:")
    }) {
        let next = next
            .strip_prefix("Backend-reported next stage:")
            .or_else(|| next.strip_prefix("Verified next stage:"))
            .map(|value| format!("Verified next stage:{}", value))
            .or_else(|| {
                (next.starts_with("Available action:")
                    || next.starts_with("Available actions:")
                    || next.starts_with("Available forward action:"))
                .then(|| format!("Verified {next}"))
            })
            .unwrap_or_else(|| next.to_string());
        let normalized = next.to_ascii_lowercase();
        let action = if normalized.contains("deposit the required claim basket")
            || normalized.contains("finalize the close")
            || normalized.contains("advance one lean-selected basket step")
            || normalized.contains("available actions: finalize")
            || normalized.contains("may finalize with `petri writers close --close-request")
        {
            Some(match (advance_available, cancel_available) {
                (true, true) => "Stage Advance close or Cancel close for user review.",
                (true, false) => "Stage Advance close for user review.",
                (false, true) => "Only Cancel close is currently available for user review.",
                (false, false) => {
                    "Close continuation requires finalized V3 permission and initialized business state."
                }
            })
        } else if normalized.contains("cancellation") {
            Some(if cancel_available {
                "Stage Cancel close for user review."
            } else {
                "Cancel close is unavailable under the current action mask; it cannot be staged or aliased."
            })
        } else {
            None
        };
        cues.push(next);
        if let Some(action) = action {
            cues.push(
                if action.starts_with("Stage") || action.starts_with("Only") {
                    format!("{action} The Guide cannot review, confirm, sign, or send.")
                } else {
                    action.to_string()
                },
            );
        }
    }
    cues
}

fn guide_writer_action_target_id(action: WriterAction, sleeve_address: &str) -> String {
    format!(
        "writer-action:{}:{}",
        guide_writer_action_id(action),
        sleeve_address
    )
}

fn guide_writer_form_id(form: &WriterActionForm) -> String {
    format!("form:writer:{}", guide_writer_action_id(form.action))
}

fn guide_writer_field_snapshot(
    field: &ActionFormField,
    editable: bool,
) -> guide::GuideFieldSnapshot {
    guide::GuideFieldSnapshot {
        field_id: field.key.to_string(),
        label: field.label.to_string(),
        value: (!field.secret && !field.value.is_empty()).then(|| field.value.clone()),
        required: field.required,
        editable_by_guide: editable && !field.secret,
    }
}

fn apply_guide_writer_fields(
    form: &mut WriterActionForm,
    fields: &[guide::GuideFieldValue],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut patches = Vec::new();
    for patch in fields {
        if !seen.insert(patch.field_id.as_str()) {
            return Err("A writer field was provided twice. Nothing changed.".to_string());
        }
        if patch.value.chars().count() > 160
            || patch.value.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '\u{202a}'
                            | '\u{202b}'
                            | '\u{202c}'
                            | '\u{202d}'
                            | '\u{202e}'
                            | '\u{2066}'
                            | '\u{2067}'
                            | '\u{2068}'
                            | '\u{2069}'
                    )
            })
        {
            return Err("A writer field is too long or invalid. Nothing changed.".to_string());
        }
        let Some((index, field)) = form
            .fields
            .iter()
            .enumerate()
            .find(|(_, field)| field.key == patch.field_id)
        else {
            return Err(format!(
                "{} is not a field in this writer form. Nothing changed.",
                patch.field_id
            ));
        };
        if field.secret {
            return Err(format!(
                "{} is private and cannot be changed by the Guide. Nothing changed.",
                field.label
            ));
        }
        let value = patch.value.trim().to_string();
        let validation_form = WriterActionForm {
            action: form.action,
            fields: vec![ActionFormField {
                value: value.clone(),
                ..field.clone()
            }],
            selected_field: 0,
        };
        writers::validate_writer_form(&validation_form)
            .map_err(|message| format!("{message} Nothing changed."))?;
        patches.push((index, value));
    }
    for (index, value) in patches {
        if let Some(field) = form.fields.get_mut(index) {
            field.value = value;
            form.selected_field = index;
        }
    }
    Ok(())
}

fn guide_trade_fields(
    action: TradeAction,
    price: Option<String>,
    quantity: Option<String>,
    editable: bool,
) -> Vec<guide::GuideFieldSnapshot> {
    [
        ("price", action.price_label(), price),
        ("quantity", "Contracts", quantity),
    ]
    .into_iter()
    .map(|(field_id, label, value)| guide::GuideFieldSnapshot {
        field_id: field_id.to_string(),
        label: label.to_string(),
        value,
        required: true,
        editable_by_guide: editable,
    })
    .collect()
}

// Cosmetic projection only. Final controls and their user-only authority remain
// explicit at their call sites; this table offers navigation targets only.
fn guide_navigation_targets(
    kind: &str,
    rows: &[(&str, &str, &str)],
) -> Vec<guide::GuideTargetSnapshot> {
    rows.iter()
        .map(|&(id, label, description)| {
            guide_ui_target(id, label, kind, description, "navigate", false)
        })
        .collect()
}
