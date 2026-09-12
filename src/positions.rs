//! Public, owner-scoped projection of current manager-liquidity positions.
//!
//! Lean owns discovery and policy validation. This module validates the public
//! identity of each returned row, removes backend-only account detail, and
//! renders the result without inventing balances or settlement eligibility.

use std::str::FromStr;

use chrono::DateTime;
use serde_json::{Value, json};
use solana_pubkey::Pubkey;

use crate::{
    backend::{CliError, terminal_safe_text},
    market_surface::{OptionKind, parse_current_series_id},
};

pub fn project_liquidity_positions(
    payload: &Value,
    expected_owner: &str,
) -> Result<Value, CliError> {
    let expected_owner_key = canonical_pubkey(expected_owner, "owner")?;
    let data = payload.get("data").unwrap_or(payload);
    let rows = data
        .get("positions")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new("current positions response is missing positions"))?;
    let mut positions = Vec::with_capacity(rows.len());

    for row in rows {
        let owner = required_string(row, "owner", "position owner")?;
        if canonical_pubkey(owner, "position owner")? != expected_owner_key {
            return Err(CliError::new(
                "current positions response contains a position for another wallet",
            ));
        }
        let address = required_string(row, "address", "position identifier")?;
        canonical_pubkey(address, "position identifier")?;
        let market_id = required_string(row, "marketId", "market id")?;
        if market_id != market_id.to_ascii_lowercase()
            || !market_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(CliError::new(
                "current positions response contains a noncanonical market id",
            ));
        }
        let expiry_id = required_string(row, "expiryId", "series id")?;
        let series_identity = parse_current_series_id(market_id, expiry_id).map_err(|error| {
            CliError::new(format!(
                "current positions response contains an invalid series id: {error}"
            ))
        })?;
        let option_kind = required_string(row, "optionKind", "option kind")?;
        let returned_option_kind =
            OptionKind::from_current_option_kind(option_kind).ok_or_else(|| {
                CliError::new("current positions response contains an invalid option kind")
            })?;
        if returned_option_kind != series_identity.option_kind {
            return Err(CliError::new(
                "current positions response option kind conflicts with the exact series id",
            ));
        }
        let lower_strike = canonical_u64(
            required_string(row, "lowerStrike", "lowerStrike")?,
            "lowerStrike",
        )?;
        let upper_strike = canonical_u64(
            required_string(row, "upperStrike", "upperStrike")?,
            "upperStrike",
        )?;
        if lower_strike >= upper_strike {
            return Err(CliError::new(
                "current positions response strike interval must be strictly increasing",
            ));
        }
        for field in ["liquidityShares", "feeShares"] {
            canonical_nat_text(required_string(row, field, field)?, field)?;
        }
        canonical_u64(required_string(row, "slot", "slot")?, "slot")?;
        for field in ["priceDisplayDecimals", "quoteDisplayDecimals"] {
            current_display_decimals(row, field).ok_or_else(|| {
                CliError::new(format!("current positions response has an invalid {field}"))
            })?;
        }
        let settlement_utc = required_string(row, "settlementUtc", "settlementUtc")?;
        DateTime::parse_from_rfc3339(settlement_utc).map_err(|_| {
            CliError::new("current positions response settlementUtc is not RFC3339")
        })?;
        let status = required_string(row, "status", "position status")?;
        let source_state = required_string(row, "sourceState", "position source state")?;
        if status != "current" || !matches!(source_state, "hot" | "cold") {
            return Err(CliError::new(
                "current positions response contains an invalid current-state marker",
            ));
        }

        positions.push(json!({
            "positionId": address,
            "marketId": market_id,
            "seriesId": expiry_id,
            "optionKind": option_kind,
            "lowerStrike": row.get("lowerStrike").cloned().unwrap_or(Value::Null),
            "upperStrike": row.get("upperStrike").cloned().unwrap_or(Value::Null),
            "priceDisplayDecimals": row.get("priceDisplayDecimals").cloned().unwrap_or(Value::Null),
            "quoteDisplayDecimals": row.get("quoteDisplayDecimals").cloned().unwrap_or(Value::Null),
            "settlementUtc": row.get("settlementUtc").cloned().unwrap_or(Value::Null),
            "liquidityShares": row.get("liquidityShares").cloned().unwrap_or(Value::Null),
            "feeShares": row.get("feeShares").cloned().unwrap_or(Value::Null),
            "status": status,
            "sourceState": source_state,
            "lastUpdatedSlot": row.get("slot").cloned().unwrap_or(Value::Null),
        }));
    }

    Ok(json!({
        "ok": true,
        "ownerPubkey": expected_owner,
        "count": positions.len(),
        "empty": positions.is_empty(),
        "scope": "manager_liquidity",
        "mutationInputsComplete": false,
        "issue": "The current public projection does not expose the canonical position nonce required for liquidity mutation planning.",
        "positions": positions,
        "protocol": data.get("protocol").cloned().unwrap_or(Value::Null),
    }))
}

