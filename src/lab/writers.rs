//! Integrated wallet, position, history, and collective-writer workspace state.

use super::*;
use crate::writer_action_mask::CollectiveActionKind;

const WRITER_CLOSE_CAPABILITY_STORAGE_KEY: &str = "_petriWriterCloseCapability";
const WRITER_CLOSE_PREVIEW_ROUTE: (&str, &str) = ("POST", "/dlmm/writer-sleeves/closes/preview");
const WRITER_CLOSE_BEGIN_ROUTE: (&str, &str) = ("POST", "/dlmm/writer-sleeves/closes/prepare");
const WRITER_CLOSE_ADVANCE_ROUTE: (&str, &str) =
    ("POST", "/dlmm/writer-sleeves/closes/next/prepare");
const WRITER_CLOSE_CANCEL_ROUTE: (&str, &str) =
    ("POST", "/dlmm/writer-sleeves/closes/cancel/next/prepare");
const WRITER_CLOSE_SUBMIT_ROUTE: (&str, &str) = ("POST", "/dlmm/writer-sleeves/submit");
const WRITER_CLOSE_STATUS_ROUTE: (&str, &str) =
    ("GET", "/dlmm/writer-sleeves/close-requests/:request");

impl WriterActionAvailability {
    pub(super) fn is_actionable(&self) -> bool {
        !matches!(self, Self::Disabled(_))
    }

    pub(super) fn note(&self) -> Option<&str> {
        match self {
            Self::Enabled => None,
            Self::HotOnly(note) | Self::Disabled(note) => Some(note),
        }
    }
}

impl WriterCloseCapabilityProjection {
    fn has_operation(&self, operation: &str) -> bool {
        self.operations
            .iter()
            .any(|candidate| candidate == operation)
    }

    fn has_route(&self, method: &str, path: &str) -> bool {
        self.routes
            .iter()
            .any(|route| route.method == method && route.path == path)
    }
}

pub(super) fn parse_writer_close_capabilities(
    payload: &Value,
) -> Result<WriterCloseCapabilityProjection, String> {
    crate::writer_output::validate_current_writer_close_capabilities(payload)?;
    let lifecycle = payload
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("writer.close.lifecycle"))
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close lifecycle capability is missing".to_string())?;
    let implementation_supported = match lifecycle.get("implementation").and_then(Value::as_str) {
        Some("supported") => true,
        _ => {
            return Err(
                "writer-close implementation is not supported by the current lifecycle".to_string(),
            );
        }
    };
    let runtime_enabled = parse_runtime_with_reasons(
        lifecycle.get("runtime"),
        lifecycle.get("reasonCodes"),
        "lifecycle",
    )?;
    let light_modes = lifecycle
        .get("lightAccountModes")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close custody modes are missing".to_string())?;
    let hot_runtime_enabled = parse_custody_mode(light_modes.get("hot"), "hot")?;
    let cold_runtime_enabled = parse_custody_mode(light_modes.get("cold"), "cold")?;
    let operations = exact_string_list(lifecycle.get("operations"), "operations", 16)?;
    let route_values = lifecycle
        .get("routes")
        .and_then(Value::as_array)
        .filter(|routes| routes.len() <= 32)
        .ok_or_else(|| "writer-close routes are malformed".to_string())?;
    let mut routes = Vec::with_capacity(route_values.len());
    let mut seen_routes = HashSet::new();
    for route in route_values {
        let route = route
            .as_object()
            .ok_or_else(|| "writer-close route is malformed".to_string())?;
        let method = route
            .get("method")
            .and_then(Value::as_str)
            .filter(|method| matches!(*method, "GET" | "POST"))
            .ok_or_else(|| "writer-close route method is malformed".to_string())?;
        let path = route
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| path.starts_with('/') && path.len() <= 256)
            .ok_or_else(|| "writer-close route path is malformed".to_string())?;
        if !seen_routes.insert((method.to_string(), path.to_string())) {
            return Err("writer-close routes contain a duplicate".to_string());
        }
        routes.push(WriterCloseCapabilityRoute {
            method: method.to_string(),
            path: path.to_string(),
        });
    }

    Ok(WriterCloseCapabilityProjection {
        implementation_supported,
        runtime_enabled,
        hot_runtime_enabled,
        cold_runtime_enabled,
        operations,
        routes,
    })
}

fn enabled_or_disabled(value: Option<&Value>, issue: &str) -> Result<bool, String> {
    match value.and_then(Value::as_str) {
        Some("enabled") => Ok(true),
        Some("disabled") => Ok(false),
        _ => Err(issue.to_string()),
    }
}

fn parse_custody_mode(value: Option<&Value>, label: &str) -> Result<bool, String> {
    let mode = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("writer-close {label} custody capability is missing"))?;
    parse_runtime_with_reasons(
        mode.get("runtime"),
        mode.get("reasonCodes"),
        &format!("{label} custody"),
    )
}

fn parse_runtime_with_reasons(
    runtime: Option<&Value>,
    reason_codes: Option<&Value>,
    label: &str,
) -> Result<bool, String> {
    let enabled = enabled_or_disabled(
        runtime,
        &format!("writer-close {label} runtime capability is malformed"),
    )?;
    let reason_codes = exact_string_list(reason_codes, &format!("{label} reason codes"), 16)?;
    if (enabled && !reason_codes.is_empty()) || (!enabled && reason_codes.is_empty()) {
        return Err(format!(
            "writer-close {label} runtime and reason codes contradict each other"
        ));
    }
    Ok(enabled)
}

fn exact_string_list(
    value: Option<&Value>,
    label: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() <= limit)
        .ok_or_else(|| format!("writer-close {label} are malformed"))?;
    let mut strings = Vec::with_capacity(values.len());
    let mut seen = HashSet::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or_else(|| format!("writer-close {label} are malformed"))?;
        if !seen.insert(value.to_string()) {
            return Err(format!("writer-close {label} contain a duplicate"));
        }
        strings.push(value.to_string());
    }
    Ok(strings)
}

