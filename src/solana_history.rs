use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    time::Duration,
};

use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;

use crate::{backend::CliError, onchain};

const WALLET_ACTIVITY_UNAVAILABLE: &str = "Wallet activity is temporarily unavailable.";
const TRADE_HISTORY_UNAVAILABLE: &str = "Amoeba trade history is temporarily unavailable.";
pub(crate) const AMOEBA_SPREAD_PROGRAM_ID: &str = "2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw";
const AMOEBA_SPREAD_PROGRAM_LABEL: &str = "Amoeba Spread";
const DEFAULT_SCAN_LIMIT: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackedProgram {
    program_id: String,
    label: String,
    category: String,
    source: String,
}

pub(crate) fn fetch_account_history(
    config: &onchain::OnchainConfig,
    owner_pubkey: &str,
    limit: usize,
) -> Result<Value, CliError> {
    Pubkey::from_str(owner_pubkey)
        .map_err(|error| CliError::new(format!("invalid owner pubkey {owner_pubkey}: {error}")))?;

    crate::chain_identity::verify_onchain_config(config)?;
    let limit = limit.clamp(1, 1_000);
    let rpc_url = onchain::resolve_rpc_url(config)?;
    let response = post_solana_rpc(
        &rpc_url,
        "getSignaturesForAddress",
        json!([
            owner_pubkey,
            {
                "limit": limit,
                "commitment": "confirmed"
            }
        ]),
    )?;
    let signatures = response
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Ok(json!({
        "ownerPubkey": owner_pubkey,
        "source": "solana_rpc",
        "rpcMethod": "getSignaturesForAddress",
        "commitment": "confirmed",
        "limit": limit,
        "count": signatures.len(),
        "empty": signatures.is_empty(),
        "signatures": signatures,
    }))
}

pub(crate) fn unavailable_account_history(owner_pubkey: &str, limit: usize) -> Value {
    let limit = limit.clamp(1, 1_000);
    json!({
        "ownerPubkey": owner_pubkey,
        "source": "wallet_activity",
        "available": false,
        "limit": limit,
        "count": 0,
        "empty": true,
        "signatures": [],
    })
}

pub(crate) fn build_account_ledger_payload_with_activity(
    owner_pubkey: &str,
    chain_history: Value,
    product_ledger: Option<Value>,
    amoeba_activity: Option<Value>,
    program_registry: Option<Value>,
    issues: Vec<String>,
) -> Value {
    json!({
        "ownerPubkey": owner_pubkey,
        "chainHistory": chain_history,
        "productLedger": product_ledger,
        "amoebaActivity": amoeba_activity,
        "programRegistry": program_registry,
        "issues": issues,
    })
}

pub(crate) fn account_history_is_empty(history: &Value) -> bool {
    history
        .get("empty")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| account_history_signatures(history).is_empty())
}

