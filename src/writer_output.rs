//! Plain-text projections for current collective-writer responses.
//!
//! These renderers deliberately display only fields returned by the current
//! Amoeba response. They do not derive yield, headroom, reserve, withdrawal,
//! liability, or close eligibility. Missing fields remain visibly unavailable.

use std::collections::HashSet;

use serde_json::Value;
use solana_pubkey::Pubkey;

use crate::backend::terminal_safe_text;

const UNAVAILABLE: &str = "unavailable";
const MAX_WRITER_DISPLAY_FIELD_CHARS: usize = 512;
const WRITER_CLOSE_CAPABILITY_SCHEMA: &str = "ameba.edge.capabilities.v1";
const WRITER_CLOSE_SPREAD_RELEASE: &str = "v0.1.0-rc.44";
const WRITER_CLOSE_SPREAD_COMMIT: &str = "1b2230d96e51f6582155d8284900fbfc11ff1f18";
const WRITER_CLOSE_PROTOCOL_PLAN_SDK_COMMIT: &str = "b2cd10739ecb9419115980425920d6b576caf78e";
const WRITER_CLOSE_SDK_PACKAGE_COMMIT: &str = "a21b324a7a64da87046c7650355b80ea20c47540";
const WRITER_CLOSE_SDK_CONTRACT: &str = "writer-operation-plan-v2";
const WRITER_WALLET_ACTION_ABI: &str = "collective-operations-v1";
const WRITER_WALLET_ACTION_ROUTE: (&str, &str) = (
    "GET",
    "/dlmm/writer-sleeves/:sleeve/available-actions?owner=:owner",
);

pub(crate) const WRITER_CLOSE_CANCEL_ACTION_MASK_UNAVAILABLE: &str = "Writer-close cancellation is supported by the release runtime but is unavailable in this Petri release because collective-operations-v1 has no wallet-specific close-cancel action. Petri will not alias cancellation to close continuation; nothing was prepared, signed, or sent.";

/// Validate the exact current writer-close capability contract shared by the
/// CLI/MCP read and the Writers TUI. A capability may honestly be disabled,
/// but stale identity, malformed shape, or contradictory reason codes fail.
pub(crate) fn validate_current_writer_close_capabilities(payload: &Value) -> Result<(), String> {
    let root = unwrap_data(payload);
    if root.get("schema").and_then(Value::as_str) != Some(WRITER_CLOSE_CAPABILITY_SCHEMA) {
        return Err("writer-close capability schema is not current".to_string());
    }

    let protocol = root
        .get("protocol")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close capability protocol identity is missing".to_string())?;
    if protocol.get("releaseTag").and_then(Value::as_str) != Some(WRITER_CLOSE_SPREAD_RELEASE)
        || protocol.get("releaseCommit").and_then(Value::as_str) != Some(WRITER_CLOSE_SPREAD_COMMIT)
    {
        return Err("writer-close capability protocol identity is not current".to_string());
    }

    let deployment = root
        .get("deployment")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close deployment identity is missing".to_string())?;
    if deployment.get("spreadRelease").and_then(Value::as_str) != Some(WRITER_CLOSE_SPREAD_RELEASE)
        || deployment.get("spreadSourceCommit").and_then(Value::as_str)
            != Some(WRITER_CLOSE_SPREAD_COMMIT)
        || deployment.get("sdkGitCommit").and_then(Value::as_str)
            != Some(WRITER_CLOSE_PROTOCOL_PLAN_SDK_COMMIT)
        || deployment
            .get("protocolPlanSdkCommit")
            .and_then(Value::as_str)
            != Some(WRITER_CLOSE_PROTOCOL_PLAN_SDK_COMMIT)
        || deployment
            .get("sdkPackageGitCommit")
            .and_then(Value::as_str)
            != Some(WRITER_CLOSE_SDK_PACKAGE_COMMIT)
        || deployment.get("sdkContract").and_then(Value::as_str) != Some(WRITER_CLOSE_SDK_CONTRACT)
    {
        return Err("writer-close deployment identity is not current".to_string());
    }

    let capabilities = root
        .get("capabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close capabilities are missing".to_string())?;
    let lifecycle = capabilities
        .get("writer.close.lifecycle")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close lifecycle capability is missing".to_string())?;
    if lifecycle.get("implementation").and_then(Value::as_str) != Some("supported") {
        return Err("writer-close implementation capability is not current".to_string());
    }
    validate_capability_runtime(lifecycle, "lifecycle")?;

    let modes = lifecycle
        .get("lightAccountModes")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer-close custody modes are missing".to_string())?;
    for label in ["hot", "cold"] {
        let mode = modes
            .get(label)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("writer-close {label} custody capability is missing"))?;
        validate_capability_runtime(mode, &format!("{label} custody"))?;
    }

    validate_bounded_string_array(lifecycle.get("operations"), "operations", 16)?;
    let routes = lifecycle
        .get("routes")
        .and_then(Value::as_array)
        .filter(|routes| routes.len() <= 32)
        .ok_or_else(|| "writer-close routes are malformed".to_string())?;
    let mut seen_routes = HashSet::new();
    for route in routes {
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
        if !seen_routes.insert((method, path)) {
            return Err("writer-close routes contain a duplicate".to_string());
        }
    }

    let wallet_actions = capabilities
        .get("wallet.action.mask")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer wallet-action capability is missing".to_string())?;
    if wallet_actions.get("implementation").and_then(Value::as_str) != Some("supported") {
        return Err("writer wallet-action capability is not current".to_string());
    }
    validate_capability_runtime(wallet_actions, "wallet action-mask")?;
    if wallet_actions
        .get("semanticAbiVersion")
        .and_then(Value::as_str)
        != Some(WRITER_WALLET_ACTION_ABI)
    {
        return Err("writer wallet-action ABI is not current".to_string());
    }
    let wallet_route = wallet_actions
        .get("route")
        .and_then(Value::as_object)
        .ok_or_else(|| "writer wallet-action route is missing".to_string())?;
    if wallet_route.get("method").and_then(Value::as_str) != Some(WRITER_WALLET_ACTION_ROUTE.0)
        || wallet_route.get("path").and_then(Value::as_str) != Some(WRITER_WALLET_ACTION_ROUTE.1)
    {
        return Err("writer wallet-action route is not current".to_string());
    }
    Ok(())
}

