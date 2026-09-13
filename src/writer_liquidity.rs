//! Bounded writer liquidity requests and exact read-only projections.
use serde_json::{Value, json};
use std::collections::HashSet;

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn keys(value: &Value, expected: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("expected an object")?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err("response contains missing or unexpected fields".into());
    }
    Ok(())
}

fn atoms(value: &Value, key: &str, nullable: bool) -> Result<(), String> {
    if nullable && value.get(key).is_some_and(Value::is_null) {
        return Ok(());
    }
    crate::request_validation::canonical_u64_string(text(value, key)?, key, true)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn address(value: &Value, key: &str, nullable: bool) -> Result<(), String> {
    if nullable && value.get(key).is_some_and(Value::is_null) {
        return Ok(());
    }
    crate::request_validation::canonical_pubkey_string(text(value, key)?, key)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn nullable_text(value: &Value, key: &str, maximum: usize) -> Result<(), String> {
    if value.get(key).is_some_and(Value::is_null) {
        return Ok(());
    }
    let raw = text(value, key)?;
    if raw.is_empty() || raw.len() > maximum || raw.chars().any(char::is_control) {
        return Err(format!("invalid {key}"));
    }
    Ok(())
}

fn digest(value: &Value, nullable: bool) -> Result<(), String> {
    if nullable && value.get("evidenceDigest").is_some_and(Value::is_null) {
        return Ok(());
    }
    let raw = text(value, "evidenceDigest")?;
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid evidence digest".into());
    }
    Ok(())
}

pub(crate) fn valid_cursor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 768
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(crate) fn render_inspection(payload: &Value) -> Result<String, String> {
    let root = payload.get("data").ok_or("writer read data is missing")?;
    let owner = text(root, "owner")?;
    match text(root, "schema")? {
        "writer-auction-refunds-v1" => render_refunds(payload, owner, 32),
        "writer-liquidity-v1" => {
            let series = root["seriesIndex"]
                .as_u64()
                .filter(|index| *index < 20)
                .ok_or("invalid series index")? as u8;
            render_liquidity(payload, owner, text(root, "sleeve")?, series)
        }
        _ => Err("writer read schema is unsupported".into()),
    }
}

pub(crate) fn parse_bins(bins: &[String], adding: bool) -> Result<Vec<Value>, String> {
    if bins.is_empty() || bins.len() > 8 {
        return Err("supply one through eight --bin entries".into());
    }
    let mut prior = 0;
    bins.iter().map(|raw| {
        let fields = raw.split(':').collect::<Vec<_>>();
        if fields.len() != 3 { return Err("bin must be ID:OPTION_ATOMS:QUOTE_ATOMS".into()); }
        let id = fields[0].parse::<u16>().map_err(|_| "invalid bin id")?;
        if id <= prior || id > 2048 || id.to_string() != fields[0] {
            return Err("bins must be canonical, strictly ascending, unique IDs from 1 through 2048".into());
        }
        prior = id;
        let option = crate::request_validation::canonical_u64_string(fields[1], "option atoms", true).map_err(|error| error.to_string())?;
        let quote = crate::request_validation::canonical_u64_string(fields[2], "quote atoms", true).map_err(|error| error.to_string())?;
        if option == "0" && quote == "0" { return Err("each bin needs a positive amount".into()); }
        Ok(if adding {
            json!({"binId":id,"maximumOptionAmountAtoms":option,"maximumQuoteAmountAtoms":quote})
        } else {
            json!({"binId":id,"optionAmountAtoms":option,"quoteAmountAtoms":quote})
        })
    }).collect()
}

pub(crate) fn render_refunds(payload: &Value, owner: &str, limit: u8) -> Result<String, String> {
    let root = payload.get("data").ok_or("refund data is missing")?;
    keys(
        root,
        &[
            "protocol",
            "schema",
            "owner",
            "discoverySlot",
            "inventoryScope",
            "rows",
            "nextCursor",
        ],
    )?;
    if text(root, "schema")? != "writer-auction-refunds-v1"
        || text(root, "owner")? != owner
        || text(root, "inventoryScope")? != "owner_classic_bid_accounts_at_discovery_slot"
    {
        return Err("refund discovery identity or scope mismatch".into());
    }
    atoms(root, "discoverySlot", false)?;
    nullable_text(root, "nextCursor", 768)?;
    if root["nextCursor"]
        .as_str()
        .is_some_and(|cursor| !valid_cursor(cursor))
    {
        return Err("invalid refund cursor".into());
    }
    let rows = root
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("missing refund rows")?;
    if rows.len() > usize::from(limit) {
        return Err("refund page exceeds requested limit".into());
    }
    let mut seen = HashSet::new();
    let mut lines = vec![format!(
        "Historical refunds for {owner} | discovery slot {}",
        text(root, "discoverySlot")?
    )];
    for row in rows {
        keys(
            row,
            &[
                "owner",
                "auction",
                "bid",
                "sleeve",
                "refundTokenAccount",
                "remainingRefundAtoms",
                "refundableNow",
                "status",
                "reasonCode",
                "observedSlot",
                "evidenceDigest",
            ],
        )?;
        if text(row, "owner")? != owner || !seen.insert(text(row, "bid")?) {
            return Err("refund owner mismatch or duplicate bid".into());
        }
        for key in ["owner", "bid"] {
            address(row, key, false)?;
        }
        for key in ["auction", "sleeve", "refundTokenAccount"] {
            address(row, key, true)?;
        }
        for key in ["remainingRefundAtoms", "observedSlot"] {
            atoms(row, key, true)?;
        }
        nullable_text(row, "reasonCode", 256)?;
        digest(row, true)?;
        let status = text(row, "status")?;
        let refundable = row
            .get("refundableNow")
            .and_then(Value::as_bool)
            .ok_or("missing refund eligibility")?;
        if !["refundable", "blocked", "unavailable"].contains(&status)
            || refundable != (status == "refundable")
        {
            return Err("contradictory refund status".into());
        }
        if refundable
            && (row["remainingRefundAtoms"].as_str() == Some("0")
                || [
                    "auction",
                    "sleeve",
                    "refundTokenAccount",
                    "remainingRefundAtoms",
                    "observedSlot",
                    "evidenceDigest",
                ]
                .iter()
                .any(|key| row[*key].is_null()))
        {
            return Err("refundable row lacks evidence".into());
        }
        lines.push(format!(
            "{} | {status} | refund atoms={} | auction={} | {}",
            text(row, "bid")?,
            row["remainingRefundAtoms"]
                .as_str()
                .unwrap_or("unavailable"),
            row["auction"].as_str().unwrap_or("unavailable"),
            row["reasonCode"].as_str().unwrap_or("")
        ));
    }
    if rows.is_empty() {
        lines.push("No rows in this discovery page.".into());
    }
    if let Some(cursor) = root["nextCursor"].as_str() {
        lines.push(format!(
            "Next page: petri writers refunds --owner {owner} --cursor {cursor} --limit {limit}"
        ));
    }
    lines.push("Discovery is a snapshot. Refund preparation rechecks the exact bid; an expired cursor requires a manual restart.".into());
    Ok(lines.join("\n"))
}

pub(crate) fn render_liquidity(
    payload: &Value,
    owner: &str,
    sleeve: &str,
    series: u8,
) -> Result<String, String> {
    let root = payload.get("data").ok_or("liquidity data is missing")?;
    keys(
        root,
        &[
            "protocol",
            "schema",
            "owner",
            "sleeve",
            "seriesIndex",
            "market",
            "pool",
            "policyAddress",
            "positionAddress",
            "policyAuthority",
            "managementAuthority",
            "actorCanManage",
            "policyLifecycle",
            "positionInitialized",
            "reasonCode",
            "accounting",
            "budget",
            "bins",
            "observedSlot",
            "evidenceDigest",
        ],
    )?;
    if text(root, "schema")? != "writer-liquidity-v1"
        || text(root, "owner")? != owner
        || text(root, "sleeve")? != sleeve
        || root["seriesIndex"].as_u64() != Some(u64::from(series))
    {
        return Err("writer liquidity request identity mismatch".into());
    }
    for key in [
        "owner",
        "sleeve",
        "market",
        "pool",
        "policyAddress",
        "positionAddress",
        "policyAuthority",
    ] {
        address(root, key, false)?;
    }
    address(root, "managementAuthority", true)?;
    let managing = root["actorCanManage"]
        .as_bool()
        .ok_or("missing manager eligibility")?;
    let initialized = root["positionInitialized"]
        .as_bool()
        .ok_or("missing position lifecycle")?;
    let lifecycle = text(root, "policyLifecycle")?;
    if !["uncreated", "building", "sealed"].contains(&lifecycle)
        || (managing
            && (lifecycle != "sealed" || root["managementAuthority"].as_str() != Some(owner)))
    {
        return Err("contradictory writer liquidity policy".into());
    }
    nullable_text(root, "reasonCode", 256)?;
    atoms(root, "observedSlot", false)?;
    digest(root, false)?;
    let accounting = &root["accounting"];
    const REQUIRED: &[&str] = &[
        "assetsAtoms",
        "principalAtoms",
        "grossPrimaryPremiumAtoms",
        "reserveAtoms",
        "operationalBufferAtoms",
        "physicalSupplyAtoms",
        "issuerControlledAtoms",
        "externalOpenInterestAtoms",
    ];
    const OPTIONAL: &[&str] = &[
        "freeCashAtoms",
        "sleeveCashAtoms",
        "writerPoolQuoteAtoms",
        "writerUncommittedQuoteAtoms",
        "positionOptionAtoms",
        "positionQuoteAtoms",
        "positionUncommittedQuoteAtoms",
    ];
    keys(accounting, &[REQUIRED, OPTIONAL].concat())?;
    for key in REQUIRED {
        atoms(accounting, key, false)?;
    }
    for key in OPTIONAL {
        atoms(accounting, key, true)?;
    }
    let mut lines = vec![
        format!(
            "Writer liquidity | sleeve={sleeve} | series={series} | slot={}",
            text(root, "observedSlot")?
        ),
        format!(
            "Pool={} | policy={lifecycle} | initialized={initialized} | actor can manage={managing}",
            text(root, "pool")?
        ),
    ];
    for key in REQUIRED.iter().chain(OPTIONAL.iter()) {
        lines.push(format!(
            "{key}: {}",
            accounting[*key].as_str().unwrap_or("unavailable")
        ));
    }
    let budget = &root["budget"];
    if !budget.is_null() {
        const TERMS: &[&str] = &[
            "monthStartUnixSeconds",
            "monthlyCapAtoms",
            "monthlySpentAtoms",
            "monthlyRemainingAtoms",
            "seriesMonthlyCapAtoms",
            "seriesMonthlySpentAtoms",
            "seriesMonthlyRemainingAtoms",
            "transactionCapAtoms",
            "seriesTransactionCapAtoms",
            "reserveReleaseSpendRatioPpm",
            "conservativeClaimValueAtoms",
            "sellerFloorQuoteAtoms",
        ];
        keys(budget, &[TERMS, &["priceSeparationTicks"]].concat())?;
        for key in TERMS {
            atoms(budget, key, false)?;
            lines.push(format!("{key}: {}", text(budget, key)?));
        }
        let ticks = budget["priceSeparationTicks"]
            .as_u64()
            .filter(|value| (1..=65535).contains(value))
            .ok_or("invalid price separation")?;
        lines.push(format!("priceSeparationTicks: {ticks}"));
    } else {
        lines.push("Buyback limits unavailable until the complete policy is authenticated.".into());
    }
    let bins = root["bins"].as_array().ok_or("missing bins")?;
    if bins.len() > 32 {
        return Err("writer position exceeds bounded bin count".into());
    }
    let mut prior = 0;
    for bin in bins {
        keys(bin, &["binId", "optionAtoms", "quoteAtoms"])?;
        let id = bin["binId"]
            .as_u64()
            .filter(|id| *id > prior && *id <= 2048)
            .ok_or("invalid bin order")?;
        prior = id;
        atoms(bin, "optionAtoms", false)?;
        atoms(bin, "quoteAtoms", false)?;
        lines.push(format!(
            "bin {id} | option atoms={} | quote atoms={}",
            text(bin, "optionAtoms")?,
            text(bin, "quoteAtoms")?
        ));
    }
    if let Some(reason) = root["reasonCode"].as_str() {
        lines.push(format!("State: {reason}"));
    }
    lines.push("Primary premium is historical after sale fees, before buybacks; it is not cash. These facts do not authorize a fill or withdrawal.".into());
    Ok(lines.join("\n"))
}