pub fn render_liquidity_positions(payload: &Value) -> String {
    let owner = payload
        .get("ownerPubkey")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let rows = payload
        .get("positions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut lines = vec![format!(
        "Manager liquidity positions | wallet={}",
        short(owner, 20)
    )];
    if rows.is_empty() {
        lines
            .push("No current manager-liquidity positions were found for this wallet.".to_string());
        return lines.join("\n");
    }
    lines.push("Market / series | Liquidity shares | Fee shares | State | Position".to_string());
    for row in rows {
        let field = |key: &str| {
            row.get(key)
                .and_then(|value| match value {
                    Value::String(text) => Some(terminal_safe_text(text)),
                    Value::Number(number) => Some(number.to_string()),
                    Value::Bool(flag) => Some(flag.to_string()),
                    _ => None,
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_string())
        };
        lines.push(format!(
            "{}/{} | {} | {} | {} / {} | {}",
            field("marketId"),
            field("seriesId"),
            field("liquidityShares"),
            field("feeShares"),
            field("status"),
            field("sourceState"),
            short(&field("positionId"), 20),
        ));
    }
    lines.push(
        "Note: the current read omits the position nonce needed for mutation planning.".to_string(),
    );
    lines.join("\n")
}

fn required_string<'a>(value: &'a Value, key: &str, label: &str) -> Result<&'a str, CliError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.trim() == *value)
        .ok_or_else(|| CliError::new(format!("current positions response is missing {label}")))
}

fn canonical_pubkey(raw: &str, label: &str) -> Result<Pubkey, CliError> {
    let parsed = Pubkey::from_str(raw)
        .map_err(|_| CliError::new(format!("current positions response has an invalid {label}")))?;
    if parsed.to_string() != raw {
        return Err(CliError::new(format!(
            "current positions response has a noncanonical {label}"
        )));
    }
    Ok(parsed)
}

fn canonical_u64(raw: &str, label: &str) -> Result<u64, CliError> {
    let parsed = raw
        .parse::<u64>()
        .map_err(|_| CliError::new(format!("current positions response has an invalid {label}")))?;
    if parsed.to_string() != raw {
        return Err(CliError::new(format!(
            "current positions response has a noncanonical {label}"
        )));
    }
    Ok(parsed)
}

fn canonical_nat_text<'a>(raw: &'a str, label: &str) -> Result<&'a str, CliError> {
    if raw == "0" || (!raw.starts_with('0') && raw.as_bytes().iter().all(u8::is_ascii_digit)) {
        return Ok(raw);
    }
    Err(CliError::new(format!(
        "current positions response has a noncanonical {label}"
    )))
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

fn short(value: &str, width: usize) -> String {
    let value = terminal_safe_text(value);
    if value.chars().count() <= width || width < 9 {
        return value;
    }
    let head = (width - 3) / 2;
    let tail = width - 3 - head;
    format!(
        "{}...{}",
        value.chars().take(head).collect::<String>(),
        value
            .chars()
            .rev()
            .take(tail)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
    )
}
