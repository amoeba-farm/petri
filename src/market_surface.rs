//! Shared market payload loading, parsing, and non-interactive output.
//!
//! This module owns the browser-parity market read models used by both the CLI
//! and TUI. It performs no price discovery of its own: payloads come directly
//! from the configured Amoeba API, and renderers preserve backend unavailable
//! states instead of inferring executable data.

use chrono::DateTime;
use serde_json::{Value, json};

use crate::{
    backend::{
        BackendClient, CliError, array_at_key, string_at_key, terminal_safe_text, value_at_key,
    },
    chain_identity,
    cli::Cli,
    endpoints,
};

const MAX_MARKET_DISPLAY_CHARS: usize = 256;
const MAX_MARKET_ISSUE_CHARS: usize = 512;
const CURRENT_PRICE_ATOMIC_SCALE: u64 = 1_000_000;
const CURRENT_PRICE_ATOMIC_DECIMALS: u8 = 6;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OptionKind {
    Call,
    Put,
}

impl OptionKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Call => "CALL",
            Self::Put => "PUT",
        }
    }

    pub(crate) fn from_current_option_kind(raw: &str) -> Option<Self> {
        match raw {
            "call_spread" => Some(Self::Call),
            "put_spread" => Some(Self::Put),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CurrentSeriesIdentity {
    pub(crate) option_kind: OptionKind,
    pub(crate) sequence: u8,
}

pub(crate) fn parse_current_series_id(
    market_id: &str,
    raw: &str,
) -> Result<CurrentSeriesIdentity, CliError> {
    let market_id = market_id.trim();
    let raw_input = raw;
    let raw = raw_input.trim();
    let expected_product = market_id.to_ascii_uppercase();
    let parts = raw.split('-').collect::<Vec<_>>();
    if raw.is_empty()
        || raw_input != raw
        || parts.len() != 4
        || parts[0] != expected_product
        || parts[1].len() != 6
        || !parts[1].bytes().all(|byte| byte.is_ascii_digit())
        || parts[2] != "CALL" && parts[2] != "PUT"
        || parts[3].len() != 2
        || !parts[3].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CliError::new(format!(
            "expiryId must be the full current series label {expected_product}-YYYYMM-CALL|PUT-NN"
        )));
    }
    let month = parts[1][4..6]
        .parse::<u8>()
        .map_err(|_| CliError::new("expiryId maturity must use a numeric YYYYMM month"))?;
    if !(1..=12).contains(&month) {
        return Err(CliError::new(
            "expiryId maturity month must be between 01 and 12",
        ));
    }
    let sequence = parts[3]
        .parse::<u8>()
        .map_err(|_| CliError::new("expiryId series sequence must be a two-digit number"))?;
    let option_kind = match parts[2] {
        "CALL" => OptionKind::Call,
        "PUT" => OptionKind::Put,
        _ => unreachable!("validated current series side"),
    };
    Ok(CurrentSeriesIdentity {
        option_kind,
        sequence,
    })
}

fn current_series_shape(raw: &str) -> bool {
    let raw_input = raw;
    let raw = raw_input.trim();
    let parts = raw.split('-').collect::<Vec<_>>();
    raw_input == raw
        && !raw.is_empty()
        && parts.len() == 4
        && !parts[0].is_empty()
        && parts[0]
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && parts[1].len() == 6
        && parts[1].bytes().all(|byte| byte.is_ascii_digit())
        && matches!(parts[2], "CALL" | "PUT")
        && parts[3].len() == 2
        && parts[3].bytes().all(|byte| byte.is_ascii_digit())
        && parts[1][4..6]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
}

