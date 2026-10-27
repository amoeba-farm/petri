//! Research prioritization over published oracle facts. This module cannot
//! admit sources, advance oracle phases, or determine rewards.

use crate::research::workspace::{Failure, fail_status, usdc};
use serde_json::{Map, Value, json};

const DAY_MS: i64 = 86_400_000;

fn invalid(message: &str) -> Failure {
    fail_status("ORACLE_DATA_INVALID", message, 503)
}

fn hex(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

const SOURCE_STATUSES: [&str; 8] = [
    "Candidate",
    "Frozen",
    "Inactive",
    "Rejected",
    "OpeningPending",
    "Active",
    "Merged",
    "TimedOut",
];

/// Sources that hold a product's place while a month forms (proposed,
/// frozen, opening or live), against those that lost it.
const PLACED: [&str; 4] = ["Candidate", "Frozen", "OpeningPending", "Active"];

/// A value as a JavaScript template literal shows it.
fn js(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(_)) => "[object Object]".into(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js(Some(item))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(other) => other.to_string(),
    }
}

fn identity(value: &Value) -> bool {
    ["programId", "cluster", "namespace"]
        .iter()
        .all(|key| value[*key].is_string())
}

fn same_protocol(a: &Value, b: &Value) -> bool {
    identity(a)
        && identity(b)
        && ["programId", "cluster", "namespace"]
            .iter()
            .all(|key| a[*key] == b[*key])
}

/// `Date.parse` of an ISO 8601 time: milliseconds, or None.
pub fn timestamp(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    let millis = chrono::DateTime::parse_from_rfc3339(text)
        .map(|time| time.timestamp_millis())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|time| time.and_utc().timestamp_millis())
        })
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|date| {
                date.and_hms_opt(0, 0, 0)
                    .expect("midnight")
                    .and_utc()
                    .timestamp_millis()
            })
        })
        .ok()?;
    (millis.abs() < (1 << 53)).then_some(millis)
}

/// `new Date(ms).toISOString()`.
pub fn iso(millis: i64) -> Value {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|time| json!(time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()))
        .unwrap_or(Value::Null)
}

fn fresh(value: &Value, now: i64, maximum_age: i64) -> bool {
    let observed = timestamp(&value["observedAt"]);
    value["stale"] == false
        && value["refreshHealthy"] == true
        && value["readReady"] == true
        && observed.is_some_and(|observed| observed <= now && now - observed <= maximum_age)
}

/// A verified HTTPS source's host, without `www.`.
fn host(source: &Value) -> Option<String> {
    if source["canonicalLocator"]["verified"] != true {
        return None;
    }
    let url = reqwest::Url::parse(source["canonicalLocator"]["url"].as_str()?).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?.to_lowercase();
    Some(
        host.strip_prefix("www.")
            .map(str::to_string)
            .unwrap_or(host),
    )
}

fn url_of(source: &Value) -> Value {
    if host(source).is_some() {
        source["canonicalLocator"]["url"].clone()
    } else {
        Value::Null
    }
}

/// Atoms as a digit string, or null.
fn atomic(value: &Value) -> Value {
    match value.as_str() {
        Some(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => json!(s),
        _ => Value::Null,
    }
}

/// A chip month (RAMX and NANDX expiring after 2026-11-01): the API lists its
/// commodity buckets (`buckets`) and names them (`bucketName`, "DDR5 16Gb").
/// Its sources are comparison series and its prints observations of them
/// (oracle/governance/commodity_submission_spec.v2.json).
pub fn chip_month(fact: Option<&Value>) -> bool {
    let Some(fact) = fact else { return false };
    fact["buckets"].as_array().is_some_and(|b| !b.is_empty())
        || fact["skus"].as_array().is_some_and(|skus| {
            skus.iter()
                .any(|sku| sku["bucketName"].as_str().is_some_and(|n| !n.is_empty()))
        })
}

fn named<'a>(items: impl IntoIterator<Item = Option<&'a Value>>) -> Value {
    items
        .into_iter()
        .flatten()
        .filter_map(|item| item["bucketName"].as_str().filter(|n| !n.is_empty()))
        .map(|name| json!(name))
        .next()
        .unwrap_or(Value::Null)
}