pub(crate) fn account_history_signatures(history: &Value) -> Vec<Value> {
    history
        .get("signatures")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn account_history_is_available(history: &Value) -> bool {
    history
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

pub(crate) fn wallet_activity_unavailable_issue() -> String {
    WALLET_ACTIVITY_UNAVAILABLE.to_string()
}

pub(crate) fn trade_history_unavailable_issue() -> String {
    TRADE_HISTORY_UNAVAILABLE.to_string()
}

pub(crate) fn normalize_amoeba_program_registry(payload: Option<&Value>) -> Value {
    let _ = payload;
    let rows = vec![tracked_program_to_json(&TrackedProgram {
        program_id: AMOEBA_SPREAD_PROGRAM_ID.to_string(),
        label: AMOEBA_SPREAD_PROGRAM_LABEL.to_string(),
        category: "amoeba_spread".to_string(),
        source: "petri_defaults".to_string(),
    })];
    json!({
        "source": "petri_defaults",
        "programs": rows,
        "issues": []
    })
}

pub(crate) fn scan_amoeba_activity(
    config: &onchain::OnchainConfig,
    chain_history: &Value,
    _registry_payload: Option<&Value>,
    scan_limit: usize,
) -> Value {
    let signatures = account_history_signatures(chain_history);
    let limit = normalized_scan_limit(scan_limit);
    let mut transaction_details = HashMap::new();
    let mut unavailable = 0usize;

    for item in signatures.iter().take(limit) {
        let Some(signature) = string_at_path(item, &["signature"]) else {
            continue;
        };
        match fetch_transaction_detail(config, &signature) {
            Ok(detail) => {
                transaction_details.insert(signature, detail);
            }
            Err(_) => {
                unavailable += 1;
            }
        }
    }

    build_amoeba_activity_scan_from_details(
        chain_history,
        &transaction_details,
        None,
        scan_limit,
        unavailable,
    )
}

pub(crate) fn build_amoeba_activity_scan_from_details(
    chain_history: &Value,
    transaction_details: &HashMap<String, Value>,
    _registry_payload: Option<&Value>,
    scan_limit: usize,
    transaction_detail_unavailable_count: usize,
) -> Value {
    let registry = normalize_amoeba_program_registry(None);
    let tracked_programs = tracked_programs_from_registry(Some(&registry));
    let signatures = account_history_signatures(chain_history);
    let limit = normalized_scan_limit(scan_limit);
    let mut matches = Vec::new();
    let mut scanned = 0usize;

    for item in signatures.iter().take(limit) {
        let Some(signature) = string_at_path(item, &["signature"]) else {
            continue;
        };
        scanned += 1;
        let Some(transaction) = transaction_details.get(&signature) else {
            continue;
        };
        let matched_programs = matched_programs_for_transaction(transaction, &tracked_programs);
        if matched_programs.is_empty() {
            continue;
        }
        matches.push(json!({
            "signature": signature,
            "slot": string_at_path(item, &["slot"])
                .or_else(|| string_at_path(transaction_result(transaction), &["slot"])),
            "blockTime": string_at_path(item, &["blockTime", "block_time"])
                .or_else(|| string_at_path(transaction_result(transaction), &["blockTime", "block_time"])),
            "confirmationStatus": string_at_path(item, &["confirmationStatus", "confirmation_status"])
                .or_else(|| string_at_path(transaction_result(transaction), &["confirmationStatus", "confirmation_status"])),
            "err": item
                .get("err")
                .cloned()
                .or_else(|| transaction_result(transaction).pointer("/meta/err").cloned())
                .unwrap_or(Value::Null),
            "matchedPrograms": matched_programs
                .iter()
                .map(tracked_program_to_json)
                .collect::<Vec<_>>(),
        }));
    }

    json!({
        "available": account_history_is_available(chain_history),
        "source": "solana_rpc_transaction_scan",
        "scannedTransactionCount": scanned,
        "transactionDetailUnavailableCount": transaction_detail_unavailable_count,
        "programCount": tracked_programs.len(),
        "programs": registry.get("programs").cloned().unwrap_or_else(|| json!([])),
        "matches": matches,
    })
}

pub(crate) fn user_facing_ledger_issue(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let is_trade_history = lower.contains("amoeba") || lower.contains("trade");
    let leaks_implementation = [
        "backend",
        "rpc",
        "response body",
        "getsignaturesforaddress",
        "failed to parse",
        "error decoding",
        "http",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    if leaks_implementation && is_trade_history {
        TRADE_HISTORY_UNAVAILABLE.to_string()
    } else if leaks_implementation {
        WALLET_ACTIVITY_UNAVAILABLE.to_string()
    } else if raw.trim().is_empty() {
        "Ledger activity is temporarily unavailable.".to_string()
    } else {
        raw.trim().to_string()
    }
}

pub(crate) fn format_ledger_timestamp(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return "-".to_string();
    }

    parse_ledger_timestamp_utc(trimmed)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%b %-d, %Y %-I:%M %p %:z")
                .to_string()
        })
        .unwrap_or_else(|| trimmed.to_string())
}

fn parse_ledger_timestamp_utc(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(raw) {
        return Some(timestamp.with_timezone(&Utc));
    }

    if let Ok(timestamp) = raw.parse::<i64>() {
        return timestamp_from_numeric_value(timestamp);
    }

    let timestamp = raw.parse::<f64>().ok()?;
    if !timestamp.is_finite() || timestamp < i64::MIN as f64 || timestamp > i64::MAX as f64 {
        return None;
    }
    timestamp_from_numeric_value(timestamp.round() as i64)
}

fn timestamp_from_numeric_value(timestamp: i64) -> Option<DateTime<Utc>> {
    let abs_timestamp = timestamp.checked_abs().unwrap_or(i64::MAX);
    if abs_timestamp >= 100_000_000_000 {
        Utc.timestamp_millis_opt(timestamp).single()
    } else {
        Utc.timestamp_opt(timestamp, 0).single()
    }
}

fn fetch_transaction_detail(
    config: &onchain::OnchainConfig,
    signature: &str,
) -> Result<Value, CliError> {
    crate::chain_identity::verify_onchain_config(config)?;
    let rpc_url = onchain::resolve_rpc_url(config)?;
    post_solana_rpc(
        &rpc_url,
        "getTransaction",
        json!([
            signature,
            {
                "encoding": "jsonParsed",
                "commitment": "confirmed",
                "maxSupportedTransactionVersion": 0
            }
        ]),
    )
}