fn validate_capability_runtime(
    capability: &serde_json::Map<String, Value>,
    label: &str,
) -> Result<(), String> {
    let enabled = match capability.get("runtime").and_then(Value::as_str) {
        Some("enabled") => true,
        Some("disabled") => false,
        _ => return Err(format!("writer-close {label} runtime is malformed")),
    };
    let reasons = validate_bounded_string_array(capability.get("reasonCodes"), "reason codes", 16)?;
    if enabled != reasons.is_empty() {
        return Err(format!(
            "writer-close {label} runtime and reason codes contradict each other"
        ));
    }
    Ok(())
}

fn validate_bounded_string_array(
    value: Option<&Value>,
    label: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    let values = value
        .and_then(Value::as_array)
        .filter(|values| values.len() <= limit)
        .ok_or_else(|| format!("writer-close {label} are malformed"))?;
    let mut parsed = Vec::with_capacity(values.len());
    let mut seen = HashSet::new();
    for value in values {
        let value = value
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .ok_or_else(|| format!("writer-close {label} are malformed"))?;
        if !seen.insert(value) {
            return Err(format!("writer-close {label} contain a duplicate"));
        }
        parsed.push(value.to_string());
    }
    Ok(parsed)
}

/// Render the deployed writer-close runtime and custody-mode projection.
pub fn render_writer_capabilities(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let deployment = root.get("deployment").unwrap_or(&Value::Null);
    let lifecycle = root
        .get("capabilities")
        .and_then(|value| value.get("writer.close.lifecycle"))
        .unwrap_or(&Value::Null);
    let modes = lifecycle.get("lightAccountModes").unwrap_or(&Value::Null);
    let hot = modes.get("hot").unwrap_or(&Value::Null);
    let cold = modes.get("cold").unwrap_or(&Value::Null);
    let wallet_actions = root
        .get("capabilities")
        .and_then(|value| value.get("wallet.action.mask"))
        .unwrap_or(&Value::Null);

    [
        "Writer close capabilities".to_string(),
        format!(
            "Release: {} | Spread: {}",
            field(deployment, "spreadRelease"),
            field(deployment, "spreadSourceCommit")
        ),
        format!(
            "SDK package: {} | Protocol plans: {}",
            field(deployment, "sdkPackageGitCommit"),
            field(deployment, "protocolPlanSdkCommit")
        ),
        format!("SDK contract: {}", field(deployment, "sdkContract")),
        format!(
            "Wallet action mask: {} / {} | ABI: {}",
            field(wallet_actions, "implementation"),
            field(wallet_actions, "runtime"),
            field(wallet_actions, "semanticAbiVersion")
        ),
        format!(
            "Lifecycle: {} / {}",
            field(lifecycle, "implementation"),
            field(lifecycle, "runtime")
        ),
        format!(
            "Hot Light accounts: {}{}",
            field(hot, "runtime"),
            reason_suffix(hot)
        ),
        format!(
            "Cold Light accounts: {}{}",
            field(cold, "runtime"),
            reason_suffix(cold)
        ),
        format!("Operations: {}", array_field(lifecycle, "operations")),
        "This release read needs no wallet and does not authorize an action. Before review or signing, Petri separately requires the exact current owner+sleeve action mask, then validates the SDK plan and fresh chain state."
            .to_string(),
    ]
    .join("\n")
}

