//! Oracle reads from the futures API: the public snapshot (every month's
//! phase, source count and settlement status, plus each month's verified
//! source inventory), the catalog of periods and settlement times, and the
//! source-candidate store. Nothing here signs or touches a wallet.

use serde_json::{Value, json};

use super::skus;
use crate::futures::{
    Report,
    ctx::Ctx,
    error::{Error, Result, bail},
    units::{short, trim_decimal},
    view::{Paint, Table},
};

use crate::endpoints::{ORACLE_CATALOG as CATALOG, ORACLE_SNAPSHOT as SNAPSHOT};

/// `data` of an `{ ok: true, data }` oracle document.
pub(crate) fn document(ctx: &Ctx, path: &str) -> Result<Value> {
    document_at(&ctx.api, path)
}

/// `document`, from the API client alone (it can be read from any thread).
pub(crate) fn document_at(api: &crate::futures::api::Api, path: &str) -> Result<Value> {
    let value = api.get(path)?;
    if value.get("ok") != Some(&Value::Bool(true)) {
        bail(format!("{path}: unexpected oracle response"))?;
    }
    value
        .get("data")
        .cloned()
        .ok_or_else(|| Error::msg(format!("{path}: oracle response without data")))
}

pub fn snapshot(ctx: &Ctx) -> Result<Value> {
    document(ctx, SNAPSHOT)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("-")
}

fn number(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => "-".to_string(),
    }
}

/// An atomic six-decimal value (`"75000000"` → `75`).
fn value6(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u128>().ok())
        .map(|a| trim_decimal(a, 6))
        .unwrap_or_else(|| "-".to_string())
}

fn market_matches(fact: &Value, market: Option<&str>) -> bool {
    market.is_none_or(|m| {
        let m = m.trim().to_ascii_lowercase();
        text(fact, "marketId") == m || text(fact, "expiryId").eq_ignore_ascii_case(&m)
    })
}

fn freshness_line(snapshot: &Value) -> String {
    let f = &snapshot["freshness"];
    let protocol = &snapshot["protocol"];
    format!(
        "Oracle {} ({} {}) | observed {} | {}",
        short(text(protocol, "programId")),
        text(protocol, "cluster"),
        text(protocol, "releaseTag"),
        text(f, "observedAt"),
        if f["readReady"] == true {
            "fresh"
        } else {
            "stale"
        }
    )
}

fn months_table(facts: &[&Value]) -> Vec<String> {
    let mut table =
        Table::new(&["Month", "Phase", "Sources", "Settlement", "Month account"]).right(&[2]);
    for fact in facts {
        table.row(vec![
            text(fact, "expiryId").to_string(),
            text(fact, "phase").to_string(),
            number(fact, "sourceCount"),
            text(fact, "settlementStatus").to_string(),
            text(fact, "oracleMonthAddress").to_string(),
        ]);
    }
    table.lines()
}

