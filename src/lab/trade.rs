//! Protected options trade-ticket review, confirmation, and submission state.

use super::*;

impl LabApp {
    pub(super) fn open_chain(&mut self) {
        self.set_screen(LabScreen::Chain);
        self.status = "fixed-risk contracts".to_string();
    }

    pub(super) fn select_trade_action(&mut self, action: TradeAction) {
        if self.trading.submit_is_running() {
            self.status =
                "A transaction is still submitting. Wait for its result before opening another ticket."
                    .to_string();
            return;
        }
        let already_on_chain = self.screen == LabScreen::Chain;
        self.trading.action = action;
        if !already_on_chain {
            self.open_chain();
        }
        self.begin_trade_ticket(action);
        if self.trading.ticket.is_none() && self.selected_quote().is_none() {
            self.status = format!(
                "Select a contract, then press {} to {}.",
                match action {
                    TradeAction::Buy => "B",
                    TradeAction::Sell => "S",
                },
                action.label().to_ascii_lowercase()
            );
        }
    }

    pub(super) fn begin_trade_ticket(&mut self, action: TradeAction) {
        if self.trading.submit_is_running() {
            self.status =
                "An order is still submitting. Wait for its result before changing the ticket."
                    .to_string();
            return;
        }
        if let Err(error) = self.require_selected_contract_tradeable() {
            self.trading.ticket = None;
            self.trading.ticket_field_flash = None;
            self.status = error;
            return;
        }
        let premium = self
            .selected_quote()
            .and_then(|quote| trade_route_price(quote, action));
        if self.selected_quote().is_none() {
            self.trading.ticket = None;
            self.trading.ticket_field_flash = None;
            return;
        }
        self.trading.action = action;
        self.trading.ticket = Some(TradeTicket::new(action, premium));
        self.trading.ticket_field_flash = None;
        self.status = match action {
            TradeAction::Buy => {
                "Buy ticket opened. Price and contracts determine max loss.".to_string()
            }
            TradeAction::Sell => {
                "Sell owned options. Enter contracts and minimum sale price; this does not open a short.".to_string()
            }
        };
    }

