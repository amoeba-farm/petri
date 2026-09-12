//! Settlement reads and presentation data for the Lab.
//!
//! The three public settlement endpoints are deliberately kept independent:
//! an unavailable record must not hide readiness or oracle evidence, and vice
//! versa. Every successful payload has passed the current backend identity gate
//! and, where the response carries identity fields, exact market/expiry binding.

use serde_json::Value;

use crate::{
    backend::{BackendClient, CliError, terminal_safe_text},
    chain_identity, endpoints,
};

const ISSUE_DISPLAY_CHARS: usize = 240;
const LINE_DISPLAY_CHARS: usize = 640;
const SELECTOR_DISPLAY_CHARS: usize = 120;
const LIST_ITEM_DISPLAY_CHARS: usize = 120;
const MAX_LIST_ITEMS: usize = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettlementEndpointKind {
    Record,
    Readiness,
    Oracle,
}

impl SettlementEndpointKind {
    fn data_key(self) -> &'static str {
        match self {
            Self::Record => "settlement",
            Self::Readiness => "preflight",
            Self::Oracle => "oracle",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Record => "Settlement record",
            Self::Readiness => "Settlement readiness",
            Self::Oracle => "Settlement oracle evidence",
        }
    }

    fn requires_payload_identity(self) -> bool {
        matches!(self, Self::Record | Self::Oracle)
    }
}

/// One independently fetched and validated settlement endpoint.
///
/// `Available` contains the validated current-envelope `data` object, including
/// its `protocol` and `freshness` evidence. Callers must not render this raw JSON
/// directly; use the line helpers below or a similarly bounded projection.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SettlementEndpointData {
    Available(Value),
    Unavailable { issue: String },
}

impl SettlementEndpointData {
    pub(super) fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }
}

/// Exact market/month settlement state assembled from three separately scoped
/// reads. The requested IDs are retained so stale jobs can be rejected before
/// this bundle enters a TUI cache.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SettlementBundle {
    pub(super) market_id: String,
    pub(super) expiry_id: String,
    pub(super) record: SettlementEndpointData,
    pub(super) readiness: SettlementEndpointData,
    pub(super) oracle: SettlementEndpointData,
}

impl SettlementBundle {
    pub(super) fn cache_key(&self) -> (&str, &str) {
        (&self.market_id, &self.expiry_id)
    }

    pub(super) fn available_endpoint_count(&self) -> usize {
        [&self.record, &self.readiness, &self.oracle]
            .into_iter()
            .filter(|endpoint| endpoint.is_available())
            .count()
    }

    pub(super) fn issue_count(&self) -> usize {
        3usize.saturating_sub(self.available_endpoint_count())
    }
}

/// Builds a client and fetches all three settlement reads. A client setup
/// failure is represented independently on every endpoint so callers always
/// receive a cacheable, exact-key bundle.
pub(super) fn load_settlement_bundle(
    backend_url: &str,
    market_id: &str,
    expiry_id: &str,
) -> SettlementBundle {
    match BackendClient::new(backend_url.to_string()) {
        Ok(backend) => fetch_settlement_bundle(&backend, market_id, expiry_id),
        Err(error) => unavailable_bundle(market_id, expiry_id, &error.to_string()),
    }
}

/// Fetches all three reads through an existing backend client.
pub(super) fn fetch_settlement_bundle(
    backend: &BackendClient,
    market_id: &str,
    expiry_id: &str,
) -> SettlementBundle {
    fetch_settlement_bundle_with(market_id, expiry_id, |path| backend.get(path))
}

fn fetch_settlement_bundle_with<F>(market_id: &str, expiry_id: &str, mut get: F) -> SettlementBundle
where
    F: FnMut(&str) -> Result<Value, CliError>,
{
    if let Err(error) = validate_exact_selector(market_id, "market")
        .and_then(|_| validate_exact_selector(expiry_id, "expiry"))
    {
        return unavailable_bundle(market_id, expiry_id, &error.to_string());
    }

    let record = fetch_endpoint(
        SettlementEndpointKind::Record,
        &endpoints::dlmm_settlement(market_id, expiry_id),
        market_id,
        expiry_id,
        &mut get,
    );
    let readiness = fetch_endpoint(
        SettlementEndpointKind::Readiness,
        &endpoints::dlmm_settlement_oracle_preflight(market_id, expiry_id),
        market_id,
        expiry_id,
        &mut get,
    );
    let oracle = fetch_endpoint(
        SettlementEndpointKind::Oracle,
        &endpoints::dlmm_settlement_oracle(market_id, expiry_id),
        market_id,
        expiry_id,
        &mut get,
    );

    SettlementBundle {
        market_id: market_id.to_string(),
        expiry_id: expiry_id.to_string(),
        record,
        readiness,
        oracle,
    }
}