impl LabApp {
    pub(super) fn writer_close_capability_state(&self) -> WriterCloseCapabilityState {
        let Some(stored) = self
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.get(WRITER_CLOSE_CAPABILITY_STORAGE_KEY))
        else {
            return WriterCloseCapabilityState::Checking;
        };
        match stored.get("state").and_then(Value::as_str) {
            Some("ready") => projection_from_stored_value(stored)
                .map(WriterCloseCapabilityState::Ready)
                .unwrap_or(WriterCloseCapabilityState::Unavailable),
            Some("unavailable") => WriterCloseCapabilityState::Unavailable,
            _ => WriterCloseCapabilityState::Unavailable,
        }
    }

    pub(super) fn store_writer_close_capability(
        &mut self,
        result: Result<WriterCloseCapabilityProjection, String>,
    ) {
        let stored = match result {
            Ok(projection) => projection_to_stored_value(&projection),
            Err(_) => serde_json::json!({ "state": "unavailable" }),
        };
        let ledger = self
            .ledger
            .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !ledger.is_object() {
            *ledger = Value::Object(serde_json::Map::new());
        }
        if let Some(object) = ledger.as_object_mut() {
            object.insert(WRITER_CLOSE_CAPABILITY_STORAGE_KEY.to_string(), stored);
        }
    }

    pub(super) fn clear_writer_close_capability(&mut self) {
        let remove_empty_ledger = self.ledger.as_mut().is_some_and(|ledger| {
            ledger.as_object_mut().is_some_and(|object| {
                object.remove(WRITER_CLOSE_CAPABILITY_STORAGE_KEY);
                object.is_empty()
            })
        });
        if remove_empty_ledger {
            self.ledger = None;
        }
    }

    pub(super) fn replace_ledger_preserving_writer_close_capability(
        &mut self,
        replacement: Option<Value>,
    ) {
        self.writers.clear_action_mask_check();
        let stored = self
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.get(WRITER_CLOSE_CAPABILITY_STORAGE_KEY))
            .cloned();
        self.ledger = replacement.and_then(|mut ledger| {
            let object = ledger.as_object_mut()?;
            object.remove(WRITER_CLOSE_CAPABILITY_STORAGE_KEY);
            Some(ledger)
        });
        if let Some(stored) = stored {
            let ledger = self
                .ledger
                .get_or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Some(object) = ledger.as_object_mut() {
                object.insert(WRITER_CLOSE_CAPABILITY_STORAGE_KEY.to_string(), stored);
            }
        }
    }

    pub(super) fn ledger_matches_owner(&self, owner_pubkey: &str) -> bool {
        self.ledger
            .as_ref()
            .and_then(|ledger| string_at_key(ledger, &["ownerPubkey"]))
            .as_deref()
            == Some(owner_pubkey)
    }

    pub(super) fn writer_action_availability(
        &self,
        action: WriterAction,
    ) -> WriterActionAvailability {
        if action.signs_and_submits() {
            if let Err(error) = crate::current_release::require_current_write_release() {
                return WriterActionAvailability::Disabled(error.to_string());
            }
            // Keep review unavailable until current owner state has been loaded.
            // The child command independently proves finalized runtime permission.
            if !self.wallet.is_attached() {
                return WriterActionAvailability::Disabled(
                    "Attach a wallet before reviewing a writer change. Nothing was prepared, signed, or sent.".to_string(),
                );
            }
        }
        if action == WriterAction::CancelClose {
            return WriterActionAvailability::Disabled(
                crate::writer_output::WRITER_CLOSE_CANCEL_ACTION_MASK_UNAVAILABLE.to_string(),
            );
        }
        if !matches!(
            action,
            WriterAction::ClosePreview
                | WriterAction::BeginClose
                | WriterAction::CloseStatus
                | WriterAction::AdvanceClose
                | WriterAction::CancelClose
        ) {
            return WriterActionAvailability::Enabled;
        }
        let projection = match self.writer_close_capability_state() {
            WriterCloseCapabilityState::Checking => {
                return WriterActionAvailability::Disabled(
                    "Checking current writer-close availability. Refresh Writers if this does not complete."
                        .to_string(),
                );
            }
            WriterCloseCapabilityState::Unavailable => {
                return WriterActionAvailability::Disabled(
                    "Current writer-close availability could not be verified. Refresh Writers; nothing will be signed or sent."
                        .to_string(),
                );
            }
            WriterCloseCapabilityState::Ready(projection) => projection,
        };
        match action {
            WriterAction::ClosePreview => route_availability(
                &projection,
                WRITER_CLOSE_PREVIEW_ROUTE,
                "Close preview is not exposed by the current release.",
            ),
            WriterAction::CloseStatus => route_availability(
                &projection,
                WRITER_CLOSE_STATUS_ROUTE,
                "Close status is not exposed by the current release.",
            ),
            WriterAction::BeginClose => signed_close_availability(
                &projection,
                &["close_begin"],
                &[WRITER_CLOSE_BEGIN_ROUTE, WRITER_CLOSE_SUBMIT_ROUTE],
                true,
            ),
            WriterAction::AdvanceClose => signed_close_availability(
                &projection,
                &["close_basket", "close_finalize"],
                &[WRITER_CLOSE_ADVANCE_ROUTE, WRITER_CLOSE_SUBMIT_ROUTE],
                true,
            ),
            WriterAction::CancelClose => signed_close_availability(
                &projection,
                &["close_cancel"],
                &[WRITER_CLOSE_CANCEL_ROUTE, WRITER_CLOSE_SUBMIT_ROUTE],
                false,
            ),
            _ => WriterActionAvailability::Enabled,
        }
    }

    pub(super) fn writer_close_capability_cue(&self) -> String {
        match self.writer_close_capability_state() {
            WriterCloseCapabilityState::Checking => {
                "Writer-close reads: checking current Preview/Status availability. Wallet changes require finalized runtime permission."
                    .to_string()
            }
            WriterCloseCapabilityState::Unavailable => {
                "Writer-close reads: current Preview/Status availability could not be verified. Wallet changes require finalized runtime permission."
                    .to_string()
            }
            WriterCloseCapabilityState::Ready(projection) => {
                let preview = if route_availability(
                    &projection,
                    WRITER_CLOSE_PREVIEW_ROUTE,
                    "Preview unavailable",
                )
                .is_actionable()
                {
                    "available"
                } else {
                    "unavailable"
                };
                let status = if route_availability(
                    &projection,
                    WRITER_CLOSE_STATUS_ROUTE,
                    "Status unavailable",
                )
                .is_actionable()
                {
                    "available"
                } else {
                    "unavailable"
                };
                format!(
                    "Writer-close reads: Preview {preview}; Status {status}. Wallet changes require finalized runtime permission."
                )
            }
        }
    }

    pub(super) fn select_ledger_view(
        &mut self,
        view: LedgerView,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.interaction_is_locked() || self.writers.confirmation.is_some() {
            self.status =
                "Finish the open writer review before changing wallet sections.".to_string();
            return;
        }
        if self.liquidity_preview_is_running() {
            self.status =
                "Wait for the unsigned liquidity preview before changing wallet sections."
                    .to_string();
            return;
        }
        self.writers.form = None;
        self.liquidity_preview_form = None;
        self.ledger_view = view;
        self.ledger_pane = LedgerPane::Tabs;
        self.reset_panel_scroll(LabFocus::Ledger);
        self.status = match view {
            LedgerView::Account => "Account balances, collateral, and safe settings.".to_string(),
            LedgerView::Positions => {
                "Manager-liquidity positions and an unsigned Lean preview. Execution remains unavailable."
                    .to_string()
            }
            LedgerView::Writers => {
                "Global collective-writer sleeves. Select a sleeve, then choose an action."
                    .to_string()
            }
            LedgerView::History => {
                "Authoritative combined wallet history. Select a row for details.".to_string()
            }
        };
        self.request_ledger(backend_url, fetch_tx, false);
        if view == LedgerView::Writers {
            self.request_writer_close_capabilities(backend_url, fetch_tx);
        }
    }

    pub(super) fn move_ledger_view(
        &mut self,
        offset: isize,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if offset == 0 {
            return;
        }
        let current = LedgerView::ALL
            .iter()
            .position(|view| *view == self.ledger_view)
            .unwrap_or(0) as isize;
        let next = (current + offset).rem_euclid(LedgerView::ALL.len() as isize) as usize;
        self.select_ledger_view(LedgerView::ALL[next], backend_url, fetch_tx);
    }

    pub(super) fn move_ledger_pane(&mut self, offset: isize) {
        if offset == 0 || self.writers.form.is_some() || self.liquidity_preview_form.is_some() {
            return;
        }
        let available: &[LedgerPane] = match self.ledger_view {
            LedgerView::Account => &[LedgerPane::Tabs, LedgerPane::Actions, LedgerPane::Detail],
            LedgerView::Positions => &LedgerPane::ALL,
            LedgerView::History => &[LedgerPane::Tabs, LedgerPane::List, LedgerPane::Detail],
            LedgerView::Writers => &LedgerPane::ALL,
        };
        let current = available
            .iter()
            .position(|pane| *pane == self.ledger_pane)
            .unwrap_or(0) as isize;
        self.ledger_pane =
            available[(current + offset).rem_euclid(available.len() as isize) as usize];
        self.status = match self.ledger_pane {
            LedgerPane::Tabs => "Choose an account section.".to_string(),
            LedgerPane::List => "Choose a row to inspect.".to_string(),
            LedgerPane::Actions => "Choose an available action.".to_string(),
            LedgerPane::Detail => "Detail panel focused; use Page Up/Down to scroll.".to_string(),
        };
    }

    pub(super) fn move_ledger_selection(&mut self, offset: isize) {
        if offset == 0 {
            return;
        }
        if self.ledger_view == LedgerView::Writers && self.writers.interaction_is_locked() {
            self.status = "The selected writer sleeve is locked until the current action or availability check finishes."
                .to_string();
            return;
        }
        if self.ledger_view == LedgerView::Positions && self.liquidity_preview_is_running() {
            self.status =
                "The visible liquidity request is locked until its preview returns.".to_string();
            return;
        }
        match (self.ledger_view, self.ledger_pane) {
            (LedgerView::Account, LedgerPane::Actions) => {
                self.ledger_account_action_selected = offset_wrapped_index(
                    self.ledger_account_action_selected,
                    AccountAction::ALL.len(),
                    offset,
                );
                self.status = self.selected_account_action().label().to_string();
            }
            (LedgerView::Positions, LedgerPane::List) => {
                self.ledger_position_selected = offset_clamped_index(
                    self.ledger_position_selected,
                    self.liquidity_position_rows().len(),
                    offset,
                );
                self.liquidity_preview_result = None;
                self.status = "Manager-liquidity position selected.".to_string();
            }
            (LedgerView::Writers, LedgerPane::List) => {
                self.writers.clear_action_mask_check();
                self.ledger_writer_selected = offset_clamped_index(
                    self.ledger_writer_selected,
                    self.writer_sleeve_rows().len(),
                    offset,
                );
                self.writers.action_result = None;
                self.status = "Collective-writer sleeve selected.".to_string();
            }
            (LedgerView::Writers, LedgerPane::Actions) => {
                self.writers.action_selected = offset_wrapped_index(
                    self.writers.action_selected,
                    WriterAction::ALL.len(),
                    offset,
                );
                let action = self.writers.selected_action();
                let availability = self.writer_action_availability(action);
                self.status = match availability.note() {
                    Some(note) => format!("{}: {} {note}", action.label(), action.detail()),
                    None => format!("{}: {}", action.label(), action.detail()),
                };
            }
            (LedgerView::History, LedgerPane::List) => {
                self.ledger_history_selected = offset_clamped_index(
                    self.ledger_history_selected,
                    self.ledger_history_rows().len(),
                    offset,
                );
                self.status = "Wallet activity row selected.".to_string();
            }
            _ => {}
        }
    }

    pub(super) fn selected_account_action(&self) -> AccountAction {
        AccountAction::ALL
            .get(self.ledger_account_action_selected)
            .copied()
            .unwrap_or(AccountAction::SwitchWallet)
    }

    pub(super) fn writer_sleeve_rows(&self) -> &[Value] {
        let Some(payload) = self.ledger.as_ref() else {
            return &[];
        };
        value_at_key(payload, &["currentWriterSleeves"])
            .and_then(|value| array_at_key(value, &["sleeves", "writerSleeves", "rows"]))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn liquidity_position_rows(&self) -> &[Value] {
        let Some(payload) = self.ledger.as_ref() else {
            return &[];
        };
        value_at_key(payload, &["liquidityPositions"])
            .and_then(|value| array_at_key(value, &["positions"]))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(super) fn ledger_history_rows(&self) -> Vec<&Value> {
        let Some(payload) = self.ledger.as_ref() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if let Some(product) =
            value_at_key(payload, &["productLedger"]).or_else(|| value_at_key(payload, &["ledger"]))
            && let Some(events) = array_at_key(product, &["events", "entries", "activity"])
        {
            rows.extend(events.iter());
        }
        if let Some(chain) = value_at_key(payload, &["chainHistory"])
            && let Some(events) = array_at_key(chain, &["transactions", "entries", "history"])
        {
            rows.extend(events.iter());
        }
        rows
    }

    pub(super) fn selected_writer_sleeve(&self) -> Option<&Value> {
        self.writer_sleeve_rows().get(self.ledger_writer_selected)
    }

    pub(super) fn selected_writer_sleeve_address(&self) -> Option<String> {
        self.selected_writer_sleeve().and_then(|row| {
            string_at_key(row, &["address", "sleeve", "sleeveAddress"])
                .filter(|value| canonical_pubkey(value).is_ok())
        })
    }

    pub(super) fn activate_ledger_control(
        &mut self,
        cli: &Cli,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        match (self.ledger_view, self.ledger_pane) {
            (LedgerView::Account, LedgerPane::Actions) => {
                self.activate_account_action(backend_url, fetch_tx)
            }
            (LedgerView::Writers, LedgerPane::Actions) => {
                self.activate_writer_action(cli, backend_url, fetch_tx)
            }
            (LedgerView::Writers, LedgerPane::List) => {
                self.writers.action_selected = WriterAction::ALL
                    .iter()
                    .position(|action| *action == WriterAction::Show)
                    .unwrap_or(0);
                self.ledger_pane = LedgerPane::Actions;
                self.status = "Sleeve selected. Choose a writer action.".to_string();
            }
            (LedgerView::Positions | LedgerView::History, LedgerPane::List) => {
                self.ledger_pane = LedgerPane::Detail;
                self.status = "Selected row detail focused.".to_string();
            }
            (LedgerView::Positions, LedgerPane::Actions) => {
                self.open_liquidity_preview_form();
            }
            _ => self.request_ledger(backend_url, fetch_tx, true),
        }
    }

    fn activate_account_action(&mut self, _backend_url: &str, _fetch_tx: &Sender<LabFetchResult>) {
        match self.selected_account_action() {
            AccountAction::SwitchWallet => self.begin_wallet_switch(),
        }
    }

    pub(super) fn activate_writer_action(
        &mut self,
        cli: &Cli,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.interaction_is_locked() {
            self.status = "A writer action or availability check is still running.".to_string();
            return;
        }
        let action = self.writers.selected_action();
        self.writers.action_result = None;
        if action == WriterAction::Refresh {
            self.writers.clear_action_mask_check();
            self.request_ledger(backend_url, fetch_tx, true);
            self.request_writer_close_capabilities(backend_url, fetch_tx);
            return;
        }
        let availability = self.writer_action_availability(action);
        if let WriterActionAvailability::Disabled(issue) = &availability {
            self.status = issue.clone();
            return;
        }
        if matches!(action, WriterAction::Show | WriterAction::Policy) {
            let Some(sleeve) = self.selected_writer_sleeve_address() else {
                self.status = "Select a current sleeve first.".to_string();
                return;
            };
            let args = writer_action_args(action, &[("sleeve", sleeve)]);
            self.start_writer_command(cli, backend_url, fetch_tx, action, args);
            return;
        }
        self.writers.clear_action_mask_check();
        let mut form = writer_form_for_action(action, self.selected_writer_sleeve());
        if let Some(owner) = self.wallet.pubkey.as_ref() {
            for field in &mut form.fields {
                if field.key == "owner" {
                    field.value = owner.clone();
                }
            }
        }
        self.writers.form = Some(form);
        self.status = format!(
            "{} form opened. Values are exact atoms unless stated otherwise.",
            action.label()
        );
        if let WriterActionAvailability::HotOnly(warning) = availability {
            self.status.push(' ');
            self.status.push_str(&warning);
        }
    }

    pub(super) fn move_writer_form_field(&mut self, offset: isize) {
        if self.writers.action_mask_is_loading() {
            return;
        }
        let Some(form) = self.writers.form.as_mut() else {
            return;
        };
        if form.fields.is_empty() || offset == 0 {
            return;
        }
        form.selected_field =
            (form.selected_field as isize + offset).rem_euclid(form.fields.len() as isize) as usize;
        if let Some(field) = form.fields.get(form.selected_field) {
            self.status = format!("Editing {}", field.label);
        }
    }

    pub(super) fn cancel_writer_form(&mut self) {
        self.writers.clear_action_mask_check();
        self.writers.form = None;
        self.status = "Writer form cancelled. Nothing was signed or sent.".to_string();
    }

    pub(super) fn review_or_run_writer_form(
        &mut self,
        cli: &Cli,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        if self.writers.action_mask_is_loading() {
            self.status = "Wallet-specific availability is still being checked.".to_string();
            return;
        }
        let Some(form) = self.writers.form.clone() else {
            return;
        };
        let availability = self.writer_action_availability(form.action);
        if let WriterActionAvailability::Disabled(issue) = &availability {
            self.status = issue.clone();
            return;
        }
        if let Err(error) = validate_writer_form(&form) {
            self.status = error;
            return;
        }
        let values = form
            .fields
            .iter()
            .map(|field| (field.key, field.value.trim().to_string()))
            .collect::<Vec<_>>();
        let args = writer_action_args(form.action, &values);
        if form.action.signs_and_submits() {
            let Some(owner) = self.wallet.pubkey.clone() else {
                self.status =
                    "Attach a wallet before opening final writer review. Nothing was signed or sent."
                        .to_string();
                return;
            };
            let sleeve = match writer_review_sleeve_binding(self.selected_writer_sleeve(), &form) {
                Ok(sleeve) => sleeve,
                Err(issue) => {
                    self.status = format!("{issue} Nothing was signed or sent.");
                    return;
                }
            };
            let confirmation = WriterActionConfirmation {
                action: form.action,
                args,
                summary: writer_confirmation_summary(
                    &form,
                    self.wallet.account_label(),
                    &self.onchain_config.network,
                    &availability,
                ),
                choice: UserActionConfirmationChoice::Cancel,
            };
            self.writers.action_mask_request = self.writers.action_mask_request.wrapping_add(1);
            self.writers.action_mask_inflight = Some(self.writers.action_mask_request);
            self.writers.pending_review = Some(PendingWriterReview {
                owner: owner.clone(),
                sleeve: sleeve.clone(),
                form,
                confirmation,
            });
            self.writers.confirmation = None;
            self.status = "Checking exact wallet-and-sleeve availability before opening review..."
                .to_string();
            spawn_writer_action_mask_fetch(
                backend_url.to_string(),
                fetch_tx.clone(),
                self.writers.action_mask_request,
                self.writers
                    .pending_review
                    .as_ref()
                    .expect("pending review was just stored")
                    .confirmation
                    .action,
                owner,
                sleeve,
            );
        } else {
            let action = form.action;
            self.writers.form = None;
            self.start_writer_command(cli, backend_url, fetch_tx, action, args);
        }
    }

    pub(super) fn cancel_writer_confirmation(&mut self) {
        if self.writers.action_is_running() {
            self.status = "The writer transaction is already running.".to_string();
            return;
        }
        self.writers.confirmation = None;
        self.status = "Writer review cancelled. Nothing was signed or sent.".to_string();
    }

    pub(super) fn activate_writer_confirmation(
        &mut self,
        cli: &Cli,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
    ) {
        let Some(confirmation) = self.writers.confirmation.clone() else {
            return;
        };
        if confirmation.choice == UserActionConfirmationChoice::Cancel {
            self.cancel_writer_confirmation();
            return;
        }
        if let Err(error) = crate::current_release::require_current_write_release() {
            self.writers.confirmation = None;
            self.status = error.to_string();
            return;
        }
        let same_form = self.writers.form.as_ref().is_some_and(|form| {
            let values = form
                .fields
                .iter()
                .map(|field| (field.key, field.value.trim().to_string()))
                .collect::<Vec<_>>();
            form.action == confirmation.action
                && writer_action_args(form.action, &values) == confirmation.args
        });
        if !same_form
            || !self
                .writer_action_availability(confirmation.action)
                .is_actionable()
        {
            self.writers.confirmation = None;
            self.status = "Writer review changed or is unavailable. Review the current form again. Nothing was signed or sent.".to_string();
            return;
        }
        self.writers.form = None;
        self.start_writer_command(
            cli,
            backend_url,
            fetch_tx,
            confirmation.action,
            confirmation.args,
        );
    }

    fn start_writer_command(
        &mut self,
        cli: &Cli,
        backend_url: &str,
        fetch_tx: &Sender<LabFetchResult>,
        action: WriterAction,
        command_args: Vec<String>,
    ) {
        if self.writers.interaction_is_locked() {
            self.status = "A writer action or availability check is already running.".to_string();
            return;
        }
        if let WriterActionAvailability::Disabled(issue) = self.writer_action_availability(action) {
            self.status = issue;
            return;
        }
        let mut args = vec!["--json".to_string(), "--quiet".to_string()];
        let mut envs = vec![
            ("AMEBA_BACKEND_URL".to_string(), backend_url.to_string()),
            (
                "AMEBA_CLUSTER".to_string(),
                self.onchain_config.network.clone(),
            ),
        ];
        if let Some(solana_config) = cli.solana_config.as_deref() {
            envs.push(("SOLANA_CONFIG".to_string(), solana_config.to_string()));
        }
        if let Some(commitment) = self.onchain_config.commitment.as_deref() {
            envs.push(("SOLANA_COMMITMENT".to_string(), commitment.to_string()));
        }
        if let Some(keypair) = self.onchain_config.keypair_path.as_deref() {
            envs.push(("SOLANA_KEYPAIR".to_string(), keypair.to_string()));
        }
        if self.onchain_config.allow_insecure_keypair || cli.allow_insecure_keypair {
            envs.push((
                "AMEBA_ALLOW_INSECURE_KEYPAIR".to_string(),
                "true".to_string(),
            ));
        }
        if action.signs_and_submits() {
            args.push("--yes".to_string());
        }
        args.extend(command_args);
        self.writers.action_request = self.writers.action_request.wrapping_add(1);
        self.writers.action_inflight = Some(self.writers.action_request);
        self.writers.confirmation = None;
        self.status = if action.signs_and_submits() {
            format!("{}: verifying, signing, and submitting...", action.label())
        } else {
            format!("Loading {}...", action.label().to_ascii_lowercase())
        };
        spawn_writer_command(
            args,
            envs,
            fetch_tx.clone(),
            self.writers.action_request,
            action,
        );
    }
}

fn writer_review_sleeve_binding(
    selected_sleeve: Option<&Value>,
    form: &WriterActionForm,
) -> Result<String, String> {
    if form.action == WriterAction::Refund {
        let sleeve = form
            .field("sleeve")
            .ok_or("Enter the sleeve from the historical refund row.")?;
        canonical_pubkey(sleeve)
            .map_err(|_| "The historical refund sleeve is invalid.".to_string())?;
        return Ok(sleeve.to_string());
    }
    let selected_sleeve = selected_sleeve
        .ok_or_else(|| "Select the current writer sleeve again before review.".to_string())?;
    let sleeve = string_at_key(selected_sleeve, &["address"])
        .filter(|value| canonical_pubkey(value).is_ok())
        .ok_or_else(|| {
            "The selected writer sleeve has no current canonical address.".to_string()
        })?;
    match form.action {
        WriterAction::Deposit
        | WriterAction::Withdraw
        | WriterAction::Refund
        | WriterAction::LiquidityInitialize
        | WriterAction::LiquidityAdd
        | WriterAction::LiquidityRemove
        | WriterAction::LiquiditySweep
        | WriterAction::BeginClose
        | WriterAction::ClaimLong
        | WriterAction::ClaimFlat
        | WriterAction::TransferFlat => {
            if form.field("sleeve") != Some(sleeve.as_str()) {
                return Err(
                    "The form sleeve no longer matches the exact selected writer sleeve."
                        .to_string(),
                );
            }
        }
        WriterAction::Bid => {
            let active_auction = string_at_key(selected_sleeve, &["activeAuction"])
                .filter(|value| canonical_pubkey(value).is_ok())
                .ok_or_else(|| {
                    "The selected sleeve has no current active auction for this bid.".to_string()
                })?;
            if form.field("auction") != Some(active_auction.as_str()) {
                return Err(
                    "The bid auction no longer matches the selected sleeve's exact active auction."
                        .to_string(),
                );
            }
        }
        WriterAction::AdvanceClose | WriterAction::CancelClose => {
            let active_request = string_at_key(selected_sleeve, &["activeCloseRequest"])
                .filter(|value| canonical_pubkey(value).is_ok())
                .ok_or_else(|| {
                    "The selected sleeve has no current active close request.".to_string()
                })?;
            if form.field("close-request") != Some(active_request.as_str()) {
                return Err(
                    "The close request no longer matches the selected sleeve's exact active request."
                        .to_string(),
                );
            }
        }
        WriterAction::Show
        | WriterAction::Liquidity
        | WriterAction::Refunds
        | WriterAction::Policy
        | WriterAction::ClosePreview
        | WriterAction::CloseStatus
        | WriterAction::Refresh => {
            return Err("This writer action does not require a signed review.".to_string());
        }
    }
    Ok(sleeve)
}

pub(super) fn require_writer_review_action(
    mask: &WriterActionMask,
    action: WriterAction,
) -> Result<CollectiveActionKind, String> {
    let kind = match action {
        WriterAction::Deposit => CollectiveActionKind::WriterDeposit,
        WriterAction::Withdraw => CollectiveActionKind::WriterWithdraw,
        WriterAction::Refund => CollectiveActionKind::AuctionRefund,
        WriterAction::LiquidityInitialize => CollectiveActionKind::WriterLiquidityInitialize,
        WriterAction::LiquidityAdd => CollectiveActionKind::WriterLiquidityAdd,
        WriterAction::LiquidityRemove => CollectiveActionKind::WriterLiquidityRemove,
        WriterAction::LiquiditySweep => CollectiveActionKind::WriterLiquiditySweep,
        WriterAction::Bid => CollectiveActionKind::AuctionBid,
        WriterAction::BeginClose => CollectiveActionKind::WriterCloseBegin,
        WriterAction::ClaimFlat => CollectiveActionKind::ClaimFlat,
        WriterAction::TransferFlat => CollectiveActionKind::FlatTransfer,
        WriterAction::AdvanceClose => {
            let continuation = mask.require_enabled(CollectiveActionKind::WriterCloseContinue);
            let finalization = mask.require_enabled(CollectiveActionKind::WriterCloseFinalize);
            return match (continuation, finalization) {
                (Ok(()), Err(_)) => Ok(CollectiveActionKind::WriterCloseContinue),
                (Err(_), Ok(())) => Ok(CollectiveActionKind::WriterCloseFinalize),
                (Ok(()), Ok(())) => Err(
                    "the current mask ambiguously enables both close continuation and finalization"
                        .to_string(),
                ),
                (Err(continuation), Err(finalization)) => Err(format!(
                    "neither close continuation nor finalization is currently permitted ({continuation}; {finalization})"
                )),
            };
        }
        WriterAction::ClaimLong => CollectiveActionKind::ClaimLong,
        WriterAction::CancelClose => {
            return Err(
                crate::writer_output::WRITER_CLOSE_CANCEL_ACTION_MASK_UNAVAILABLE.to_string(),
            );
        }
        WriterAction::Show
        | WriterAction::Liquidity
        | WriterAction::Refunds
        | WriterAction::Policy
        | WriterAction::ClosePreview
        | WriterAction::CloseStatus
        | WriterAction::Refresh => {
            return Err(
                "this read-only writer action does not require wallet authorization".to_string(),
            );
        }
    };
    mask.require_enabled(kind)?;
    Ok(kind)
}

fn route_availability(
    projection: &WriterCloseCapabilityProjection,
    route: (&str, &str),
    unavailable: &str,
) -> WriterActionAvailability {
    if projection.has_route(route.0, route.1) {
        WriterActionAvailability::Enabled
    } else {
        WriterActionAvailability::Disabled(unavailable.to_string())
    }
}

fn signed_close_availability(
    projection: &WriterCloseCapabilityProjection,
    operations: &[&str],
    routes: &[(&str, &str)],
    requires_hot_custody: bool,
) -> WriterActionAvailability {
    if !projection.implementation_supported
        || !projection.runtime_enabled
        || operations
            .iter()
            .any(|operation| !projection.has_operation(operation))
        || routes
            .iter()
            .any(|(method, path)| !projection.has_route(method, path))
    {
        return WriterActionAvailability::Disabled(
            "The current release does not permit this signed close stage. Nothing will be signed or sent."
                .to_string(),
        );
    }
    if !requires_hot_custody {
        return WriterActionAvailability::Enabled;
    }
    if !projection.hot_runtime_enabled {
        return WriterActionAvailability::Disabled(
            "Hot custody is unavailable for this signed close stage. Nothing will be signed or sent."
                .to_string(),
        );
    }
    if projection.cold_runtime_enabled {
        WriterActionAvailability::Enabled
    } else {
        WriterActionAvailability::HotOnly(
            "Hot custody is available; cold Light setup is unavailable. Finalized permission is checked before submission."
                .to_string(),
        )
    }
}

fn projection_to_stored_value(projection: &WriterCloseCapabilityProjection) -> Value {
    serde_json::json!({
        "state": "ready",
        "implementationSupported": projection.implementation_supported,
        "runtimeEnabled": projection.runtime_enabled,
        "hotRuntimeEnabled": projection.hot_runtime_enabled,
        "coldRuntimeEnabled": projection.cold_runtime_enabled,
        "operations": projection.operations,
        "routes": projection.routes.iter().map(|route| serde_json::json!({
            "method": route.method,
            "path": route.path,
        })).collect::<Vec<_>>(),
    })
}

fn projection_from_stored_value(value: &Value) -> Option<WriterCloseCapabilityProjection> {
    let implementation_supported = value.get("implementationSupported")?.as_bool()?;
    let runtime_enabled = value.get("runtimeEnabled")?.as_bool()?;
    let hot_runtime_enabled = value.get("hotRuntimeEnabled")?.as_bool()?;
    let cold_runtime_enabled = value.get("coldRuntimeEnabled")?.as_bool()?;
    let operations = exact_string_list(value.get("operations"), "stored operations", 16).ok()?;
    let route_values = value.get("routes")?.as_array()?;
    if route_values.len() > 32 {
        return None;
    }
    let mut routes = Vec::with_capacity(route_values.len());
    let mut seen_routes = HashSet::new();
    for route in route_values {
        let method = route.get("method")?.as_str()?;
        let path = route.get("path")?.as_str()?;
        if !matches!(method, "GET" | "POST") || !path.starts_with('/') || path.len() > 256 {
            return None;
        }
        if !seen_routes.insert((method.to_string(), path.to_string())) {
            return None;
        }
        routes.push(WriterCloseCapabilityRoute {
            method: method.to_string(),
            path: path.to_string(),
        });
    }
    Some(WriterCloseCapabilityProjection {
        implementation_supported,
        runtime_enabled,
        hot_runtime_enabled,
        cold_runtime_enabled,
        operations,
        routes,
    })
}

pub(super) fn writer_form_for_action(
    action: WriterAction,
    sleeve: Option<&Value>,
) -> WriterActionForm {
    let sleeve_address = sleeve
        .and_then(|row| string_at_key(row, &["address", "sleeve", "sleeveAddress"]))
        .unwrap_or_default();
    let auction_address = sleeve
        .and_then(|row| {
            string_at_key(
                row,
                &[
                    "activeAuction",
                    "auction",
                    "auctionAddress",
                    "currentAuction",
                ],
            )
            .or_else(|| {
                value_at_key(row, &["auction", "currentAuction"])
                    .and_then(|auction| string_at_key(auction, &["address"]))
            })
        })
        .unwrap_or_default();
    let close_request = sleeve
        .and_then(writer_close_request_address)
        .unwrap_or_default();
    let fields = match action {
        WriterAction::Deposit | WriterAction::Withdraw => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("amount", "Principal atoms", ""),
        ],
        WriterAction::Refunds => vec![
            ActionFormField::new("owner", "Refund owner", ""),
            ActionFormField {
                key: "cursor",
                label: "Next cursor (optional)",
                value: String::new(),
                required: false,
                secret: false,
            },
            ActionFormField::new("limit", "Rows (1-32)", "16"),
        ],
        WriterAction::Refund => vec![
            ActionFormField::new("sleeve", "Sleeve from refund row", ""),
            ActionFormField::new("auction", "Historical auction", ""),
            ActionFormField::new("bid", "Historical bid", ""),
        ],
        WriterAction::Liquidity => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("owner", "Wallet actor", ""),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
        ],
        WriterAction::LiquidityInitialize | WriterAction::LiquiditySweep => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
        ],
        WriterAction::LiquidityAdd => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
            ActionFormField::new("issue-amount", "Issue contract atoms", "0"),
            ActionFormField::new("bins", "Bins ID:option:quote (comma separated)", ""),
        ],
        WriterAction::LiquidityRemove => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
            ActionFormField::new("bins", "Bins ID:option:quote (comma separated)", ""),
        ],
        WriterAction::Bid => vec![
            ActionFormField::new("auction", "Auction", auction_address),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
            ActionFormField::new("price", "Price per contract atoms", ""),
            ActionFormField::new("amount", "Contract atoms", ""),
        ],
        WriterAction::ClosePreview | WriterAction::BeginClose => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("amount", "Flat atoms", ""),
            ActionFormField::new("minimum-withdrawal", "Minimum USDC atoms", "0"),
        ],
        WriterAction::CloseStatus | WriterAction::AdvanceClose | WriterAction::CancelClose => {
            vec![ActionFormField::new(
                "close-request",
                "Close request",
                close_request,
            )]
        }
        WriterAction::ClaimLong => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("series-index", "Series index (0-19)", ""),
            ActionFormField::new("amount", "Claim token atoms", ""),
        ],
        WriterAction::ClaimFlat => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("amount", "Flat claim atoms", ""),
        ],
        WriterAction::TransferFlat => vec![
            ActionFormField::new("sleeve", "Sleeve", sleeve_address),
            ActionFormField::new("destination", "Destination wallet", ""),
            ActionFormField::new("amount", "Flat atoms", ""),
        ],
        WriterAction::Show | WriterAction::Policy | WriterAction::Refresh => Vec::new(),
    };
    WriterActionForm {
        action,
        fields,
        selected_field: 0,
    }
}

