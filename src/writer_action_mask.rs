//! Exact owner-and-sleeve wallet action availability for current RC44 writer flows.
//!
//! The release capability only advertises that this projection exists. This
//! module validates each fresh wallet-specific response before Petri opens a
//! TUI review or prepares a direct writer transaction.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use solana_pubkey::Pubkey;

use crate::chain_identity;

pub const SEMANTIC_ABI_VERSION: &str = "collective-operations-v1";
pub const ACTION_SCHEMA_JSON: &str = include_str!("../schemas/collective-actions.v1.json");

pub fn action_schema() -> Value {
    serde_json::from_str(ACTION_SCHEMA_JSON).expect("bundled action schema")
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CollectiveActionKind {
    TradeBuy,
    TradeSell,
    AuctionBid,
    AuctionRefund,
    WriterDeposit,
    WriterWithdraw,
    WriterCloseBegin,
    WriterCloseContinue,
    WriterCloseFinalize,
    ClaimLong,
    ClaimFlat,
    FlatTransfer,
    StakeQueue,
    StakeActivate,
    UnstakeRequest,
    UnstakeComplete,
    DisputeCommit,
    DisputeReveal,
    WriterLiquidityInitialize,
    WriterLiquidityAdd,
    WriterLiquidityRemove,
    WriterLiquiditySweep,
}

impl CollectiveActionKind {
    pub const ALL: [Self; 22] = [
        Self::TradeBuy,
        Self::TradeSell,
        Self::AuctionBid,
        Self::AuctionRefund,
        Self::WriterDeposit,
        Self::WriterWithdraw,
        Self::WriterCloseBegin,
        Self::WriterCloseContinue,
        Self::WriterCloseFinalize,
        Self::ClaimLong,
        Self::ClaimFlat,
        Self::FlatTransfer,
        Self::StakeQueue,
        Self::StakeActivate,
        Self::UnstakeRequest,
        Self::UnstakeComplete,
        Self::DisputeCommit,
        Self::DisputeReveal,
        Self::WriterLiquidityInitialize,
        Self::WriterLiquidityAdd,
        Self::WriterLiquidityRemove,
        Self::WriterLiquiditySweep,
    ];

    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::TradeBuy => "trade_buy",
            Self::TradeSell => "trade_sell",
            Self::AuctionBid => "auction_bid",
            Self::AuctionRefund => "auction_refund",
            Self::WriterDeposit => "writer_deposit",
            Self::WriterWithdraw => "writer_withdraw",
            Self::WriterCloseBegin => "writer_close_begin",
            Self::WriterCloseContinue => "writer_close_continue",
            Self::WriterCloseFinalize => "writer_close_finalize",
            Self::ClaimLong => "claim_long",
            Self::ClaimFlat => "claim_flat",
            Self::FlatTransfer => "flat_transfer",
            Self::StakeQueue => "stake_queue",
            Self::StakeActivate => "stake_activate",
            Self::UnstakeRequest => "unstake_request",
            Self::UnstakeComplete => "unstake_complete",
            Self::DisputeCommit => "dispute_commit",
            Self::DisputeReveal => "dispute_reveal",
            Self::WriterLiquidityInitialize => "writer_liquidity_initialize",
            Self::WriterLiquidityAdd => "writer_liquidity_add",
            Self::WriterLiquidityRemove => "writer_liquidity_remove",
            Self::WriterLiquiditySweep => "writer_liquidity_sweep",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailableAction {
    pub kind: CollectiveActionKind,
    pub enabled: bool,
    pub blocking_code: Option<String>,
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterActionMask {
    pub owner: String,
    pub sleeve: String,
    pub observed_slot: u64,
    pub observed_at: DateTime<Utc>,
    pub available_actions: Vec<AvailableAction>,
}

impl WriterActionMask {
    pub fn action(&self, kind: CollectiveActionKind) -> Option<&AvailableAction> {
        self.available_actions
            .iter()
            .find(|action| action.kind == kind)
    }

    pub fn require_enabled(&self, kind: CollectiveActionKind) -> Result<(), String> {
        self.require_enabled_at(kind, Utc::now())
    }

    pub fn require_enabled_at(
        &self,
        kind: CollectiveActionKind,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let action = self.action(kind).ok_or_else(|| {
            format!(
                "the current wallet action mask did not advertise {}",
                kind.wire_name()
            )
        })?;
        if !action.enabled {
            return Err(format!(
                "the current wallet action mask blocked {} ({})",
                kind.wire_name(),
                action.blocking_code.as_deref().unwrap_or("UNKNOWN_BLOCKER")
            ));
        }
        if action.deadline.is_some_and(|deadline| deadline <= now) {
            return Err(format!(
                "the current wallet action mask deadline for {} has elapsed",
                kind.wire_name()
            ));
        }
        Ok(())
    }
}

pub fn validate_current_writer_action_mask(
    payload: &Value,
    expected_owner: &str,
    expected_sleeve: &str,
) -> Result<WriterActionMask, String> {
    let schema = action_schema();
    let vocabulary = schema["actions"]
        .as_array()
        .ok_or("Action vocabulary is missing")?;
    let encoded = serde_json::to_vec(vocabulary).map_err(|_| "Action vocabulary is malformed")?;
    let digest: String = solana_program::hash::hash(&encoded)
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if schema["sdkCommit"] != crate::current_release::SDK_PACKAGE_COMMIT
        || schema["semanticAbiVersion"] != SEMANTIC_ABI_VERSION
        || schema["vocabularySha256"] != digest
        || vocabulary.len() != CollectiveActionKind::ALL.len()
        || vocabulary
            .iter()
            .zip(CollectiveActionKind::ALL)
            .any(|(name, kind)| name.as_str() != Some(kind.wire_name()))
    {
        return Err("The packaged action contract does not match this release.".into());
    }
    let root = if payload.get("ok").is_some() {
        chain_identity::validate_current_backend_envelope(payload)
            .map_err(|_| "writer action mask protocol identity is not current".to_string())?;
        payload
            .get("data")
            .ok_or_else(|| "writer action mask data is missing".to_string())?
    } else {
        chain_identity::validate_current_protocol_data(payload)
            .map_err(|_| "writer action mask protocol identity is not current".to_string())?;
        payload
    };
    require_exact_keys(
        root,
        &[
            "protocol",
            "protocolIdentity",
            "semanticAbiVersion",
            "owner",
            "sleeve",
            "observedSlot",
            "observedAt",
            "stale",
            "tradeReady",
            "availableActions",
        ],
        "writer action mask",
    )?;
    if root.get("protocolIdentity") != root.get("protocol") {
        return Err("writer action mask repeats a mismatched protocol identity".to_string());
    }
    if root.get("semanticAbiVersion").and_then(Value::as_str) != Some(SEMANTIC_ABI_VERSION) {
        return Err("writer action mask semantic ABI is not current".to_string());
    }

    let expected_owner = canonical_pubkey(expected_owner, true, "expected owner")?;
    let expected_sleeve = canonical_pubkey(expected_sleeve, false, "expected sleeve")?;
    let owner = canonical_pubkey(
        root.get("owner")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        true,
        "action-mask owner",
    )?;
    let sleeve = canonical_pubkey(
        root.get("sleeve")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        false,
        "action-mask sleeve",
    )?;
    if owner != expected_owner {
        return Err("writer action mask owner does not match the attached wallet".to_string());
    }
    if sleeve != expected_sleeve {
        return Err("writer action mask sleeve does not match the selected sleeve".to_string());
    }

    let observed_slot = canonical_u64(root.get("observedSlot"), "observed slot")?;
    if observed_slot == 0 {
        return Err("writer action mask observed slot must be nonzero".to_string());
    }
    let observed_at = exact_iso(
        root.get("observedAt")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "observed time",
    )?;
    if root.get("stale") != Some(&Value::Bool(false)) {
        return Err("writer action mask is stale".to_string());
    }
    let trade_ready = root
        .get("tradeReady")
        .and_then(Value::as_bool)
        .ok_or_else(|| "writer action mask lacks explicit trade readiness".to_string())?;

    let raw_actions = root
        .get("availableActions")
        .and_then(Value::as_array)
        .filter(|actions| actions.len() == CollectiveActionKind::ALL.len())
        .ok_or_else(|| {
            "writer action mask must contain the exact 22-action vocabulary".to_string()
        })?;
    let mut available_actions = Vec::with_capacity(raw_actions.len());
    for (index, raw) in raw_actions.iter().enumerate() {
        let kind = CollectiveActionKind::ALL[index];
        require_exact_optional_keys(
            raw,
            &["kind", "enabled"],
            &["blockingCode", "deadline"],
            "writer action entry",
        )?;
        if raw.get("kind").and_then(Value::as_str) != Some(kind.wire_name()) {
            return Err(
                "writer action mask vocabulary is missing, reordered, or unknown".to_string(),
            );
        }
        let enabled = raw
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "writer action enabled flag is malformed".to_string())?;
        if enabled
            && !trade_ready
            && matches!(
                kind,
                CollectiveActionKind::TradeBuy | CollectiveActionKind::TradeSell
            )
        {
            return Err("trade action contradicts current trade readiness".to_string());
        }
        let blocking_code = raw
            .get("blockingCode")
            .map(|value| {
                value
                    .as_str()
                    .filter(|code| {
                        matches!(
                            *code,
                            "CURRENT_OBSERVATION_STALE"
                                | "ACTION_NOT_RELEASED"
                                | "TRADE_NOT_READY"
                                | "ACTION_STATE_BLOCKED"
                        )
                    })
                    .map(str::to_string)
                    .ok_or_else(|| "writer action blocking code is malformed".to_string())
            })
            .transpose()?;
        if enabled == blocking_code.is_some() {
            return Err("writer action enabled state contradicts its blocking code".to_string());
        }
        let deadline = raw
            .get("deadline")
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "writer action deadline is malformed".to_string())
                    .and_then(|value| exact_iso(value, "action deadline"))
            })
            .transpose()?;
        available_actions.push(AvailableAction {
            kind,
            enabled,
            blocking_code,
            deadline,
        });
    }

    Ok(WriterActionMask {
        owner,
        sleeve,
        observed_slot,
        observed_at,
        available_actions,
    })
}