/// Render the global current collective-writer sleeve catalog.
pub fn render_writer_sleeves(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let sleeves = root
        .as_array()
        .or_else(|| root.get("sleeves").and_then(Value::as_array));
    let mut lines = vec!["Collective writer sleeves".to_string()];

    let Some(sleeves) = sleeves else {
        lines.push("Sleeves: unavailable".to_string());
        return lines.join("\n");
    };
    if sleeves.is_empty() {
        lines.push("No collective writer sleeves were returned.".to_string());
        return lines.join("\n");
    }

    for (index, sleeve) in sleeves.iter().enumerate() {
        lines.push(String::new());
        lines.push(format!(
            "{}. Sleeve {}",
            index + 1,
            short(&field(sleeve, "address"), 24)
        ));
        lines.push(format!(
            "   Status: {} | Expiry (Unix): {} | Series: {}",
            field(sleeve, "status"),
            field(sleeve, "expiryTs"),
            field(sleeve, "seriesCount")
        ));
        lines.push(format!(
            "   Writer principal atoms: {} | Locked primary premium atoms: {}",
            field(sleeve, "writerPrincipalAtoms"),
            field(sleeve, "lockedPrimaryPremiumAtoms")
        ));
        lines.push(format!(
            "   Accounted asset atoms: {} | Exact reserve atoms: {}",
            field(sleeve, "accountedAssetAtoms"),
            field(sleeve, "exactReserveAtoms")
        ));
        lines.push(format!(
            "   Flat par supply atoms: {} | Security exposure atoms: {}",
            field(sleeve, "flatParSupplyAtoms"),
            field(sleeve, "securityExposureAtoms")
        ));
        lines.push(format!(
            "   Policy version: {} | Security mode: {}",
            field(sleeve, "policyVersion"),
            field(sleeve, "securityMode")
        ));
        lines.push(format!(
            "   Active auction: {} | Active close request: {}",
            short(&nullable_field(sleeve, "activeAuction"), 24),
            short(&nullable_field(sleeve, "activeCloseRequest"), 24)
        ));
    }

    lines.join("\n")
}

/// Render one current collective-writer sleeve and its registered series.
pub fn render_writer_sleeve(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let sleeve = nested_or_self(root, "sleeve", &["address", "writerPrincipalAtoms"]);
    let group = root
        .get("settlementGroup")
        .filter(|value| value.is_object());
    let book = root.get("seriesBook").filter(|value| value.is_object());
    let policy = root.get("policySnapshot").filter(|value| value.is_object());
    let sleeve_value = sleeve.unwrap_or(&Value::Null);
    let group_value = group.unwrap_or(&Value::Null);
    let book_value = book.unwrap_or(&Value::Null);
    let policy_value = policy.unwrap_or(&Value::Null);

    let mut lines = vec![format!(
        "Collective writer sleeve | {}",
        field(sleeve_value, "address")
    )];
    push_sleeve_summary(&mut lines, sleeve_value);

    lines.push(String::new());
    lines.push("Settlement and oracle evidence".to_string());
    lines.push(format!(
        "Group status: {} | Settlement (Unix): {} | Settlement price atomic: {}",
        field(group_value, "status"),
        field(group_value, "settlementTs"),
        field(group_value, "settlementPriceAtomic")
    ));
    lines.push(format!(
        "Security cap atoms: {} | Finalized slot: {}",
        field(group_value, "securityCapAtoms"),
        field(group_value, "finalizedSlot")
    ));
    lines.push(format!("Recipe hash: {}", field(group_value, "recipeHash")));
    lines.push(format!(
        "Settlement source digest: {}",
        field(group_value, "settlementSourceDigest")
    ));
    lines.push(format!(
        "Active-weight manifest hash: {}",
        field(group_value, "activeWeightManifestHash")
    ));

    push_series(&mut lines, book_value);
    push_policy_summary(&mut lines, policy_value);
    lines.join("\n")
}

/// Render the current policy snapshot returned by the writer policy-audit read.
pub fn render_writer_policy_audit(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let sleeve =
        nested_or_self(root, "sleeve", &["address", "policyVersion"]).unwrap_or(&Value::Null);
    let policy = root
        .get("policySnapshot")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let group = root
        .get("settlementGroup")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);

    let mut lines = vec![format!(
        "Writer policy audit | sleeve={}",
        field(sleeve, "address")
    )];
    lines.push(format!(
        "Sleeve status: {} | Expiry (Unix): {} | Security mode: {}",
        field(sleeve, "status"),
        field(sleeve, "expiryTs"),
        field(sleeve, "securityMode")
    ));
    lines.push(format!(
        "Sleeve policy version: {} | Policy hash: {}",
        field(sleeve, "policyVersion"),
        field(sleeve, "policyHash")
    ));
    lines.push(format!(
        "Scenario-set hash: {}",
        field(sleeve, "scenarioSetHash")
    ));
    lines.push(format!(
        "Risk-limit hash: {}",
        field(sleeve, "riskLimitHash")
    ));
    push_policy_details(&mut lines, policy);
    lines.push(String::new());
    lines.push("Settlement-source evidence".to_string());
    lines.push(format!("Recipe hash: {}", field(group, "recipeHash")));
    lines.push(format!(
        "Settlement source digest: {}",
        field(group, "settlementSourceDigest")
    ));
    lines.push(format!(
        "Active-weight manifest hash: {}",
        field(group, "activeWeightManifestHash")
    ));
    lines.join("\n")
}