pub(super) fn writer_close_request_address(sleeve: &Value) -> Option<String> {
    string_at_key(sleeve, &["closeRequest", "activeCloseRequest"]).or_else(|| {
        value_at_key(sleeve, &["closeRequest", "activeClose"])
            .and_then(|request| string_at_key(request, &["address"]))
    })
}

pub(super) fn validate_writer_form(form: &WriterActionForm) -> Result<(), String> {
    for field in &form.fields {
        if field.required && field.value.trim().is_empty() {
            return Err(format!("{} is required.", field.label));
        }
    }
    for key in [
        "sleeve",
        "auction",
        "bid",
        "owner",
        "close-request",
        "destination",
    ] {
        if let Some(value) = form.field(key) {
            canonical_pubkey(value)
                .map_err(|_| format!("{} is not a valid address.", field_label(form, key)))?;
        }
    }
    if let Some(value) = form.field("series-index") {
        let index = value
            .parse::<u8>()
            .map_err(|_| "Series index must be a whole number from 0 through 19.".to_string())?;
        if index > 19 || index.to_string() != value {
            return Err(
                "Series index must be a canonical whole number from 0 through 19.".to_string(),
            );
        }
    }
    for key in ["amount", "price"] {
        if let Some(value) = form.field(key) {
            validate_unsigned_atoms(field_label(form, key), value, false)?;
        }
    }
    if let Some(value) = form.field("minimum-withdrawal") {
        validate_unsigned_atoms(field_label(form, "minimum-withdrawal"), value, true)?;
    }
    if let Some(value) = form.field("issue-amount") {
        validate_unsigned_atoms("Issue contract atoms", value, true)?;
    }
    if let Some(value) = form.field("bins") {
        crate::writer_liquidity::parse_bins(
            &value.split(',').map(str::to_string).collect::<Vec<_>>(),
            form.action == WriterAction::LiquidityAdd,
        )?;
    }
    if let Some(value) = form.field("cursor") {
        if !value.is_empty() && !crate::writer_liquidity::valid_cursor(value) {
            return Err(
                "Refund cursor must be the exact value from discovery; omit it to restart.".into(),
            );
        }
    }
    if let Some(value) = form.field("limit") {
        if !value
            .parse::<u8>()
            .ok()
            .is_some_and(|limit| (1..=32).contains(&limit) && limit.to_string() == value)
        {
            return Err("Refund rows must be a canonical integer from 1 through 32.".into());
        }
    }
    Ok(())
}

