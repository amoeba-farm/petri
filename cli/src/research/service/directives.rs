//! The owner's standing research directives (coverage targets per market),
//! what discovery made of them (work, kept per discovery key) and the
//! owner's guidance on each work item.

use super::{
    coverage::describe_gap,
    db::{self, Row, atomic, count, parsed},
};
use crate::research::workspace::{Failure, digest, fail, fail_status, js_trim, object, text, uuid};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashSet};

fn view(row: &Row) -> Value {
    json!({ "id": row["id"], "marketIds": parsed(row, "markets_json"), "minActiveSources": row["min_sources"],
        "minDistinctHosts": row["min_hosts"], "maxObservationAgeMs": row["max_age_ms"], "enabled": row["enabled"] == 1,
        "revision": row["revision"], "createdAt": row["created_at"], "updatedAt": row["updated_at"] })
}

/// A JSON number that is a whole number (`Number.isInteger`).
fn integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|n| n.fract() == 0.0 && n.abs() < 9.0e15)
            .map(|n| n as i64)
    })
}

fn market_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        let bytes = id.as_bytes();
        (1..=64).contains(&bytes.len())
            && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    })
}

/// JavaScript's truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

pub fn save(db: &Connection, now: i64, body: &Value) -> Result<Value, Failure> {
    let body = object(
        body,
        &[
            "id",
            "marketIds",
            "minActiveSources",
            "minDistinctHosts",
            "maxObservationAgeMs",
            "enabled",
        ],
        &["marketIds", "minActiveSources", "minDistinctHosts"],
    )?;
    let invalid = |message: &str| fail("INVALID_DIRECTIVE", message);
    let ids = body["marketIds"]
        .as_array()
        .filter(|ids| ids.iter().all(market_id))
        .ok_or_else(|| {
            invalid("Choose canonical market IDs, or leave the list empty for all markets.")
        })?;
    let target = |key: &str| {
        integer(&body[key])
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| invalid("Coverage targets must be whole numbers from 1 to 64."))
    };
    let (sources, hosts) = (target("minActiveSources")?, target("minDistinctHosts")?);
    if hosts > sources {
        return Err(invalid("The host target cannot exceed the source target."));
    }
    let age = match body.get("maxObservationAgeMs") {
        None | Some(Value::Null) => Some(600_000),
        Some(age) => integer(age),
    };
    let enabled = body.get("enabled");
    let (Some(age), true) = (
        age.filter(|age| *age >= 1),
        enabled.is_none_or(Value::is_boolean),
    ) else {
        return Err(invalid("Invalid directive freshness or enabled setting."));
    };
    let old = match body.get("id").filter(|id| truthy(id)) {
        Some(id) => Some(
            db::get(
                db,
                "SELECT * FROM directives WHERE id=?",
                [text(id, 36, "directive ID")?],
            )?
            .ok_or_else(|| fail_status("DIRECTIVE_NOT_FOUND", "Directive not found.", 404))?,
        ),
        None => None,
    };
    if old.is_none() && count(db, "SELECT count(*) AS n FROM directives", [])? >= 32 {
        return Err(fail_status(
            "DIRECTIVE_LIMIT",
            "The workspace supports at most 32 standing directives.",
            429,
        ));
    }
    let id = old
        .as_ref()
        .and_then(|o| o["id"].as_str().map(str::to_string))
        .unwrap_or_else(uuid);
    let markets: BTreeSet<&str> = ids.iter().filter_map(Value::as_str).collect();
    let enabled = match enabled {
        Some(Value::Bool(enabled)) => *enabled,
        _ => old.as_ref().is_none_or(|o| o["enabled"] == 1),
    };
    let revision = old
        .as_ref()
        .and_then(|o| o["revision"].as_i64())
        .unwrap_or(0)
        + 1;
    let created = old
        .as_ref()
        .and_then(|o| o["created_at"].as_i64())
        .unwrap_or(now);
    atomic(db, || {
        db::run(
            db,
            "INSERT INTO directives VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET markets_json=excluded.markets_json,
        min_sources=excluded.min_sources,min_hosts=excluded.min_hosts,max_age_ms=excluded.max_age_ms,enabled=excluded.enabled,
        revision=excluded.revision,updated_at=excluded.updated_at",
            params![id, json!(markets).to_string(), sources, hosts, age, i64::from(enabled), revision, created, now],
        )?;
        // Editing targets withdraws old offers until they have been reassessed.
        for row in db::all(
            db,
            "SELECT id,objective_json FROM work WHERE objective_json IS NOT NULL",
            [],
        )? {
            if parsed(&row, "objective_json")["directiveId"] == id.as_str() {
                db::run(
                    db,
                    "UPDATE work SET availability='unknown',valid_until=NULL WHERE id=?",
                    [row["id"].as_str()],
                )?;
            }
        }
        Ok(())
    })?;
    let row = db::get(db, "SELECT * FROM directives WHERE id=?", [&id])?.unwrap_or_default();
    Ok(view(&row))
}