#[derive(Clone, Debug)]
pub(crate) struct DishSummary {
    pub(crate) id: String,
    pub(crate) symbol: String,
    pub(crate) title: String,
    pub(crate) expiry_count: String,
    pub(crate) series_labels: Vec<String>,
    pub(crate) source: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DishDetail {
    pub(crate) id: String,
    pub(crate) symbol: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) current_print: String,
    pub(crate) base: String,
    pub(crate) expiry_id: String,
    pub(crate) expiry_label: String,
    pub(crate) settlement: String,
    pub(crate) days: String,
    pub(crate) cap_width: String,
    pub(crate) listed_notional: String,
    pub(crate) rows: String,
    pub(crate) freshness: String,
    pub(crate) execution: String,
    pub(crate) issues: Vec<String>,
    pub(crate) expiries: Vec<ExpirySummary>,
    pub(crate) option_quotes: Vec<OptionQuote>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct ExpirySummary {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) option_kind: OptionKind,
    pub(crate) price_display_decimals: u8,
    pub(crate) quote_display_decimals: u8,
    pub(crate) settlement: String,
    pub(crate) scramble_start_utc: Option<String>,
    pub(crate) listing_utc: Option<String>,
    pub(crate) scramble_start_ts: Option<i64>,
    pub(crate) listing_ts: Option<i64>,
    pub(crate) expiry_ts: Option<i64>,
    pub(crate) schedule_version: Option<u64>,
    pub(crate) days: String,
    pub(crate) current_print: String,
    pub(crate) base: String,
    pub(crate) cap_width: String,
    pub(crate) listed_notional: String,
    pub(crate) rows: String,
    pub(crate) option_quotes: Vec<OptionQuote>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct OptionQuote {
    pub(crate) kind: OptionKind,
    pub(crate) strike: String,
    pub(crate) lower_strike: String,
    pub(crate) upper_strike: String,
    pub(crate) bid: Option<f64>,
    pub(crate) ask: Option<f64>,
    pub(crate) mid: Option<f64>,
    pub(crate) probability_itm: Option<f64>,
    pub(crate) probability_cap_hit: Option<f64>,
    pub(crate) depth_usd: Option<f64>,
    pub(crate) volume: Option<f64>,
    pub(crate) open_interest: Option<f64>,
    pub(crate) prepare_eligible: bool,
    pub(crate) status: String,
}

pub fn dish_list_payload(backend: &BackendClient) -> Result<Value, CliError> {
    let raw_backend = backend.get(endpoints::dlmm_markets())?;
    chain_identity::validate_current_backend_envelope(&raw_backend)?;
    validate_current_market_list_payload(&raw_backend)?;
    let mut issues = Vec::new();
    if let Some(message) = raw_backend
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
    {
        issues.push(format!("market list: {message}"));
    }
    let freshness = raw_backend["data"]["freshness"].clone();
    let (_, freshness_issues, _, _) = freshness_summary(Some(&freshness));
    issues.extend(freshness_issues);
    let dishes = extract_dish_summaries(&raw_backend, "api");
    let market_values = dishes.iter().map(dish_to_value).collect::<Vec<Value>>();
    Ok(json!({
        "ok": true,
        "source": "amoeba_api",
        "markets": market_values,
        "freshness": freshness,
        "issues": issues,
    }))
}

pub fn dish_snapshot_payload(backend: &BackendClient, dish: &str) -> Result<Value, CliError> {
    let market_id = dish.trim().to_ascii_lowercase();
    let payload = backend.get(&endpoints::dlmm_market_snapshot(&market_id))?;
    chain_identity::validate_current_backend_envelope(&payload)?;
    validate_current_market_snapshot_payload(&payload, &market_id)?;
    Ok(payload)
}

pub(crate) fn live_dish_snapshot_payload(
    backend: &BackendClient,
    dish: &str,
) -> Result<Value, CliError> {
    let market_id = dish.trim().to_ascii_lowercase();
    let payload = backend.get(&endpoints::dlmm_market_snapshot(&market_id))?;
    chain_identity::validate_current_backend_envelope(&payload)?;
    validate_current_market_snapshot_payload(&payload, &market_id)?;
    Ok(payload)
}

fn exact_object_keys(value: &Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn required_nonempty_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, CliError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| {
            !text.is_empty()
                && text.trim() == *text
                && text.chars().count() <= MAX_MARKET_DISPLAY_CHARS
                && terminal_safe_text(text) == *text
        })
        .ok_or_else(|| {
            CliError::new(format!(
                "current market field {key} must be a bounded terminal-safe non-empty string"
            ))
        })
}

fn bounded_terminal_safe_text(raw: &str, maximum_chars: usize) -> String {
    terminal_safe_text(raw)
        .chars()
        .take(maximum_chars)
        .collect()
}

fn market_display_text(raw: String) -> String {
    bounded_terminal_safe_text(&raw, MAX_MARKET_DISPLAY_CHARS)
}

fn market_issue_text(raw: String) -> String {
    bounded_terminal_safe_text(&raw, MAX_MARKET_ISSUE_CHARS)
}

fn current_pool_status(raw: &str) -> bool {
    matches!(
        raw,
        "unavailable" | "Pending" | "Active" | "Paused" | "Settled" | "Closed"
    )
}

fn freshness_trade_ready(snapshot: &Value) -> Option<bool> {
    snapshot
        .get("freshness")
        .and_then(|freshness| freshness.get("tradeReady"))
        .and_then(Value::as_bool)
}