/// Render an authoritative current close preview without recomputing its math.
pub fn render_writer_close_preview(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let semantic = root
        .get("semantic")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let observation = root
        .get("currentObservation")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let preview = root
        .get("preview")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let commitments = preview
        .get("commitments")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);

    let mut lines = vec![format!(
        "Writer close preview | sleeve={}",
        field(semantic, "sleeve")
    )];
    lines.push(format!(
        "Requested Flat atoms: {} | Minimum withdrawal atoms: {}",
        field(semantic, "flatParAtoms"),
        field(semantic, "minimumWithdrawalAtoms")
    ));
    lines.push(format!(
        "Preview accepted: {} | Finalized observation slot: {}",
        field(preview, "ok"),
        first_field(observation, &["observedAtSlot", "finalizedSlot"])
    ));
    lines.push(format!(
        "Withdrawal atoms: {} | Retained writer surplus atoms: {}",
        field(preview, "withdrawal"),
        field(preview, "retainedWriterSurplus")
    ));
    lines.push(format!(
        "Reserve atoms: before {} | after {} | reduction {}",
        field(preview, "reserveBefore"),
        field(preview, "reserveAfter"),
        field(preview, "reserveReduction")
    ));
    lines.push(format!(
        "Lower-tail reserve atoms: before {} | after {}",
        field(preview, "lowerTailReserveBefore"),
        field(preview, "lowerTailReserveAfter")
    ));
    lines.push(format!(
        "Upper-tail reserve atoms: before {} | after {}",
        field(preview, "upperTailReserveBefore"),
        field(preview, "upperTailReserveAfter")
    ));
    lines.push(format!(
        "Security after atoms: {} | Minimum safe withdrawal atoms: {}",
        field(preview, "securityAfter"),
        field(preview, "minimumSafeWithdrawal")
    ));
    lines.push(format!(
        "Required claim atoms by series: {}",
        array_field(preview, "required")
    ));
    lines.push(format!(
        "Remaining external open interest by series: {}",
        array_field(preview, "remainingExternalOi")
    ));
    lines.push(format!(
        "Candidate settlements atomic: {}",
        array_field(preview, "candidateSettlements")
    ));
    lines.push(format!(
        "Candidate safe withdrawals atoms: {}",
        array_field(preview, "candidateSafeWithdrawals")
    ));
    lines.push(format!(
        "Binding settlement atomic: {} | Candidate count: {}",
        field(preview, "statewiseBindingSettlement"),
        field(preview, "candidateCount")
    ));
    lines.push(format!(
        "Commitments: group {} | book {} | policy {} | deployment {}",
        field(commitments, "group"),
        field(commitments, "book"),
        field(commitments, "policy"),
        field(commitments, "deployment")
    ));
    lines.join("\n")
}

/// Render one staged close request and the user intents permitted by its
/// returned status. Lean still selects the exact bounded instruction.
pub fn render_writer_close_status(payload: &Value) -> String {
    let root = unwrap_data(payload);
    let sleeve = root
        .get("sleeve")
        .filter(|value| value.is_object())
        .unwrap_or(&Value::Null);
    let request = nested_or_self(root, "closeRequest", &["requestNonce", "flatAmountAtoms"])
        .unwrap_or(&Value::Null);

    let mut lines = vec![format!(
        "Writer close status | request={}",
        field(request, "address")
    )];
    lines.push(format!(
        "Request status: {} | Sleeve status: {}",
        field(request, "status"),
        field(sleeve, "status")
    ));
    lines.push(format!(
        "Sleeve: {} | Owner: {} | Request nonce: {}",
        field(request, "sleeve"),
        field(request, "owner"),
        field(request, "requestNonce")
    ));
    lines.push(format!(
        "Flat amount atoms: {} | Minimum withdrawal atoms: {} | Final withdrawal atoms: {}",
        field(request, "flatAmountAtoms"),
        field(request, "minimumWithdrawalAtoms"),
        field(request, "finalWithdrawalAtoms")
    ));
    lines.push(format!(
        "Deadline (Unix): {} | Series count: {}",
        field(request, "deadlineTs"),
        field(request, "seriesCount")
    ));
    lines.push(format!(
        "Next deposit index: {} | Next cancellation index: {}",
        field(request, "nextDepositIndex"),
        field(request, "nextCancelIndex")
    ));
    lines.push(format!(
        "Required claim atoms by series: {}",
        array_field(request, "requiredClaimAtoms")
    ));
    lines.push(format!(
        "Deposited claim atoms by series: {}",
        array_field(request, "depositedClaimAtoms")
    ));
    lines.push(format!(
        "Snapshot external open interest by series: {}",
        array_field(request, "snapshotExternalOiAtoms")
    ));
    lines.push(format!(
        "Snapshot asset atoms: {} | Snapshot reserve atoms: {}",
        field(request, "snapshotAssetAtoms"),
        field(request, "snapshotReserveAtoms")
    ));
    lines.push(format!(
        "Snapshot writer principal atoms: {} | Snapshot locked premium atoms: {}",
        field(request, "snapshotWriterPrincipalAtoms"),
        field(request, "snapshotLockedPrimaryPremiumAtoms")
    ));
    lines.push(format!(
        "Snapshot Flat supply atoms: {} | Snapshot security exposure atoms: {}",
        field(request, "snapshotFlatSupplyAtoms"),
        field(request, "snapshotSecurityExposureAtoms")
    ));
    lines.push(format!(
        "Snapshot policy version: {} | Finalized slot: {} | Last updated slot: {}",
        field(request, "snapshotPolicyVersion"),
        field(request, "finalizedSlot"),
        field(request, "lastUpdatedSlot")
    ));
    lines.push(close_next_stage(request));
    lines.join("\n")
}

