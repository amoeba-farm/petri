//! Feature-owned trading state and local transitions. Network and signing effects stay in the coordinator.
use super::settlement_data;
use crate::{
    chart,
    cli::{ChartArgs, ChartRangeValue},
    market_surface::{DishDetail, DishSummary, OptionKind, format_decimal},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use std::time::{Duration, Instant};

const TRADE_RESULT_MODAL_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum DetailView {
    #[default]
    Overview,
    Settlement,
}

impl DetailView {
    pub(super) const ALL: [Self; 2] = [Self::Overview, Self::Settlement];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Settlement => "Settlement",
        }
    }
}

#[derive(Clone)]
pub(super) struct TradeTicketSubmit {
    pub(super) args: Vec<String>,
    pub(super) envs: Vec<(String, String)>,
    pub(super) command: String,
    pub(super) summary: TradeConfirmationSummary,
}

impl std::fmt::Debug for TradeTicketSubmit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TradeTicketSubmit")
            .field("summary", &self.summary)
            .field("prepared", &true)
            .finish()
    }
}

#[derive(Debug)]
pub(super) struct TradeSubmitLaunch {
    pub(super) submit: TradeTicketSubmit,
    pub(super) request_id: u64,
    pub(super) action: TradeAction,
}

fn first_quote_index_by_kind(detail: &DishDetail, kind: OptionKind) -> Option<usize> {
    detail
        .option_quotes
        .iter()
        .position(|quote| quote.kind == kind)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TradeAction {
    Buy,
    Sell,
}

impl TradeAction {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    pub(super) fn side_label(self) -> &'static str {
        match self {
            Self::Buy => "Bid",
            Self::Sell => "Ask",
        }
    }

    pub(super) fn price_label(self) -> &'static str {
        match self {
            Self::Buy => "Price",
            Self::Sell => "Premium",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChainFocus {
    Markets,
    Calls,
    Puts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TradeTicketField {
    Premium,
    Quantity,
}

pub(super) const TRADE_TICKET_FIELD_FLASH_TICKS: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TradeTicketFieldFlash {
    pub(super) field: TradeTicketField,
    pub(super) ticks_remaining: u8,
    pub(super) visible: bool,
}

#[derive(Clone, Debug)]
pub(super) struct TradeTicketResult {
    pub(super) ok: bool,
    pub(super) message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TradeConfirmationChoice {
    Cancel,
    Confirm,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TradeConfirmationSummary {
    pub(super) action: TradeAction,
    pub(super) symbol: String,
    pub(super) expiry: String,
    pub(super) kind: OptionKind,
    pub(super) lower_strike: String,
    pub(super) upper_strike: String,
    pub(super) price: f64,
    pub(super) qty: u64,
    pub(super) entry_total: f64,
    pub(super) total_max_loss: f64,
    pub(super) total_max_gain: f64,
    pub(super) total_max_payout: f64,
    pub(super) probability_itm: Option<f64>,
    pub(super) probability_cap_hit: Option<f64>,
    pub(super) account: String,
}

#[derive(Clone, Debug)]
pub(super) struct TradeConfirmation {
    pub(super) prepared: TradeTicketSubmit,
    pub(super) choice: TradeConfirmationChoice,
}

#[derive(Clone, Debug)]
pub(super) struct TradeResultModal {
    pub(super) ok: bool,
    pub(super) waiting: bool,
    pub(super) action: TradeAction,
    pub(super) summary: Option<TradeConfirmationSummary>,
    pub(super) details_in_ticket: bool,
    pub(super) failure_reason: Option<String>,
    pub(super) expires_at: Instant,
}

impl TradeResultModal {
    pub(super) fn new(
        ok: bool,
        action: TradeAction,
        summary: Option<TradeConfirmationSummary>,
        details_in_ticket: bool,
        now: Instant,
    ) -> Self {
        Self {
            ok,
            waiting: false,
            action,
            summary,
            details_in_ticket,
            failure_reason: None,
            expires_at: now + Duration::from_secs(TRADE_RESULT_MODAL_SECONDS),
        }
    }

    pub(super) fn remaining_seconds_at(&self, now: Instant) -> u64 {
        let remaining_millis = self.expires_at.saturating_duration_since(now).as_millis();
        if remaining_millis == 0 {
            0
        } else {
            ((remaining_millis + 999) / 1_000) as u64
        }
    }

    pub(super) fn is_expired_at(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[derive(Clone, Debug)]
pub(super) struct TradeTicket {
    pub(super) action: TradeAction,
    pub(super) premium_input: String,
    pub(super) quantity_input: String,
    pub(super) field: TradeTicketField,
    pub(super) confirmation: Option<TradeConfirmation>,
    pub(super) submitting: bool,
    pub(super) submit_request_id: Option<u64>,
    pub(super) last_command: Option<String>,
    pub(super) result: Option<TradeTicketResult>,
}

impl TradeTicket {
    pub(super) fn new(action: TradeAction, premium: Option<f64>) -> Self {
        Self {
            action,
            premium_input: premium
                .map(|value| format_decimal(value, 3))
                .unwrap_or_default(),
            quantity_input: String::new(),
            field: TradeTicketField::Premium,
            confirmation: None,
            submitting: false,
            submit_request_id: None,
            last_command: None,
            result: None,
        }
    }

    pub(super) fn clear_review(&mut self) {
        self.confirmation = None;
        self.result = None;
    }
}

pub(super) struct TradeState {
    pub(super) dishes: Vec<DishSummary>,
    pub(super) selected: usize,
    pub(super) pending_initial_dish: Option<String>,
    pub(super) selected_option: usize,
    pub(super) active_option_kind: OptionKind,
    pub(super) chain_focus: ChainFocus,
    pub(super) market_series_open: bool,
    pub(super) chart_expiry: usize,
    pub(super) initial_chart_expiry: Option<String>,
    pub(super) chart_range: ChartRangeValue,
    pub(super) chart_launch_options: Option<ChartArgs>,
    pub(super) chart_last_refresh_at: Option<Instant>,
    pub(super) pending_initial_chart: bool,
    pub(super) action: TradeAction,
    pub(super) ticket: Option<TradeTicket>,
    pub(super) review_scroll: u16,
    pub(super) ticket_field_flash: Option<TradeTicketFieldFlash>,
    pub(super) result_modal: Option<TradeResultModal>,
    pub(super) suppress_trade_result_escape_repeat: bool,
    pub(super) detail: Option<DishDetail>,
    pub(super) detail_view: DetailView,
    pub(super) settlement_bundle: Option<settlement_data::SettlementBundle>,
    pub(super) settlement_issue: Option<String>,
    pub(super) chart: Option<chart::EmbeddedChart>,
    pub(super) list_request: u64,
    pub(super) detail_request: u64,
    pub(super) settlement_request: u64,
    pub(super) chart_request: u64,
    pub(super) submit_request: u64,
    pub(super) submit_inflight: Option<u64>,
    pub(super) loading_list: bool,
    pub(super) loading_detail: bool,
    pub(super) loading_settlement: bool,
    pub(super) loading_chart: bool,
}

impl TradeState {
    pub(super) fn tick_field_flash(&mut self) {
        if let Some(flash) = self.ticket_field_flash.as_mut() {
            if flash.ticks_remaining == 0 {
                self.ticket_field_flash = None;
            } else {
                flash.visible = !flash.visible;
                flash.ticks_remaining = flash.ticks_remaining.saturating_sub(1);
                if flash.ticks_remaining == 0 {
                    self.ticket_field_flash = None;
                }
            }
        }
    }
    pub(super) fn new(
        dishes: Vec<DishSummary>,
        selected: usize,
        pending_initial_dish: Option<String>,
    ) -> Self {
        Self {
            dishes,
            selected,
            pending_initial_dish,
            selected_option: 0,
            active_option_kind: OptionKind::Call,
            chain_focus: ChainFocus::Markets,
            market_series_open: false,
            chart_expiry: 0,
            initial_chart_expiry: None,
            chart_range: ChartRangeValue::TwentyFourHours,
            chart_launch_options: None,
            chart_last_refresh_at: None,
            pending_initial_chart: false,
            action: TradeAction::Buy,
            ticket: None,
            review_scroll: 0,
            ticket_field_flash: None,
            result_modal: None,
            suppress_trade_result_escape_repeat: false,
            detail: None,
            detail_view: DetailView::Overview,
            settlement_bundle: None,
            settlement_issue: None,
            chart: None,
            list_request: 0,
            detail_request: 0,
            settlement_request: 0,
            chart_request: 0,
            submit_request: 0,
            submit_inflight: None,
            loading_list: false,
            loading_detail: false,
            loading_settlement: false,
            loading_chart: false,
        }
    }

    pub(super) fn move_ticket_field(&mut self, direction: isize) {
        if self.submit_is_running() {
            return;
        }
        let Some(ticket) = self.ticket.as_mut() else {
            return;
        };
        if direction == 0 {
            return;
        }
        ticket.field = match ticket.field {
            TradeTicketField::Premium => TradeTicketField::Quantity,
            TradeTicketField::Quantity => TradeTicketField::Premium,
        };
        ticket.clear_review();
        self.ticket_field_flash = None;
    }

    pub(super) fn flash_ticket_field(&mut self, field: TradeTicketField) {
        self.ticket_field_flash = Some(TradeTicketFieldFlash {
            field,
            ticks_remaining: TRADE_TICKET_FIELD_FLASH_TICKS,
            visible: true,
        });
    }

    pub(super) fn ticket_field_flash_visible(&self, field: TradeTicketField) -> bool {
        self.ticket_field_flash
            .filter(|flash| flash.field == field)
            .map(|flash| flash.visible)
            .unwrap_or(false)
    }

    pub(super) fn push_ticket_char(&mut self, character: char) {
        if self.submit_is_running() {
            return;
        }
        let Some(ticket) = self.ticket.as_mut() else {
            return;
        };
        if ticket.submitting {
            return;
        }
        match ticket.field {
            TradeTicketField::Premium => {
                if character.is_ascii_digit()
                    || (character == '.' && !ticket.premium_input.contains('.'))
                {
                    ticket.premium_input.push(character);
                    ticket.clear_review();
                    self.ticket_field_flash = None;
                }
            }
            TradeTicketField::Quantity => {
                if character.is_ascii_digit() {
                    if ticket.quantity_input == "0" {
                        ticket.quantity_input.clear();
                    }
                    ticket.quantity_input.push(character);
                    ticket.clear_review();
                    self.ticket_field_flash = None;
                }
            }
        }
    }

    pub(super) fn backspace_ticket_input(&mut self) {
        if self.submit_is_running() {
            return;
        }
        let Some(ticket) = self.ticket.as_mut() else {
            return;
        };
        if ticket.submitting {
            return;
        }
        match ticket.field {
            TradeTicketField::Premium => {
                ticket.premium_input.pop();
            }
            TradeTicketField::Quantity => {
                ticket.quantity_input.pop();
            }
        }
        ticket.clear_review();
        self.ticket_field_flash = None;
    }

    pub(super) fn move_confirmation_choice(&mut self, direction: isize) {
        if direction == 0 {
            return;
        }
        let Some(confirmation) = self
            .ticket
            .as_mut()
            .and_then(|ticket| ticket.confirmation.as_mut())
        else {
            return;
        };
        confirmation.choice = if direction < 0 {
            TradeConfirmationChoice::Cancel
        } else {
            TradeConfirmationChoice::Confirm
        };
    }

    pub(super) fn submit_is_running(&self) -> bool {
        self.submit_inflight.is_some()
    }

    pub(super) fn confirmation_is_open(&self) -> bool {
        self.ticket
            .as_ref()
            .is_some_and(|ticket| ticket.confirmation.is_some())
    }

    pub(super) fn result_modal_is_open(&self) -> bool {
        self.result_modal.is_some()
    }

    pub(super) fn dismiss_result_modal(&mut self) {
        self.result_modal = None;
    }

    pub(super) fn expire_result_modal_at(&mut self, now: Instant) {
        if self
            .result_modal
            .as_ref()
            .is_some_and(|modal| modal.is_expired_at(now))
        {
            self.result_modal = None;
        }
    }

    pub(super) fn handle_result_modal_key(&mut self, key: &KeyEvent) -> bool {
        if self.result_modal_is_open() {
            if key.code == KeyCode::Esc && key.kind != KeyEventKind::Release {
                self.dismiss_result_modal();
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

    pub(super) fn sync_selected_expiry(&mut self) {
        let count = self
            .detail
            .as_ref()
            .map(|detail| detail.expiries.len())
            .unwrap_or(0);
        if count == 0 {
            self.chart_expiry = 0;
            return;
        }
        if self.chart_expiry >= count {
            self.chart_expiry = count - 1;
        }
        self.apply_selected_expiry_to_detail();
    }

    pub(super) fn apply_selected_expiry_to_detail(&mut self) {
        let index = self.chart_expiry;
        let Some(detail) = &mut self.detail else {
            return;
        };
        let Some(expiry) = detail.expiries.get(index).cloned() else {
            return;
        };
        detail.expiry_id = expiry.id;
        detail.expiry_label = expiry.label;
        detail.settlement = expiry.settlement;
        detail.days = expiry.days;
        detail.current_print = expiry.current_print;
        detail.base = expiry.base;
        detail.cap_width = expiry.cap_width;
        detail.listed_notional = expiry.listed_notional;
        detail.rows = expiry.rows;
        detail.option_quotes = expiry.option_quotes;
    }

    pub(super) fn clamp_selected_option(&mut self) {
        let count = self
            .detail
            .as_ref()
            .map(|detail| detail.option_quotes.len())
            .unwrap_or(0);
        if count == 0 {
            self.selected_option = 0;
        } else if self.selected_option >= count {
            self.selected_option = count - 1;
        }
        if let Some(detail) = &self.detail {
            let selected_matches_active = detail
                .option_quotes
                .get(self.selected_option)
                .map(|quote| quote.kind == self.active_option_kind)
                .unwrap_or(false);
            if let Some(index) = first_quote_index_by_kind(detail, self.active_option_kind)
                .filter(|_| !selected_matches_active)
                .or_else(|| (!detail.option_quotes.is_empty()).then_some(self.selected_option))
            {
                self.selected_option = index;
                self.active_option_kind = detail.option_quotes[index].kind;
            }
        }
        let expiry_count = self
            .detail
            .as_ref()
            .map(|detail| detail.expiries.len())
            .unwrap_or(0);
        if expiry_count == 0 {
            self.chart_expiry = 0;
        } else if self.chart_expiry >= expiry_count {
            self.chart_expiry = expiry_count - 1;
        }
    }
}