pub(crate) fn validate_current_market_row(market_id: &str, row: &Value) -> Result<(), CliError> {
    const ROW_KEYS: &[&str] = &[
        "expiryId",
        "label",
        "optionKind",
        "lowerStrike",
        "upperStrike",
        "priceDisplayDecimals",
        "quoteDisplayDecimals",
        "fairPrice",
        "settlementUtc",
        "poolStatus",
    ];
    if !exact_object_keys(row, ROW_KEYS) {
        return Err(CliError::new(
            "current market series row does not match the exact current projection",
        ));
    }
    let expiry_id = required_nonempty_string(row, "expiryId")?;
    let identity = parse_current_series_id(market_id, expiry_id)?;
    let option_kind = required_nonempty_string(row, "optionKind")?;
    if OptionKind::from_current_option_kind(option_kind) != Some(identity.option_kind) {
        return Err(CliError::new(
            "current market optionKind conflicts with the exact expiryId",
        ));
    }
    let lower = current_u64_field(row, "lowerStrike")
        .ok_or_else(|| CliError::new("current market lowerStrike is not canonical u64 text"))?;
    let upper = current_u64_field(row, "upperStrike")
        .ok_or_else(|| CliError::new("current market upperStrike is not canonical u64 text"))?;
    if lower >= upper {
        return Err(CliError::new(
            "current market strike interval must be strictly increasing",
        ));
    }
    current_display_decimals(row, "priceDisplayDecimals")
        .ok_or_else(|| CliError::new("current market priceDisplayDecimals is invalid"))?;
    current_display_decimals(row, "quoteDisplayDecimals")
        .ok_or_else(|| CliError::new("current market quoteDisplayDecimals is invalid"))?;
    required_nonempty_string(row, "label")?;
    let settlement = required_nonempty_string(row, "settlementUtc")?;
    DateTime::parse_from_rfc3339(settlement)
        .map_err(|_| CliError::new("current market settlementUtc is not RFC3339"))?;
    let pool_status = required_nonempty_string(row, "poolStatus")?;
    if !current_pool_status(pool_status) {
        return Err(CliError::new(
            "current market poolStatus is not a current program status",
        ));
    }
    if !matches!(row.get("fairPrice"), Some(Value::Null | Value::Number(_))) {
        return Err(CliError::new(
            "current market fairPrice must be a number or null",
        ));
    }
    Ok(())
}

fn validate_current_market_object(
    market: &Value,
    requested_market_id: Option<&str>,
    snapshot: bool,
) -> Result<(), CliError> {
    const LIST_KEYS: &[&str] = &[
        "marketId",
        "name",
        "displayName",
        "symbol",
        "title",
        "subtitle",
        "status",
        "onChainAvailable",
        "expiries",
    ];
    const SNAPSHOT_KEYS: &[&str] = &[
        "protocol",
        "marketId",
        "name",
        "displayName",
        "symbol",
        "title",
        "subtitle",
        "status",
        "onChainAvailable",
        "expiries",
        "observedAtSlot",
        "freshness",
    ];
    let expected = if snapshot { SNAPSHOT_KEYS } else { LIST_KEYS };
    if !exact_object_keys(market, expected) {
        return Err(CliError::new(if snapshot {
            "current market snapshot does not match the exact current projection"
        } else {
            "current market list row does not match the exact current projection"
        }));
    }
    let market_id = required_nonempty_string(market, "marketId")?;
    if market_id != market_id.to_ascii_lowercase()
        || requested_market_id.is_some_and(|requested| requested != market_id)
    {
        return Err(CliError::new(
            "current market response does not match the requested canonical marketId",
        ));
    }
    for key in [
        "name",
        "displayName",
        "symbol",
        "title",
        "subtitle",
        "status",
    ] {
        required_nonempty_string(market, key)?;
    }
    if !matches!(
        market.get("status").and_then(Value::as_str),
        Some("current" | "paused")
    ) {
        return Err(CliError::new(
            "current market status must be exactly current or paused",
        ));
    }
    if market
        .get("onChainAvailable")
        .and_then(Value::as_bool)
        .is_none()
    {
        return Err(CliError::new(
            "current market onChainAvailable must be an explicit boolean",
        ));
    }
    let expiries = market
        .get("expiries")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("current market expiries must be an array"))?;
    for row in expiries {
        validate_current_market_row(market_id, row)?;
    }
    if snapshot {
        let slot = required_nonempty_string(market, "observedAtSlot")?;
        if (slot != "0" && slot.starts_with('0')) || slot.parse::<u64>().is_err() {
            return Err(CliError::new(
                "current market observedAtSlot must be canonical u64 text",
            ));
        }
        let freshness = market
            .get("freshness")
            .ok_or_else(|| CliError::new("current market snapshot is missing freshness"))?;
        chain_identity::validate_current_freshness(freshness)?;
        if freshness.get("observedAtSlot").and_then(Value::as_str) != Some(slot) {
            return Err(CliError::new(
                "current market snapshot slot conflicts with its freshness observation",
            ));
        }
    }
    Ok(())
}

