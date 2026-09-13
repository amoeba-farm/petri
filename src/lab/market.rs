//! Market, month, option, chart-range, and selected-contract state transitions.

use super::*;

impl LabApp {
    pub(super) fn set_chart_range(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        range: ChartRangeValue,
    ) {
        self.trading.chart_range = range;
        self.request_chart(backend_url, fetch_tx, false);
    }

    pub(super) fn select_prev_option(&mut self) -> bool {
        if self.trading.submit_is_running() {
            return false;
        }
        let Some(detail) = &self.trading.detail else {
            return false;
        };
        let indices = quote_indices_by_kind(detail, self.trading.active_option_kind);
        if indices.is_empty() {
            return false;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.trading.selected_option)
            .unwrap_or(0);
        if current == 0 {
            return false;
        }
        self.trading.selected_option = indices[current - 1];
        self.panel_scrolls.insert(
            option_kind_focus(self.trading.active_option_kind),
            current.saturating_sub(1),
        );
        self.trading.ticket = None;
        true
    }

    pub(super) fn select_next_option(&mut self) -> bool {
        if self.trading.submit_is_running() {
            return false;
        }
        let Some(detail) = &self.trading.detail else {
            return false;
        };
        let indices = quote_indices_by_kind(detail, self.trading.active_option_kind);
        if indices.is_empty() {
            return false;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.trading.selected_option)
            .unwrap_or(0);
        if current + 1 >= indices.len() {
            return false;
        }
        self.trading.selected_option = indices[current + 1];
        self.panel_scrolls.insert(
            option_kind_focus(self.trading.active_option_kind),
            current + 1,
        );
        self.trading.ticket = None;
        true
    }

    pub(super) fn focus_markets(&mut self) {
        self.focus = LabFocus::Markets;
        self.trading.chain_focus = ChainFocus::Markets;
    }

    pub(super) fn focus_market_series(&mut self) {
        self.trading.market_series_open = true;
        self.focus = LabFocus::MarketSeries;
        self.trading.chain_focus = ChainFocus::Markets;
    }

    pub(super) fn focus_option_side(&mut self, target: OptionKind) {
        if self.trading.submit_is_running() {
            return;
        }
        self.focus = match target {
            OptionKind::Call => LabFocus::Calls,
            OptionKind::Put => LabFocus::Puts,
        };
        self.trading.chain_focus = match target {
            OptionKind::Call => ChainFocus::Calls,
            OptionKind::Put => ChainFocus::Puts,
        };
        self.select_option_side(target);
    }

    pub(super) fn select_option_side(&mut self, target: OptionKind) {
        if self.trading.submit_is_running() {
            return;
        }
        self.trading.active_option_kind = target;
        let Some(detail) = &self.trading.detail else {
            return;
        };
        let target_indices = quote_indices_by_kind(detail, target);
        if target_indices.is_empty() {
            return;
        }
        let target_rank = self
            .focused_panel_scroll(option_kind_focus(target))
            .min(target_indices.len().saturating_sub(1));
        self.trading.selected_option = target_indices[target_rank];
        self.trading.ticket = None;
    }

    pub(super) fn select_option_index(&mut self, target: OptionKind, index: usize) -> bool {
        if self.trading.submit_is_running() {
            self.status =
                "Order submission is running. Wait before selecting another contract.".to_string();
            return false;
        }
        let label = self
            .trading
            .detail
            .as_ref()
            .and_then(|detail| detail.option_quotes.get(index))
            .filter(|quote| quote.kind == target)
            .map(|quote| {
                format!(
                    "{} {}/{}",
                    target.label(),
                    quote.lower_strike,
                    quote.upper_strike
                )
            });
        let Some(label) = label else {
            return false;
        };

        self.focus_option_side(target);
        self.trading.selected_option = index;
        self.trading.active_option_kind = target;
        if let Some(rank) = self
            .trading
            .detail
            .as_ref()
            .and_then(|detail| quote_rank_by_kind(detail, index, target))
        {
            self.panel_scrolls.insert(option_kind_focus(target), rank);
        }
        self.trading.ticket = None;
        self.status = format!("{label} selected. Press B to buy or S to sell.");
        true
    }