fn push_sleeve_summary(lines: &mut Vec<String>, sleeve: &Value) {
    lines.push(format!(
        "Status: {} | Expiry (Unix): {} | Series: {}",
        field(sleeve, "status"),
        field(sleeve, "expiryTs"),
        field(sleeve, "seriesCount")
    ));
    lines.push(format!(
        "Writer principal atoms: {} | Locked primary premium atoms: {}",
        field(sleeve, "writerPrincipalAtoms"),
        field(sleeve, "lockedPrimaryPremiumAtoms")
    ));
    lines.push(format!(
        "Accounted asset atoms: {} | Exact reserve atoms: {}",
        field(sleeve, "accountedAssetAtoms"),
        field(sleeve, "exactReserveAtoms")
    ));
    lines.push(format!(
        "Lower-tail reserve atoms: {} | Upper-tail reserve atoms: {}",
        field(sleeve, "lowerTailReserveAtoms"),
        field(sleeve, "upperTailReserveAtoms")
    ));
    lines.push(format!(
        "Flat par supply atoms: {} | Security exposure atoms: {} | Operational buffer atoms: {}",
        field(sleeve, "flatParSupplyAtoms"),
        field(sleeve, "securityExposureAtoms"),
        field(sleeve, "operationalBufferAtoms")
    ));
    lines.push(format!(
        "Long liability atoms: initial {} | remaining {}",
        field(sleeve, "longLiabilityInitialAtoms"),
        field(sleeve, "longLiabilityRemainingAtoms")
    ));
    lines.push(format!(
        "Flat residual atoms: initial {} | remaining {}",
        field(sleeve, "flatResidualInitialAtoms"),
        field(sleeve, "flatResidualRemainingAtoms")
    ));
    lines.push(format!(
        "Policy version: {} | Security mode: {}",
        field(sleeve, "policyVersion"),
        field(sleeve, "securityMode")
    ));
    lines.push(format!(
        "Active auction: {} | Active close request: {}",
        nullable_field(sleeve, "activeAuction"),
        nullable_field(sleeve, "activeCloseRequest")
    ));
}

fn push_series(lines: &mut Vec<String>, book: &Value) {
    lines.push(String::new());
    lines.push(format!(
        "Registered series | declared={} | frozen={}",
        field(book, "seriesCount"),
        field(book, "frozen")
    ));
    let Some(records) = book.get("records").and_then(Value::as_array) else {
        lines.push("Series records: unavailable".to_string());
        return;
    };
    if records.is_empty() {
        lines.push("No series records were returned.".to_string());
        return;
    }
    for (index, record) in records.iter().enumerate() {
        lines.push(format!(
            "Series {index} | active={} | kind={} | custody={} | settlement={}",
            field(record, "active"),
            field(record, "optionKind"),
            field(record, "custodyStatus"),
            field(record, "settlementStatus")
        ));
        lines.push(format!(
            "  Strike atomic: {} | Cap/floor atomic: {} | Contract size atoms: {}",
            field(record, "strikePriceAtomic"),
            field(record, "capOrFloorPriceAtomic"),
            field(record, "contractSizeAtoms")
        ));
        lines.push(format!(
            "  Maximum payout per contract atoms: {} | External open interest atoms: {}",
            field(record, "maxPayoutPerContractAtoms"),
            field(record, "externalOpenInterestAtoms")
        ));
        lines.push(format!(
            "  Total physical supply atoms: {} | Issuer-controlled atoms: {}",
            field(record, "totalPhysicalSupplyAtoms"),
            field(record, "issuerControlledAtoms")
        ));
        lines.push(format!(
            "  Primary premium collected atoms: {} | Settlement liability atoms: initial {} | remaining {}",
            field(record, "primaryPremiumCollectedAtoms"),
            field(record, "settlementLiabilityInitialAtoms"),
            field(record, "settlementLiabilityRemainingAtoms")
        ));
    }
}