fn validate_current_market_list_payload(payload: &Value) -> Result<(), CliError> {
    if !exact_object_keys(payload, &["ok", "data"]) || payload.get("ok") != Some(&Value::Bool(true))
    {
        return Err(CliError::new(
            "current market list does not match the exact current envelope",
        ));
    }
    let data = payload
        .get("data")
        .ok_or_else(|| CliError::new("current market list is missing data"))?;
    if !exact_object_keys(data, &["protocol", "markets", "freshness"]) {
        return Err(CliError::new(
            "current market list does not match the exact current envelope",
        ));
    }
    let markets = data
        .get("markets")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("current market list markets must be an array"))?;
    for market in markets {
        validate_current_market_object(market, None, false)?;
    }
    chain_identity::validate_current_freshness(
        data.get("freshness")
            .ok_or_else(|| CliError::new("current market list is missing freshness"))?,
    )?;
    Ok(())
}

pub(crate) fn validate_current_market_snapshot_payload(
    payload: &Value,
    requested_market_id: &str,
) -> Result<(), CliError> {
    if !exact_object_keys(payload, &["ok", "data"]) || payload.get("ok") != Some(&Value::Bool(true))
    {
        return Err(CliError::new(
            "current market snapshot does not match the exact current envelope",
        ));
    }
    let data = payload
        .get("data")
        .ok_or_else(|| CliError::new("current market snapshot is missing data"))?;
    validate_current_market_object(data, Some(requested_market_id), true)
}

pub fn render_lab_overview(cli: &Cli, backend: &BackendClient, payload: &Value) -> String {
    let lines = vec![
        render_dish_list_plain(cli, backend, payload),
        "Open the TUI for bid/ask, depth, and buy/sell actions: petri".to_string(),
    ];
    lines.join("\n")
}

pub fn render_dish_list_plain(_cli: &Cli, _backend: &BackendClient, payload: &Value) -> String {
    let dishes = extract_dish_summaries(payload, "payload");
    let mut lines = vec!["Markets".to_string()];

    if dishes.is_empty() {
        lines.push("No markets are available right now.".to_string());
    } else {
        for dish in dishes {
            lines.push(format!(
                "{} ({}) | {} | {} monthly contracts",
                dish.id, dish.symbol, dish.title, dish.expiry_count
            ));
        }
    }

    let issues = string_array(payload, &["issues"]);
    for issue in issues {
        lines.push(format!("Note: {issue}"));
    }
    lines.join("\n")
}

pub(crate) fn market_price_context(detail: &DishDetail) -> String {
    if is_known_value(&detail.current_print) {
        format!(
            "current print {} | starting index {}",
            detail.current_print, detail.base
        )
    } else if is_known_value(&detail.base) {
        format!("live print unavailable | reference level {}", detail.base)
    } else {
        "live print unavailable | reference level unavailable".to_string()
    }
}

pub fn render_dish_open(
    _cli: &Cli,
    _backend: &BackendClient,
    dish: &str,
    payload: &Value,
) -> String {
    let detail = detail_from_payload(dish, payload);
    let mut lines = vec![
        format!("{} ({})", detail.symbol, detail.id),
        format!("{} | {}", detail.title, market_price_context(&detail)),
        format!(
            "{} | settles {} | {} days | {} contracts",
            detail.expiry_label, detail.settlement, detail.days, detail.rows
        ),
        format!(
            "Fixed-risk contracts | no liquidation | cap width {} | selected contract shows max loss | listed notional {}",
            detail.cap_width, detail.listed_notional
        ),
        detail.freshness.clone(),
        detail.execution.clone(),
    ];
    if !detail.issues.is_empty() {
        lines.push(format!("Note: {}", detail.issues.join(" | ")));
    }
    lines.push(format!(
        "Next: petri | petri contracts --market {} | petri markets chart {}",
        detail.id, detail.id
    ));
    lines.join("\n")
}

pub fn render_dish_status(
    _cli: &Cli,
    _backend: &BackendClient,
    dish: &str,
    payload: &Value,
) -> String {
    let detail = detail_from_payload(dish, payload);
    let mut lines = vec![
        format!("{} ({})", detail.symbol, detail.id),
        format!(
            "{} | {} | {}",
            market_status_label(&detail.status),
            market_price_context(&detail),
            detail.expiry_label
        ),
        format!(
            "settles {} | fixed-risk contracts | no liquidation | cap width {} | {} contracts",
            detail.settlement, detail.cap_width, detail.rows
        ),
        detail.freshness.clone(),
    ];
    if !detail.issues.is_empty() {
        lines.push(format!("Note: {}", detail.issues.join(" | ")));
    }
    lines.join("\n")
}