fn post_solana_rpc(rpc_url: &str, method: &str, params: Value) -> Result<Value, CliError> {
    let http = crate::backend::pinned_blocking_http_client_builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| CliError::new(WALLET_ACTIVITY_UNAVAILABLE))?;
    let response = http
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": format!("petri-cli:{method}"),
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|_| CliError::new(WALLET_ACTIVITY_UNAVAILABLE))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|_| CliError::new(WALLET_ACTIVITY_UNAVAILABLE))?;
    let payload = serde_json::from_str::<Value>(&body)
        .map_err(|_| CliError::new(WALLET_ACTIVITY_UNAVAILABLE))?;
    if !status.is_success() {
        return Err(CliError::new(WALLET_ACTIVITY_UNAVAILABLE));
    }
    if payload.get("error").is_some() {
        return Err(CliError::new(WALLET_ACTIVITY_UNAVAILABLE));
    }
    Ok(payload)
}

fn normalized_scan_limit(value: usize) -> usize {
    value.clamp(1, DEFAULT_SCAN_LIMIT)
}

fn tracked_programs_from_registry(payload: Option<&Value>) -> Vec<TrackedProgram> {
    let Some(payload) = payload else {
        return Vec::new();
    };
    let candidates = [
        payload.get("programs"),
        payload.pointer("/data/programs"),
        payload.pointer("/data/registry/programs"),
        payload.pointer("/registry/programs"),
    ];

    candidates
        .into_iter()
        .flatten()
        .find_map(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(tracked_program_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn tracked_program_from_value(value: &Value) -> Option<TrackedProgram> {
    if value
        .get("active")
        .and_then(Value::as_bool)
        .is_some_and(|active| !active)
    {
        return None;
    }
    let program_id = string_at_path(value, &["programId", "program_id"])?;
    if program_id.trim().is_empty() {
        return None;
    }
    Some(TrackedProgram {
        label: string_at_path(value, &["label", "name"])
            .unwrap_or_else(|| "Amoeba program".to_string()),
        category: string_at_path(value, &["category", "kind"])
            .unwrap_or_else(|| "related".to_string()),
        source: string_at_path(value, &["source"]).unwrap_or_else(|| "registry".to_string()),
        program_id,
    })
}

fn tracked_program_to_json(program: &TrackedProgram) -> Value {
    json!({
        "programId": program.program_id,
        "label": program.label,
        "category": program.category,
        "source": program.source,
        "active": true,
    })
}

fn matched_programs_for_transaction(
    transaction: &Value,
    programs: &[TrackedProgram],
) -> Vec<TrackedProgram> {
    let references = referenced_program_ids(transaction);
    programs
        .iter()
        .filter(|program| references.contains(&program.program_id))
        .cloned()
        .collect()
}

fn referenced_program_ids(transaction: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    let root = transaction_result(transaction);
    collect_program_id_fields(root, &mut ids);
    collect_account_key_fields(root, &mut ids);
    ids
}

fn transaction_result(transaction: &Value) -> &Value {
    transaction.get("result").unwrap_or(transaction)
}

fn collect_program_id_fields(value: &Value, ids: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            if let Some(program_id) = object.get("programId").and_then(Value::as_str) {
                ids.insert(program_id.to_string());
            }
            for child in object.values() {
                collect_program_id_fields(child, ids);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_program_id_fields(child, ids);
            }
        }
        _ => {}
    }
}

fn collect_account_key_fields(value: &Value, ids: &mut HashSet<String>) {
    if let Some(items) = value
        .pointer("/transaction/message/accountKeys")
        .and_then(Value::as_array)
    {
        for item in items {
            if let Some(pubkey) = item
                .as_str()
                .map(str::to_string)
                .or_else(|| string_at_path(item, &["pubkey"]))
            {
                ids.insert(pubkey);
            }
        }
    }

    for pointer in [
        "/meta/loadedAddresses/writable",
        "/meta/loadedAddresses/readonly",
    ] {
        if let Some(items) = value.pointer(pointer).and_then(Value::as_array) {
            for item in items {
                if let Some(pubkey) = item.as_str() {
                    ids.insert(pubkey.to_string());
                }
            }
        }
    }
}

fn string_at_path(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = value.get(*key) {
            if let Some(text) = value.as_str() {
                if !text.trim().is_empty() {
                    return Some(text.trim().to_string());
                }
            } else if value.is_number() || value.is_boolean() {
                return Some(value.to_string());
            }
        }
    }
    None
}
