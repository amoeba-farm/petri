//! Feature request launch and stale-result reduction for background Lab messages.

use super::*;

impl LabApp {
    pub(super) fn retry_oracle_tree_if_due(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.screen != LabScreen::Oracle
            || self.oracle.loading_tree
            || self.oracle.tree_issue.is_none()
        {
            return;
        }
        if !self
            .oracle
            .tree_issue
            .as_deref()
            .is_some_and(oracle_tree_issue_is_retryable)
        {
            return;
        }
        let Some(retry_after_tick) = self.oracle.tree_retry_after_tick else {
            return;
        };
        if self.spinner_tick < retry_after_tick {
            return;
        }
        self.request_oracle_tree(backend_url, fetch_tx, true);
    }

    pub(super) fn request_market_list(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.trading.list_request = self.trading.list_request.wrapping_add(1);
        self.trading.loading_list = true;
        self.status = "Loading live markets...".to_string();
        spawn_market_list_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.trading.list_request,
        );
    }

    pub(super) fn request_market_refresh(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        self.ensure_read_cache_scope(backend_url);
        self.invalidate_market_caches();
        if self.screen == LabScreen::Home {
            self.request_market_list(backend_url, fetch_tx);
            if !self.trading.dishes.is_empty() {
                self.request_selected_detail(backend_url, fetch_tx, true);
            }
        } else if self.trading.dishes.is_empty() {
            self.request_market_list(backend_url, fetch_tx);
        } else {
            self.request_selected_detail(backend_url, fetch_tx, true);
        }
    }

    pub(super) fn request_selected_detail(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        self.ensure_read_cache_scope(backend_url);
        let market_id = self.selected_id();
        if force {
            self.cache.details_mut().remove(&market_id);
        }
        if !force {
            if let Some(detail) = self.cache.details().get(&market_id).cloned() {
                self.trading.detail = Some(detail);
                if self.screen == LabScreen::Chart {
                    self.apply_initial_chart_expiry();
                }
                self.trading.sync_selected_expiry();
                self.trading.clamp_selected_option();
                self.trading.loading_detail = false;
                self.status = format!("{} market ready", market_id.to_uppercase());
                match self.screen {
                    LabScreen::Chart => self.request_chart(backend_url, fetch_tx, false),
                    LabScreen::Chain | LabScreen::Oracle => {
                        self.request_oracle_live(backend_url, fetch_tx, false);
                        if self.screen == LabScreen::Oracle {
                            self.request_oracle_rewards(backend_url, fetch_tx, false);
                        }
                    }
                    _ => {}
                }
                return;
            }
        }

        if !read_cache::begin_cached_read(
            self.cache
                .details_mut()
                .begin_fetch(market_id.clone(), force),
            &mut self.trading.detail_request,
            &mut self.trading.loading_detail,
        ) {
            return;
        }
        self.status = if force {
            format!("Refreshing {} market...", market_id.to_uppercase())
        } else {
            format!("Loading {} market...", market_id.to_uppercase())
        };
        spawn_detail_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.trading.detail_request,
            market_id,
        );
    }

    pub(super) fn request_chart(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        self.ensure_read_cache_scope(backend_url);
        let args = self.chart_args();
        let key = chart_cache_key(&args);
        if force {
            self.cache.charts_mut().remove(&key);
        }
        let month_label = self
            .selected_chart_expiry()
            .map(|expiry| expiry.label.clone())
            .unwrap_or_else(|| "selected month".to_string());

        if !force {
            if let Some(chart) = self.cache.charts().get(&key).cloned() {
                let points = chart.point_count();
                self.trading.chart = Some(chart);
                self.trading.loading_chart = false;
                self.trading.chart_last_refresh_at = Some(Instant::now());
                self.status = format!(
                    "{} {} chart ready ({points} points)",
                    args.market.to_uppercase(),
                    month_label
                );
                return;
            }
        }

        if !self.cache.charts().contains_key(&key) {
            self.trading.chart = None;
        }
        if !read_cache::begin_cached_read(
            self.cache.charts_mut().begin_fetch(key.clone(), force),
            &mut self.trading.chart_request,
            &mut self.trading.loading_chart,
        ) {
            return;
        }
        self.status = if force {
            format!(
                "Refreshing {} {} chart...",
                args.market.to_uppercase(),
                month_label
            )
        } else {
            format!(
                "Loading {} {} chart...",
                args.market.to_uppercase(),
                month_label
            )
        };
        spawn_chart_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.trading.chart_request,
            key,
            args.market.clone(),
            month_label,
            args,
        );
    }

    pub(super) fn request_settlement(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        refresh: bool,
    ) {
        self.ensure_read_cache_scope(backend_url);
        let Some((market_id, expiry_id)) = self.selected_settlement_identity() else {
            self.trading.loading_settlement = false;
            self.trading.settlement_bundle = None;
            self.trading.settlement_issue =
                Some("Select a listed month before opening settlement evidence.".to_string());
            self.status = "Select a listed month before opening settlement evidence.".to_string();
            return;
        };
        let key = (market_id.clone(), expiry_id.clone());
        if refresh {
            self.cache.settlements_mut().remove(&key);
        }
        if !refresh && let Some(bundle) = self.cache.settlements().get(&key).cloned() {
            self.trading.settlement_bundle = Some(bundle);
            self.trading.settlement_issue = None;
            self.trading.loading_settlement = false;
            self.status = "Settlement evidence ready.".to_string();
            return;
        }
        if !read_cache::begin_cached_read(
            self.cache.settlements_mut().begin_fetch(key, refresh),
            &mut self.trading.settlement_request,
            &mut self.trading.loading_settlement,
        ) {
            return;
        }
        self.trading.settlement_issue = None;
        self.status = format!("Loading settlement evidence for {expiry_id}...");
        spawn_settlement_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.trading.settlement_request,
            market_id,
            expiry_id,
        );
    }

    fn selected_settlement_identity(&self) -> Option<(String, String)> {
        let market_id = self.selected_id();
        if market_id.trim().is_empty() {
            return None;
        }
        let expiry_id = self
            .selected_chart_expiry()
            .map(|expiry| expiry.id.clone())
            .or_else(|| {
                self.trading
                    .detail
                    .as_ref()
                    .map(|detail| detail.expiry_id.clone())
                    .filter(|expiry| !expiry.is_empty() && expiry != "-")
            })?;
        Some((market_id, expiry_id))
    }

    pub(super) fn request_ledger(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        refresh: bool,
    ) {
        let Some(owner_pubkey) = self.wallet.pubkey.clone() else {
            self.loading_ledger = false;
            self.replace_ledger_preserving_writer_close_capability(None);
            self.status =
                "No attached wallet. Set --keypair, SOLANA_KEYPAIR, or Solana CLI keypair_path."
                    .to_string();
            return;
        };
        if self.ledger_matches_owner(&owner_pubkey) && !refresh {
            self.status = format!("Ledger ready for {}", short_pubkey(&owner_pubkey));
            return;
        }
        self.ledger_request = self.ledger_request.wrapping_add(1);
        self.loading_ledger = true;
        self.status = format!("Loading ledger for {}...", short_pubkey(&owner_pubkey));
        spawn_ledger_fetch(
            backend_url.to_string(),
            self.onchain_config.clone(),
            fetch_tx.clone(),
            self.ledger_request,
            owner_pubkey,
        );
    }

    pub(super) fn request_writer_close_capabilities(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.interaction_is_locked() {
            return;
        }
        self.writers.action_request = self.writers.action_request.wrapping_add(1);
        self.clear_writer_close_capability();
        spawn_writer_capabilities_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.writers.action_request,
        );
    }

    pub(super) fn request_staking_status(
        &mut self,
        fetch_tx: &Sender<LabFetchResult>,
        refresh: bool,
    ) {
        let Some(owner_pubkey) = self.wallet.pubkey.clone() else {
            self.loading_staking = false;
            self.staking_status = None;
            self.staking_issue = Some("Attach a wallet to view staking balances.".to_string());
            self.status = "Attach a wallet to view staking balances.".to_string();
            return;
        };
        if let Some(status) = self.staking_status.as_ref()
            && !refresh
        {
            self.status = if staking_status_is_available(status) {
                format!("Staking ready for {}", short_pubkey(&owner_pubkey))
            } else {
                staking_availability_note(status)
            };
            return;
        }
        self.staking_status_request = self.staking_status_request.wrapping_add(1);
        self.loading_staking = true;
        self.staking_issue = None;
        self.status = if refresh {
            "Refreshing staking balances...".to_string()
        } else {
            format!("Loading staking for {}...", short_pubkey(&owner_pubkey))
        };
        spawn_staking_status_fetch(
            self.onchain_config.clone(),
            fetch_tx.clone(),
            self.staking_status_request,
            owner_pubkey,
        );
    }

    pub(super) fn request_oracle_tree(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        let market_id = self.selected_id();
        if !force
            && self
                .oracle
                .tree
                .as_ref()
                .map(|tree| tree.market_id.eq_ignore_ascii_case(&market_id))
                .unwrap_or(false)
        {
            self.oracle.loading_tree = false;
            return;
        }

        if self
            .oracle
            .tree
            .as_ref()
            .is_some_and(|tree| !tree.market_id.eq_ignore_ascii_case(&market_id))
        {
            self.oracle.tree = None;
            self.oracle.node_selected = DEFAULT_ORACLE_NODE_INDEX;
        }
        self.oracle.tree_request = self.oracle.tree_request.wrapping_add(1);
        self.oracle.loading_tree = true;
        self.oracle.tree_issue = None;
        self.oracle.tree_retry_after_tick = None;
        self.status = format!(
            "Loading {} oracle source recipe...",
            market_id.to_uppercase()
        );
        spawn_oracle_tree_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.oracle.tree_request,
            market_id,
        );
    }

    pub(super) fn request_oracle_live(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        let market_id = self.selected_id();
        let Some(expiry_id) = self.selected_chart_expiry().map(|expiry| expiry.id.clone()) else {
            self.oracle.live = None;
            self.oracle.live_issue =
                Some("Select a monthly series to load oracle evidence.".into());
            self.oracle.loading_live = false;
            return;
        };
        if !force
            && self.oracle.live.as_ref().is_some_and(|state| {
                state.market_id.eq_ignore_ascii_case(&market_id)
                    && state.expiry_id.eq_ignore_ascii_case(&expiry_id)
            })
        {
            self.oracle.loading_live = false;
            return;
        }

        if self.oracle.live.as_ref().is_some_and(|state| {
            !state.market_id.eq_ignore_ascii_case(&market_id)
                || !state.expiry_id.eq_ignore_ascii_case(&expiry_id)
        }) {
            self.oracle.live = None;
            self.oracle.live_issue = None;
        }
        self.oracle.live_request = self.oracle.live_request.wrapping_add(1);
        self.oracle.loading_live = true;
        self.oracle.live_issue = None;
        spawn_oracle_live_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.oracle.live_request,
            market_id,
            expiry_id,
        );
    }

    pub(super) fn request_oracle_rewards(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        force: bool,
    ) {
        let market_id = self.selected_id();
        let Some(expiry_id) = self.selected_chart_expiry().map(|expiry| expiry.id.clone()) else {
            self.oracle.rewards = None;
            self.oracle.reward_issue = None;
            self.oracle.loading_rewards = false;
            self.clamp_oracle_selection();
            return;
        };
        let Some(owner_pubkey) = self.wallet.pubkey.clone() else {
            self.oracle.rewards = None;
            self.oracle.reward_issue = None;
            self.oracle.loading_rewards = false;
            self.clamp_oracle_selection();
            return;
        };
        if !force
            && self.oracle.rewards.as_ref().is_some_and(|state| {
                state.market_id.eq_ignore_ascii_case(&market_id)
                    && state.expiry_id.eq_ignore_ascii_case(&expiry_id)
                    && state.owner_pubkey == owner_pubkey
            })
        {
            self.oracle.loading_rewards = false;
            return;
        }

        if self.oracle.rewards.as_ref().is_some_and(|state| {
            !state.market_id.eq_ignore_ascii_case(&market_id)
                || !state.expiry_id.eq_ignore_ascii_case(&expiry_id)
                || state.owner_pubkey != owner_pubkey
        }) {
            self.oracle.rewards = None;
            self.oracle.reward_issue = None;
            self.clamp_oracle_selection();
        }
        self.oracle.reward_request = self.oracle.reward_request.wrapping_add(1);
        self.oracle.loading_rewards = true;
        self.oracle.reward_issue = None;
        spawn_oracle_rewards_fetch(
            backend_url.to_string(),
            fetch_tx.clone(),
            self.oracle.reward_request,
            market_id,
            expiry_id,
            owner_pubkey,
        );
    }

    pub(super) fn apply_completed_fetches(
        &mut self,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        fetch_rx: &Receiver<LabFetchResult>,
    ) {
        while let Ok(result) = fetch_rx.try_recv() {
            match result {
                LabFetchResult::ActionPanel {
                    id,
                    executed,
                    result,
                } => self.apply_action_panel(id, executed, result, backend_url, fetch_tx),
                LabFetchResult::ReadPanel { id, result } => {
                    if let Some(panel) = self.read_panel.as_mut().filter(|p| p.id == id) {
                        match result {
                            Ok((content, operations)) => {
                                panel.content = content;
                                panel.operations = operations;
                                panel.issue = None;
                            }
                            Err(error) => panel.issue = Some(error),
                        }
                    }
                }
                LabFetchResult::MarketList { request_id, result } => {
                    self.apply_market_list_result(request_id, result, backend_url, fetch_tx);
                }
                LabFetchResult::Detail {
                    request_id,
                    market_id,
                    result,
                } => {
                    self.apply_detail_result(request_id, market_id, result, backend_url, fetch_tx);
                }
                LabFetchResult::Settlement {
                    request_id,
                    market_id,
                    expiry_id,
                    bundle,
                } => self.apply_settlement_result(request_id, market_id, expiry_id, bundle),
                LabFetchResult::Chart {
                    request_id,
                    key,
                    market_id,
                    month_label,
                    result,
                } => {
                    self.apply_chart_result(request_id, key, market_id, month_label, result);
                }
                LabFetchResult::Ledger {
                    request_id,
                    owner_pubkey,
                    result,
                } => self.apply_ledger_result(request_id, owner_pubkey, result),
                LabFetchResult::LiquidityPreview { request_id, result } => {
                    self.apply_liquidity_preview_result(request_id, result)
                }
                LabFetchResult::WriterCommand {
                    request_id,
                    action,
                    result,
                } => self.apply_writer_command_result(
                    request_id,
                    action,
                    result,
                    backend_url,
                    fetch_tx,
                ),
                LabFetchResult::WriterCapabilities { request_id, result } => {
                    self.apply_writer_close_capabilities_result(request_id, result)
                }
                LabFetchResult::WriterActionMask {
                    request_id,
                    action,
                    owner,
                    sleeve,
                    result,
                } => {
                    self.apply_writer_action_mask_result(request_id, action, owner, sleeve, result)
                }
                LabFetchResult::StakingStatus {
                    request_id,
                    owner_pubkey,
                    result,
                } => self.apply_staking_status_result(request_id, owner_pubkey, result),
                LabFetchResult::StakingAction {
                    request_id,
                    action,
                    result,
                } => self.apply_staking_action_result(request_id, action, result, fetch_tx),
                LabFetchResult::TradePrepare {
                    request_id,
                    owner,
                    expiry,
                    mut submit,
                    result,
                } => {
                    if self.trading.submit_inflight != Some(request_id) {
                        continue;
                    }
                    self.trading.submit_inflight = None;
                    let scope_matches = self.wallet.pubkey.as_deref() == Some(owner.as_str())
                        && self.selected_chart_expiry().is_some_and(|e| e.id == expiry);
                    if let Some(ticket) = self
                        .trading
                        .ticket
                        .as_mut()
                        .filter(|t| t.submit_request_id == Some(request_id))
                    {
                        ticket.submitting = false;
                        ticket.submit_request_id = None;
                        if !scope_matches {
                            ticket.clear_review();
                            self.status = "Selection changed. Prepare a fresh ticket.".into();
                            continue;
                        }
                        match result {
                            Ok(payload) => {
                                if let Some(id) = payload["operationId"].as_str() {
                                    if let Some(index) =
                                        submit.args.iter().position(|arg| arg == "trades")
                                    {
                                        submit.args.truncate(index);
                                        submit.args.extend([
                                            "trades".into(),
                                            "execute".into(),
                                            id.into(),
                                            "--yes".into(),
                                        ]);
                                        submit.command = display_command(&submit.args);
                                        ticket.last_command = Some(submit.command.clone());
                                        ticket.result = Some(TradeTicketResult {
                                            ok: true,
                                            message: crate::trade_service::render_response(
                                                &payload,
                                            ),
                                        });
                                        self.trading.review_scroll = 0;
                                        ticket.confirmation = Some(TradeConfirmation {
                                            prepared: submit,
                                            choice: TradeConfirmationChoice::Cancel,
                                        });
                                        self.status = "Trade prepared. Review the exact amounts and explicitly confirm.".into();
                                    }
                                } else {
                                    self.status = "Preparation did not return an operation. Nothing was signed.".into();
                                }
                            }
                            Err(error) => {
                                ticket.result = Some(TradeTicketResult {
                                    ok: false,
                                    message: error.clone(),
                                });
                                self.status = error;
                            }
                        }
                    }
                }
                LabFetchResult::TradeSubmit {
                    request_id,
                    action,
                    summary,
                    command,
                    result,
                } => {
                    let confirmed =
                        result.is_ok() && self.trading.submit_inflight == Some(request_id);
                    self.apply_trade_submit_result_at(
                        request_id,
                        action,
                        Some(summary),
                        command,
                        result,
                        Instant::now(),
                    );
                    if confirmed {
                        self.request_ledger(backend_url, fetch_tx, true);
                        self.request_selected_detail(backend_url, fetch_tx, true);
                    }
                }
                LabFetchResult::OracleTree {
                    request_id,
                    market_id,
                    result,
                } => self.apply_oracle_tree_result(request_id, market_id, result),
                LabFetchResult::OracleLive {
                    request_id,
                    market_id,
                    expiry_id,
                    result,
                } => self.apply_oracle_live_result(request_id, market_id, expiry_id, result),
                LabFetchResult::OracleRewards {
                    request_id,
                    market_id,
                    expiry_id,
                    owner_pubkey,
                    result,
                } => self.apply_oracle_rewards_result(
                    request_id,
                    market_id,
                    expiry_id,
                    owner_pubkey,
                    result,
                ),
                LabFetchResult::HelpIndex { request_id, result } => {
                    if self.apply_help_index_result(request_id, result) {
                        self.request_current_help_page(fetch_tx, true);
                    }
                }
                LabFetchResult::HelpPage {
                    request_id,
                    page_id,
                    result,
                } => self.apply_help_page_result(request_id, page_id, result),
                LabFetchResult::HelpPreviewPage {
                    request_id,
                    page_id,
                    result,
                } => self.apply_help_preview_page_result(request_id, page_id, result),
                LabFetchResult::UpdateCheck { request_id, result } => {
                    self.apply_update_check_result(request_id, result)
                }
                LabFetchResult::GuideProbe { request_id, status } => {
                    self.apply_guide_probe_result(request_id, status)
                }
                LabFetchResult::GuideProgress { request_id, event } => {
                    self.apply_guide_progress(request_id, event)
                }
                LabFetchResult::GuideReply {
                    request_id,
                    state_revision,
                    result,
                } => self.apply_guide_reply(
                    request_id,
                    state_revision,
                    result,
                    backend_url,
                    fetch_tx,
                ),
            }
            self.continue_guide_after_navigation(fetch_tx);
        }
    }

    pub(super) fn apply_guide_probe_result(
        &mut self,
        request_id: u64,
        status: guide::GuideProviderStatus,
    ) {
        if request_id != self.guide.request_id || self.guide.loading {
            return;
        }
        self.guide.selected_provider = 0;
        if matches!(status, guide::GuideProviderStatus::Choose { .. }) {
            self.guide.composing = false;
        }
        self.guide.provider_status = status;
        if let Some(issue) = self.guide.config.issue.clone() {
            self.guide
                .push_message(guide::GuideConversationRole::Assistant, issue);
        }
    }

    pub(super) fn apply_guide_progress(&mut self, request_id: u64, event: guide::GuideStreamEvent) {
        if request_id != self.guide.request_id || !self.guide.loading {
            return;
        }
        let progress = match event {
            guide::GuideStreamEvent::Started => "starting",
            guide::GuideStreamEvent::Thinking => "thinking",
            guide::GuideStreamEvent::Finalizing => "finishing",
        };
        self.guide.progress = Some(progress.to_string());
    }

    pub(super) fn apply_guide_reply(
        &mut self,
        request_id: u64,
        state_revision: String,
        result: Result<guide::GuideProviderReply, String>,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if request_id != self.guide.request_id {
            return;
        }
        self.guide.loading = false;
        self.guide.progress = None;
        match result {
            Ok(reply) => {
                let _ = reply.session_id;
                if state_revision != self.guide_state_revision() {
                    self.guide.active_question = None;
                    self.guide.allow_continuation = false;
                    self.guide.pending_continuation = None;
                    self.guide.push_message(
                        guide::GuideConversationRole::Assistant,
                        reply.response.assistant_text,
                    );
                    self.guide.push_message(
                        guide::GuideConversationRole::Assistant,
                        "The screen changed while I was answering, so I left the TUI where it is. Ask again to navigate from the current view.",
                    );
                    self.status =
                        "Guide answer ready; navigation skipped because the screen changed."
                            .to_string();
                } else {
                    self.apply_guide_response(reply.response, backend_url, fetch_tx);
                }
            }
            Err(error) => {
                self.guide.active_question = None;
                self.guide.allow_continuation = false;
                self.guide.pending_continuation = None;
                self.guide
                    .push_message(guide::GuideConversationRole::Assistant, error.clone());
                self.status = error;
            }
        }
    }

    pub(super) fn apply_update_check_result(
        &mut self,
        request_id: u64,
        result: Result<WorkspaceUpdateReport, String>,
    ) {
        if let Some(status) = self.updates.apply(request_id, result) {
            self.status = status;
        }
    }

    pub(super) fn apply_market_list_result(
        &mut self,
        request_id: u64,
        result: Result<Value, String>,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if request_id != self.trading.list_request {
            return;
        }
        self.trading.loading_list = false;
        match result {
            Ok(payload) => {
                let next_dishes = extract_dish_summaries(&payload, "payload");
                self.issues = string_array(&payload, &["issues"]);
                if !next_dishes.is_empty() {
                    let preferred = self.trading.pending_initial_dish.take().or_else(|| {
                        self.trading
                            .dishes
                            .get(self.trading.selected)
                            .map(|dish| dish.id.clone())
                    });
                    self.trading.dishes = next_dishes;
                    self.trading.selected = preferred
                        .as_deref()
                        .and_then(|market_id| {
                            self.trading
                                .dishes
                                .iter()
                                .position(|dish| dish.id.eq_ignore_ascii_case(market_id))
                        })
                        .unwrap_or(0);
                }
                if self.trading.detail.is_none() && !self.trading.loading_detail {
                    self.request_selected_detail(backend_url, fetch_tx, false);
                } else if !self.trading.loading_detail && !self.trading.loading_chart {
                    self.status = "Markets ready".to_string();
                }
            }
            Err(error) => {
                self.issues.push(format!("market list: {error}"));
                if !self.trading.loading_detail && !self.trading.loading_chart {
                    self.status = if self.trading.dishes.is_empty() {
                        "Market list could not load. Press r to retry.".to_string()
                    } else {
                        "Market refresh failed; showing last loaded markets.".to_string()
                    };
                }
            }
        }
    }

    pub(super) fn apply_detail_result(
        &mut self,
        request_id: u64,
        market_id: String,
        result: Result<DishDetail, String>,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if request_id != self.trading.detail_request {
            return;
        }
        self.cache.details_mut().finish_fetch(&market_id);
        let is_current = self.selected_id().eq_ignore_ascii_case(&market_id);
        match result {
            Ok(detail) => {
                if request_id == self.trading.detail_request && is_current {
                    self.trading.loading_detail = false;
                    self.cache
                        .details_mut()
                        .insert(market_id.clone(), detail.clone());
                    self.trading.detail = Some(detail);
                    if self.screen == LabScreen::Chart {
                        self.apply_initial_chart_expiry();
                    }
                    self.trading.sync_selected_expiry();
                    self.trading.clamp_selected_option();
                    self.status = current_detail_status(&market_id, self.trading.detail.as_ref());
                    match self.screen {
                        LabScreen::Chart => self.request_chart(backend_url, fetch_tx, false),
                        LabScreen::Chain | LabScreen::Oracle => {
                            self.request_oracle_live(backend_url, fetch_tx, false);
                            if self.screen == LabScreen::Oracle {
                                self.request_oracle_rewards(backend_url, fetch_tx, false);
                            }
                        }
                        _ => {}
                    }
                    self.continue_guide_after_navigation(fetch_tx);
                }
            }
            Err(_error) => {
                if request_id == self.trading.detail_request && is_current {
                    self.trading.loading_detail = false;
                    if self.trading.detail.is_none() {
                        self.trading.selected_option = 0;
                        self.trading.active_option_kind = OptionKind::Call;
                        self.trading.chart_expiry = 0;
                        self.trading.chart = None;
                    }
                    self.status = format!("{} market could not load", market_id.to_uppercase());
                    let pending_matches = self
                        .guide
                        .pending_continuation
                        .as_ref()
                        .and_then(|pending| pending.market_id.as_deref())
                        .is_some_and(|pending_market| {
                            pending_market.eq_ignore_ascii_case(&market_id)
                        });
                    if pending_matches {
                        self.guide.pending_continuation = None;
                        self.guide.active_question = None;
                        self.guide.allow_continuation = false;
                        self.guide.push_message(
                            guide::GuideConversationRole::Assistant,
                            format!(
                                "{} market data could not load. I left the market open, and no ticket or order was created. You can retry the request or choose another market.",
                                market_id.to_uppercase()
                            ),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn apply_chart_result(
        &mut self,
        request_id: u64,
        key: String,
        market_id: String,
        month_label: String,
        result: Result<chart::EmbeddedChart, String>,
    ) {
        // Invalidation/refresh fences older jobs before they can refill storage.
        if request_id != self.trading.chart_request {
            return;
        }
        self.cache.charts_mut().finish_fetch(&key);
        let current_key = chart_cache_key(&self.chart_args());
        let is_current = request_id == self.trading.chart_request
            && key == current_key
            && self.screen == LabScreen::Chart;
        if is_current {
            self.trading.chart_last_refresh_at = Some(Instant::now());
        }
        match result {
            Ok(chart) => {
                let points = chart.point_count();
                if points == 0 {
                    self.cache.charts_mut().remove(&key);
                } else {
                    self.cache.charts_mut().insert(key, chart.clone());
                }
                if is_current {
                    self.trading.loading_chart = false;
                    self.trading.chart = Some(chart);
                    self.status = if points == 0 {
                        format!(
                            "No {} {} chart history yet. Try a wider range or another listed month.",
                            market_id.to_uppercase(),
                            month_label
                        )
                    } else {
                        format!(
                            "{} {} chart ready ({points} points)",
                            market_id.to_uppercase(),
                            month_label
                        )
                    };
                }
            }
            Err(_error) => {
                if is_current {
                    self.trading.loading_chart = false;
                    self.trading.chart = None;
                    self.status = format!(
                        "{} {} chart could not load",
                        market_id.to_uppercase(),
                        month_label
                    );
                }
            }
        }
    }

    pub(super) fn apply_ledger_result(
        &mut self,
        request_id: u64,
        owner_pubkey: String,
        result: Result<Value, String>,
    ) {
        if request_id != self.ledger_request {
            return;
        }
        self.loading_ledger = false;
        match result {
            Ok(payload) => {
                self.replace_ledger_preserving_writer_close_capability(Some(payload));
                self.status = format!("Ledger ready for {}", short_pubkey(&owner_pubkey));
            }
            Err(_error) => {
                self.replace_ledger_preserving_writer_close_capability(None);
                self.status = "Wallet activity is temporarily unavailable.".to_string();
            }
        }
    }

    pub(super) fn apply_writer_close_capabilities_result(
        &mut self,
        request_id: u64,
        result: Result<WriterCloseCapabilityProjection, String>,
    ) {
        if request_id != self.writers.action_request || self.writers.action_inflight.is_some() {
            return;
        }
        self.store_writer_close_capability(result);
    }

    pub(super) fn apply_writer_action_mask_result(
        &mut self,
        request_id: u64,
        action: WriterAction,
        owner: String,
        sleeve: String,
        result: Result<WriterActionMask, String>,
    ) {
        if self.writers.action_mask_inflight != Some(request_id) {
            return;
        }
        self.writers.action_mask_inflight = None;
        let Some(mut pending) = self.writers.pending_review.take() else {
            return;
        };
        if let Err(error) = crate::current_release::require_current_write_release() {
            self.writers.confirmation = None;
            self.status = error.to_string();
            return;
        }
        let still_bound = pending.confirmation.action == action
            && pending.owner == owner
            && pending.sleeve == sleeve
            && self.wallet.pubkey.as_deref() == Some(owner.as_str())
            && self.selected_writer_sleeve_address().as_deref() == Some(sleeve.as_str())
            && self.writers.form.as_ref() == Some(&pending.form);
        if !still_bound {
            self.status = "Wallet, sleeve, or writer form changed while availability was checked. Review again; nothing was signed or sent."
                .to_string();
            return;
        }
        let mask = match result {
            Ok(mask) => mask,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if mask.owner != owner || mask.sleeve != sleeve {
            self.status = "Wallet-specific availability did not bind the exact requested owner and sleeve. Review again; nothing was signed or sent."
                .to_string();
            return;
        }
        let authorized_kind = match writers::require_writer_review_action(&mask, action) {
            Ok(kind) => kind,
            Err(issue) => {
                self.status = format!(
                    "Current wallet-specific writer action is unavailable: {issue}. Nothing was signed or sent."
                );
                return;
            }
        };
        pending.confirmation.summary.push(format!(
            "Wallet-specific check: {} enabled at finalized slot {} ({}) for this exact owner and sleeve.",
            authorized_kind.wire_name(),
            mask.observed_slot,
            mask.observed_at.to_rfc3339(),
        ));
        self.writers.confirmation = Some(pending.confirmation);
        self.status = "Wallet-specific availability was verified. Review the values; execution rechecks finalized runtime permission. Nothing was signed or sent."
            .to_string();
    }

    pub(super) fn apply_settlement_result(
        &mut self,
        request_id: u64,
        market_id: String,
        expiry_id: String,
        bundle: settlement_data::SettlementBundle,
    ) {
        if request_id != self.trading.settlement_request {
            return;
        }
        self.cache
            .settlements_mut()
            .finish_fetch(&(market_id.clone(), expiry_id.clone()));
        self.trading.loading_settlement = false;
        let selection_matches = self
            .selected_settlement_identity()
            .is_some_and(|selection| selection == (market_id.clone(), expiry_id.clone()));
        if !selection_matches {
            self.status =
                "Settlement selection changed while evidence was loading; stale data was discarded."
                    .to_string();
            return;
        }
        if bundle.cache_key() != (market_id.as_str(), expiry_id.as_str()) {
            self.trading.settlement_bundle = None;
            self.trading.settlement_issue = Some(
                "Settlement evidence did not match the exact selected market and month."
                    .to_string(),
            );
            self.status = "Settlement evidence failed exact identity binding.".to_string();
            return;
        }
        let available = bundle.available_endpoint_count();
        let issues = bundle.issue_count();
        if issues == 0 {
            self.cache
                .settlements_mut()
                .insert((market_id, expiry_id), bundle.clone());
        }
        self.trading.settlement_issue = (available == 0)
            .then(|| "Settlement evidence is currently unavailable for this month.".to_string());
        self.trading.settlement_bundle = Some(bundle);
        self.status = if issues == 0 {
            "Settlement record, readiness, and oracle evidence are ready.".to_string()
        } else {
            format!("Settlement evidence loaded with {issues} scoped unavailable section(s).")
        };
    }

    pub(super) fn apply_writer_command_result(
        &mut self,
        request_id: u64,
        action: WriterAction,
        result: Result<Value, String>,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.action_inflight != Some(request_id) {
            return;
        }
        self.writers.action_inflight = None;
        self.writers.confirmation = None;
        match result {
            Ok(payload) => {
                let message = if action.signs_and_submits() {
                    format!("{} confirmed.", action.label())
                } else {
                    format!("{} ready.", action.label())
                };
                self.writers.action_result = Some(WriterActionResult {
                    action,
                    ok: true,
                    payload: Some(payload),
                    message: message.clone(),
                });
                self.status = message;
                if action.signs_and_submits() {
                    self.invalidate_market_caches();
                    self.replace_ledger_preserving_writer_close_capability(None);
                    self.request_ledger(backend_url, fetch_tx, true);
                }
            }
            Err(error) => {
                self.writers.action_result = Some(WriterActionResult {
                    action,
                    ok: false,
                    payload: None,
                    message: error.clone(),
                });
                self.status = error;
            }
        }
        self.request_writer_close_capabilities(backend_url, fetch_tx);
    }

    pub(super) fn apply_staking_status_result(
        &mut self,
        request_id: u64,
        owner_pubkey: String,
        result: Result<Value, String>,
    ) {
        if request_id != self.staking_status_request
            || self.wallet.pubkey.as_deref() != Some(owner_pubkey.as_str())
        {
            return;
        }
        self.loading_staking = false;
        match result {
            Ok(payload) => {
                self.status = if staking_status_is_available(&payload) {
                    format!("Staking ready for {}", short_pubkey(&owner_pubkey))
                } else {
                    staking_availability_note(&payload)
                };
                self.staking_status = Some(payload);
                self.staking_issue = None;
            }
            Err(error) => {
                self.staking_status = None;
                self.staking_issue = Some(error.clone());
                self.status = error;
            }
        }
    }

    pub(super) fn apply_staking_action_result(
        &mut self,
        request_id: u64,
        action: StakingAction,
        result: Result<(Value, String), String>,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.staking_action_inflight != Some(request_id) {
            return;
        }
        self.staking_action_inflight = None;
        self.staking_confirmation = None;
        match result {
            Ok((_payload, message)) => {
                self.staking_action_result = Some(StakingActionResult {
                    action,
                    ok: true,
                    message: message.clone(),
                });
                self.status = message;
                self.request_staking_status(fetch_tx, true);
            }
            Err(error) => {
                self.staking_action_result = Some(StakingActionResult {
                    action,
                    ok: false,
                    message: error.clone(),
                });
                self.status = error;
            }
        }
    }

    pub(super) fn apply_trade_submit_result_at(
        &mut self,
        request_id: u64,
        action: TradeAction,
        summary: Option<TradeConfirmationSummary>,
        command: String,
        result: Result<String, String>,
        now: Instant,
    ) {
        if Some(request_id) != self.trading.submit_inflight {
            return;
        }
        self.trading.submit_inflight = None;
        // Even an uncertain submission may have changed on-chain display data.
        self.invalidate_market_caches();
        let (ok, message) = match result {
            Ok(message) => (true, message),
            Err(message) => (false, message),
        };
        let waiting = !ok
            && (message.contains("Waiting for current finalized state")
                || message.contains("operations resume"));
        let mut details_in_ticket = false;
        if let Some(ticket) = self
            .trading
            .ticket
            .as_mut()
            .filter(|ticket| ticket.submit_request_id == Some(request_id))
        {
            ticket.submitting = false;
            ticket.submit_request_id = None;
            ticket.confirmation = None;
            ticket.last_command = Some(command);
            ticket.result = Some(TradeTicketResult {
                ok,
                message: message.clone(),
            });
            details_in_ticket = true;
        }
        self.status = if waiting {
            format!(
                "{} order needs status recovery. Inspect Operations; do not resubmit.",
                action.label()
            )
        } else if ok {
            format!("{} order confirmed.", action.label())
        } else if details_in_ticket {
            format!(
                "{} order was not confirmed. Inspect the receipt before another action.",
                action.label()
            )
        } else {
            format!(
                "{} order was not confirmed. Check Operations before another action.",
                action.label()
            )
        };
        let mut result_modal = TradeResultModal::new(ok, action, summary, details_in_ticket, now);
        result_modal.waiting = waiting;
        if waiting {
            result_modal.failure_reason = Some(message.clone());
        } else if !ok && details_in_ticket {
            result_modal.failure_reason =
                Some(user_safe_trade_failure_reason(&message).to_string());
        }
        self.trading.result_modal = Some(result_modal);
    }

    pub(super) fn apply_oracle_tree_result(
        &mut self,
        request_id: u64,
        market_id: String,
        result: Result<OracleTreeFetch, String>,
    ) {
        if request_id != self.oracle.tree_request
            || !self.selected_id().eq_ignore_ascii_case(&market_id)
        {
            return;
        }
        self.oracle.loading_tree = false;
        match result {
            Ok(fetch) => {
                let OracleTreeFetch { tree, notice } = fetch;
                self.oracle.node_selected = tree.root_index();
                self.oracle.tree = Some(tree);
                self.oracle.tree_issue = None;
                self.oracle.tree_retry_after_tick = None;
                if self.screen == LabScreen::Oracle {
                    self.status = if self.oracle.view == OracleView::Earn {
                        "Task details loaded. Checking current eligibility and rewards.".to_string()
                    } else {
                        notice.unwrap_or_else(|| {
                            format!("{} source recipe loaded.", market_id.to_uppercase())
                        })
                    };
                }
            }
            Err(error) => {
                let has_current_tree = self
                    .oracle
                    .tree
                    .as_ref()
                    .is_some_and(|tree| tree.market_id.eq_ignore_ascii_case(&market_id));
                if !has_current_tree {
                    self.oracle.tree = None;
                }
                let retryable = oracle_tree_issue_is_retryable(&error);
                let status_message = if retryable {
                    format!(
                        "Could not load {} oracle source recipe.",
                        market_id.to_uppercase()
                    )
                } else {
                    error.clone()
                };
                self.oracle.tree_issue = Some(error);
                self.oracle.tree_retry_after_tick =
                    retryable.then(|| self.spinner_tick.wrapping_add(ORACLE_TREE_RETRY_TICKS));
                if self.screen == LabScreen::Oracle {
                    self.status = if self.oracle.view == OracleView::Earn {
                        "Petri could not load the details required to match work.".to_string()
                    } else {
                        status_message
                    };
                }
            }
        }
    }

    pub(super) fn apply_oracle_live_result(
        &mut self,
        request_id: u64,
        market_id: String,
        expiry_id: String,
        result: Result<SpreadOracleLiveState, String>,
    ) {
        if request_id != self.oracle.live_request
            || !self.selected_id().eq_ignore_ascii_case(&market_id)
            || self
                .selected_chart_expiry()
                .is_none_or(|expiry| !expiry.id.eq_ignore_ascii_case(&expiry_id))
        {
            return;
        }
        self.oracle.loading_live = false;
        match result {
            Ok(state) => {
                let observation_count = state.observations.len();
                let emergency_count = state.active_emergency_count();
                self.oracle.live = Some(state);
                self.oracle.live_issue = None;
                if self.screen == LabScreen::Oracle && !self.oracle.loading_tree {
                    self.status = if self.oracle.view == OracleView::Earn {
                        "Current Oracle work status checked.".to_string()
                    } else if emergency_count > 0 {
                        format!(
                            "{emergency_count} emergency vote active in {observation_count} observations."
                        )
                    } else {
                        format!("{observation_count} live oracle observations loaded.")
                    };
                }
            }
            Err(error) => {
                self.oracle.live_issue = Some(error);
                if self.oracle.live.as_ref().is_some_and(|state| {
                    !state.market_id.eq_ignore_ascii_case(&market_id)
                        || !state.expiry_id.eq_ignore_ascii_case(&expiry_id)
                }) {
                    self.oracle.live = None;
                }
                if self.screen == LabScreen::Oracle && !self.oracle.loading_tree {
                    self.status = if self.oracle.view == OracleView::Earn {
                        "Petri could not verify current task availability.".to_string()
                    } else {
                        "Live oracle observations are temporarily unavailable.".to_string()
                    };
                }
            }
        }
    }

    pub(super) fn apply_oracle_rewards_result(
        &mut self,
        request_id: u64,
        market_id: String,
        expiry_id: String,
        owner_pubkey: String,
        result: Result<SpreadOracleRewardState, String>,
    ) {
        if request_id != self.oracle.reward_request
            || !self.selected_id().eq_ignore_ascii_case(&market_id)
            || !self
                .selected_chart_expiry()
                .is_some_and(|expiry| expiry.id.eq_ignore_ascii_case(&expiry_id))
            || self.wallet.pubkey.as_deref() != Some(owner_pubkey.as_str())
        {
            return;
        }
        self.oracle.loading_rewards = false;
        match result {
            Ok(state) => {
                let claim_count = state.claims.len();
                self.oracle.rewards = Some(state);
                self.oracle.reward_issue = None;
                self.clamp_oracle_selection();
                if self.screen == LabScreen::Oracle && !self.oracle.loading_tree {
                    self.status = if self.oracle.view == OracleView::Earn {
                        if claim_count > 0 {
                            format!(
                                "{claim_count} completed Oracle reward {} ready to review.",
                                if claim_count == 1 { "is" } else { "are" }
                            )
                        } else {
                            format!(
                                "No funded reward is ready for {} {} right now.",
                                market_id.to_uppercase(),
                                expiry_id.to_uppercase()
                            )
                        }
                    } else if claim_count > 0 {
                        format!(
                            "{claim_count} oracle reward {} available.",
                            if claim_count == 1 { "is" } else { "are" }
                        )
                    } else {
                        self.status.clone()
                    };
                }
            }
            Err(error) => {
                self.oracle.reward_issue = Some(error);
                self.oracle.rewards = None;
                self.clamp_oracle_selection();
                if self.screen == LabScreen::Oracle && self.oracle.view == OracleView::Earn {
                    self.status =
                        "Petri could not verify wallet-specific rewards. Nothing changed."
                            .to_string();
                }
            }
        }
    }
}