pub fn render_print(_cli: &Cli, _backend: &BackendClient, dish: &str, payload: &Value) -> String {
    let detail = detail_from_payload(dish, payload);
    [
        format!("{} print", detail.symbol),
        format!("{} | {}", market_price_context(&detail), detail.expiry_id),
        format!(
            "settles {} | fixed-risk contracts | no liquidation | cap width {}",
            detail.settlement, detail.cap_width
        ),
    ]
    .join("\n")
}

pub(crate) fn is_known_value(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed != "-"
}

pub(crate) fn series_label_from_expiry(expiry: &ExpirySummary) -> Option<String> {
    current_series_shape(&expiry.id).then(|| expiry.id.clone())
}

fn series_label_from_market_expiry(market_id: &str, expiry: &Value) -> Option<String> {
    let id = string_at_key(expiry, &["expiryId"])?;
    parse_current_series_id(market_id, &id).ok()?;
    Some(id)
}

pub(crate) fn compact_series_labels(labels: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for label in labels {
        let Some(label) = clean_series_label(&label) else {
            continue;
        };
        if unique
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(&label))
        {
            continue;
        }
        unique.push(label);
    }
    unique
}

fn clean_series_label(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn detail_from_payload(requested_dish: &str, payload: &Value) -> DishDetail {
    let data = value_at_key(payload, &["data"]).unwrap_or(payload);
    let snapshot = value_at_key(data, &["snapshot"])
        .or_else(|| value_at_key(payload, &["snapshot"]))
        .unwrap_or(data);
    let id = string_at_key(snapshot, &["marketId"])
        .map(market_display_text)
        .unwrap_or_else(|| market_display_text(requested_dish.to_string()));
    let symbol = string_at_key(snapshot, &["symbol"])
        .map(market_display_text)
        .unwrap_or_else(|| market_display_text(id.to_uppercase()));
    let title = string_at_key(snapshot, &["title"])
        .map(market_display_text)
        .unwrap_or_else(|| market_display_text(format!("{symbol} options")));
    let market_prepare_eligible = snapshot.get("status").and_then(Value::as_str) == Some("current")
        && snapshot.get("onChainAvailable") == Some(&Value::Bool(true))
        && freshness_trade_ready(snapshot).unwrap_or(true);
    let mut expiry =
        select_front_expiry(snapshot).and_then(|expiry| expiry_summary_from_value(&id, expiry));
    if let Some(expiry) = expiry.as_mut() {
        for quote in &mut expiry.option_quotes {
            quote.prepare_eligible &= market_prepare_eligible;
        }
    }
    let expiry_id = expiry
        .as_ref()
        .map(|expiry| expiry.id.clone())
        .unwrap_or_else(|| "-".to_string());
    let expiry_label = expiry
        .as_ref()
        .map(|expiry| expiry.label.clone())
        .unwrap_or_else(|| expiry_id.clone());
    let settlement = expiry
        .as_ref()
        .map(|expiry| expiry.settlement.clone())
        .unwrap_or_else(|| "-".to_string());
    let days = expiry
        .as_ref()
        .map(|expiry| expiry.days.clone())
        .unwrap_or_else(|| "-".to_string());
    let current_print = expiry
        .as_ref()
        .map(|expiry| expiry.current_print.clone())
        .unwrap_or_else(|| "-".to_string());
    let base = expiry
        .as_ref()
        .map(|expiry| expiry.base.clone())
        .unwrap_or_else(|| "-".to_string());
    let cap_width = expiry
        .as_ref()
        .map(|expiry| expiry.cap_width.clone())
        .unwrap_or_else(|| "-".to_string());
    let listed_notional = expiry
        .as_ref()
        .map(|expiry| expiry.listed_notional.clone())
        .unwrap_or_else(|| "-".to_string());
    let rows = expiry
        .as_ref()
        .map(|expiry| expiry.rows.clone())
        .unwrap_or_else(|| "0".to_string());
    let (freshness, freshness_issues, stale, degraded) =
        freshness_summary(value_at_key(snapshot, &["freshness"]));
    let (execution, ready) = execution_summary(value_at_key(snapshot, &["execution"]));
    let status = match (ready, stale, degraded) {
        (Some(false), _, _) => "Not ready".to_string(),
        (_, true, _) => "Stale".to_string(),
        (_, _, true) => "Degraded".to_string(),
        _ => "Active".to_string(),
    };
    let mut issues = freshness_issues;
    issues.extend(string_array(snapshot, &["issues"]));
    let mut expiries = expiry_summaries_from_snapshot(&id, snapshot);
    for expiry in &mut expiries {
        for quote in &mut expiry.option_quotes {
            quote.prepare_eligible &= market_prepare_eligible;
        }
    }
    let option_quotes = expiry
        .as_ref()
        .map(|expiry| expiry.option_quotes.clone())
        .unwrap_or_default();

    DishDetail {
        id,
        symbol,
        title,
        status,
        current_print,
        base,
        expiry_id,
        expiry_label,
        settlement,
        days,
        cap_width,
        listed_notional,
        rows,
        freshness,
        execution,
        issues,
        expiries,
        option_quotes,
    }
}

fn expiry_summaries_from_snapshot(market_id: &str, snapshot: &Value) -> Vec<ExpirySummary> {
    let Some(expiries) = array_at_key(snapshot, &["expiries"]) else {
        return Vec::new();
    };
    expiries
        .iter()
        .filter_map(|expiry| expiry_summary_from_value(market_id, expiry))
        .collect()
}

fn current_u64_field(value: &Value, key: &str) -> Option<u64> {
    match value.get(key)? {
        Value::Number(number) => number.as_u64(),
        Value::String(raw) if !raw.is_empty() && (raw == "0" || !raw.starts_with('0')) => {
            raw.parse::<u64>().ok()
        }
        _ => None,
    }
}

fn current_display_decimals(value: &Value, key: &str) -> Option<u8> {
    let decimals = current_u64_field(value, key)?;
    (decimals <= 18).then_some(decimals as u8)
}

fn expiry_summary_from_value(market_id: &str, expiry: &Value) -> Option<ExpirySummary> {
    let id = string_at_key(expiry, &["expiryId"])?;
    let identity = parse_current_series_id(market_id, &id).ok()?;
    let option_kind = string_at_key(expiry, &["optionKind"])
        .and_then(|value| OptionKind::from_current_option_kind(&value))?;
    if option_kind != identity.option_kind
        || current_u64_field(expiry, "lowerStrike")? >= current_u64_field(expiry, "upperStrike")?
    {
        return None;
    }
    let price_display_decimals = current_display_decimals(expiry, "priceDisplayDecimals")?;
    let quote_display_decimals = current_display_decimals(expiry, "quoteDisplayDecimals")?;
    let label = market_display_text(string_at_key(expiry, &["label"])?);
    let settlement = market_display_text(string_at_key(expiry, &["settlementUtc"])?);
    let expiry_ts = DateTime::parse_from_rfc3339(&settlement)
        .ok()
        .map(|value| value.timestamp());
    let days = string_at_key(expiry, &["daysToExpiry"])
        .map(market_display_text)
        .unwrap_or_else(|| "-".to_string());
    let current_print = value_at_key(expiry, &["fairPrice"])
        .and_then(number_from_value)
        .map(|value| format_decimal(value, quote_display_decimals as usize))
        .unwrap_or_else(|| "-".to_string());
    let base = value_at_key(expiry, &["baseOracle"])
        .and_then(number_from_value)
        .map(|value| format_decimal(value, price_display_decimals as usize))
        .unwrap_or_else(|| "-".to_string());
    let cap_width = current_u64_field(expiry, "upperStrike")
        .and_then(|upper| {
            current_u64_field(expiry, "lowerStrike").and_then(|lower| upper.checked_sub(lower))
        })
        .map(|value| format_atomic_decimal(value, price_display_decimals))
        .unwrap_or_else(|| "-".to_string());
    let listed_notional = value_at_key(expiry, &["listedNotional"])
        .and_then(number_from_value)
        .map(format_usd)
        .unwrap_or_else(|| "-".to_string());
    let rows = "1".to_string();
    let option_quotes = option_quotes_from_expiry(Some(expiry));

    Some(ExpirySummary {
        id,
        label,
        option_kind,
        price_display_decimals,
        quote_display_decimals,
        settlement,
        scramble_start_utc: None,
        listing_utc: None,
        scramble_start_ts: None,
        listing_ts: None,
        expiry_ts,
        schedule_version: None,
        days,
        current_print,
        base,
        cap_width,
        listed_notional,
        rows,
        option_quotes,
    })
}

pub(crate) fn option_quotes_from_expiry(expiry: Option<&Value>) -> Vec<OptionQuote> {
    let Some(expiry) = expiry else {
        return Vec::new();
    };
    if expiry.get("expiryId").is_none() {
        return Vec::new();
    }
    option_quote_from_current_series(expiry)
        .into_iter()
        .collect()
}

fn format_atomic_decimal(value: u64, display_decimals: u8) -> String {
    // Current Spread prices always use six-decimal atomic units. The market's
    // priceDisplayDecimals controls presentation only; it is not the divisor.
    let display_decimals = display_decimals.min(CURRENT_PRICE_ATOMIC_DECIMALS);
    let omitted_decimals = CURRENT_PRICE_ATOMIC_DECIMALS - display_decimals;
    let rounding_unit = 10_u64.pow(u32::from(omitted_decimals));
    let mut whole = value / CURRENT_PRICE_ATOMIC_SCALE;
    let remainder = value % CURRENT_PRICE_ATOMIC_SCALE;
    let mut fraction = (remainder + rounding_unit / 2) / rounding_unit;
    let display_scale = 10_u64.pow(u32::from(display_decimals));
    if fraction == display_scale {
        whole += 1;
        fraction = 0;
    }
    if display_decimals == 0 || fraction == 0 {
        return whole.to_string();
    }
    let mut output = format!(
        "{whole}.{fraction:0width$}",
        width = display_decimals as usize
    );
    while output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    output
}

fn option_quote_from_current_series(expiry: &Value) -> Option<OptionQuote> {
    let kind = string_at_key(expiry, &["optionKind"])
        .and_then(|value| OptionKind::from_current_option_kind(&value))?;
    let price_display_decimals = current_display_decimals(expiry, "priceDisplayDecimals")?;
    let lower = current_u64_field(expiry, "lowerStrike")?;
    let upper = current_u64_field(expiry, "upperStrike")?;
    let strike = format_atomic_decimal(lower, price_display_decimals);
    let lower_strike = strike.clone();
    let upper_strike = format_atomic_decimal(upper, price_display_decimals);
    let bid = positive_quote(value_at_key(expiry, &["bid", "bestBid"]).and_then(number_from_value));
    let ask = positive_quote(value_at_key(expiry, &["ask", "bestAsk"]).and_then(number_from_value));
    let mid = value_at_key(expiry, &["fairPrice"])
        .and_then(number_from_value)
        .or_else(|| bid.zip(ask).map(|(bid, ask)| (bid + ask) / 2.0))
        .filter(|value| value.is_finite() && *value > 0.0);
    let depth_usd = value_at_key(expiry, &["liquidityUsd"])
        .and_then(number_from_value)
        .or_else(|| {
            value_at_key(expiry, &["pool"])
                .and_then(|pool| value_at_key(pool, &["liquidityUsd"]))
                .and_then(number_from_value)
        });
    let status = string_at_key(expiry, &["poolStatus"])
        .map(market_display_text)
        .unwrap_or_else(|| "unverified_on_chain_state".to_string());
    let prepare_eligible = status == "Active";
    Some(OptionQuote {
        kind,
        strike,
        lower_strike,
        upper_strike,
        bid,
        ask,
        mid,
        probability_itm: value_at_key(expiry, &["probabilityItm"])
            .and_then(number_from_value)
            .and_then(normalize_probability),
        probability_cap_hit: value_at_key(expiry, &["probabilityCapHit"])
            .and_then(number_from_value)
            .and_then(normalize_probability),
        depth_usd,
        volume: value_at_key(expiry, &["volume"]).and_then(number_from_value),
        open_interest: value_at_key(expiry, &["openInterest"]).and_then(number_from_value),
        prepare_eligible,
        status,
    })
}

pub(crate) fn normalize_probability(value: f64) -> Option<f64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    if value <= 1.0 {
        Some(value)
    } else if value <= 100.0 {
        Some(value / 100.0)
    } else {
        None
    }
}