    pub(super) fn require_selected_contract_tradeable(&self) -> Result<(), String> {
        let detail = self
            .trading
            .detail
            .as_ref()
            .ok_or_else(|| "Load a market before opening an order ticket.".to_string())?;
        let expiry = self
            .selected_chart_expiry()
            .ok_or_else(|| "Select a contract month before opening an order ticket.".to_string())?;
        let phase = self.oracle_phase();
        // Opening an editable ticket grants no authority. An independently loading
        // Oracle panel must not block exact-series SDK/Lean preparation.
        if !self.oracle.loading_live
            && phase != OraclePhase::Unavailable
            && phase != OraclePhase::GameMode
        {
            return Err(format!(
                "Order unavailable: {} {} is in {}, not Game Mode. Contracts cannot be listed or traded before Game Mode.",
                detail.symbol,
                expiry.label,
                phase.label()
            ));
        }
        let quote = self
            .selected_quote()
            .ok_or_else(|| "Select a contract before opening an order ticket.".to_string())?;
        if !quote.prepare_eligible {
            return Err(
                "Order unavailable: the selected contract is waiting for current market and in-program pool eligibility."
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(super) fn cancel_trade_ticket(&mut self) {
        if self.trading.submit_is_running() {
            self.status =
                "Order submission is running. The ticket will unlock when it finishes.".to_string();
            return;
        }
        self.trading.ticket = None;
        self.trading.ticket_field_flash = None;
        self.status = "Order ticket closed.".to_string();
    }

    pub(super) fn select_trade_ticket_field(&mut self, field: TradeTicketField) {
        if self.trading.submit_is_running() {
            return;
        }
        let Some(ticket) = self.trading.ticket.as_mut() else {
            return;
        };
        let price_label = ticket.action.price_label();
        ticket.field = field;
        ticket.clear_review();
        self.trading.flash_ticket_field(field);
        self.status = match field {
            TradeTicketField::Premium => {
                format!(
                    "{} field active. Type a price, then Tab to contracts.",
                    price_label
                )
            }
            TradeTicketField::Quantity => {
                "Contracts field active. Type quantity, then Enter to review.".to_string()
            }
        };
    }

    pub(super) fn review_or_submit_trade_ticket(
        &mut self,
        cli: &Cli,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.trading.submit_is_running() {
            self.status =
                "An order is already submitting. Wait for its result before submitting again."
                    .to_string();
            return;
        }
        let submit = match build_trade_ticket_submit(cli, self) {
            Ok(submit) => submit,
            Err(error) => {
                let field = trade_ticket_validation_field(&error);
                if let Some(ticket) = self.trading.ticket.as_mut() {
                    ticket.confirmation = None;
                    ticket.result = Some(TradeTicketResult {
                        ok: false,
                        message: error.clone(),
                    });
                    if let Some(field) = field {
                        ticket.field = field;
                    }
                }
                if let Some(field) = field {
                    self.trading.flash_ticket_field(field);
                }
                self.status = error;
                return;
            }
        };
        let Some(ticket) = self.trading.ticket.as_mut() else {
            return;
        };
        ticket.last_command = Some(submit.command.clone());
        ticket.confirmation = None;
        ticket.result = None;
        ticket.submitting = true;
        self.trading.submit_request = self.trading.submit_request.wrapping_add(1);
        let request_id = self.trading.submit_request;
        ticket.submit_request_id = Some(request_id);
        self.trading.submit_inflight = Some(request_id);
        self.status =
            "Preparing exact trade and checking current permission. Nothing is signed yet.".into();
        spawn_trade_prepare(
            submit,
            fetch_tx.clone(),
            request_id,
            self.wallet.pubkey.clone().unwrap_or_default(),
            self.selected_chart_expiry()
                .map(|e| e.id.clone())
                .unwrap_or_default(),
        );
    }

    pub(super) fn cancel_trade_confirmation(&mut self) {
        let Some(ticket) = self.trading.ticket.as_mut() else {
            return;
        };
        ticket.confirmation = None;
        self.status = "Order confirmation canceled. No order was submitted.".to_string();
    }

    pub(super) fn activate_trade_confirmation(
        &mut self,
        cli: &Cli,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let choice = self
            .trading
            .ticket
            .as_ref()
            .and_then(|ticket| ticket.confirmation.as_ref())
            .map(|confirmation| confirmation.choice);
        match choice {
            Some(TradeConfirmationChoice::Cancel) => self.cancel_trade_confirmation(),
            Some(TradeConfirmationChoice::Confirm) => {
                self.submit_confirmed_trade_ticket(cli, fetch_tx)
            }
            None => {}
        }
    }

    pub(super) fn submit_confirmed_trade_ticket(
        &mut self,
        _cli: &Cli,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let launch = match self.begin_confirmed_trade_submit() {
            Ok(launch) => launch,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let TradeSubmitLaunch {
            submit,
            request_id,
            action,
        } = launch;
        let TradeTicketSubmit {
            args,
            envs,
            command,
            summary,
        } = submit;
        spawn_trade_submit(
            args,
            envs,
            fetch_tx.clone(),
            request_id,
            action,
            summary,
            command,
        );
    }

    pub(super) fn begin_confirmed_trade_submit(&mut self) -> Result<TradeSubmitLaunch, String> {
        crate::current_release::require_current_write_release()
            .map_err(|error| error.to_string())?;
        if self.trading.submit_is_running() {
            return Err(
                "An order is already submitting. Wait for its result before submitting again."
                    .to_string(),
            );
        }
        self.require_selected_contract_tradeable()?;
        let Some(ticket) = self.trading.ticket.as_mut() else {
            return Err("Open an order ticket before submitting.".to_string());
        };
        let confirmation = ticket
            .confirmation
            .take()
            .ok_or_else(|| "Open the order confirmation before submitting.".to_string())?;
        let submit = confirmation.prepared;
        let action = submit.summary.action;
        ticket.submitting = true;
        ticket.last_command = Some(submit.command.clone());
        self.trading.submit_request = self.trading.submit_request.wrapping_add(1);
        let request_id = self.trading.submit_request;
        self.trading.submit_inflight = Some(request_id);
        ticket.submit_request_id = Some(request_id);
        self.status = format!(
            "Submitting {} order...",
            action.label().to_ascii_lowercase()
        );
        Ok(TradeSubmitLaunch {
            submit,
            request_id,
            action,
        })
    }
}