fn fetch_endpoint<F>(
    kind: SettlementEndpointKind,
    path: &str,
    market_id: &str,
    expiry_id: &str,
    get: &mut F,
) -> SettlementEndpointData
where
    F: FnMut(&str) -> Result<Value, CliError>,
{
    let result = get(path).and_then(|envelope| {
        chain_identity::validate_current_backend_envelope(&envelope)?;
        let data = envelope
            .get("data")
            .filter(|value| value.is_object())
            .ok_or_else(|| CliError::new("current settlement response is missing data"))?;
        validate_endpoint_payload(kind, data, market_id, expiry_id)?;
        Ok(data.clone())
    });

    match result {
        Ok(payload) => SettlementEndpointData::Available(payload),
        Err(error) => SettlementEndpointData::Unavailable {
            issue: scoped_issue(kind, market_id, expiry_id, &error.to_string()),
        },
    }
}

fn validate_endpoint_payload(
    kind: SettlementEndpointKind,
    data: &Value,
    market_id: &str,
    expiry_id: &str,
) -> Result<(), CliError> {
    let subject = data
        .get(kind.data_key())
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            CliError::new(format!(
                "current {} response is missing {}",
                kind.label().to_ascii_lowercase(),
                kind.data_key()
            ))
        })?;

    validate_optional_identity(
        subject,
        market_id,
        expiry_id,
        kind.requires_payload_identity(),
    )?;

    if kind == SettlementEndpointKind::Readiness {
        if let Some(selected_month) = subject
            .get("selectedOracleMonth")
            .filter(|value| value.is_object())
        {
            validate_optional_identity(selected_month, market_id, expiry_id, false)?;
        }
    }
    Ok(())
}

fn validate_optional_identity(
    value: &Value,
    market_id: &str,
    expiry_id: &str,
    required: bool,
) -> Result<(), CliError> {
    validate_identity_field(value, "marketId", market_id, required)?;
    validate_identity_field(value, "expiryId", expiry_id, required)
}

fn validate_identity_field(
    value: &Value,
    key: &str,
    expected: &str,
    required: bool,
) -> Result<(), CliError> {
    match value.get(key) {
        Some(Value::String(actual)) if actual == expected => Ok(()),
        Some(Value::String(_)) => Err(CliError::new(format!(
            "current settlement response {key} does not match the selected contract"
        ))),
        Some(_) => Err(CliError::new(format!(
            "current settlement response {key} must be a string"
        ))),
        None if required => Err(CliError::new(format!(
            "current settlement response is missing {key}"
        ))),
        None => Ok(()),
    }
}

fn validate_exact_selector(value: &str, label: &str) -> Result<(), CliError> {
    if value.is_empty()
        || value.trim() != value
        || value
            .chars()
            .any(|character| character.is_control() || is_bidi_control(character))
    {
        return Err(CliError::new(format!(
            "selected settlement {label} identifier is invalid"
        )));
    }
    Ok(())
}

fn unavailable_bundle(market_id: &str, expiry_id: &str, cause: &str) -> SettlementBundle {
    let unavailable = |kind| SettlementEndpointData::Unavailable {
        issue: scoped_issue(kind, market_id, expiry_id, cause),
    };
    SettlementBundle {
        market_id: market_id.to_string(),
        expiry_id: expiry_id.to_string(),
        record: unavailable(SettlementEndpointKind::Record),
        readiness: unavailable(SettlementEndpointKind::Readiness),
        oracle: unavailable(SettlementEndpointKind::Oracle),
    }
}