pub(crate) fn extract_dish_summaries(payload: &Value, source: &str) -> Vec<DishSummary> {
    let data = value_at_key(payload, &["data"]).unwrap_or(payload);
    let markets = array_at_key(data, &["markets"]).or_else(|| array_at_key(payload, &["markets"]));
    let Some(markets) = markets else {
        return Vec::new();
    };
    markets
        .iter()
        .filter_map(|market| {
            let id = market_display_text(
                string_at_key(market, &["marketId", "id"])?.to_ascii_lowercase(),
            );
            let symbol = string_at_key(market, &["symbol"])
                .map(market_display_text)
                .unwrap_or_else(|| market_display_text(id.to_uppercase()));
            let title = string_at_key(market, &["title"])
                .map(market_display_text)
                .unwrap_or_else(|| market_display_text(format!("{symbol} options")));
            let expiry_count = string_at_key(market, &["expiryCount"])
                .map(market_display_text)
                .or_else(|| {
                    array_at_key(market, &["expiries"]).map(|items| items.len().to_string())
                })
                .unwrap_or_else(|| "-".to_string());
            let series_labels = array_at_key(market, &["expiries"])
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|expiry| series_label_from_market_expiry(&id, expiry))
                        .collect::<Vec<String>>()
                })
                .map(compact_series_labels)
                .unwrap_or_default();
            let item_source = string_at_key(market, &["source"])
                .map(market_display_text)
                .unwrap_or_else(|| market_display_text(source.to_string()));
            Some(DishSummary {
                id,
                symbol,
                title,
                expiry_count,
                series_labels,
                source: item_source,
            })
        })
        .collect()
}