/// `petri oracle`, `petri oracle months [MARKET]`.
pub fn months(ctx: &Ctx, p: Paint, market: Option<&str>) -> Result<Report> {
    let snapshot = snapshot(ctx)?;
    let facts: Vec<&Value> = snapshot["state"]["months"]
        .as_array()
        .map(|a| a.iter().filter(|f| market_matches(f, market)).collect())
        .unwrap_or_default();
    let mut lines = vec![p.bold(&freshness_line(&snapshot))];
    if facts.is_empty() {
        lines.push(match market {
            Some(m) => format!("No oracle months for {m}."),
            None => "No oracle months yet.".to_string(),
        });
    } else {
        lines.extend(months_table(&facts));
        lines.push(
            p.dim("Sources: `petri oracle sources <market>`. The market: `petri market <market>`."),
        );
    }
    let payload = json!({
        "ok": true,
        "protocol": snapshot["protocol"],
        "freshness": snapshot["freshness"],
        "status": snapshot["state"]["status"],
        "months": facts,
    });
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle latest [MARKET]`: the most recently updated month.
pub fn latest(ctx: &Ctx, p: Paint, market: Option<&str>) -> Result<Report> {
    let snapshot = snapshot(ctx)?;
    let latest = snapshot["state"]["history"]
        .as_array()
        .and_then(|h| h.iter().find(|f| market_matches(f, market)))
        .cloned();
    let mut lines = vec![p.bold(&freshness_line(&snapshot))];
    match &latest {
        Some(fact) => lines.extend(months_table(&[fact])),
        None => lines.push("No oracle month has been updated yet.".to_string()),
    }
    let payload = json!({"ok": true, "protocol": snapshot["protocol"], "freshness": snapshot["freshness"], "latest": latest});
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle history [MARKET]`: months by last update, newest first.
pub fn history(ctx: &Ctx, p: Paint, market: Option<&str>) -> Result<Report> {
    let snapshot = snapshot(ctx)?;
    let facts: Vec<&Value> = snapshot["state"]["history"]
        .as_array()
        .map(|a| a.iter().filter(|f| market_matches(f, market)).collect())
        .unwrap_or_default();
    let mut lines = vec![p.bold(&freshness_line(&snapshot))];
    if facts.is_empty() {
        lines.push("No oracle month has been updated yet.".to_string());
    } else {
        let mut table =
            Table::new(&["Month", "Phase", "Settlement", "Last update slot"]).right(&[3]);
        for fact in &facts {
            table.row(vec![
                text(fact, "expiryId").to_string(),
                text(fact, "phase").to_string(),
                text(fact, "settlementStatus").to_string(),
                number(fact, "lastUpdatedSlot"),
            ]);
        }
        lines.extend(table.lines());
    }
    let payload = json!({"ok": true, "protocol": snapshot["protocol"], "freshness": snapshot["freshness"], "history": facts});
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle catalog`: each product's periods and settlement times.
pub fn catalog(ctx: &Ctx, p: Paint) -> Result<Report> {
    let catalog = document(ctx, CATALOG)?;
    let mut table = Table::new(&["Market", "Month", "Settles (UTC)"]);
    for market in catalog["markets"].as_array().into_iter().flatten() {
        for expiry in market["expiries"].as_array().into_iter().flatten() {
            table.row(vec![
                text(market, "marketId").to_string(),
                text(expiry, "expiryId").to_string(),
                text(expiry, "settlementUtc").to_string(),
            ]);
        }
    }
    let mut lines = vec![p.bold(&freshness_line(&catalog))];
    if table.is_empty() {
        lines.push("No oracle periods yet.".to_string());
    } else {
        lines.extend(table.lines());
    }
    let mut payload = catalog;
    payload["ok"] = json!(true);
    Ok(Report::new(payload, lines.join("\n")))
}

/// The source inventories of `market` (optionally one month), with `query`
/// matching a bucket label or id, a source id or address, or the URL.
fn inventories<'a>(
    snapshot: &'a Value,
    market: Option<&str>,
    expiry: Option<&str>,
) -> Vec<&'a Value> {
    snapshot["sourceLinks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|inv| market_matches(inv, market))
                .filter(|inv| expiry.is_none_or(|e| expiry_matches(text(inv, "expiryId"), e)))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a month id (`RAMX-202610`) is the month `wanted` names: `oct26`,
/// `2026-10`, `202610` or the full id.
fn expiry_matches(expiry_id: &str, wanted: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let squash = |t: &str| {
        t.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let id = squash(expiry_id);
    let mut wanted = squash(wanted);
    if wanted.len() == 5
        && let Some(month) = MONTHS.iter().position(|m| wanted.starts_with(m))
    {
        wanted = format!("20{}{:02}", &wanted[3..], month + 1);
    }
    !wanted.is_empty() && (id == wanted || id.ends_with(&wanted))
}

fn source_matches(source: &Value, query: Option<&str>) -> bool {
    query.is_none_or(|q| {
        let q = q.trim().to_ascii_lowercase();
        [
            text(source, "bucketLabel"),
            text(source, "bucketId"),
            text(source, "sourceId"),
            text(source, "sourceAddress"),
            text(source, "status"),
            source["canonicalLocator"]["url"].as_str().unwrap_or(""),
        ]
        .iter()
        .any(|field| field.to_ascii_lowercase().contains(&q))
    })
}

fn bucket_name(source: &Value) -> String {
    source["bucketLabel"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            let id = text(source, "bucketId");
            skus::parse_hex32(id, "bucket")
                .ok()
                .and_then(|b| skus::decode_label(&b))
                .unwrap_or_else(|| format!("{}…", &id[..id.len().min(12)]))
        })
}

/// `petri oracle sources [MARKET] [--expiry] [--query] [--limit]`.
pub fn sources(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    query: Option<&str>,
    limit: usize,
) -> Result<Report> {
    let snapshot = snapshot(ctx)?;
    let mut lines = vec![p.bold(&freshness_line(&snapshot))];
    let mut months = Vec::new();
    for inventory in inventories(&snapshot, market, expiry) {
        let sources: Vec<&Value> = inventory["sources"]
            .as_array()
            .map(|a| a.iter().filter(|s| source_matches(s, query)).collect())
            .unwrap_or_default();
        lines.push(String::new());
        lines.push(p.bold(&format!(
            "{} | {} | {} of {} sources",
            text(inventory, "expiryId"),
            text(inventory, "phase"),
            sources.len(),
            number(inventory, "sourceCount")
        )));
        let mut table = Table::new(&[
            "Source id",
            "Bucket",
            "Status",
            "Weight",
            "Current",
            "Baseline",
            "URL",
        ])
        .right(&[3, 4, 5]);
        for source in sources.iter().take(limit) {
            table.row(vec![
                format!(
                    "{}…",
                    &text(source, "sourceId")[..16.min(text(source, "sourceId").len())]
                ),
                bucket_name(source),
                text(source, "status").to_string(),
                format!("{} bps", number(source, "bucketWeightBps")),
                value6(source, "currentStateAtomic"),
                value6(source, "baselineStateAtomic"),
                source["canonicalLocator"]["url"]
                    .as_str()
                    .unwrap_or("(locator not published)")
                    .to_string(),
            ]);
        }
        if table.is_empty() {
            lines.push("  no matching sources".to_string());
        } else {
            lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        }
        if sources.len() > limit {
            lines.push(p.dim(&format!(
                "  … {} more; raise --limit",
                sources.len() - limit
            )));
        }
        months.push(json!({
            "marketId": inventory["marketId"],
            "expiryId": inventory["expiryId"],
            "oracleMonthAddress": inventory["oracleMonthAddress"],
            "phase": inventory["phase"],
            "sourceCount": inventory["sourceCount"],
            "observedAt": inventory["observedAt"],
            "sources": sources.into_iter().take(limit).collect::<Vec<_>>(),
        }));
    }
    if months.is_empty() {
        lines.push("No published source inventory matches.".to_string());
    }
    let payload = json!({"ok": true, "protocol": snapshot["protocol"], "freshness": snapshot["freshness"], "months": months});
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle recipe [MARKET]`: buckets with their weight and sources.
pub fn recipe(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    query: Option<&str>,
    limit: usize,
) -> Result<Report> {
    let snapshot = snapshot(ctx)?;
    let mut lines = vec![p.bold(&freshness_line(&snapshot))];
    let mut months = Vec::new();
    for inventory in inventories(&snapshot, market, expiry) {
        let mut buckets: Vec<(String, String, u64, Vec<&Value>)> = Vec::new();
        for source in inventory["sources"].as_array().into_iter().flatten() {
            if !source_matches(source, query) {
                continue;
            }
            let id = text(source, "bucketId").to_string();
            let weight = source["bucketWeightBps"].as_u64().unwrap_or(0);
            match buckets.iter_mut().find(|b| b.0 == id) {
                Some(bucket) => {
                    bucket.2 = bucket.2.max(weight);
                    bucket.3.push(source);
                }
                None => buckets.push((id, bucket_name(source), weight, vec![source])),
            }
        }
        buckets.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
        lines.push(String::new());
        lines.push(p.bold(&format!(
            "{} | {} | {} buckets",
            text(inventory, "expiryId"),
            text(inventory, "phase"),
            buckets.len()
        )));
        let mut table = Table::new(&["Bucket", "Weight", "Sources", "Active", "Median current"])
            .right(&[1, 2, 3, 4]);
        for (_, name, weight, sources) in buckets.iter().take(limit) {
            let mut values: Vec<u128> = sources
                .iter()
                .filter_map(|s| s["currentStateAtomic"].as_str()?.parse().ok())
                .collect();
            values.sort_unstable();
            let median = values
                .get(values.len().saturating_sub(1) / 2)
                .map(|v| trim_decimal(*v, 6))
                .unwrap_or_else(|| "-".to_string());
            table.row(vec![
                name.clone(),
                format!("{weight} bps"),
                sources.len().to_string(),
                sources
                    .iter()
                    .filter(|s| s["status"] == "Active")
                    .count()
                    .to_string(),
                median,
            ]);
        }
        lines.extend(table.lines().into_iter().map(|l| format!("  {l}")));
        months.push(json!({
            "marketId": inventory["marketId"],
            "expiryId": inventory["expiryId"],
            "phase": inventory["phase"],
            "buckets": buckets.iter().map(|(id, name, weight, sources)| json!({
                "bucketId": id, "bucket": name, "bucketWeightBps": weight,
                "sources": sources.iter().map(|s| s["sourceId"].clone()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }));
    }
    if months.is_empty() {
        lines.push("No published source inventory matches.".to_string());
    }
    let payload = json!({"ok": true, "protocol": snapshot["protocol"], "freshness": snapshot["freshness"], "months": months});
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle candidates [--market] [--expiry] [--agent] [--limit]`.
pub fn candidates(
    ctx: &Ctx,
    p: Paint,
    market: Option<&str>,
    expiry: Option<&str>,
    agent: Option<&str>,
    limit: u32,
) -> Result<Report> {
    let mut filters = Vec::new();
    for (key, value) in [
        ("marketId", market),
        ("expiryId", expiry),
        ("agentId", agent),
    ] {
        if let Some(value) = value {
            filters.push((key, value));
        }
    }
    let data = document(ctx, &crate::endpoints::oracle_candidates(&filters, limit))?;
    let list = data["candidates"].as_array().cloned().unwrap_or_default();
    let mut table = Table::new(&["Content hash", "Month", "Bucket", "Agent", "URL"]);
    for c in &list {
        let details = &c["details"];
        table.row(vec![
            format!(
                "{}…",
                &text(c, "contentHash")[..16.min(text(c, "contentHash").len())]
            ),
            text(details, "expiryId").to_string(),
            {
                let id = text(details, "bucketId");
                skus::parse_hex32(id, "bucket")
                    .ok()
                    .and_then(|b| skus::decode_label(&b))
                    .unwrap_or_else(|| format!("{}…", &id[..id.len().min(12)]))
            },
            format!(
                "{}…",
                &text(c, "agentId")[..12.min(text(c, "agentId").len())]
            ),
            text(details, "canonicalUrl").to_string(),
        ]);
    }
    let mut lines = vec![p.bold(&format!(
        "Source candidates ({} shown; candidates carry no oracle weight)",
        list.len()
    ))];
    if table.is_empty() {
        lines.push("No candidates match.".to_string());
    } else {
        lines.extend(table.lines());
    }
    let payload = json!({"ok": true, "protocol": data["protocol"], "candidates": list, "nextCursor": data["nextCursor"]});
    Ok(Report::new(payload, lines.join("\n")))
}

/// `petri oracle candidates show <HASH>`.
pub fn candidate(ctx: &Ctx, p: Paint, hash: &str) -> Result<Report> {
    let hash = skus::hex(&skus::parse_hex32(hash, "content hash")?);
    let data = document(ctx, &crate::endpoints::oracle_candidate(&hash))?;
    let c = &data["candidate"];
    let d = &c["details"];
    let mut lines = vec![p.bold(&format!("Candidate {hash}"))];
    for (label, value) in [
        ("Month", text(d, "expiryId")),
        ("Bucket", text(d, "bucketId")),
        ("URL", text(d, "canonicalUrl")),
        ("Source type", text(d, "sourceType")),
        ("Price field", text(d, "priceField")),
        ("Region", text(d, "region")),
        ("Currency", text(d, "currency")),
        ("Agent", text(c, "agentId")),
        ("Payout", text(c, "payoutAuthority")),
    ] {
        lines.push(format!("  {label:<12} {value}"));
    }
    lines.push(format!(
        "  {:<12} {}",
        "Definition",
        text(d, "productDefinition")
    ));
    let payload = json!({"ok": true, "protocol": data["protocol"], "candidate": c});
    Ok(Report::new(payload, lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_filters_match_labels_ids_and_urls() {
        let source = json!({
            "bucketLabel": "KSM64R52BD8-32HA", "bucketId": "ab", "sourceId": "cd",
            "sourceAddress": "Addr", "status": "Active",
            "canonicalLocator": {"url": "https://example.com/ram"},
        });
        assert!(source_matches(&source, Some("ksm64")));
        assert!(source_matches(&source, Some("example.com")));
        assert!(source_matches(&source, Some("active")));
        assert!(!source_matches(&source, Some("nand")));
        assert!(source_matches(&source, None));
    }

    #[test]
    fn months_match_every_spelling() {
        for wanted in ["oct26", "OCT26", "2026-10", "202610", "RAMX-202610"] {
            assert!(expiry_matches("RAMX-202610", wanted), "{wanted}");
        }
        assert!(!expiry_matches("RAMX-202610", "nov26"));
        assert!(!expiry_matches("RAMX-202610", ""));
    }

    #[test]
    fn markets_match_by_id_or_month() {
        let fact = json!({"marketId": "ramx", "expiryId": "RAMX-202610"});
        assert!(market_matches(&fact, Some("RAMX")));
        assert!(market_matches(&fact, Some("ramx-202610")));
        assert!(!market_matches(&fact, Some("nandx")));
    }
}