fn scoped_issue(
    kind: SettlementEndpointKind,
    market_id: &str,
    expiry_id: &str,
    cause: &str,
) -> String {
    bounded_terminal_text(
        &format!(
            "{} is unavailable for {}/{}: {}",
            kind.label(),
            bounded_terminal_text(market_id, SELECTOR_DISPLAY_CHARS),
            bounded_terminal_text(expiry_id, SELECTOR_DISPLAY_CHARS),
            bounded_terminal_text(cause, ISSUE_DISPLAY_CHARS)
        ),
        LINE_DISPLAY_CHARS,
    )
}

/// Concise combined projection suitable for a narrow detail panel.
pub(super) fn settlement_bundle_lines(bundle: &SettlementBundle) -> Vec<String> {
    let mut lines = settlement_record_lines(bundle);
    lines.push(String::new());
    lines.extend(settlement_readiness_lines(bundle));
    lines.push(String::new());
    lines.extend(settlement_oracle_lines(bundle));
    lines
}

pub(super) fn settlement_record_lines(bundle: &SettlementBundle) -> Vec<String> {
    let mut lines = vec![heading("Settlement record", bundle)];
    let Some(data) = available_data(&bundle.record, &mut lines) else {
        return lines;
    };
    let Some(record) = data.get("settlement").filter(|value| value.is_object()) else {
        lines.push(safe_line("Record details are unavailable."));
        return lines;
    };
    let stored_record = record
        .get("settlementRecord")
        .filter(|value| value.is_object())
        .unwrap_or(record);

    lines.push(safe_line(format!(
        "Status: {} | final: {} | publication: {}",
        field_text(record, &["status"]),
        field_bool(record, &["settlementFinal"]),
        field_bool(record, &["publicationAuthorized"]),
    )));
    lines.push(safe_line(format!(
        "Settlement value: {} | atomic: {}",
        field_text_from(&[stored_record, record], &["settlementPriceDisplay"]),
        field_text_from(&[stored_record, record], &["settlementPriceAtomic"]),
    )));
    lines.push(safe_line(format!(
        "Settlement time: {} | claims ready: {}",
        field_text_from(&[stored_record, record], &["settlementUtc", "settledAt"]),
        field_bool(record, &["claimPreparationAvailable"]),
    )));
    lines.push(safe_line(format!(
        "Evidence: {} observation(s) | source: {}",
        array_len_text(stored_record, &["observations"]),
        field_text_from(&[stored_record, record], &["sourceUri"]),
    )));
    append_freshness_line(data, &mut lines);
    lines
}

pub(super) fn settlement_readiness_lines(bundle: &SettlementBundle) -> Vec<String> {
    let mut lines = vec![heading("Settlement readiness", bundle)];
    let Some(data) = available_data(&bundle.readiness, &mut lines) else {
        return lines;
    };
    let Some(readiness) = data.get("preflight").filter(|value| value.is_object()) else {
        lines.push(safe_line("Readiness details are unavailable."));
        return lines;
    };

    lines.push(safe_line(format!(
        "Market: {} | oracle month: {} | settlement record: {}",
        field_bool(readiness, &["marketExists"]),
        field_bool(readiness, &["oracleMonthExists"]),
        field_bool(readiness, &["settlementExists"]),
    )));
    lines.push(safe_line(format!(
        "Final: {} | claim preparation: {} | finalizable: {}",
        field_bool(readiness, &["settlementFinal"]),
        field_bool(readiness, &["canPrepareClaim"]),
        field_bool(readiness, &["finalizable"]),
    )));
    lines.push(safe_line(format!(
        "Package ready: {} | signing ready: {} | submission ready: {}",
        field_bool(readiness, &["readyForPackage"]),
        field_bool(readiness, &["readyForSigning"]),
        field_bool(readiness, &["readyForSubmission"]),
    )));
    lines.push(safe_line(format!(
        "Reason: {}",
        field_text(readiness, &["finalityReason", "statusReason"]),
    )));
    lines.push(safe_line(format!(
        "Blockers: {}",
        list_text(readiness, &["blockers"])
    )));
    lines.push(safe_line(format!(
        "Issues: {}",
        list_text(readiness, &["issues"])
    )));
    append_freshness_line(data, &mut lines);
    lines
}