pub fn render_writer_action_mask(mask: &WriterActionMask) -> String {
    let mut lines = vec![
        "Wallet-specific writer actions".to_string(),
        format!("Owner: {}", mask.owner),
        format!("Sleeve: {}", mask.sleeve),
        format!(
            "Finalized observation: slot {} | {}",
            mask.observed_slot,
            mask.observed_at
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        ),
        format!("Semantic ABI: {SEMANTIC_ABI_VERSION}"),
    ];
    for action in &mask.available_actions {
        let status = if action.enabled {
            "enabled".to_string()
        } else {
            format!(
                "disabled ({})",
                action.blocking_code.as_deref().unwrap_or("UNKNOWN_BLOCKER")
            )
        };
        let deadline = action
            .deadline
            .map(|value| {
                format!(
                    " | deadline {}",
                    value.to_rfc3339_opts(SecondsFormat::Millis, true)
                )
            })
            .unwrap_or_default();
        lines.push(format!("{}: {status}{deadline}", action.kind.wire_name()));
    }
    lines.push(
        "This read is wallet-specific availability, not transaction authorization. Every signed action still requires a fresh admitted SDK plan and finalized re-observation."
            .to_string(),
    );
    lines.join("\n")
}

fn canonical_pubkey(raw: &str, require_on_curve: bool, label: &str) -> Result<String, String> {
    let parsed = raw
        .parse::<Pubkey>()
        .map_err(|_| format!("writer action mask {label} is not a canonical address"))?;
    if parsed == Pubkey::default()
        || parsed.to_string() != raw
        || (require_on_curve && !parsed.is_on_curve())
    {
        return Err(format!(
            "writer action mask {label} is not a canonical{} address",
            if require_on_curve { " on-curve" } else { "" }
        ));
    }
    Ok(raw.to_string())
}

