//! Oracle workbench navigation, forms, action availability, and Home action routing.

use super::*;

impl LabApp {
    pub(super) fn selected_home_action(&self) -> HomeAction {
        HomeAction::ALL
            .get(self.home_selected)
            .copied()
            .unwrap_or(HomeAction::Trade)
    }

    pub(super) fn current_oracle_rewards(&self) -> Option<&SpreadOracleRewardState> {
        let selected_market = self.selected_id();
        let selected_expiry = self.selected_chart_expiry()?.id.as_str();
        let selected_owner = self.wallet.pubkey.as_deref()?;
        self.oracle.rewards.as_ref().filter(|state| {
            state.market_id.eq_ignore_ascii_case(&selected_market)
                && state.expiry_id.eq_ignore_ascii_case(selected_expiry)
                && state.owner_pubkey == selected_owner
        })
    }

    pub(super) fn selected_oracle_reward_claim(&self) -> Option<&SpreadOracleRewardClaim> {
        self.current_oracle_rewards()?
            .claims
            .get(self.oracle.earn_selected)
    }

    pub(super) fn has_claimable_oracle_reward(&self) -> bool {
        self.current_oracle_rewards()
            .is_some_and(|state| !state.claims.is_empty())
    }

    pub(super) fn select_oracle_earn_reward(&mut self, index: usize) -> bool {
        let Some(claim_count) = self
            .current_oracle_rewards()
            .map(|state| state.claims.len())
        else {
            self.oracle.earn_selected = 0;
            return false;
        };
        if index >= claim_count {
            return false;
        }
        self.oracle.earn_selected = index;
        true
    }

    pub(super) fn select_prev_oracle_earn_reward(&mut self) -> bool {
        let next = self.oracle.earn_selected.saturating_sub(1);
        next != self.oracle.earn_selected && self.select_oracle_earn_reward(next)
    }

    pub(super) fn select_next_oracle_earn_reward(&mut self) -> bool {
        self.select_oracle_earn_reward(self.oracle.earn_selected.saturating_add(1))
    }

    pub(super) fn first_settlement_eligible_oracle_escrow(&self) -> Option<&SpreadOracleEscrow> {
        current_spread_oracle_live(self)?.first_settlement_eligible_escrow()
    }

    pub(super) fn visible_oracle_actions(&self) -> Vec<OracleAction> {
        let mut actions = OracleAction::ALL.to_vec();
        if self.has_claimable_oracle_reward() {
            let insert_at = actions
                .iter()
                .position(|action| *action == OracleAction::DepositAmba)
                .unwrap_or(actions.len());
            actions.insert(insert_at, OracleAction::ClaimReward);
        }
        if self.first_settlement_eligible_oracle_escrow().is_some() {
            let insert_at = actions
                .iter()
                .position(|action| *action == OracleAction::DepositAmba)
                .unwrap_or(actions.len());
            actions.insert(insert_at, OracleAction::SettleStake);
        }
        actions
    }

    pub(super) fn selected_oracle_action(&self) -> OracleAction {
        self.visible_oracle_actions()
            .get(self.oracle.selected)
            .copied()
            .unwrap_or(OracleAction::ReviewQueue)
    }

    pub(super) fn select_oracle_action(&mut self, action: OracleAction) -> bool {
        let Some(index) = self
            .visible_oracle_actions()
            .iter()
            .position(|candidate| *candidate == action)
        else {
            return false;
        };
        self.oracle.selected = index;
        true
    }