pub(super) fn settlement_oracle_lines(bundle: &SettlementBundle) -> Vec<String> {
    let mut lines = vec![heading("Settlement oracle evidence", bundle)];
    let Some(data) = available_data(&bundle.oracle, &mut lines) else {
        return lines;
    };
    let Some(oracle) = data.get("oracle").filter(|value| value.is_object()) else {
        lines.push(safe_line("Oracle evidence is unavailable."));
        return lines;
    };

    lines.push(safe_line(format!(
        "Status: {} | finalizable: {} | last slot: {}",
        field_text(oracle, &["status"]),
        field_bool(oracle, &["finalizable"]),
        field_text(oracle, &["lastUpdatedSlot"]),
    )));
    lines.push(safe_line(format!(
        "Settlement time: {} | computation: {}",
        field_text(oracle, &["settlementUtc", "settlementTs"]),
        field_text(oracle, &["computation", "model"]),
    )));
    lines.push(safe_line(format!(
        "Base oracle: {} | index delta: {} bps | settlement: {}",
        field_text(oracle, &["baseOracleAtomic"]),
        field_text(oracle, &["indexDeltaBps"]),
        field_text(oracle, &["settlementPriceDisplay", "settlementPriceAtomic"]),
    )));
    lines.push(safe_line(format!(
        "Evidence: {} observation(s) | source: {}",
        array_len_text(oracle, &["observations"]),
        field_text(oracle, &["sourceUri"]),
    )));
    lines.push(safe_line(format!(
        "Source digest: {}",
        field_text(oracle, &["sourceDigestHex"]),
    )));
    lines.push(safe_line(format!(
        "Signed payload: {}",
        field_text(oracle, &["signedPayloadSha256Hex", "messageSha256Hex"]),
    )));
    append_freshness_line(data, &mut lines);
    lines
}

fn heading(label: &str, bundle: &SettlementBundle) -> String {
    safe_line(format!(
        "{} | {}/{}",
        label,
        bounded_terminal_text(&bundle.market_id, SELECTOR_DISPLAY_CHARS),
        bounded_terminal_text(&bundle.expiry_id, SELECTOR_DISPLAY_CHARS)
    ))
}

fn available_data<'a>(
    endpoint: &'a SettlementEndpointData,
    lines: &mut Vec<String>,
) -> Option<&'a Value> {
    match endpoint {
        SettlementEndpointData::Available(payload) => Some(payload),
        SettlementEndpointData::Unavailable { issue } => {
            lines.push(safe_line(issue));
            None
        }
    }
}

fn append_freshness_line(data: &Value, lines: &mut Vec<String>) {
    let Some(freshness) = data.get("freshness").filter(|value| value.is_object()) else {
        return;
    };
    lines.push(safe_line(format!(
        "Observed: {} | stale: {} | refresh healthy: {}",
        field_text(freshness, &["observedAt"]),
        field_bool(freshness, &["stale"]),
        field_bool(freshness, &["refreshHealthy"]),
    )));
}

fn field_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(value_text))
        .unwrap_or_else(|| "unknown".to_string())
}

fn field_text_from(values: &[&Value], keys: &[&str]) -> String {
    values
        .iter()
        .find_map(|value| {
            keys.iter()
                .find_map(|key| value.get(*key).and_then(value_text))
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn field_bool(value: &Value, keys: &[&str]) -> &'static str {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
        .map(|flag| if flag { "yes" } else { "no" })
        .unwrap_or("unknown")
}

fn array_len_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(|items| items.len().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn list_text(value: &Value, keys: &[&str]) -> String {
    let Some(items) = keys
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
    else {
        return "unknown".to_string();
    };
    if items.is_empty() {
        return "none reported".to_string();
    }
    let mut rendered = items
        .iter()
        .take(MAX_LIST_ITEMS)
        .filter_map(value_text)
        .map(|item| bounded_terminal_text(&item, LIST_ITEM_DISPLAY_CHARS))
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        return "details unavailable".to_string();
    }
    if items.len() > MAX_LIST_ITEMS {
        rendered.push(format!("+{} more", items.len() - MAX_LIST_ITEMS));
    }
    rendered.join("; ")
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.trim().is_empty() => {
            Some(bounded_terminal_text(text, LINE_DISPLAY_CHARS))
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn safe_line(value: impl AsRef<str>) -> String {
    bounded_terminal_text(value.as_ref(), LINE_DISPLAY_CHARS)
}

fn bounded_terminal_text(raw: &str, limit: usize) -> String {
    terminal_safe_text(raw).chars().take(limit).collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}