fn push_policy_summary(lines: &mut Vec<String>, policy: &Value) {
    lines.push(String::new());
    lines.push("Policy limits".to_string());
    lines.push(format!(
        "Version: {} | Security mode: {} | Primary fee bps: {}",
        field(policy, "policyVersion"),
        field(policy, "securityMode"),
        field(policy, "primaryFeeBps")
    ));
    lines.push(format!(
        "Maximum auction issue atoms: {} | Maximum close Flat atoms: {}",
        field(policy, "maxAuctionIssueAtoms"),
        field(policy, "maxCloseFlatAtoms")
    ));
    lines.push(format!(
        "Lower-tail maximum settlement atomic: {} | Upper-tail minimum settlement atomic: {}",
        field(policy, "lowerTailMaxSettlementAtomic"),
        field(policy, "upperTailMinSettlementAtomic")
    ));
    lines.push(format!("Policy hash: {}", field(policy, "policyHash")));
}

fn push_policy_details(lines: &mut Vec<String>, policy: &Value) {
    lines.push(String::new());
    lines.push(format!(
        "Policy snapshot | address={}",
        field(policy, "address")
    ));
    lines.push(format!(
        "Version: {} | Regime input version: {} | Security mode: {}",
        field(policy, "policyVersion"),
        field(policy, "regimeInputVersion"),
        field(policy, "securityMode")
    ));
    lines.push(format!(
        "Primary fee bps: {} | Drawdown scale: {}",
        field(policy, "primaryFeeBps"),
        field(policy, "drawdownScale")
    ));
    lines.push(format!(
        "Drawdown limits: worst {} | upper {} | lower {}",
        field(policy, "worstDrawdownLimit"),
        field(policy, "upperDrawdownLimit"),
        field(policy, "lowerDrawdownLimit")
    ));
    lines.push(format!(
        "Tail settlement bounds atomic: lower maximum {} | upper minimum {}",
        field(policy, "lowerTailMaxSettlementAtomic"),
        field(policy, "upperTailMinSettlementAtomic")
    ));
    lines.push(format!(
        "Operational buffer atoms: {} | Maximum auction issue atoms: {} | Maximum close Flat atoms: {}",
        field(policy, "operationalBufferAtoms"),
        field(policy, "maxAuctionIssueAtoms"),
        field(policy, "maxCloseFlatAtoms")
    ));
    lines.push(format!(
        "Created slot: {} | Sealed slot: {}",
        field(policy, "createdSlot"),
        field(policy, "sealedSlot")
    ));
    lines.push(format!("Policy hash: {}", field(policy, "policyHash")));
    lines.push(format!(
        "Scenario-set hash: {}",
        field(policy, "scenarioSetHash")
    ));
    lines.push(format!(
        "Risk-limit hash: {}",
        field(policy, "riskLimitHash")
    ));
    lines.push(format!(
        "Series-family hash: {}",
        field(policy, "seriesFamilyHash")
    ));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurrentCloseStatus {
    Collecting,
    Complete,
    Finalized,
    Cancelling,
    Cancelled,
}

struct CurrentCloseAction<'a> {
    status: CurrentCloseStatus,
    address: &'a str,
    owner: &'a str,
    deadline: &'a str,
    next_deposit_index: usize,
    next_cancel_index: usize,
}

const CURRENT_CLOSE_REQUEST_FIELDS: [&str; 29] = [
    "address",
    "bump",
    "sleeve",
    "owner",
    "flatEscrow",
    "flatMint",
    "requestNonce",
    "flatAmountAtoms",
    "minimumWithdrawalAtoms",
    "snapshotAssetAtoms",
    "snapshotReserveAtoms",
    "snapshotWriterPrincipalAtoms",
    "snapshotLockedPrimaryPremiumAtoms",
    "snapshotFlatSupplyAtoms",
    "snapshotSecurityExposureAtoms",
    "snapshotPolicyVersion",
    "snapshotGroupCommitment",
    "snapshotBookDigest",
    "deadlineTs",
    "status",
    "seriesCount",
    "nextDepositIndex",
    "nextCancelIndex",
    "requiredClaimAtoms",
    "depositedClaimAtoms",
    "snapshotExternalOiAtoms",
    "finalWithdrawalAtoms",
    "finalizedSlot",
    "lastUpdatedSlot",
];

fn canonical_current_pubkey(value: Option<&Value>) -> Option<&str> {
    let raw = value?.as_str()?;
    let pubkey = raw.parse::<Pubkey>().ok()?;
    (pubkey != Pubkey::default() && pubkey.to_string() == raw).then_some(raw)
}

fn canonical_current_u64(value: Option<&Value>, positive: bool) -> Option<u64> {
    let raw = value?.as_str()?;
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let value = raw.parse::<u64>().ok()?;
    (!positive || value > 0).then_some(value)
}