fn series_id(source: &Value) -> Value {
    match source["series"]["seriesId"].as_str() {
        Some(id) => json!(id),
        None => Value::Null,
    }
}

fn label(value: &Value) -> Value {
    if value.is_string() {
        value.clone()
    } else {
        Value::Null
    }
}

/// What a month takes now, by its phase's rules: new sources while it forms
/// and placement is open (SourceSubmission, Scramble days 1-4), opening
/// values for its frozen sources in Opening, updates to its live sources in
/// Game, its reward claims once Settled with its rewards frozen, otherwise
/// nothing (Scramble's challenge and freeze days, a settled month not frozen
/// yet). Opening follows the chain's phase, not this clock: the oracle takes
/// openings until listing on the chain's own clock, which can run behind.
fn stage(fact: &Value, now: i64) -> &'static str {
    match fact["phase"].as_str() {
        Some("SourceSubmission") => "sources",
        Some("Scramble") => match timestamp(&fact["schedule"]["scrambleStartUtc"]) {
            Some(start) if now < start + 4 * DAY_MS => "sources",
            _ => "closed",
        },
        Some("Opening") => "openings",
        Some("Settled") if fact["rewards"]["phase"] == "EntitlementsFinalized" => "claims",
        Some("Game") => "updates",
        _ => "closed",
    }
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}

/// `fact.phase` when the fact has one (JSON leaves an undefined one out).
fn phase(map: &mut Map<String, Value>, fact: &Value) {
    if let Some(phase) = fact.get("phase") {
        map.insert("phase".into(), phase.clone());
    }
}

fn find<'a>(list: &[(String, &'a Value)], key: &str) -> Option<&'a Value> {
    list.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
}

fn with(base: &Map<String, Value>, fields: Value) -> Map<String, Value> {
    let mut out = base.clone();
    if let Value::Object(fields) = fields {
        out.extend(fields);
    }
    out
}