    pub(super) fn select_prev(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.trading.submit_is_running() {
            return false;
        }
        if self.trading.dishes.is_empty() || self.trading.selected == 0 {
            return false;
        }
        self.trading.selected -= 1;
        self.after_market_selection_changed(backend_url, fetch_tx);
        true
    }

    pub(super) fn select_next(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.trading.submit_is_running() {
            return false;
        }
        if self.trading.dishes.is_empty() || self.trading.selected + 1 >= self.trading.dishes.len()
        {
            return false;
        }
        self.trading.selected += 1;
        self.after_market_selection_changed(backend_url, fetch_tx);
        true
    }

    pub(super) fn select_market_index(
        &mut self,
        index: usize,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        if self.trading.submit_is_running() {
            self.status = "Order submission is running. Wait before changing markets.".to_string();
            return false;
        }
        if index >= self.trading.dishes.len() {
            return false;
        }
        self.set_focus(LabFocus::Markets);
        if self.trading.selected == index {
            return true;
        }
        self.trading.selected = index;
        self.after_market_selection_changed(backend_url, fetch_tx);
        true
    }

    pub(super) fn activate_market_row(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let market_id = self.selected_id();
        self.trading.market_series_open = true;
        let has_loaded_series = self
            .trading
            .detail
            .as_ref()
            .filter(|detail| detail.id.eq_ignore_ascii_case(&market_id))
            .map(|detail| !detail.expiries.is_empty())
            .unwrap_or(false);

        if has_loaded_series {
            self.focus_market_series();
            self.status = format!(
                "Select an open {} month, then press Enter for options.",
                market_id.to_uppercase()
            );
        } else {
            self.focus_market_series();
            self.status = format!("Loading {} open months...", market_id.to_uppercase());
            self.request_selected_detail(backend_url, fetch_tx, false);
        }
    }

    pub(super) fn activate_market_series(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.selected_chart_expiry().is_none() {
            self.status = format!(
                "Loading {} open months...",
                self.selected_id().to_uppercase()
            );
            self.request_selected_detail(backend_url, fetch_tx, false);
            return;
        }

        let label = self
            .selected_chart_expiry()
            .map(|expiry| expiry.label.clone())
            .unwrap_or_else(|| "selected month".to_string());
        self.trading.sync_selected_expiry();
        self.trading.clamp_selected_option();
        if self.screen == LabScreen::Oracle {
            self.refresh_oracle_series_in_place(backend_url, fetch_tx, true);
            return;
        }
        self.open_chain();
        self.request_oracle_live(backend_url, fetch_tx, true);
        self.status = format!("{} fixed-risk contracts", label);
    }

    pub(super) fn select_prev_market_series(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        let selected = self.select_market_series_by_offset(-1);
        if selected {
            match self.screen {
                LabScreen::Chart => self.request_chart(backend_url, fetch_tx, false),
                LabScreen::Chain => self.request_oracle_live(backend_url, fetch_tx, false),
                LabScreen::Oracle => {
                    self.refresh_oracle_series_in_place(backend_url, fetch_tx, false)
                }
                _ => {}
            }
        }
        selected
    }

    pub(super) fn select_next_market_series(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) -> bool {
        let selected = self.select_market_series_by_offset(1);
        if selected {
            match self.screen {
                LabScreen::Chart => self.request_chart(backend_url, fetch_tx, false),
                LabScreen::Chain => self.request_oracle_live(backend_url, fetch_tx, false),
                LabScreen::Oracle => {
                    self.refresh_oracle_series_in_place(backend_url, fetch_tx, false)
                }
                _ => {}
            }
        }
        selected
    }