fn current_close_amounts(value: Option<&Value>, series_count: usize) -> Option<Vec<u64>> {
    let values = value?.as_array()?;
    (values.len() == series_count).then(|| {
        values
            .iter()
            .map(|value| canonical_current_u64(Some(value), false))
            .collect::<Option<Vec<_>>>()
    })?
}

fn current_close_status(value: Option<&Value>) -> Option<CurrentCloseStatus> {
    match value?.as_str()? {
        "Collecting" => Some(CurrentCloseStatus::Collecting),
        "Complete" => Some(CurrentCloseStatus::Complete),
        "Finalized" => Some(CurrentCloseStatus::Finalized),
        "Cancelling" => Some(CurrentCloseStatus::Cancelling),
        "Cancelled" => Some(CurrentCloseStatus::Cancelled),
        _ => None,
    }
}

fn current_close_action(request: &Value) -> Option<CurrentCloseAction<'_>> {
    let object = request.as_object()?;
    if object.len() != CURRENT_CLOSE_REQUEST_FIELDS.len()
        || CURRENT_CLOSE_REQUEST_FIELDS
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return None;
    }

    let address = canonical_current_pubkey(request.get("address"))?;
    let _sleeve = canonical_current_pubkey(request.get("sleeve"))?;
    let owner = canonical_current_pubkey(request.get("owner"))?;
    let _flat_escrow = canonical_current_pubkey(request.get("flatEscrow"))?;
    let _flat_mint = canonical_current_pubkey(request.get("flatMint"))?;
    if request.get("bump")?.as_u64()? > u8::MAX.into() {
        return None;
    }

    canonical_current_u64(request.get("requestNonce"), true)?;
    canonical_current_u64(request.get("flatAmountAtoms"), true)?;
    for field in [
        "minimumWithdrawalAtoms",
        "snapshotAssetAtoms",
        "snapshotReserveAtoms",
        "snapshotWriterPrincipalAtoms",
        "snapshotLockedPrimaryPremiumAtoms",
        "snapshotFlatSupplyAtoms",
        "snapshotSecurityExposureAtoms",
        "snapshotPolicyVersion",
        "finalWithdrawalAtoms",
        "lastUpdatedSlot",
    ] {
        canonical_current_u64(request.get(field), false)?;
    }
    let finalized_slot = canonical_current_u64(request.get("finalizedSlot"), false)?;
    let deadline = request.get("deadlineTs")?.as_str()?;
    canonical_current_u64(request.get("deadlineTs"), true)?;
    for field in ["snapshotGroupCommitment", "snapshotBookDigest"] {
        let digest = request.get(field)?.as_str()?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return None;
        }
    }

    let series_count = usize::try_from(request.get("seriesCount")?.as_u64()?).ok()?;
    if !(1..=ameba_sdk::constants::WRITER_MAX_LIVE_SERIES).contains(&series_count) {
        return None;
    }
    let next_deposit_index = usize::try_from(request.get("nextDepositIndex")?.as_u64()?).ok()?;
    let next_cancel_index = usize::try_from(request.get("nextCancelIndex")?.as_u64()?).ok()?;
    if next_deposit_index > series_count || next_cancel_index > series_count {
        return None;
    }

    let required = current_close_amounts(request.get("requiredClaimAtoms"), series_count)?;
    let deposited = current_close_amounts(request.get("depositedClaimAtoms"), series_count)?;
    current_close_amounts(request.get("snapshotExternalOiAtoms"), series_count)?;
    let status = current_close_status(request.get("status"))?;
    let deposit_is_zero_or_exact = deposited
        .iter()
        .zip(&required)
        .all(|(deposited, required)| *deposited == 0 || deposited == required);
    let finalized = finalized_slot > 0;
    let status_consistent = match status {
        CurrentCloseStatus::Collecting => {
            next_deposit_index < series_count
                && next_cancel_index == 0
                && !finalized
                && required[next_deposit_index] > 0
                && deposited[..next_deposit_index] == required[..next_deposit_index]
                && deposited[next_deposit_index..]
                    .iter()
                    .all(|amount| *amount == 0)
        }
        CurrentCloseStatus::Complete => {
            next_deposit_index == series_count
                && next_cancel_index == 0
                && !finalized
                && deposited == required
        }
        CurrentCloseStatus::Finalized => {
            next_deposit_index == series_count
                && next_cancel_index == 0
                && finalized
                && deposited == required
        }
        CurrentCloseStatus::Cancelling => {
            next_cancel_index > 0
                && !finalized
                && deposit_is_zero_or_exact
                && deposited[next_deposit_index..]
                    .iter()
                    .all(|amount| *amount == 0)
                && deposited[..next_cancel_index]
                    .iter()
                    .all(|amount| *amount == 0)
                && (next_cancel_index == series_count || deposited[next_cancel_index] > 0)
        }
        CurrentCloseStatus::Cancelled => {
            next_cancel_index == series_count
                && !finalized
                && deposited.iter().all(|amount| *amount == 0)
        }
    };
    status_consistent.then_some(CurrentCloseAction {
        status,
        address,
        owner,
        deadline,
        next_deposit_index,
        next_cancel_index,
    })
}