pub fn assess_coverage(
    observations: &Value,
    directives: &[Value],
    now: i64,
) -> Result<Value, Failure> {
    let snapshot = &observations["snapshot"];
    let catalog = &observations["catalog"];
    let (Some(months), Some(links), Some(markets)) = (
        snapshot["state"]["months"].as_array(),
        snapshot["sourceLinks"].as_array(),
        catalog["markets"].as_array(),
    ) else {
        return Err(invalid("The oracle coverage response is incomplete."));
    };
    if !same_protocol(&snapshot["protocol"], &catalog["protocol"]) {
        return Err(invalid("The oracle coverage response is incomplete."));
    }
    // Every series the catalog lists, in its order: key, the expiry with its
    // market's id and on-chain flag.
    let mut expiries: Vec<(String, Value)> = Vec::new();
    for market in markets {
        let expiry_list = market["expiries"]
            .as_array()
            .ok_or_else(|| invalid("The market catalog is incomplete."))?;
        for expiry in expiry_list {
            let key = format!(
                "{}:{}",
                js(market.get("marketId")),
                js(expiry.get("expiryId"))
            );
            if expiries.iter().any(|(k, _)| *k == key) {
                return Err(invalid(
                    "The market catalog contains conflicting identities.",
                ));
            }
            let mut entry = expiry.as_object().cloned().unwrap_or_default();
            entry.insert(
                "marketId".into(),
                market.get("marketId").cloned().unwrap_or(Value::Null),
            );
            entry.insert(
                "onChainAvailable".into(),
                market
                    .get("onChainAvailable")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            expiries.push((key, Value::Object(entry)));
        }
    }
    let mut facts: Vec<(String, &Value)> = Vec::new();
    for fact in months {
        let (m, e) = (js(fact.get("marketId")), js(fact.get("expiryId")));
        let key = format!("{m}:{e}");
        if !market_id(&m) || !expiry_id(&e) || facts.iter().any(|(k, _)| *k == key) {
            return Err(invalid("The oracle series identities are invalid."));
        }
        facts.push((key, fact));
    }
    let mut inventories: Vec<(String, &Value)> = Vec::new();
    for inventory in links {
        let key = format!(
            "{}:{}",
            js(inventory.get("marketId")),
            js(inventory.get("expiryId"))
        );
        if inventories.iter().any(|(k, _)| *k == key) {
            return Err(invalid("The oracle source inventories conflict."));
        }
        inventories.push((key, inventory));
    }
    let mut assessments: Vec<Value> = Vec::new();
    for directive in directives.iter().filter(|d| d["enabled"] == true) {
        let max_age = directive["maxObservationAgeMs"].as_i64().unwrap_or(0);
        let ids = directive["marketIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (key, expiry) in &expiries {
            if !ids.is_empty() && !ids.contains(&expiry["marketId"]) {
                continue;
            }
            let fact = find(&facts, key);
            let mut base = Map::new();
            base.insert("directiveId".into(), directive["id"].clone());
            base.insert("directiveRevision".into(), directive["revision"].clone());
            base.insert("marketId".into(), expiry["marketId"].clone());
            base.insert(
                "expiryId".into(),
                expiry.get("expiryId").cloned().unwrap_or(Value::Null),
            );
            base.insert(
                "oracleMonthAddress".into(),
                fact.map(|f| f["oracleMonthAddress"].clone())
                    .unwrap_or(Value::Null),
            );
            base.insert("bucketId".into(), Value::Null);
            base.insert("bucketLabel".into(), Value::Null);
            base.insert("bucketName".into(), Value::Null);
            base.insert("chip".into(), json!(chip_month(fact)));
            base.insert("protocol".into(), snapshot["protocol"].clone());
            base.insert("assessedAt".into(), json!(now));
            base.insert(
                "observedAt".into(),
                snapshot["freshness"]["observedAt"].clone(),
            );
            base.insert(
                "targets".into(),
                json!({ "activeSources": directive["minActiveSources"], "distinctHosts": directive["minDistinctHosts"] }),
            );
            base.insert("reasons".into(), json!([]));
            base.insert("sources".into(), json!([]));
            let mut add = |map: Map<String, Value>| assessments.push(Value::Object(map));
            let unknown =
                |reason: &str| with(&base, json!({ "status": "unknown", "reason": reason }));
            if !fresh(&snapshot["freshness"], now, max_age)
                || !fresh(&catalog["freshness"], now, max_age)
            {
                add(unknown("Oracle observations are stale or unavailable."));
                continue;
            }
            let observed_at =
                |value: &Value| timestamp(&value["freshness"]["observedAt"]).unwrap_or(0);
            // Settled with its rewards frozen: one task to claim them.
            if let Some(fact) = fact
                && fact["oracleMonthAvailable"] == true
                && stage(fact, now) == "claims"
            {
                let mut claims = with(
                    &base,
                    json!({ "kind": "claims", "status": "gap", "reasons": ["claims"], "reason": null,
                        "validUntil": observed_at(snapshot).min(observed_at(catalog)) + max_age }),
                );
                phase(&mut claims, fact);
                add(claims);
                continue;
            }
            let Some(settlement) = timestamp(&expiry["settlementUtc"]) else {
                add(unknown("The exact settlement time is unavailable."));
                continue;
            };
            if settlement <= now {
                add(with(
                    &base,
                    json!({ "status": "inactive", "reason": "This series has reached settlement." }),
                ));
                continue;
            }
            let fact = match fact {
                Some(fact)
                    if expiry["onChainAvailable"] == true
                        && fact["onChainAvailable"] == true
                        && fact["oracleMonthAvailable"] == true =>
                {
                    fact
                }
                _ => {
                    add(unknown(
                        "The oracle month is not initialized or its state is unavailable.",
                    ));
                    continue;
                }
            };
            let in_list =
                |value: &Value, list: [&str; 2]| value.as_str().is_some_and(|v| list.contains(&v));
            if in_list(&fact["phase"], ["Finalized", "Settled"])
                || in_list(&fact["settlementStatus"], ["Settled", "Finalized"])
            {
                add(with(
                    &base,
                    json!({ "status": "inactive", "reason": "The oracle series is finalized." }),
                ));
                continue;
            }
            let inventory = find(&inventories, key);
            let observed = inventory.and_then(|i| timestamp(&i["observedAt"]));
            let complete = match (inventory, observed) {
                (Some(inventory), Some(observed)) => {
                    let count = &inventory["sourceCount"];
                    let integer = count.as_f64().filter(|n| n.fract() == 0.0 && *n >= 0.0);
                    // While a month takes sources its count is its supported
                    // sources and the inventory lists every proposal; from
                    // the freeze on they match.
                    let counted = match integer {
                        Some(n) if stage(fact, now) == "sources" => {
                            !number(&fact["sourceCount"]).is_some_and(|wanted| n < wanted)
                        }
                        Some(n) => number(&fact["sourceCount"]) == Some(n),
                        None => false,
                    };
                    inventory["usage"] == "source-display-only"
                        && inventory.get("oracleMonthAddress") == fact.get("oracleMonthAddress")
                        && inventory.get("phase") == fact.get("phase")
                        && counted
                        && inventory["sources"]
                            .as_array()
                            .is_some_and(|s| integer == Some(s.len() as f64))
                        && observed <= now
                        && now - observed <= max_age
                }
                _ => false,
            };
            let (Some(inventory), Some(observed), true) = (inventory, observed, complete) else {
                add(unknown(
                    "A fresh complete source inventory is unavailable for this exact series.",
                ));
                continue;
            };
            let sources = inventory["sources"].as_array().cloned().unwrap_or_default();
            let mut ids_seen: Vec<&Value> = Vec::new();
            // Buckets in the order their first source appears.
            let mut buckets: Vec<(Value, Vec<Value>)> = Vec::new();
            let mut bad = false;
            for source in &sources {
                let status_ok = source["status"]
                    .as_str()
                    .is_some_and(|s| SOURCE_STATUSES.contains(&s));
                if !hex(&source["sourceId"])
                    || !hex(&source["bucketId"])
                    || ids_seen.contains(&&source["sourceId"])
                    || !status_ok
                {
                    bad = true;
                    break;
                }
                ids_seen.push(&source["sourceId"]);
                match buckets.iter_mut().find(|(id, _)| *id == source["bucketId"]) {
                    Some((_, list)) => list.push(source.clone()),
                    None => buckets.push((source["bucketId"].clone(), vec![source.clone()])),
                }
            }
            if bad {
                add(unknown(
                    "The source inventory has invalid or duplicate identities.",
                ));
                continue;
            }
            let valid_until = (observed + max_age)
                .min(observed_at(snapshot) + max_age)
                .min(observed_at(catalog) + max_age)
                .min(settlement);
            let observed_at_value = inventory.get("observedAt").cloned().unwrap_or(Value::Null);
            let skus: Vec<&Value> = fact["skus"]
                .as_array()
                .map(|s| s.iter().collect())
                .unwrap_or_default();
            match stage(fact, now) {
                "closed" => {
                    let reason = if fact["phase"] == "Scramble" {
                        "The month is past placement: its sources are set until Opening."
                    } else {
                        "The month takes no research in its phase."
                    };
                    let mut closed = with(&base, json!({ "status": "inactive", "reason": reason }));
                    phase(&mut closed, fact);
                    add(closed);
                }
                "openings" => {
                    // Opening: each frozen source still without an opening
                    // value takes one (bonded with its product's opening
                    // bond, archived after the Opening's start); an opened or
                    // pending one takes nothing.
                    let sku_of = |bucket: &Value| {
                        skus.iter()
                            .rev()
                            .find(|sku| hex(&sku["bucketId"]) && sku["bucketId"] == *bucket)
                            .copied()
                    };
                    // The oracle's calendar: Scramble's seven days, then the
                    // Opening's one until listing. Archive captures count
                    // from the Opening's start.
                    let scramble = timestamp(&fact["schedule"]["scrambleStartUtc"]);
                    let listing = timestamp(&fact["schedule"]["listingUtc"])
                        .or(scramble.map(|s| s + 8 * DAY_MS));
                    let start = listing.map(|l| l - DAY_MS);
                    for source in sources
                        .iter()
                        .filter(|s| s["status"] == "Frozen" && s["openingSubmitted"] != true)
                    {
                        let sku = sku_of(&source["bucketId"]);
                        let sku_field =
                            |key: &str| sku.map(|s| s[key].clone()).unwrap_or(Value::Null);
                        let bucket_label = if source["bucketLabel"].is_string() {
                            source["bucketLabel"].clone()
                        } else {
                            label(&sku_field("bucketLabel"))
                        };
                        let mut opening = with(
                            &base,
                            json!({ "kind": "opening", "bucketId": source["bucketId"],
                            "bucketLabel": bucket_label, "bucketName": named([Some(source), sku]),
                            "seriesId": series_id(source), "sourceId": source["sourceId"],
                            "sourceAddress": label(&source["sourceAddress"]), "url": url_of(source),
                            "observedAt": observed_at_value, "validUntil": valid_until,
                            "status": "gap", "reasons": ["opening_value"], "reason": null,
                            "openingBondAtomic": atomic(&sku_field("openingBondAtomic")),
                            "openingBudgetAtomic": atomic(&sku_field("openingBudgetAtomic")),
                            "openingStartUtc": start.map(iso).unwrap_or(Value::Null),
                            "listingUtc": listing.map(iso).unwrap_or(Value::Null), "sources": [] }),
                        );
                        phase(&mut opening, fact);
                        add(opening);
                    }
                }
                "updates" => {
                    // Game: each live source takes updates (a staked, archived new value).
                    let rewards = fact["rewards"]["remainingAtomic"].as_str().unwrap_or("0");
                    for source in sources.iter().filter(|s| s["status"] == "Active") {
                        let mut update = with(
                            &base,
                            json!({ "kind": "update", "bucketId": source["bucketId"],
                            "bucketLabel": label(&source["bucketLabel"]), "bucketName": named([Some(source)]),
                            "seriesId": series_id(source), "sourceId": source["sourceId"],
                            "sourceAddress": label(&source["sourceAddress"]), "url": url_of(source),
                            "observedAt": observed_at_value, "validUntil": valid_until,
                            "status": "gap", "reasons": ["game_update"], "reason": null,
                            "rewardsRemainingAtomic": rewards, "sources": [] }),
                        );
                        phase(&mut update, fact);
                        add(update);
                    }
                }
                _ => {
                    // Taking sources: every product the month lists (its SKU
                    // pools), and any bucket a source is in.
                    let mut labels: Vec<(Value, Value)> = Vec::new();
                    let mut names: Vec<(Value, Value)> = Vec::new();
                    let set = |list: &mut Vec<(Value, Value)>, key: &Value, value: Value| match list
                        .iter_mut()
                        .find(|(k, _)| k == key)
                    {
                        Some((_, v)) => *v = value,
                        None => list.push((key.clone(), value)),
                    };
                    for sku in &skus {
                        if !hex(&sku["bucketId"]) {
                            continue;
                        }
                        if !buckets.iter().any(|(id, _)| *id == sku["bucketId"]) {
                            buckets.push((sku["bucketId"].clone(), Vec::new()));
                        }
                        if sku["bucketLabel"].is_string() {
                            set(&mut labels, &sku["bucketId"], sku["bucketLabel"].clone());
                        }
                        let name = named([Some(*sku)]);
                        if !name.is_null() {
                            set(&mut names, &sku["bucketId"], name);
                        }
                    }
                    // A truly empty inventory is a confirmed market-wide
                    // research gap; no bucket identifier is synthesized from
                    // labels, dates or hashes.
                    if buckets.is_empty() {
                        buckets.push((Value::Null, Vec::new()));
                    }
                    let lookup = |list: &[(Value, Value)], key: &Value| {
                        list.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
                    };
                    let min_sources = directive["minActiveSources"].as_f64().unwrap_or(0.0);
                    let min_hosts = directive["minDistinctHosts"].as_f64().unwrap_or(0.0);
                    for (bucket_id, list) in &buckets {
                        let active: Vec<&Value> = list
                            .iter()
                            .filter(|s| s["status"].as_str().is_some_and(|st| PLACED.contains(&st)))
                            .collect();
                        let mut hosts: Vec<String> =
                            active.iter().filter_map(|s| host(s)).collect();
                        hosts.sort();
                        hosts.dedup();
                        let missing_locators = active.iter().any(|s| host(s).is_none());
                        let mut reasons = Vec::new();
                        if (active.len() as f64) < min_sources {
                            reasons.push("insufficient_active_sources");
                        }
                        if !missing_locators && (hosts.len() as f64) < min_hosts {
                            reasons.push("insufficient_distinct_hosts");
                        }
                        let hosts_uncertain = missing_locators && (hosts.len() as f64) < min_hosts;
                        let status = if !reasons.is_empty() {
                            "gap"
                        } else if hosts_uncertain {
                            "unknown"
                        } else {
                            "covered"
                        };
                        let bucket_label = list
                            .iter()
                            .find(|s| s["bucketLabel"].is_string())
                            .map(|s| s["bucketLabel"].clone())
                            .or_else(|| lookup(&labels, bucket_id))
                            .unwrap_or(Value::Null);
                        let bucket_name = match named(list.iter().map(Some)) {
                            Value::Null => lookup(&names, bucket_id).unwrap_or(Value::Null),
                            name => name,
                        };
                        let listed: Vec<Value> = list
                            .iter()
                            .map(|s| json!({ "sourceId": s["sourceId"], "status": s["status"], "url": url_of(s) }))
                            .collect();
                        let mut source = with(
                            &base,
                            json!({ "kind": "source", "bucketId": bucket_id,
                            "bucketLabel": bucket_label, "bucketName": bucket_name,
                            "observedAt": observed_at_value, "validUntil": valid_until, "status": status,
                            "reasons": reasons,
                            "reason": if hosts_uncertain { json!("Some active sources lack verified locators; host coverage is unknown.") } else { Value::Null },
                            "activeSources": active.len(),
                            "distinctHosts": if missing_locators { Value::Null } else { json!(hosts.len()) },
                            "knownHosts": hosts, "sources": listed }),
                        );
                        phase(&mut source, fact);
                        add(source);
                    }
                }
            }
        }
    }
    // Every series the catalog lists, assessed or not (a directive may leave
    // some out): what the workspace's work may still be about.
    let series: Vec<Value> = expiries
        .iter()
        .map(|(_, e)| json!({ "marketId": e["marketId"], "expiryId": e.get("expiryId").cloned().unwrap_or(Value::Null) }))
        .collect();
    Ok(
        json!({ "assessedAt": now, "protocol": snapshot["protocol"], "series": series, "assessments": assessments }),
    )
}

/// `^[a-z0-9][a-z0-9_-]{0,63}$`: a market id.
fn market_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$`: an expiry id.
fn expiry_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// A gap's title and brief, for the owner and the agents that take it.
pub fn describe_gap(assessment: &Value) -> (String, String) {
    let text = |key: &str| js(assessment.get(key));
    let market = text("marketId").to_uppercase();
    let expiry = text("expiryId");
    if assessment["kind"] == "claims" {
        return (
            format!("{market} \u{b7} {expiry} \u{b7} claims"),
            format!(
                "{expiry} settled and its rewards are frozen: claim what the owner's wallet earned."
            ),
        );
    }
    let filled = |key: &str| {
        assessment[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let product = filled("bucketName")
        .or_else(|| filled("bucketLabel"))
        .or_else(|| {
            filled("bucketId")
                .map(|id| format!("product {}", id.chars().take(10).collect::<String>()))
        })
        .unwrap_or_else(|| "source coverage".into());
    // On a chip month a source is a comparison series and a print one observation of it.
    let chip = assessment["chip"] == true;
    let or = |key: &str, fallback: String| {
        if assessment[key].is_null() {
            fallback
        } else {
            text(key)
        }
    };
    let page = or("url", format!("source {}", text("sourceId")));
    let address = or("sourceAddress", "unknown".into());
    let series = or("seriesId", "(see its definition)".into());
    if assessment["kind"] == "opening" {
        let bond = match usdc(&assessment["openingBondAtomic"]) {
            Some(amount) => format!("its product's opening bond ({amount} USDC)"),
            None => "its product's opening bond".into(),
        };
        let since = filled("openingStartUtc")
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let until = filled("listingUtc")
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        let mut brief = vec![
            format!(
                "Give this frozen source of {expiry} its opening value: {page} (source address {address}, product {product})."
            ),
            format!(
                "Read the value the page publishes for this exact product and field. Archive the page in the Wayback Machine; the capture time must be at or after the Opening's start{since} and not after the chain's clock."
            ),
            format!(
                "Submit it with petri, staking {bond}; after its challenge window (300 slots) accept it before listing{until}. A wrong one can be challenged and loses its bond; a source with no opening value by listing sits out the month."
            ),
        ];
        if chip {
            brief.push(format!("This source is comparison series {series}: its opening value is one observation of it, sent with petri oracle observe (the observation's details go with it)."));
        }
        return (
            format!("{market} \u{b7} {product} \u{b7} opening"),
            brief.join("\n"),
        );
    }
    if assessment["kind"] == "update" {
        let mut brief = vec![
            format!("Update this live source of {expiry} with its current value: {page} (source address {address}, product {product})."),
            "Read the value the page publishes for this exact product and field. Archive the page in the Wayback Machine first; the archived time must be after the month's listing.".to_string(),
            "Submit it as a Game update with petri: commit it with its stake, then reveal it with the archived URL. An update that does not change the value earns nothing; a wrong one can be challenged and loses its stake.".to_string(),
        ];
        if chip {
            brief.push(format!("This source is comparison series {series}: each update is one observation of it, sent with petri oracle observe."));
        }
        return (
            format!("{market} \u{b7} {product} \u{b7} update"),
            brief.join("\n"),
        );
    }
    let bucket = match filled("bucketId") {
        Some(id) => format!(" / bucket {id}"),
        None => ". Establish the governed product group before submitting a candidate".into(),
    };
    let hosts = if assessment["distinctHosts"].is_null() {
        "unknown".into()
    } else {
        text("distinctHosts")
    };
    let brief = [
        format!("Research additional public sources for {expiry}{bucket}."),
        "A page behind the owner's sign-in (a subscription, a distributor portal) counts too, proven with petri evidence prove after the owner signs in themselves.".to_string(),
        format!("Observed active sources: {}; target: {}.", text("activeSources"), js(assessment["targets"].get("activeSources"))),
        format!("Observed distinct hosts: {hosts}; target: {}.", js(assessment["targets"].get("distinctHosts"))),
        if chip {
            format!("This month is on commodity chip buckets: each source is a comparison series of {product} (a published contract assessment, or a direct contract price), defined field by field as the commodity spec requires. Never guess a field: leave what the source doesn't say unset.")
        } else {
            "Match the exact governed product, region and price definition. Avoid existing source URLs. Human guidance below refines the research scope.".to_string()
        },
        "Completion requires a later fresh oracle inventory meeting the coverage targets. Submitting candidates or finishing a run does not resolve this gap.".to_string(),
    ];
    (format!("{market} \u{b7} {product}"), brief.join("\n"))
}

#[cfg(test)]
mod parity {
    use super::{assess_coverage, describe_gap};
    use serde_json::Value;

    /// The Node coverage policy (research/service/coverage-policy.mjs, which
    /// this replaced in 2026-10) on 46 sets of oracle documents: its assessments,
    /// the briefs of its work and its refusals, generated by it.
    #[test]
    fn coverage_matches_the_node_service_word_for_word() {
        let cases: Value = serde_json::from_str(include_str!("coverage-fixtures.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let name = &case["name"];
            let directives = case["directives"].as_array().unwrap();
            let now = case["now"].as_i64().unwrap();
            let expected = &case["expected"];
            match assess_coverage(&case["input"], directives, now) {
                Err(error) => {
                    assert_eq!(error.code.as_deref(), expected["error"].as_str(), "{name}")
                }
                Ok(result) => {
                    assert!(
                        expected["error"].is_null(),
                        "{name}: expected {}",
                        expected["error"]
                    );
                    let (actual, wanted) =
                        (&result["assessments"], &expected["result"]["assessments"]);
                    assert_eq!(
                        actual.as_array().map(Vec::len),
                        wanted.as_array().map(Vec::len),
                        "{name}"
                    );
                    for (index, (a, w)) in actual
                        .as_array()
                        .unwrap()
                        .iter()
                        .zip(wanted.as_array().unwrap())
                        .enumerate()
                    {
                        for key in a
                            .as_object()
                            .unwrap()
                            .keys()
                            .chain(w.as_object().unwrap().keys())
                        {
                            assert_eq!(a.get(key), w.get(key), "{name}: assessment {index}, {key}");
                        }
                        let (title, brief) = describe_gap(a);
                        assert_eq!(
                            title, expected["briefs"][index]["title"],
                            "{name}: title {index}"
                        );
                        assert_eq!(
                            brief, expected["briefs"][index]["brief"],
                            "{name}: brief {index}"
                        );
                    }
                    assert_eq!(result, expected["result"], "{name}");
                }
            }
        }
    }
}