/// Discovery's assessment, as work: a gap is offered, covered work is
/// resolved, the rest withheld until assessed again.
pub fn record(db: &Connection, now: i64, result: &Value) -> Result<(), Failure> {
    atomic(db, || {
        db::run(
            db,
            "UPDATE work SET availability='unknown',valid_until=NULL WHERE discovery_key IS NOT NULL",
            [],
        )?;
        let empty = Vec::new();
        let assessments = result["assessments"].as_array().unwrap_or(&empty);
        for assessment in assessments {
            // An opening or update is one per source, claims one per month; a
            // source gap keeps its original key.
            let kind = assessment["kind"].as_str();
            let per_source = matches!(kind, Some("update" | "opening"));
            let protocol = &assessment["protocol"];
            let mut parts = vec![
                protocol["programId"].clone(),
                protocol["cluster"].clone(),
                protocol["namespace"].clone(),
                assessment["directiveId"].clone(),
                assessment["marketId"].clone(),
                assessment["expiryId"].clone(),
                assessment["oracleMonthAddress"].clone(),
                assessment["bucketId"].clone(),
            ];
            if per_source {
                parts.extend([assessment["kind"].clone(), assessment["sourceId"].clone()]);
            } else if kind == Some("claims") {
                parts.push(json!("claims"));
            }
            let key = digest(json!(parts).to_string());
            let previous = db::get(db, "SELECT id FROM work WHERE discovery_key=?", [&key])?;
            let status = assessment["status"].as_str().unwrap_or_default();
            if status != "gap" && previous.is_none() {
                continue;
            }
            if previous.is_none() && count(db, "SELECT count(*) AS n FROM work", [])? >= 5000 {
                return Err(fail_status(
                    "WORK_LIMIT",
                    "Workspace research history capacity reached.",
                    429,
                ));
            }
            let (title, brief) = describe_gap(assessment);
            let mut scope = Map::new();
            scope.insert("marketId".into(), assessment["marketId"].clone());
            scope.insert("expiryId".into(), assessment["expiryId"].clone());
            if truthy(&assessment["bucketId"]) {
                scope.insert("bucketId".into(), assessment["bucketId"].clone());
            }
            if per_source {
                scope.insert("sourceId".into(), assessment["sourceId"].clone());
            }
            let availability = match status {
                "gap" => "available",
                "covered" => "resolved",
                other => other,
            };
            let id = previous
                .as_ref()
                .and_then(|p| p["id"].as_str().map(str::to_string))
                .unwrap_or_else(uuid);
            db::run(
                db,
                "INSERT INTO work(id,title,brief,created_at,scope_json,objective_json,discovery_key,availability,valid_until)
          VALUES (?,?,?,?,?,?,?,?,?) ON CONFLICT(discovery_key) WHERE discovery_key IS NOT NULL DO UPDATE SET title=excluded.title,
          brief=excluded.brief,objective_json=excluded.objective_json,availability=excluded.availability,valid_until=excluded.valid_until",
                params![
                    id,
                    title,
                    brief,
                    now,
                    Value::Object(scope).to_string(),
                    assessment.to_string(),
                    key,
                    availability,
                    assessment["validUntil"].as_i64()
                ],
            )?;
        }
        // A series the oracle no longer lists takes its work with it:
        // archived, and back if the series is listed again. Work of a series
        // still listed stays, whatever its assessment; the owner's own
        // archive is theirs.
        let name = |item: &Value| json!([item["marketId"], item["expiryId"]]).to_string();
        let listed: HashSet<String> = result["series"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .chain(assessments)
            .map(name)
            .collect();
        for row in db::all(
            db,
            "SELECT id,scope_json,objective_json,retired_at FROM work WHERE discovery_key IS NOT NULL AND (archived_at IS NULL OR retired_at IS NOT NULL)",
            [],
        )? {
            let item = match row["scope_json"].as_str() {
                Some(_) => parsed(&row, "scope_json"),
                None => parsed(&row, "objective_json"),
            };
            let served = listed.contains(&name(&item));
            let id = row["id"].as_str();
            if !served && row["retired_at"].is_null() {
                db::run(
                    db,
                    "UPDATE work SET archived_at=?,retired_at=? WHERE id=?",
                    params![now, now, id],
                )?;
            } else if served && !row["retired_at"].is_null() {
                db::run(
                    db,
                    "UPDATE work SET archived_at=NULL,retired_at=NULL WHERE id=?",
                    [id],
                )?;
            }
        }
        let mut status = result.as_object().cloned().unwrap_or_default();
        status.insert("error".into(), Value::Null);
        set_status(db, &Value::Object(status))
    })
}

fn set_status(db: &Connection, status: &Value) -> Result<(), Failure> {
    db::run(
        db,
        "INSERT INTO settings VALUES ('discovery',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [status.to_string()],
    )?;
    Ok(())
}

pub fn list(db: &Connection) -> Result<Vec<Value>, Failure> {
    Ok(
        db::all(db, "SELECT * FROM directives ORDER BY created_at,id", [])?
            .iter()
            .map(view)
            .collect(),
    )
}

/// A new workspace's directive: every market, 3 active sources and 2 hosts
/// per product group. Work from before directives is archived.
pub fn seed(db: &Connection, now: i64) -> Result<(), Failure> {
    if db::get(
        db,
        "SELECT value FROM settings WHERE key='directives_initialized'",
        [],
    )?
    .is_some()
    {
        return Ok(());
    }
    save(
        db,
        now,
        &json!({ "marketIds": [], "minActiveSources": 3, "minDistinctHosts": 2 }),
    )?;
    db::run(
        db,
        "UPDATE work SET archived_at=coalesce(archived_at,?) WHERE discovery_key IS NULL",
        [now],
    )?;
    db::run(
        db,
        "INSERT INTO settings VALUES ('directives_initialized','true')",
        [],
    )?;
    Ok(())
}

/// Discovery failed: offers are withheld until a fresh complete assessment.
pub fn failed(db: &Connection) -> Result<(), Failure> {
    atomic(db, || {
        db::run(
            db,
            "UPDATE work SET availability='unknown',valid_until=NULL WHERE discovery_key IS NOT NULL",
            [],
        )?;
        let mut status = status(db)?.as_object().cloned().unwrap_or_default();
        status.insert(
            "error".into(),
            json!("Oracle coverage could not be refreshed. New work is withheld until a fresh complete assessment is available."),
        );
        set_status(db, &Value::Object(status))
    })
}

pub fn status(db: &Connection) -> Result<Value, Failure> {
    Ok(
        db::get(db, "SELECT value FROM settings WHERE key='discovery'", [])?
            .map(|row| parsed(&row, "value"))
            .unwrap_or_else(|| json!({ "assessedAt": null, "assessments": [], "error": null })),
    )
}

/// The owner's guidance on a work item (empty clears it).
pub fn refine(
    db: &Connection,
    now: i64,
    work_id: &Value,
    instruction: &Value,
) -> Result<Value, Failure> {
    let row = db::get(
        db,
        "SELECT * FROM work WHERE id=?",
        [text(work_id, 36, "work ID")?],
    )?
    .filter(|row| {
        row["objective_json"]
            .as_str()
            .is_some_and(|o| !o.is_empty())
    })
    .ok_or_else(|| fail_status("WORK_NOT_FOUND", "Choose a discovered coverage gap.", 404))?;
    let instruction_text = instruction
        .as_str()
        .ok_or_else(|| fail("INVALID_GUIDANCE", "Provide guidance text."))?;
    let value = if js_trim(instruction_text).is_empty() {
        String::new()
    } else {
        text(instruction, 8000, "research guidance")?
    };
    let old = parsed(&row, "guidance_json");
    let guidance = json!({ "instruction": value, "authorType": "human",
        "revision": old["revision"].as_i64().unwrap_or(0) + 1, "updatedAt": now });
    db::run(
        db,
        "UPDATE work SET guidance_json=? WHERE id=?",
        params![guidance.to_string(), work_id.as_str()],
    )?;
    Ok(guidance)
}