pub(crate) fn validate_current_writer_close_status(payload: &Value) -> Result<(), &'static str> {
    let root = unwrap_data(payload);
    let request = nested_or_self(root, "closeRequest", &["requestNonce", "flatAmountAtoms"])
        .ok_or("writer close status response is missing the exact current close-request DTO")?;
    current_close_action(request)
        .map(|_| ())
        .ok_or("writer close status response is not the exact current close-request DTO")
}

fn close_next_stage(request: &Value) -> String {
    let Some(current) = current_close_action(request) else {
        return "Next stage: unavailable".to_string();
    };

    let address = current.address;
    let owner = current.owner;
    let deadline = current.deadline;
    if current.status == CurrentCloseStatus::Collecting {
        return format!(
            "Available forward action: eligibility is conditional on the finalized Clock and signer. Through and including deadline {deadline}, only request owner {owner} may advance one Lean-selected basket step (next reported series index {}) with `petri writers close --close-request {address}`. Cancellation is supported by the protocol runtime but unavailable in this Petri release because the current wallet action-mask ABI has no close-cancel kind; Petri does not alias it to continuation. Strictly after the deadline, forward progress is unavailable. Petri and Lean recheck eligibility before signing; check status again after confirmation.",
            current.next_deposit_index,
        );
    }
    if current.status == CurrentCloseStatus::Complete {
        return format!(
            "Available forward action: eligibility is conditional on the finalized Clock and signer. Through and including deadline {deadline}, only request owner {owner} may finalize with `petri writers close --close-request {address}`. Cancellation is supported by the protocol runtime but unavailable in this Petri release because the current wallet action-mask ABI has no close-cancel kind; Petri does not alias it to finalization. Strictly after the deadline, finalization is unavailable. Lean selects the exact stage and Petri rechecks eligibility before signing; check status again after confirmation."
        );
    }
    if current.status == CurrentCloseStatus::Finalized {
        return "Next stage: none; this close request is finalized.".to_string();
    }
    if current.status == CurrentCloseStatus::Cancelling {
        return format!(
            "Cancellation state: request {address} belongs to {owner}, deadline {deadline}, and its next reported cancellation index is {}. The protocol runtime can advance this state, but Petri cannot open review or sign it because collective-operations-v1 has no wallet-specific close-cancel action. Petri will not substitute continuation or finalization for that missing authorization.",
            current.next_cancel_index,
        );
    }
    if current.status == CurrentCloseStatus::Cancelled {
        return "Next stage: none; this close request is cancelled.".to_string();
    }
    "Next stage: unavailable".to_string()
}

fn unwrap_data(payload: &Value) -> &Value {
    payload.get("data").unwrap_or(payload)
}

fn nested_or_self<'a>(root: &'a Value, key: &str, self_markers: &[&str]) -> Option<&'a Value> {
    root.get(key).filter(|value| value.is_object()).or_else(|| {
        (root.is_object() && self_markers.iter().any(|marker| root.get(marker).is_some()))
            .then_some(root)
    })
}

fn first_field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| scalar(value.get(*key)))
        .unwrap_or_else(|| UNAVAILABLE.to_string())
}

fn field(value: &Value, key: &str) -> String {
    scalar(value.get(key)).unwrap_or_else(|| UNAVAILABLE.to_string())
}

fn nullable_field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::Null) => "none".to_string(),
        value => scalar(value).unwrap_or_else(|| UNAVAILABLE.to_string()),
    }
}

fn array_field(value: &Value, key: &str) -> String {
    let Some(values) = value.get(key).and_then(Value::as_array) else {
        return UNAVAILABLE.to_string();
    };
    if values.is_empty() {
        return "none".to_string();
    }
    values
        .iter()
        .map(|value| scalar(Some(value)).unwrap_or_else(|| UNAVAILABLE.to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn reason_suffix(value: &Value) -> String {
    let reasons = array_field(value, "reasonCodes");
    if matches!(reasons.as_str(), "none" | UNAVAILABLE) {
        String::new()
    } else {
        format!(" ({reasons})")
    }
}

fn scalar(value: Option<&Value>) -> Option<String> {
    let rendered = match value? {
        Value::String(value) => clean(value),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };
    (!rendered.is_empty()).then_some(rendered)
}

fn clean(value: &str) -> String {
    terminal_safe_text(value.trim())
        .chars()
        .take(MAX_WRITER_DISPLAY_FIELD_CHARS)
        .collect()
}

fn short(value: &str, width: usize) -> String {
    if value.chars().count() <= width || width < 9 {
        return value.to_string();
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