    fn refresh_oracle_series_in_place(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        self.trading.sync_selected_expiry();
        self.trading.clamp_selected_option();
        self.clamp_oracle_selection();
        self.oracle.form = None;
        self.oracle.form_field_flash = None;
        self.oracle.locked_flash = None;
        self.request_oracle_tree(backend_url, fetch_tx, false);
        self.request_oracle_live(backend_url, fetch_tx, force);
        self.request_oracle_rewards(backend_url, fetch_tx, force);
        let label = self
            .selected_chart_expiry()
            .map(|expiry| expiry.label.clone())
            .unwrap_or_else(|| "selected month".to_string());
        self.status = format!("Loading {label} oracle evidence...");
    }

    pub(super) fn select_market_series_by_offset(&mut self, offset: isize) -> bool {
        let Some(detail) = &self.trading.detail else {
            return false;
        };
        if detail.expiries.is_empty() {
            return false;
        }
        let next = self.trading.chart_expiry as isize + offset;
        if next < 0 || next >= detail.expiries.len() as isize {
            return false;
        }
        let next = next as usize;
        self.select_market_series_index(next)
    }

    pub(super) fn market_series_count(&self) -> usize {
        self.trading
            .detail
            .as_ref()
            .filter(|detail| detail.id.eq_ignore_ascii_case(&self.selected_id()))
            .map(|detail| detail.expiries.len())
            .unwrap_or(0)
    }

    pub(super) fn select_next_after_market_series(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.select_next(backend_url, fetch_tx) {
            self.set_focus(LabFocus::Markets);
        }
    }

    pub(super) fn select_market_series_index(&mut self, index: usize) -> bool {
        if self.trading.submit_is_running() {
            return false;
        }
        let count = self
            .trading
            .detail
            .as_ref()
            .map(|detail| detail.expiries.len())
            .unwrap_or(0);
        if count == 0 {
            self.trading.chart_expiry = 0;
            return false;
        }
        let next = index.min(count - 1);
        if self.trading.chart_expiry == next {
            self.trading.sync_selected_expiry();
            self.trading.clamp_selected_option();
            if self.screen == LabScreen::Oracle {
                self.clamp_oracle_selection();
            }
            return true;
        }
        self.trading.chart_expiry = next;
        self.clear_guide_context_actions();
        self.trading.chart = None;
        self.trading.settlement_bundle = None;
        self.trading.settlement_issue = None;
        self.trading.selected_option = 0;
        self.trading.active_option_kind = OptionKind::Call;
        self.trading.ticket = None;
        self.trading.sync_selected_expiry();
        self.trading.clamp_selected_option();
        if self.screen == LabScreen::Oracle {
            self.clamp_oracle_selection();
        }
        let label = self
            .selected_chart_expiry()
            .map(|expiry| expiry.label.clone())
            .unwrap_or_else(|| "selected month".to_string());
        self.status = format!("{label} selected. Press Enter for options.");
        true
    }

    pub(super) fn after_market_selection_changed(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.clear_guide_context_actions();
        self.trading.selected_option = 0;
        self.trading.active_option_kind = OptionKind::Call;
        self.trading.chart_expiry = 0;
        self.trading.chart = None;
        self.trading.settlement_bundle = None;
        self.trading.settlement_issue = None;
        if !self.trading.submit_is_running() {
            self.trading.ticket = None;
        }
        self.trading.market_series_open = false;
        self.ensure_read_cache_scope(backend_url);
        let market_id = self.selected_id();
        if let Some(detail) = self.cache.details().get(&market_id).cloned() {
            self.trading.detail = Some(detail);
            self.trading.sync_selected_expiry();
            self.trading.clamp_selected_option();
            self.trading.loading_detail = false;
            self.status = format!("{} market ready", market_id.to_uppercase());
        } else {
            self.trading.detail = None;
            self.status = format!("Loading {} market...", market_id.to_uppercase());
        }
        self.request_selected_detail(backend_url, fetch_tx, false);
    }
}