fn canonical_u64(value: Option<&Value>, label: &str) -> Result<u64, String> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("writer action mask {label} is malformed"))?;
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("writer action mask {label} is malformed"));
    }
    let parsed = raw
        .parse::<u64>()
        .map_err(|_| format!("writer action mask {label} is outside the u64 range"))?;
    if parsed.to_string() != raw {
        return Err(format!("writer action mask {label} is not canonical"));
    }
    Ok(parsed)
}

fn exact_iso(raw: &str, label: &str) -> Result<DateTime<Utc>, String> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|_| format!("writer action mask {label} is not an exact ISO timestamp"))?
        .with_timezone(&Utc);
    let canonical = parsed.to_rfc3339_opts(SecondsFormat::Millis, true);
    let without_zero_millis = canonical
        .strip_suffix(".000Z")
        .map(|prefix| format!("{prefix}Z"));
    if raw != canonical && without_zero_millis.as_deref() != Some(raw) {
        return Err(format!(
            "writer action mask {label} is not an exact ISO timestamp"
        ));
    }
    Ok(parsed)
}

fn require_exact_keys(value: &Value, keys: &[&str], label: &str) -> Result<(), String> {
    require_exact_optional_keys(value, keys, &[], label)
}

fn require_exact_optional_keys(
    value: &Value,
    required: &[&str],
    optional: &[&str],
    label: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} is not an object"))?;
    let allowed = required
        .iter()
        .chain(optional)
        .copied()
        .collect::<HashSet<_>>();
    if required.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| !allowed.contains(key.as_str()))
    {
        return Err(format!("{label} has malformed fields"));
    }
    Ok(())
}
