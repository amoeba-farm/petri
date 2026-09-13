//! Ledger and staking screen entry, forms, confirmation, and action state.

use super::*;

impl LabApp {
    pub(super) fn open_detail(&mut self) {
        self.trading.detail_view = DetailView::Overview;
        self.set_screen(LabScreen::Detail);
        self.status = "market details".to_string();
    }

    pub(super) fn select_detail_view(
        &mut self,
        view: DetailView,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.trading.detail_view = view;
        self.reset_panel_scroll(LabFocus::Detail);
        if view == DetailView::Settlement {
            self.request_settlement(backend_url, fetch_tx, false);
        } else {
            self.status = "Market overview.".to_string();
        }
    }

    pub(super) fn move_detail_view(
        &mut self,
        offset: isize,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let current = DetailView::ALL
            .iter()
            .position(|view| *view == self.trading.detail_view)
            .unwrap_or(0) as isize;
        let next = (current + offset).rem_euclid(DetailView::ALL.len() as isize) as usize;
        self.select_detail_view(DetailView::ALL[next], backend_url, fetch_tx);
    }

    pub(super) fn open_activity(&mut self) {
        self.set_screen(LabScreen::Activity);
        self.status = "selected contract trades".to_string();
    }

    pub(super) fn open_ledger(&mut self, backend_url: &str, fetch_tx: &Sender<LabFetchResult>) {
        self.set_screen(LabScreen::Ledger);
        self.ledger_pane = LedgerPane::Tabs;
        self.request_ledger(backend_url, fetch_tx, false);
    }

    pub(super) fn open_staking(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        self.staking_form = None;
        self.staking_confirmation = None;
        self.set_screen(LabScreen::Staking);
        self.request_staking_status(fetch_tx, false);
    }

    pub(super) fn selected_staking_action(&self) -> StakingAction {
        StakingAction::ALL
            .get(self.staking_selected)
            .copied()
            .unwrap_or(StakingAction::Stake)
    }

    pub(super) fn move_staking_action(&mut self, offset: isize) {
        if self.staking_action_is_running() || offset == 0 {
            return;
        }
        let len = StakingAction::ALL.len() as isize;
        self.staking_selected = (self.staking_selected as isize + offset).rem_euclid(len) as usize;
        let action = self.selected_staking_action();
        self.status = match self.staking_action_availability(action) {
            Ok(()) => format!("{}: {}", action.label(), action.detail()),
            Err(issue) => format!("{}: {issue}", action.label()),
        };
    }

    pub(super) fn staking_action_availability(&self, action: StakingAction) -> Result<(), String> {
        if action == StakingAction::Refresh {
            return if !self.wallet.is_attached() {
                Err("Attach a wallet before refreshing staking balances.".to_string())
            } else if self.loading_staking {
                Err("Staking balances are already refreshing.".to_string())
            } else {
                Ok(())
            };
        }
        if !self.wallet.is_attached() {
            return Err("Attach a wallet before preparing a staking action.".into());
        }
        if action == StakingAction::CancelQueue {
            return Err(
                "The current SDK does not expose a public queued-stake cancellation action.".into(),
            );
        }
        Ok(())
    }

    pub(super) fn activate_staking_action(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        if self.staking_action_is_running() {
            self.status = "A staking transaction is still running.".to_string();
            return;
        }
        let action = self.selected_staking_action();
        if action == StakingAction::Refresh {
            if let Err(error) = self.staking_action_availability(action) {
                self.status = error;
                return;
            }
            self.request_staking_status(fetch_tx, true);
            return;
        }
        if let Err(error) = self.staking_action_availability(action) {
            self.status = error;
            return;
        }
        let current = match action {
            StakingAction::Stake => Some(crate::participation::Action::Stake),
            StakingAction::Activate => Some(crate::participation::Action::ActivateStake),
            StakingAction::Unstake => Some(crate::participation::Action::Unstake),
            StakingAction::Claim => Some(crate::participation::Action::CompleteUnstake),
            _ => None,
        };
        if let Some(current) = current {
            self.open_specific_action(current);
            return;
        }
        self.staking_action_result = None;
        if matches!(action, StakingAction::Claim | StakingAction::CancelQueue) {
            let amount_keys: &[&str] = if action == StakingAction::Claim {
                &[
                    "unbondingAmba",
                    "unbonding_amba",
                    "pendingUnstakeAmba",
                    "pending_unstake_amba",
                    "ownerPendingUnstakeAmba",
                    "owner_pending_unstake_amba",
                ]
            } else {
                &["queuedAmba", "queued_amba"]
            };
            self.staking_confirmation = Some(StakingConfirmation {
                action,
                amount: staking_status_value(self.staking_status.as_ref(), amount_keys),
                minimum_received: None,
                choice: StakingConfirmationChoice::Cancel,
            });
            self.status = if action == StakingAction::Claim {
                "Review the ready AMBA claim. Confirm moves it into your available balance."
                    .to_string()
            } else {
                "Review cancellation. Confirm returns queued AMBA to the available balance."
                    .to_string()
            };
            return;
        }
        self.staking_form = Some(StakingForm::new(action));
        self.status = match action {
            StakingAction::Stake => {
                "Queue form opened. Enter the exact AMBA amount to reserve for seven days."
                    .to_string()
            }
            StakingAction::Activate => {
                "Activation form opened. Minimum sAMBA is optional; blank uses the current quote."
                    .to_string()
            }
            StakingAction::Unstake => {
                "Unstake form opened. Enter an amount; minimum received is optional.".to_string()
            }
            StakingAction::CancelQueue | StakingAction::Claim | StakingAction::Refresh => {
                unreachable!("non-form staking action")
            }
        };
    }