    pub(super) fn oracle_phase(&self) -> OraclePhase {
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_secs()).ok());
        current_spread_oracle_live(self)
            .and_then(|live| now_ts.and_then(|now_ts| live.scheduled_phase_at(now_ts)))
            .unwrap_or(OraclePhase::Unavailable)
    }

    pub(super) fn select_prev_home_action(&mut self) -> bool {
        if self.home_selected == 0 {
            return false;
        }
        self.home_selected -= 1;
        self.reset_panel_scroll(LabFocus::HomeActions);
        true
    }

    pub(super) fn select_next_home_action(&mut self) -> bool {
        if self.home_selected + 1 >= HomeAction::ALL.len() {
            return false;
        }
        self.home_selected += 1;
        self.reset_panel_scroll(LabFocus::HomeActions);
        true
    }

    pub(super) fn select_prev_oracle_action(&mut self) -> bool {
        self.select_oracle_action_by_offset(-1)
    }

    pub(super) fn select_next_oracle_action(&mut self) -> bool {
        self.select_oracle_action_by_offset(1)
    }

    pub(super) fn select_oracle_action_by_offset(&mut self, offset: isize) -> bool {
        let len = self.visible_oracle_actions().len();
        let start = self.oracle.selected.min(len.saturating_sub(1));
        let next = start as isize + offset;
        if next < 0 || next >= len as isize {
            return false;
        }
        self.oracle.selected = next as usize;
        self.oracle.locked_flash = None;
        self.reset_panel_scroll(LabFocus::OracleActions);
        true
    }

    pub(super) fn clamp_oracle_selection(&mut self) {
        self.oracle.selected = self
            .oracle
            .selected
            .min(self.visible_oracle_actions().len().saturating_sub(1));
        let claim_count = self
            .current_oracle_rewards()
            .map(|state| state.claims.len())
            .unwrap_or_default();
        self.oracle.earn_selected = self.oracle.earn_selected.min(claim_count.saturating_sub(1));
    }

    pub(super) fn oracle_tree(&self) -> Option<&OracleIndexTree> {
        let selected_market = self.selected_id();
        self.oracle
            .tree
            .as_ref()
            .filter(|tree| tree.market_id.eq_ignore_ascii_case(&selected_market))
    }

    pub(super) fn selected_oracle_node(&self) -> Option<&RamxOracleNode> {
        self.oracle_tree()
            .and_then(|tree| tree.node(self.oracle.node_selected))
    }

    pub(super) fn selected_oracle_node_index(&self) -> usize {
        self.oracle_tree()
            .map(|tree| {
                self.oracle
                    .node_selected
                    .min(tree.nodes.len().saturating_sub(1))
            })
            .unwrap_or(DEFAULT_ORACLE_NODE_INDEX)
    }

    pub(super) fn selected_oracle_action_context(&self) -> Option<OracleActionContext> {
        let node = self.selected_oracle_node()?;
        Some(OracleActionContext {
            phase: self.oracle_phase(),
            node_kind: node.kind,
            source: self.oracle_source_action_state(node),
            market_expired: self.oracle_phase() == OraclePhase::MonthClose,
        })
    }

    pub(super) fn oracle_source_action_state(
        &self,
        node: &RamxOracleNode,
    ) -> OracleSourceActionState {
        let mut state = OracleSourceActionState::default();

        for record in self
            .oracle
            .submissions
            .iter()
            .filter(|record| oracle_submission_matches_node(record, node))
        {
            match record.title.as_str() {
                "Opening print" => {
                    state.opening_status = OpeningClaimViewStatus::Pending;
                }
                "Challenge" if record.update_state == Some(OracleUpdateState::Challenged) => {
                    state.has_unresolved_challenge = true;
                    if record.phase == "Opening Print" {
                        state.opening_status = OpeningClaimViewStatus::Challenged;
                    }
                }
                _ => {}
            }
        }

        if let Some(live) = current_spread_oracle_live(self)
            && let Some(observation) = live
                .observations
                .iter()
                .find(|observation| spread_oracle_observation_matches_node(observation, node))
        {
            state.has_active_emergency = observation.emergency.is_some();
            state.has_unresolved_challenge =
                oracle_source_status_has_unresolved_challenge(&observation.status)
                    || observation.opening_status == OpeningClaimViewStatus::Challenged;
            state.opening_status = observation.opening_status;
            state.opening_finalizable = observation.opening_finalizable;
        }

        state
    }

    pub(super) fn select_oracle_node(&mut self, index: usize) {
        let Some(tree) = self.oracle_tree() else {
            self.oracle.node_selected = DEFAULT_ORACLE_NODE_INDEX;
            self.status = self.oracle.tree_load_status();
            return;
        };
        self.oracle.node_selected = index.min(tree.nodes.len().saturating_sub(1));
        self.oracle.locked_flash = None;
        self.reset_panel_scroll(LabFocus::OracleTasks);
        self.clamp_oracle_selection();
        if let Some(node) = self.selected_oracle_node() {
            self.status = format!(
                "Oracle node selected: {} ({})",
                node.label.as_str(),
                node.kind.label()
            );
        }
    }

    pub(super) fn select_oracle_node_by_offset(&mut self, offset: isize) -> bool {
        let candidates = self.oracle_navigation_candidates();
        if candidates.is_empty() {
            return false;
        }
        let current = self.selected_oracle_node_index();
        let current_position = candidates.iter().position(|index| *index == current);
        let next_position = match (current_position, offset.is_positive()) {
            (Some(position), true) if position + 1 < candidates.len() => Some(position + 1),
            (Some(position), false) if position > 0 => Some(position - 1),
            (None, true) => Some(0),
            (None, false) => candidates.len().checked_sub(1),
            _ => None,
        };
        if let Some(position) = next_position {
            self.select_oracle_node(candidates[position]);
            return true;
        }
        false
    }

    pub(super) fn oracle_navigation_candidates(&self) -> Vec<usize> {
        let Some(tree) = self.oracle_tree() else {
            return Vec::new();
        };
        let matches = tree.search_nodes(&self.oracle.search_input);
        if !matches.is_empty() {
            return matches;
        }
        let selected = self.selected_oracle_node_index();
        if selected == tree.root_index() {
            return tree.child_indices(tree.root_index());
        }
        tree.sibling_indices(selected)
    }

    pub(super) fn open_oracle_child(&mut self) -> bool {
        let Some(tree) = self.oracle_tree() else {
            self.status = self.oracle.tree_load_status();
            return false;
        };
        let selected = self.selected_oracle_node_index();
        if let Some(child) = tree.first_child(selected) {
            self.select_oracle_node(child);
            return true;
        }
        self.set_focus(LabFocus::OracleActions);
        self.status = format!(
            "Source selected: {}. Use Context actions to submit, archive, or challenge evidence.",
            self.selected_oracle_node()
                .map(|node| node.label.as_str())
                .unwrap_or("selected source")
        );
        true
    }

    pub(super) fn open_oracle_parent(&mut self) -> bool {
        let Some(tree) = self.oracle_tree() else {
            return false;
        };
        let selected = self.selected_oracle_node_index();
        if let Some(parent) = tree.parent(selected) {
            self.select_oracle_node(parent);
            return true;
        }
        false
    }

    pub(super) fn select_prev_oracle_pin(&mut self) {
        self.select_oracle_pin_by_offset(-1);
    }

    pub(super) fn select_next_oracle_pin(&mut self) {
        self.select_oracle_pin_by_offset(1);
    }

    pub(super) fn select_oracle_pin_by_offset(&mut self, offset: isize) {
        let Some(tree) = self.oracle_tree() else {
            return;
        };
        let pins = tree.terminal_pin_indices();
        if pins.is_empty() {
            return;
        }
        let current_pin = tree
            .first_terminal_pin(self.selected_oracle_node_index())
            .unwrap_or_else(|| pins[0]);
        let current_position = pins
            .iter()
            .position(|index| *index == current_pin)
            .unwrap_or(0);
        let next_position = if offset.is_negative() {
            current_position.checked_sub(1).unwrap_or(pins.len() - 1)
        } else {
            (current_position + 1) % pins.len()
        };
        self.select_oracle_node(pins[next_position]);
    }

    pub(super) fn begin_oracle_search(&mut self) {
        self.oracle.search_editing = true;
        self.status = "Search RAMX-MOD by SKU, row, source, or form factor.".to_string();
    }

    pub(super) fn cancel_oracle_search(&mut self) {
        self.oracle.search_editing = false;
        self.status = "Oracle search closed.".to_string();
    }

    pub(super) fn push_oracle_search_char(&mut self, character: char) {
        if character.is_control() {
            return;
        }
        self.oracle.search_input.push(character);
        self.status = format!("Searching RAMX-MOD for \"{}\"", self.oracle.search_input);
    }

    pub(super) fn backspace_oracle_search_input(&mut self) {
        self.oracle.search_input.pop();
        self.status = if self.oracle.search_input.is_empty() {
            "Search RAMX-MOD by SKU, row, source, or form factor.".to_string()
        } else {
            format!("Searching RAMX-MOD for \"{}\"", self.oracle.search_input)
        };
    }

    pub(super) fn apply_oracle_search(&mut self) {
        self.oracle.search_editing = false;
        let Some(tree) = self.oracle_tree() else {
            self.status = self.oracle.tree_load_status();
            return;
        };
        let matches = tree.search_nodes(&self.oracle.search_input);
        if let Some(index) = matches.first().copied() {
            self.select_oracle_node(index);
            let node_label = self
                .selected_oracle_node()
                .map(|node| node.label.as_str())
                .unwrap_or("selected source");
            self.status = format!(
                "Search matched {}. Enter drills; right opens phase actions.",
                node_label
            );
        } else if self.oracle.search_input.trim().is_empty() {
            self.status = "Search cleared.".to_string();
        } else {
            self.status = format!("No RAMX-MOD match for \"{}\".", self.oracle.search_input);
        }
    }

    pub(super) fn oracle_form_draft_for_selected(
        &self,
        mode: OracleFormMode,
    ) -> Option<OracleFormDraft> {
        let tree = self.oracle_tree()?;
        let mut form = OracleFormDraft::new(
            mode,
            self.selected_oracle_node_index(),
            self.oracle_phase(),
            tree,
        )?;
        if mode == OracleFormMode::RewardClaim {
            let claim = self.selected_oracle_reward_claim()?;
            form.set_field_value("Reward kind", claim.kind.clone());
            form.set_field_value(
                "Source id",
                claim
                    .source_id_hex
                    .as_deref()
                    .unwrap_or(&claim.subject_pda)
                    .to_string(),
            );
            form.set_field_value("Claim id", claim.claim_id_hex.clone().unwrap_or_default());
            form.set_field_value(
                "Challenge id",
                claim.challenge_id_hex.clone().unwrap_or_default(),
            );
            form.set_field_value("Claim PDA", claim.claim_pda.clone().unwrap_or_default());
            form.set_field_value("Subject PDA", claim.subject_pda.clone());
        }
        if mode == OracleFormMode::StakeSettlement {
            let escrow = self.first_settlement_eligible_oracle_escrow()?;
            form.set_field_value("Stake kind", escrow.kind.clone());
            form.set_field_value("Subject PDA", escrow.subject_pda.clone());
            form.set_field_value("Owner", escrow.owner_pubkey.clone());
            form.set_field_value("Amount", escrow.amount_label.clone());
            form.set_field_value("Terminal outcome", escrow.terminal_outcome.clone());
            form.set_field_value("Disposition", escrow.disposition.clone());
            form.set_field_value("Settlement eligibility", "ready to settle");
        }
        Some(form)
    }

    pub(super) fn begin_oracle_form(&mut self, mode: OracleFormMode) {
        use crate::participation::Action;
        if mode == OracleFormMode::RewardClaim {
            self.begin_oracle_reward_claim_form();
            return;
        }
        let current_action = match mode {
            OracleFormMode::SourceProposal => Some(Action::ProposeSource),
            OracleFormMode::SourceSupport => Some(Action::SupportSource),
            OracleFormMode::OpeningPrint => Some(Action::SubmitOpening),
            OracleFormMode::UpdateClaim => Some(Action::CommitUpdate),
            OracleFormMode::Challenge => Some(match self.oracle_phase() {
                OraclePhase::OpeningPrint => Action::ChallengeOpening,
                OraclePhase::GameMode => Action::ChallengeUpdate,
                _ => Action::ChallengeSource,
            }),
            _ => None,
        };
        if let Some(action) = current_action {
            self.oracle.form = None;
            self.oracle.search_editing = false;
            self.open_specific_action(action);
            return;
        }
        self.oracle.search_editing = false;
        self.oracle.form_field_flash = None;
        self.oracle.locked_flash = None;
        let Some(form) = self.oracle_form_draft_for_selected(mode) else {
            self.status = "Selected oracle source is unavailable.".to_string();
            return;
        };
        let node_label = self
            .selected_oracle_node()
            .map(|node| node.label.as_str())
            .unwrap_or("selected source")
            .to_string();
        let field_selected = form.field_selected;
        self.oracle.form = Some(form);
        self.oracle.flash_form_field(field_selected);
        self.reset_panel_scroll(LabFocus::OracleActions);
        self.status = format!(
            "{} form opened for {}. This saves a local semantic draft only; it cannot prepare, sign, or send.",
            mode.title(),
            node_label
        );
    }

    pub(super) fn begin_oracle_reward_claim_form(&mut self) {
        let Some(claim) = self.selected_oracle_reward_claim().cloned() else {
            self.status = "No oracle reward is available for this wallet.".to_string();
            self.clamp_oracle_selection();
            return;
        };
        if !self.open_oracle_reward_action(&claim) {
            return;
        }
        self.oracle.form = None;
        self.oracle.search_editing = false;
        self.status = format!(
            "Claim reward form opened: {} {}.",
            claim.label, claim.amount_label
        );
    }

    pub(super) fn begin_oracle_stake_settlement_form(&mut self) {
        let Some(escrow) = self.first_settlement_eligible_oracle_escrow().cloned() else {
            self.status = "No terminal oracle stake or bond is ready to settle.".to_string();
            self.clamp_oracle_selection();
            return;
        };
        self.begin_oracle_form(OracleFormMode::StakeSettlement);
        if self.oracle.form.is_none() {
            return;
        }
        self.status = format!(
            "Settle stake form opened: {} {} ({} derived on-chain).",
            escrow.kind.replace('_', " "),
            escrow.amount_label,
            escrow.terminal_outcome
        );
    }

    pub(super) fn cancel_oracle_form(&mut self) {
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
        self.reset_panel_scroll(LabFocus::OracleActions);
        self.status = "Oracle form cancelled.".to_string();
    }

    pub(super) fn move_oracle_form_field(&mut self, offset: isize) {
        let selected = if let Some(form) = self.oracle.form.as_mut() {
            let previous = form.field_selected;
            form.move_field(offset);
            if let Some(field) = form.fields.get(form.field_selected) {
                self.status = format!("Editing {}", field.label);
            }
            (form.field_selected != previous).then_some(form.field_selected)
        } else {
            None
        };
        if let Some(selected) = selected {
            self.oracle.flash_form_field(selected);
        }
    }

    pub(super) fn select_oracle_form_field_index(&mut self, index: usize) -> bool {
        let Some(form) = self.oracle.form.as_mut() else {
            return false;
        };
        if index >= form.fields.len() {
            return false;
        }
        form.field_selected = index;
        if let Some(field) = form.fields.get(index) {
            self.status = format!("Editing {}", field.label);
        }
        self.oracle.flash_form_field(index);
        true
    }

    pub(super) fn push_oracle_form_char(&mut self, character: char) {
        if character.is_control() {
            return;
        }
        let Some(form) = self.oracle.form.as_mut() else {
            return;
        };
        let Some(field) = form.selected_field_mut() else {
            return;
        };
        if !field.editable {
            self.status = format!("{} is autofilled.", field.label);
            return;
        }
        field.value.push(character);
        self.status = format!("Editing {}", field.label);
    }

    pub(super) fn backspace_oracle_form_input(&mut self) {
        let Some(form) = self.oracle.form.as_mut() else {
            return;
        };
        let Some(field) = form.selected_field_mut() else {
            return;
        };
        if !field.editable {
            self.status = format!("{} is autofilled.", field.label);
            return;
        }
        field.value.pop();
        self.status = format!("Editing {}", field.label);
    }

    pub(super) fn submit_oracle_form(&mut self) {
        let Some(form) = self.oracle.form.clone() else {
            return;
        };
        if let Err(message) = form.validate() {
            self.status = message;
            return;
        }
        let Some(tree) = self.oracle_tree().cloned() else {
            self.status = self.oracle.tree_load_status();
            return;
        };
        let phase = self.oracle_phase();
        let mut record = OracleSubmissionRecord::from_draft(&form, phase, &tree);
        let market_id = self.selected_id();
        let (month_label, expiry_id) = self
            .trading
            .detail
            .as_ref()
            .map(|detail| {
                (
                    detail.expiry_label.as_str(),
                    Some(detail.expiry_id.as_str()),
                )
            })
            .unwrap_or(("-", None));
        let stored_draft =
            match record.to_stored_draft(&form, &market_id, month_label, expiry_id, phase, &tree) {
                Ok(draft) => draft,
                Err(error) => {
                    self.oracle.submission_issue = Some(error.clone());
                    self.status = error;
                    return;
                }
            };
        if let Some(path) = self.oracle.submission_store_path.as_ref() {
            match oracle_submissions::append_at_path(path, stored_draft) {
                Ok(stored) => {
                    record.stored_id = Some(stored.id);
                    record.backend_status = stored.backend_status;
                    self.oracle.submission_issue = None;
                }
                Err(error) => {
                    self.oracle.submission_issue = Some(error.to_string());
                }
            }
        }
        let storage_status = record
            .stored_id
            .as_deref()
            .map(|id| format!("saved draft {id}"))
            .unwrap_or_else(|| record.backend_status.clone());
        self.status = format!(
            "Saved local draft {} for {} ({storage_status}). No instruction was prepared, signed, or sent.",
            record.title, record.node_label
        );
        self.oracle.submissions.push(record);
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
    }

    pub(super) fn activate_oracle_action(&mut self) {
        let selected = self.selected_oracle_action();
        let Some(tree) = self.oracle_tree().cloned() else {
            self.status = self.oracle.tree_load_status();
            return;
        };
        let Some((node_kind, node_label)) = self
            .selected_oracle_node()
            .map(|node| (node.kind, node.label.clone()))
        else {
            self.status = "Selected oracle source is unavailable.".to_string();
            return;
        };
        let Some(context) = self.selected_oracle_action_context() else {
            self.status = "Selected oracle source is unavailable.".to_string();
            return;
        };
        let base_availability = selected.base_availability(context.phase, node_kind);
        if base_availability != OracleActionAvailability::Active {
            if base_availability == OracleActionAvailability::ChooseSource {
                let node_index = self.selected_oracle_node_index();
                if let Some(pin_index) = tree.first_terminal_pin(node_index) {
                    self.select_oracle_node(pin_index);
                    self.set_focus(LabFocus::OracleActions);
                    if let Some(pin_context) = self.selected_oracle_action_context() {
                        if selected.availability(pin_context) == OracleActionAvailability::Active
                            && let Some(mode) = OracleFormMode::from_action(selected)
                        {
                            self.begin_oracle_form(mode);
                            return;
                        }
                        if selected.availability(pin_context) == OracleActionAvailability::Locked {
                            self.oracle.flash_action_locked(selected);
                            self.status = format!(
                                "{} is locked here in {}. {}",
                                selected.label(),
                                pin_context.phase.label(),
                                selected.contextual_detail(pin_context)
                            );
                            return;
                        }
                    }
                }
                self.status = format!(
                    "Choose a source before {}. {}",
                    selected.label().to_ascii_lowercase(),
                    selected.detail(context.phase)
                );
                return;
            }
            self.oracle.flash_action_locked(selected);
            self.status = format!(
                "{} is locked here in {}. {}",
                selected.label(),
                context.phase.label(),
                selected.contextual_detail(context)
            );
            return;
        }
        if selected.availability(context) != OracleActionAvailability::Active {
            self.oracle.flash_action_locked(selected);
            self.status = format!(
                "{} is locked here in {}. {}",
                selected.label(),
                context.phase.label(),
                selected.contextual_detail(context)
            );
            return;
        }
        if selected == OracleAction::ClaimReward {
            self.begin_oracle_reward_claim_form();
            return;
        }
        if selected == OracleAction::SettleStake {
            self.begin_oracle_stake_settlement_form();
            return;
        }
        if let Some(mode) = OracleFormMode::from_action(selected) {
            self.begin_oracle_form(mode);
            return;
        }
        self.status = match selected {
            OracleAction::ProposeSource => {
                format!(
                    "Source proposal for {}: source category, canonical locator, source definition, row target, stake/support.",
                    node_label
                )
            }
            OracleAction::EditDefinition => {
                format!(
                    "Edit source definition for {} before snapshot: product, field, currency, region, condition, quantity tier.",
                    node_label
                )
            }
            OracleAction::BackSource => {
                format!(
                    "Back source target {} with support; row weights are still applied once after row aggregation.",
                    node_label
                )
            }
            OracleAction::OpeningPrint => format!(
                "Opening claim for {}: raw value, timestamp, source definition, archive evidence, stake.",
                node_label
            ),
            OracleAction::SubmitUpdate => format!(
                "Commit update for {}: claimant-scoped claim id, hidden commit hash, and stake. Reveal later with the same claimant after the minimum delay.",
                node_label
            ),
            OracleAction::Challenge => {
                "Challenge: invalid, duplicate, wrong bucket, wrong definition, disallowed source, or bad update evidence.".to_string()
            }
            OracleAction::ClaimReward => {
                "Claim reward: collect an earned treasury reward into your oracle ledger."
                    .to_string()
            }
            OracleAction::SettleStake => {
                "Settle stake: finalize one terminal stake or bond using the program-derived refund or slash."
                    .to_string()
            }
            OracleAction::DepositAmba => {
                "Deposit AMBA: move tokens into oracle voting custody.".to_string()
            }
            OracleAction::WithdrawAmba => {
                "Withdraw AMBA: move available tokens back to your wallet.".to_string()
            }
            OracleAction::ReviewQueue => "Reviewing the current oracle work queue and final output path.".to_string(),
        };
    }

    pub(super) fn activate_home_action(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        match self.selected_home_action() {
            HomeAction::Trade => {
                self.open_chain();
            }
            HomeAction::Chart => self.open_chart(backend_url, fetch_tx),
            HomeAction::Oracle => self.open_oracle_intro(),
            HomeAction::Ledger => self.open_ledger(backend_url, fetch_tx),
            HomeAction::Staking => self.open_staking(fetch_tx),
            HomeAction::Help => self.open_home_help(HomeHelpTopic::Overview, fetch_tx),
            HomeAction::ConnectAgents => self.open_home_help(HomeHelpTopic::Agents, fetch_tx),
        }
    }

    pub(super) fn activate_oracle_intro_action(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        match self.oracle.selected_intro_action() {
            OracleIntroAction::Earn => self.open_oracle_earn(backend_url, fetch_tx),
            OracleIntroAction::Advanced => self.open_oracle(backend_url, fetch_tx),
            OracleIntroAction::ReadMore => self.open_oracle_help(),
            OracleIntroAction::BackHome => self.open_home(),
        }
    }

    pub(super) fn activate_oracle_earn(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.oracle.loading_tree || self.oracle.loading_live || self.oracle.loading_rewards {
            self.status = "Petri is still checking current Oracle work.".to_string();
            return;
        }

        if self.has_claimable_oracle_reward() {
            if self.oracle_tree().is_none() {
                self.request_oracle_tree(backend_url, fetch_tx, true);
                self.status = "Loading the details needed to review your reward.".to_string();
                return;
            }
            self.open_oracle(backend_url, fetch_tx);
            self.select_oracle_action(OracleAction::ClaimReward);
            self.set_focus(LabFocus::OracleActions);
            self.begin_oracle_reward_claim_form();
            return;
        }

        self.request_oracle_tree(backend_url, fetch_tx, true);
        self.request_oracle_live(backend_url, fetch_tx, true);
        self.request_oracle_rewards(backend_url, fetch_tx, true);
        self.status =
            "Checking for a fully specified task with funded reward and risk terms.".to_string();
    }
}