fn dish_to_value(dish: &DishSummary) -> Value {
    json!({
        "id": dish.id,
        "symbol": dish.symbol,
        "title": dish.title,
        "expiryCount": dish.expiry_count,
        "seriesLabels": dish.series_labels,
        "source": dish.source,
    })
}

fn select_front_expiry(snapshot: &Value) -> Option<&Value> {
    let expiries = array_at_key(snapshot, &["expiries"])?;
    expiries
        .iter()
        .find(|expiry| {
            value_at_key(expiry, &["fairPrice", "baseOracle"])
                .and_then(number_from_value)
                .is_some()
        })
        .or_else(|| expiries.first())
}

fn freshness_summary(freshness: Option<&Value>) -> (String, Vec<String>, bool, bool) {
    let Some(freshness) = freshness else {
        return (
            "Market data: available".to_string(),
            Vec::new(),
            false,
            false,
        );
    };
    let stale = value_at_key(freshness, &["stale"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let refresh_healthy = value_at_key(freshness, &["refreshHealthy"])
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            !value_at_key(freshness, &["degraded"])
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    let trade_ready = value_at_key(freshness, &["tradeReady"])
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let degraded = !refresh_healthy || !trade_ready;
    let mut issues = string_array(freshness, &["issues"]);
    if stale {
        let age_ms = value_at_key(freshness, &["ageMs"])
            .and_then(Value::as_u64)
            .map_or_else(|| "unknown".to_string(), |value| value.to_string());
        let maximum_age_ms = value_at_key(freshness, &["maximumAgeMs"])
            .and_then(Value::as_u64)
            .map_or_else(|| "unknown".to_string(), |value| value.to_string());
        issues.push(format!(
            "Market snapshot is stale (age {age_ms} ms; maximum {maximum_age_ms} ms)."
        ));
    }
    if !refresh_healthy {
        issues.push("Market snapshot refresh is unhealthy.".to_string());
    }
    if let Some(code) = string_at_key(freshness, &["lastErrorCode"]).map(market_issue_text) {
        issues.push(format!("Last market refresh error: {code}."));
    }
    if !trade_ready {
        issues.push(
            "Trading preparation is unavailable until the market snapshot is healthy, fresh, and trade-ready."
                .to_string(),
        );
    }
    let summary = if stale {
        "Market data: stale; last known market shown".to_string()
    } else if !refresh_healthy {
        "Market data: refresh unhealthy; last known market shown".to_string()
    } else if !trade_ready {
        "Market data: fresh; trading snapshot not ready".to_string()
    } else {
        "Market data: live".to_string()
    };
    (summary, issues, stale, degraded)
}

pub(crate) fn current_freshness_issues(snapshot: &Value) -> Vec<String> {
    freshness_summary(value_at_key(snapshot, &["freshness"])).1
}

fn execution_summary(execution: Option<&Value>) -> (String, Option<bool>) {
    let Some(execution) = execution else {
        return ("Trading: checking availability".to_string(), None);
    };
    let ready = value_at_key(execution, &["ready"]).and_then(Value::as_bool);
    let message = string_at_key(execution, &["message"])
        .map(market_issue_text)
        .unwrap_or_default();
    let summary = if ready == Some(true) {
        "Trading: buy and sell available".to_string()
    } else if message.is_empty() {
        "Trading: not available right now".to_string()
    } else {
        format!("Trading: {message}")
    };
    (summary, ready)
}

pub(crate) fn string_array(value: &Value, keys: &[&str]) -> Vec<String> {
    array_at_key(value, keys)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.as_str() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            return Some(market_issue_text(trimmed.to_string()));
                        }
                    }
                    string_at_key(item, &["message", "title", "id"]).map(market_issue_text)
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn number_from_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

pub(crate) fn market_status_label(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("live") || lower.contains("active") || lower.contains("ready") {
        "open".to_string()
    } else if lower.contains("updating") || lower.contains("stale") || lower.contains("degraded") {
        "updating".to_string()
    } else if lower.contains("unavailable") || lower.contains("missing") {
        "not available".to_string()
    } else if value.trim().is_empty() || value == "-" {
        "status pending".to_string()
    } else {
        value.to_string()
    }
}

pub(crate) fn positive_quote(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

pub(crate) fn format_decimal(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    let formatted = format!("{value:.decimals$}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(crate) fn format_usd(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let abs = value.abs();
    if abs >= 1_000_000.0 {
        format!("{sign}${:.2}M", abs / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{sign}${:.2}K", abs / 1_000.0)
    } else {
        format!("{sign}${:.2}", abs)
    }
}