    pub(super) fn cancel_staking_form(&mut self) {
        self.staking_form = None;
        self.status = "Staking form cancelled. No balance was changed.".to_string();
    }

    pub(super) fn cancel_staking_confirmation(&mut self) {
        if self.staking_action_is_running() {
            self.status =
                "The staking transaction is already running. Wait for its result.".to_string();
            return;
        }
        self.staking_confirmation = None;
        self.status = "Staking review cancelled. Nothing was signed or sent.".to_string();
    }

    pub(super) fn move_staking_form_field(&mut self) {
        let Some(form) = self.staking_form.as_mut() else {
            return;
        };
        form.move_field();
        self.status = format!("Editing {}", form.field.label(form.action));
    }

    pub(super) fn push_staking_form_char(&mut self, character: char) {
        if !staking_form_input_character(character) {
            return;
        }
        let Some(form) = self.staking_form.as_mut() else {
            return;
        };
        let input = form.selected_input_mut();
        if character == '.' && input.contains('.') {
            return;
        }
        input.push(character);
    }

    pub(super) fn backspace_staking_form_input(&mut self) {
        if let Some(form) = self.staking_form.as_mut() {
            form.selected_input_mut().pop();
        }
    }

    pub(super) fn review_staking_form(&mut self) {
        if let Err(error) = crate::current_release::require_current_write_release()
            .and_then(|()| crate::staking::require_typed_staking_submission())
        {
            self.staking_form = None;
            self.staking_confirmation = None;
            self.status = error.to_string();
            return;
        }
        let Some(form) = self.staking_form.as_ref() else {
            return;
        };
        if let Err(error) = form.validate() {
            self.status = error;
            return;
        }
        self.staking_confirmation = Some(StakingConfirmation {
            action: form.action,
            amount: (!form.amount_input.trim().is_empty())
                .then(|| form.amount_input.trim().to_string()),
            minimum_received: (!form.minimum_received_input.trim().is_empty())
                .then(|| form.minimum_received_input.trim().to_string()),
            choice: StakingConfirmationChoice::Cancel,
        });
        self.status = "Staking wallet changes are unavailable in the current release.".to_string();
    }

    pub(super) fn move_staking_confirmation_choice(&mut self, direction: isize) {
        if self.staking_action_is_running() {
            return;
        }
        let Some(confirmation) = self.staking_confirmation.as_mut() else {
            return;
        };
        confirmation.choice = if direction < 0 {
            StakingConfirmationChoice::Cancel
        } else {
            StakingConfirmationChoice::Confirm
        };
    }

    pub(super) fn activate_staking_confirmation(&mut self, fetch_tx: &Sender<LabFetchResult>) {
        let Some(confirmation) = self.staking_confirmation.clone() else {
            return;
        };
        if confirmation.choice == StakingConfirmationChoice::Cancel {
            self.cancel_staking_confirmation();
            return;
        }
        if let Err(error) = crate::current_release::require_current_write_release()
            .and_then(|()| crate::staking::require_typed_staking_submission())
        {
            self.staking_confirmation = None;
            self.status = error.to_string();
            return;
        }
        if self.staking_action_is_running() {
            self.status = "A staking transaction is already running.".to_string();
            return;
        }
        if let Err(error) = self.staking_action_availability(confirmation.action) {
            self.status = error;
            return;
        }
        self.staking_action_request = self.staking_action_request.wrapping_add(1);
        self.staking_action_inflight = Some(self.staking_action_request);
        self.staking_form = None;
        self.status = format!("{}: submitting...", confirmation.action.label());
        spawn_staking_action(
            self.onchain_config.clone(),
            fetch_tx.clone(),
            self.staking_action_request,
            confirmation,
        );
    }

    pub(super) fn staking_action_is_running(&self) -> bool {
        self.staking_action_inflight.is_some()
    }
}