fn field_label<'a>(form: &'a WriterActionForm, key: &'a str) -> &'a str {
    form.fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.label)
        .unwrap_or(key)
}

fn canonical_pubkey(value: &str) -> Result<Pubkey, ()> {
    let parsed = Pubkey::from_str(value).map_err(|_| ())?;
    (parsed.to_string() == value).then_some(parsed).ok_or(())
}

fn validate_unsigned_atoms(label: &str, value: &str, allow_zero: bool) -> Result<(), String> {
    if value.is_empty()
        || (value != "0" && value.starts_with('0'))
        || !value.as_bytes().iter().all(u8::is_ascii_digit)
    {
        return Err(format!(
            "{label} must be a canonical whole-number atom amount."
        ));
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} is outside the supported amount range."))?;
    if !allow_zero && parsed == 0 {
        return Err(format!("{label} must be greater than zero."));
    }
    Ok(())
}

fn writer_action_args(action: WriterAction, values: &[(impl AsRef<str>, String)]) -> Vec<String> {
    let value = |key: &str| {
        values
            .iter()
            .find(|(candidate, _)| candidate.as_ref() == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    let mut args = vec!["writers".to_string()];
    match action {
        WriterAction::Show => {
            args.extend(["show".to_string(), "--sleeve".to_string(), value("sleeve")])
        }
        WriterAction::Policy => args.extend([
            "policy-audit".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
        ]),
        WriterAction::Deposit | WriterAction::Withdraw => args.extend([
            if action == WriterAction::Deposit {
                "deposit"
            } else {
                "withdraw"
            }
            .to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--amount".to_string(),
            value("amount"),
        ]),
        WriterAction::Refunds => {
            args.extend([
                "refunds".to_string(),
                "--owner".to_string(),
                value("owner"),
                "--limit".to_string(),
                value("limit"),
            ]);
            if !value("cursor").is_empty() {
                args.extend(["--cursor".to_string(), value("cursor")]);
            }
        }
        WriterAction::Refund => args.extend([
            "refund".to_string(),
            "--auction".to_string(),
            value("auction"),
            "--bid".to_string(),
            value("bid"),
        ]),
        WriterAction::Liquidity
        | WriterAction::LiquidityInitialize
        | WriterAction::LiquidityAdd
        | WriterAction::LiquidityRemove
        | WriterAction::LiquiditySweep => {
            let command = match action {
                WriterAction::Liquidity => "liquidity",
                WriterAction::LiquidityInitialize => "liquidity-initialize",
                WriterAction::LiquidityAdd => "liquidity-add",
                WriterAction::LiquidityRemove => "liquidity-remove",
                _ => "liquidity-sweep",
            };
            args.extend([
                command.to_string(),
                "--sleeve".to_string(),
                value("sleeve"),
                "--series-index".to_string(),
                value("series-index"),
            ]);
            if action == WriterAction::Liquidity {
                args.extend(["--owner".to_string(), value("owner")]);
            }
            if action == WriterAction::LiquidityAdd {
                args.extend(["--issue-amount".to_string(), value("issue-amount")]);
            }
            if matches!(
                action,
                WriterAction::LiquidityAdd | WriterAction::LiquidityRemove
            ) {
                for bin in value("bins").split(',') {
                    args.extend(["--bin".to_string(), bin.to_string()]);
                }
            }
        }
        WriterAction::Bid => args.extend([
            "bid".to_string(),
            "--auction".to_string(),
            value("auction"),
            "--series-index".to_string(),
            value("series-index"),
            "--price".to_string(),
            value("price"),
            "--amount".to_string(),
            value("amount"),
        ]),
        WriterAction::ClosePreview => args.extend([
            "close-preview".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--amount".to_string(),
            value("amount"),
            "--minimum-withdrawal".to_string(),
            value("minimum-withdrawal"),
        ]),
        WriterAction::BeginClose => args.extend([
            "close".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--amount".to_string(),
            value("amount"),
            "--minimum-withdrawal".to_string(),
            value("minimum-withdrawal"),
        ]),
        WriterAction::CloseStatus => args.extend([
            "close-status".to_string(),
            "--close-request".to_string(),
            value("close-request"),
        ]),
        WriterAction::AdvanceClose => args.extend([
            "close".to_string(),
            "--close-request".to_string(),
            value("close-request"),
        ]),
        WriterAction::CancelClose => args.extend([
            "close".to_string(),
            "--close-request".to_string(),
            value("close-request"),
            "--cancel".to_string(),
        ]),
        WriterAction::ClaimLong => args.extend([
            "claim".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--variant".to_string(),
            "collective-long".to_string(),
            "--series-index".to_string(),
            value("series-index"),
            "--amount".to_string(),
            value("amount"),
        ]),
        WriterAction::ClaimFlat => args.extend([
            "claim".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--variant".to_string(),
            "flat-residual".to_string(),
            "--amount".to_string(),
            value("amount"),
        ]),
        WriterAction::TransferFlat => args.extend([
            "transfer-flat".to_string(),
            "--sleeve".to_string(),
            value("sleeve"),
            "--destination".to_string(),
            value("destination"),
            "--amount".to_string(),
            value("amount"),
        ]),
        WriterAction::Refresh => args.push("list".to_string()),
    }
    args
}

fn writer_confirmation_summary(
    form: &WriterActionForm,
    signer_wallet: &str,
    network: &str,
    availability: &WriterActionAvailability,
) -> Vec<String> {
    let mut lines = vec![
            form.action.detail().to_string(),
            format!("Signer wallet: {signer_wallet}"),
            format!("Network: {network}"),
            "This review requires finalized V3 permission and initialized business state; unavailable state prevents signing or submission."
                .to_string(),
        ];
    if let WriterActionAvailability::HotOnly(warning) = availability {
        lines.push(format!("CUSTODY WARNING: {warning}"));
    }
    for field in &form.fields {
        if !field.secret {
            let value = field.value.trim().to_string();
            lines.push(format!("{}: {value}", field.label));
        }
    }
    match form.action {
        WriterAction::BeginClose => lines.push(
            "This starts only the close request. Run Close status before advancing or cancelling."
                .to_string(),
        ),
        WriterAction::AdvanceClose => lines.push(
            "Lean selects exactly one current basket-deposit or finalization stage. Forward progress is available only to the request owner; run Close status again afterward."
                .to_string(),
        ),
        WriterAction::CancelClose => lines.push(
            "This release cannot open or sign a cancellation review because collective-operations-v1 has no close-cancel action kind; it never aliases cancellation to continuation or finalization."
                .to_string(),
        ),
        _ => {}
    }
    lines
}

fn offset_wrapped_index(current: usize, len: usize, offset: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + offset).rem_euclid(len as isize) as usize
}

fn offset_clamped_index(current: usize, len: usize, offset: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + offset).clamp(0, len.saturating_sub(1) as isize) as usize
}
