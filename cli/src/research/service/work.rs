//! Research work and what agents do with it: runs, their event feed (the
//! owner's live view), sessions and candidate submissions.

use super::db::{self, Row, atomic, count, parsed};
use crate::research::workspace::{Failure, digest, fail, fail_status, secret_in_url, text, uuid};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// What an agent reports as it works, one short line each: the owner's live
/// feed in the TUI. search: the query; read: a page it opened (its URL);
/// value: a value read off a page; submit: a petri write it made.
pub const REPORT_KINDS: [&str; 7] = [
    "search",
    "read",
    "value",
    "submit",
    "note",
    "source_found",
    "blocked",
];

/// An agent counts as researching for the owner's view while it has checked
/// in this recently: a display rule (the TUI's "researching"), never a limit
/// on agents.
const ACTIVE_MS: i64 = 20 * 60_000;

/// A work item as agents and the owner see it.
pub fn job(row: &Row, now: i64) -> Value {
    let discovered = row["discovery_key"]
        .as_str()
        .is_some_and(|key| !key.is_empty());
    let availability = row["availability"].as_str().filter(|a| !a.is_empty());
    // A discovered offer lapses with its oracle observations.
    let lapsed = discovered
        && availability == Some("available")
        && row["valid_until"].as_i64().unwrap_or(0) <= now;
    json!({ "id": row["id"], "title": row["title"], "brief": row["brief"], "createdAt": row["created_at"],
        "archivedAt": row["archived_at"], "candidateScope": parsed(row, "scope_json"),
        "objective": parsed(row, "objective_json"), "guidance": parsed(row, "guidance_json"),
        "focused": !row["focused_at"].is_null(),
        "availability": if lapsed { "unknown" } else { availability.unwrap_or("available") } })
}

fn run_view(row: &Row) -> Value {
    json!({ "id": row["id"], "workId": row["work_id"], "title": row["title"], "agentId": row["agent_id"],
        "sessionId": row["session_id"], "createdAt": row["created_at"], "finishedAt": row["finished_at"],
        "summary": row["summary"], "researchContext": parsed(row, "context_json") })
}

fn session_view(row: &Row) -> Value {
    json!({ "id": row["id"], "agentId": row["agent_id"], "harness": row["harness"], "createdAt": row["created_at"],
        "heartbeatAt": row["heartbeat_at"], "closedAt": row["closed_at"] })
}

fn event_view(row: &Row) -> Value {
    json!({ "sequence": row["seq"], "runId": row["run_id"], "actor": row["actor"], "kind": row["kind"],
        "message": row["message"], "referenceUrl": row["reference_url"], "createdAt": row["created_at"] })
}

/// A JSON number or digit string as an amount: whether it is above zero.
fn earns(atoms: &Value) -> f64 {
    let positive = match atoms {
        Value::String(s) => {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.bytes().any(|b| b != b'0')
        }
        Value::Number(n) => n.as_u64().is_some_and(|n| n > 0),
        _ => false,
    };
    if positive { 0.5 } else { 0.0 }
}

/// How much of its targets a gap still lacks, 0 to 1 (the TUI's temperature).
fn need(work: &Value) -> f64 {
    let o = &work["objective"];
    if o["targets"].is_null() || o["targets"] == false {
        return 0.0;
    }
    // An update earns from its month's remaining rewards, an opening from its
    // product's opening budget, if any.
    match o["kind"].as_str() {
        Some("update") => return earns(&o["rewardsRemainingAtomic"]),
        Some("opening") => return earns(&o["openingBudgetAtomic"]),
        Some("claims") => return 1.0,
        _ => {}
    }
    let short = |have: &Value, want: &Value| match want.as_f64() {
        Some(want) if want > 0.0 => ((want - have.as_f64().unwrap_or(0.0)) / want).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let sources = short(&o["activeSources"], &o["targets"]["activeSources"]);
    let hosts = if o["distinctHosts"].is_number() {
        short(&o["distinctHosts"], &o["targets"]["distinctHosts"])
    } else {
        0.0
    };
    sources.max(hosts)
}

/// What the owner chose to research first, then the newest.
pub fn list_work(
    db: &Connection,
    now: i64,
    include_archived: bool,
    include_unavailable: bool,
) -> Result<Vec<Value>, Failure> {
    let sql = format!(
        "SELECT * FROM work {} ORDER BY focused_at IS NULL, focused_at DESC, created_at DESC LIMIT 5000",
        if include_archived {
            ""
        } else {
            "WHERE archived_at IS NULL"
        }
    );
    Ok(db::all(db, &sql, [])?
        .iter()
        .map(|row| job(row, now))
        .filter(|item| include_unavailable || item["availability"] == "available")
        .take(200)
        .collect())
}

/// Each agent's newest open run, while its agent is still checking in.
pub fn active_runs(db: &Connection, now: i64) -> Result<Vec<Value>, Failure> {
    // A run outlives its session (a new session resumes it): the agent's
    // newest open session says whether it is still at work.
    let rows = db::all(
        db,
        "SELECT runs.id,runs.work_id,runs.agent_id,runs.created_at,work.title,seen.heartbeat_at FROM runs
      JOIN work ON work.id=runs.work_id
      JOIN (SELECT agent_id,max(heartbeat_at) AS heartbeat_at FROM sessions WHERE closed_at IS NULL GROUP BY agent_id) AS seen ON seen.agent_id=runs.agent_id
      WHERE runs.finished_at IS NULL AND seen.heartbeat_at>=?
      ORDER BY runs.created_at DESC",
        [now - ACTIVE_MS],
    )?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in rows {
        if !seen.insert(row["agent_id"].to_string()) {
            continue;
        }
        let last = db::get(
            db,
            "SELECT message,created_at FROM events WHERE run_id=? AND actor='agent' ORDER BY seq DESC LIMIT 1",
            [row["id"].as_str()],
        )?;
        out.push(json!({ "agentId": row["agent_id"], "workId": row["work_id"], "runId": row["id"],
            "title": row["title"], "startedAt": row["created_at"], "lastSeenAt": row["heartbeat_at"],
            "lastMessage": last.map(|l| l["message"].clone()).unwrap_or(Value::Null) }));
    }
    Ok(out)
}

/// The agent's open session `id`, kept alive.
fn session(db: &Connection, now: i64, agent_id: &str, id: &Value) -> Result<Row, Failure> {
    let expired = || fail_status("SESSION_EXPIRED", "Open a new agent session.", 401);
    let id = id.as_str().ok_or_else(expired)?;
    let value = db::get(
        db,
        "SELECT * FROM sessions WHERE id=? AND agent_id=? AND closed_at IS NULL",
        [id, agent_id],
    )?
    .ok_or_else(expired)?;
    let at = |key: &str| value[key].as_i64().unwrap_or(0);
    if at("heartbeat_at") < now - 12 * 3_600_000 || at("created_at") < now - 7 * 86_400_000 {
        return Err(expired());
    }
    db::run(
        db,
        "UPDATE sessions SET heartbeat_at=? WHERE id=?",
        params![now, id],
    )?;
    Ok(value)
}

pub fn owned_run(db: &Connection, agent_id: &str, id: &str) -> Result<Row, Failure> {
    db::get(
        db,
        "SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE runs.id=? AND agent_id=?",
        [id, agent_id],
    )?
    .ok_or_else(|| fail_status("RUN_NOT_FOUND", "Research run not found.", 404))
}

#[allow(clippy::too_many_arguments)]
fn add_event(
    db: &Connection,
    now: i64,
    run_id: &str,
    actor: &str,
    kind: &str,
    message: &str,
    reference: Option<&str>,
    client_key: Option<&str>,
) -> Result<(), Failure> {
    db::run(
        db,
        "INSERT INTO events(run_id,actor,kind,message,reference_url,created_at,client_key) VALUES (?,?,?,?,?,?,?)",
        params![run_id, actor, kind, message, reference, now, client_key],
    )?;
    Ok(())
}

/// A new run opens with the run's work and the owner's directive for it, if any.
fn started(db: &Connection, now: i64, run_id: &str, work: &Value) -> Result<(), Failure> {
    let title = work["title"].as_str().unwrap_or_default();
    add_event(
        db,
        now,
        run_id,
        "petri",
        "run_started",
        &format!("Started: {title}"),
        None,
        None,
    )?;
    if let Some(instruction) = work["guidance"]["instruction"]
        .as_str()
        .filter(|i| !i.is_empty())
    {
        add_event(
            db,
            now,
            run_id,
            "owner",
            "directive",
            instruction,
            None,
            None,
        )?;
    }
    Ok(())
}

fn id_of(agent: &Value) -> &str {
    agent["id"].as_str().unwrap_or_default()
}

/// A JavaScript template literal's text of `value`.
fn js_text(value: &Value) -> String {
    match value {
        Value::Null => "undefined".into(),
        Value::String(s) => s.clone(),
        Value::Object(_) => "[object Object]".into(),
        other => other.to_string(),
    }
}

/// The agent's next research run: the owner's picks first, then what needs
/// the most, skipping what another agent is on while others are open. It
/// resumes the agent's open run on the same work and closes its other open
/// runs, so one agent researches one thing at a time.
pub fn next(
    db: &Connection,
    now: i64,
    agent: &Value,
    session_id: &Value,
) -> Result<Value, Failure> {
    let agent_id = id_of(agent);
    atomic(db, || {
        let current = session(db, now, agent_id, session_id)?;
        let others: HashSet<String> = active_runs(db, now)?
            .iter()
            .filter(|run| run["agentId"].as_str() != Some(agent_id))
            .map(|run| js_text(&run["workId"]))
            .collect();
        let open = db::all(
            db,
            "SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? AND finished_at IS NULL ORDER BY runs.created_at DESC",
            [agent_id],
        )?;
        // An opening or update stakes the owner's USDC: one that earns nothing
        // (no rewards for it this month) comes up only when the owner picked it.
        let staked = |item: &Value| {
            item["objective"]["kind"] == "update" || item["objective"]["kind"] == "opening"
        };
        // A month's claims come up once the workspace worked on that month,
        // until a run on them finishes (again when the owner picks them).
        let month = |scope: &Value| {
            (!scope.is_null()).then(|| {
                format!(
                    "{}:{}",
                    js_text(&scope["marketId"]),
                    js_text(&scope["expiryId"])
                )
            })
        };
        let worked: HashSet<Option<String>> = db::all(
            db,
            "SELECT DISTINCT work.scope_json FROM runs JOIN work ON work.id=runs.work_id",
            [],
        )?
        .iter()
        .map(|row| month(&parsed(row, "scope_json")))
        .collect();
        let mut due = Vec::new();
        for item in list_work(db, now, false, false)? {
            let objective = &item["objective"];
            if objective.is_null()
                || !(!staked(&item) || item["focused"] == true || need(&item) > 0.0)
            {
                continue;
            }
            if objective["kind"] == "claims" {
                let id = item["id"].as_str().unwrap_or_default();
                let done = db::get(
                    db,
                    "SELECT max(finished_at) AS t FROM runs WHERE work_id=?",
                    [id],
                )?
                .and_then(|row| row["t"].as_i64());
                let picked = db::get(db, "SELECT focused_at FROM work WHERE id=?", [id])?
                    .and_then(|row| row["focused_at"].as_i64());
                let fresh = match (done, picked) {
                    (None, _) => true,
                    (Some(done), Some(picked)) => picked > done,
                    (Some(_), None) => false,
                };
                if !(worked.contains(&month(&item["candidateScope"])) && fresh) {
                    continue;
                }
            }
            due.push(item);
        }
        // Openings first after the owner's picks: the Opening lasts a day,
        // and a source with no opening value by listing sits out the month.
        let opening = |item: &Value| u8::from(item["objective"]["kind"] == "opening");
        let focused = |item: &Value| u8::from(item["focused"] == true);
        let mut ranked = due;
        ranked.sort_by(|a, b| {
            focused(b)
                .cmp(&focused(a))
                .then(opening(b).cmp(&opening(a)))
                .then(
                    need(b)
                        .partial_cmp(&need(a))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        let free: Vec<&Value> = ranked
            .iter()
            .filter(|item| !others.contains(&js_text(&item["id"])))
            .collect();
        let top = if free.is_empty() {
            ranked.first()
        } else {
            free.first().copied()
        };
        let resume = open
            .iter()
            .find(|run| top.is_some_and(|top| run["work_id"] == top["id"]))
            .or_else(|| {
                if top.is_some_and(|top| top["focused"] == true) {
                    return None;
                }
                open.iter()
                    .find(|run| ranked.iter().any(|item| item["id"] == run["work_id"]))
            });
        let chosen = match resume {
            Some(resume) => ranked.iter().find(|item| item["id"] == resume["work_id"]),
            None => top,
        };
        for run in &open {
            if resume.is_some_and(|resume| resume["id"] == run["id"]) {
                continue;
            }
            let summary = match chosen {
                Some(chosen) => format!("Moved on to {}.", js_text(&chosen["title"])),
                None => "Nothing left to research.".into(),
            };
            let id = run["id"].as_str().unwrap_or_default();
            db::run(
                db,
                "UPDATE runs SET finished_at=?,summary=? WHERE id=?",
                params![now, summary, id],
            )?;
            add_event(db, now, id, "petri", "run_finished", &summary, None, None)?;
        }
        let Some(chosen) = chosen else {
            return Ok(json!({ "status": "idle" }));
        };
        if let Some(resume) = resume {
            let run = owned_run(db, agent_id, resume["id"].as_str().unwrap_or_default())?;
            return Ok(
                json!({ "status": "assigned", "resumed": true, "run": run_view(&run), "assignment": chosen }),
            );
        }
        let id = uuid();
        db::run(
            db,
            "INSERT INTO runs(id,work_id,agent_id,session_id,client_key,created_at,finished_at,summary,context_json) VALUES (?,?,?,?,?,?,NULL,NULL,?)",
            params![
                id,
                chosen["id"].as_str(),
                agent_id,
                current["id"].as_str(),
                format!("next:{id}"),
                now,
                chosen.to_string()
            ],
        )?;
        started(db, now, &id, chosen)?;
        let run = owned_run(db, agent_id, &id)?;
        Ok(
            json!({ "status": "assigned", "resumed": false, "run": run_view(&run), "assignment": chosen }),
        )
    })
}

/// The owner's new directive for a work item, in the feed of every agent on
/// it as it is set (a run that starts later opens with it).
pub fn directed(
    db: &Connection,
    now: i64,
    work_id: &Value,
    instruction: &str,
) -> Result<(), Failure> {
    let message = if instruction.is_empty() {
        "Directive cleared."
    } else {
        instruction
    };
    for run in db::all(
        db,
        "SELECT id FROM runs WHERE work_id=? AND finished_at IS NULL",
        [work_id.as_str()],
    )? {
        add_event(
            db,
            now,
            run["id"].as_str().unwrap_or_default(),
            "owner",
            "directive",
            message,
            None,
            None,
        )?;
    }
    Ok(())
}

/// The owner's "Research this" (or its undoing). Agents may still take any
/// objective; focused ones come first in their list.
pub fn focus_work(db: &Connection, now: i64, id: &str, focused: &Value) -> Result<bool, Failure> {
    let focused = focused
        .as_bool()
        .ok_or_else(|| fail("INVALID_INPUT", "Say whether to focus this objective."))?;
    if db::get(
        db,
        "SELECT id FROM work WHERE id=? AND archived_at IS NULL",
        [id],
    )?
    .is_none()
    {
        return Err(fail_status(
            "WORK_NOT_FOUND",
            "Choose a discovered coverage gap.",
            404,
        ));
    }
    db::run(
        db,
        "UPDATE work SET focused_at=? WHERE id=?",
        params![focused.then_some(now), id],
    )?;
    Ok(focused)
}

pub fn archive_work(db: &Connection, now: i64, id: &str) -> Result<(), Failure> {
    if db::get(db, "SELECT id FROM work WHERE id=?", [id])?.is_none() {
        return Err(fail_status("WORK_NOT_FOUND", "Assignment not found.", 404));
    }
    // The owner's archive stays, whatever discovery sees later.
    db::run(
        db,
        "UPDATE work SET archived_at=coalesce(archived_at,?),retired_at=NULL WHERE id=?",
        params![now, id],
    )?;
    Ok(())
}

pub fn open_session(
    db: &Connection,
    now: i64,
    agent: &Value,
    harness: &Value,
) -> Result<Value, Failure> {
    let id = uuid();
    let harness = if truthy(harness) {
        harness
    } else {
        &agent["harness"]
    };
    let harness = text(harness, 60, "harness name")?;
    db::run(
        db,
        "INSERT INTO sessions VALUES (?,?,?,?,?,NULL)",
        params![id, id_of(agent), harness, now, now],
    )?;
    let row = db::get(db, "SELECT * FROM sessions WHERE id=?", [&id])?.unwrap_or_default();
    Ok(session_view(&row))
}

pub fn heartbeat(db: &Connection, now: i64, agent: &Value, id: &str) -> Result<Value, Failure> {
    session(db, now, id_of(agent), &json!(id))?;
    let row = db::get(db, "SELECT * FROM sessions WHERE id=?", [id])?.unwrap_or_default();
    Ok(session_view(&row))
}

pub fn close_session(db: &Connection, now: i64, agent: &Value, id: &str) -> Result<(), Failure> {
    session(db, now, id_of(agent), &json!(id))?;
    db::run(
        db,
        "UPDATE sessions SET closed_at=? WHERE id=?",
        params![now, id],
    )?;
    Ok(())
}

pub fn start_run(db: &Connection, now: i64, agent: &Value, body: &Value) -> Result<Value, Failure> {
    let agent_id = id_of(agent);
    let client_key = text(&body["clientKey"], 80, "request key")?;
    atomic(db, || {
        session(db, now, agent_id, &body["sessionId"])?;
        if let Some(previous) = db::get(
            db,
            "SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? AND client_key=?",
            [agent_id, &client_key],
        )? {
            if previous["work_id"] != body["workId"] {
                return Err(fail_status(
                    "REQUEST_CONFLICT",
                    "This request key belongs to another assignment.",
                    409,
                ));
            }
            return Ok(run_view(&previous));
        }
        let work = match body["workId"].as_str() {
            Some(id) => db::get(
                db,
                "SELECT * FROM work WHERE id=? AND archived_at IS NULL",
                [id],
            )?,
            None => None,
        }
        .map(|row| job(&row, now))
        .filter(|work| work["availability"] == "available")
        .ok_or_else(|| {
            fail_status(
                "WORK_NOT_FOUND",
                "This research objective is unavailable or needs a fresh oracle assessment.",
                404,
            )
        })?;
        if count(
            db,
            "SELECT count(*) AS n FROM runs WHERE agent_id=? AND finished_at IS NULL",
            [agent_id],
        )? >= 20
        {
            return Err(fail_status(
                "RUN_LIMIT",
                "Finish an existing research run before opening another.",
                429,
            ));
        }
        let id = uuid();
        db::run(
            db,
            "INSERT INTO runs(id,work_id,agent_id,session_id,client_key,created_at,finished_at,summary,context_json) VALUES (?,?,?,?,?,?,NULL,NULL,?)",
            params![
                id,
                work["id"].as_str(),
                agent_id,
                body["sessionId"].as_str(),
                client_key,
                now,
                work.to_string()
            ],
        )?;
        started(db, now, &id, &work)?;
        let mut run = run_view(&owned_run(db, agent_id, &id)?);
        run["assignment"] = work;
        Ok(run)
    })
}

/// Whether JavaScript takes `value` as true (what `||` keeps).
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

fn reference_url(value: &Value) -> Result<Option<String>, Failure> {
    if !truthy(value) {
        return Ok(None);
    }
    let reference = text(value, 1024, "source URL")?;
    let url = reqwest::Url::parse(&reference)
        .map_err(|_| fail("INVALID_REFERENCE", "Use an HTTPS reference."))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(fail(
            "INVALID_REFERENCE",
            "Use an HTTPS reference without credentials.",
        ));
    }
    if secret_in_url(&reference) {
        return Err(fail(
            "INVALID_REFERENCE",
            "Give the page URL without any token, key or session id in it.",
        ));
    }
    Ok(Some(reference))
}

pub fn report(
    db: &Connection,
    now: i64,
    agent: &Value,
    run_id: &str,
    body: &Value,
) -> Result<Value, Failure> {
    let agent_id = id_of(agent);
    atomic(db, || {
        session(db, now, agent_id, &body["sessionId"])?;
        let run = owned_run(db, agent_id, run_id)?;
        let message = text(&body["message"], 4000, "progress message")?;
        let client_key = text(&body["clientKey"], 80, "request key")?;
        let kind = body["kind"]
            .as_str()
            .filter(|kind| REPORT_KINDS.contains(kind))
            .ok_or_else(|| {
                fail(
                    "INVALID_EVENT",
                    &format!("Report one of: {}.", REPORT_KINDS.join(", ")),
                )
            })?;
        // A page may be public or behind the owner's sign-in (read in the
        // signed-in browser): either way an HTTPS URL that carries no
        // credentials, so no sign-in reaches the feed.
        if kind == "read" && !truthy(&body["referenceUrl"]) {
            return Err(fail(
                "INVALID_REFERENCE",
                "Give the page you read as an HTTPS reference.",
            ));
        }
        let reference = reference_url(&body["referenceUrl"])?;
        if let Some(previous) = db::get(
            db,
            "SELECT * FROM events WHERE run_id=? AND client_key=?",
            [run_id, &client_key],
        )? {
            if previous["kind"] != kind
                || previous["message"] != message
                || previous["reference_url"] != json!(reference)
            {
                return Err(fail_status(
                    "REQUEST_CONFLICT",
                    "This event key has different content.",
                    409,
                ));
            }
            return Ok(event_view(&previous));
        }
        if !run["finished_at"].is_null() {
            return Err(fail_status(
                "RUN_FINISHED",
                "This research run is finished.",
                409,
            ));
        }
        if count(
            db,
            "SELECT count(*) AS n FROM events WHERE run_id=?",
            [run_id],
        )? >= 2000
        {
            return Err(fail_status(
                "EVENT_LIMIT",
                "This run has reached its event limit.",
                429,
            ));
        }
        add_event(
            db,
            now,
            run_id,
            "agent",
            kind,
            &message,
            reference.as_deref(),
            Some(&client_key),
        )?;
        let row = db::get(
            db,
            "SELECT * FROM events WHERE run_id=? AND client_key=?",
            [run_id, &client_key],
        )?
        .unwrap_or_default();
        Ok(event_view(&row))
    })
}

pub fn finish_run(
    db: &Connection,
    now: i64,
    agent: &Value,
    run_id: &str,
    body: &Value,
) -> Result<Value, Failure> {
    let agent_id = id_of(agent);
    atomic(db, || {
        session(db, now, agent_id, &body["sessionId"])?;
        let run = owned_run(db, agent_id, run_id)?;
        let summary = text(&body["summary"], 4000, "completion summary")?;
        if !run["finished_at"].is_null() {
            if run["summary"] != summary {
                return Err(fail_status(
                    "REQUEST_CONFLICT",
                    "This run already has a different completion summary.",
                    409,
                ));
            }
            return Ok(run_view(&run));
        }
        db::run(
            db,
            "UPDATE runs SET finished_at=?,summary=? WHERE id=?",
            params![now, summary, run_id],
        )?;
        add_event(
            db,
            now,
            run_id,
            "agent",
            "run_finished",
            &summary,
            None,
            None,
        )?;
        Ok(run_view(&owned_run(db, agent_id, run_id)?))
    })
}

/// What a submission's start found: a finished one's result, or the row to
/// complete.
pub enum Attempt {
    Done(Value),
    Pending(i64),
}

/// `candidate_json`: the candidate as JSON, its keys in the order the agent
/// sent them (the Node service's `JSON.stringify`), so a retry with the same
/// key matches what an earlier service recorded. `bundle_sha`: the SHA-256
/// of the web proof bundle that comes with it, if any; its bytes fix the
/// bundle's digest, so a retry with the same key must bring the same bundle.
pub fn begin_submission(
    db: &Connection,
    now: i64,
    agent: &Value,
    run_id: &str,
    body: &Value,
    candidate_json: &str,
    bundle_sha: Option<&str>,
) -> Result<Attempt, Failure> {
    let agent_id = id_of(agent);
    atomic(db, || {
        session(db, now, agent_id, &body["sessionId"])?;
        let run = owned_run(db, agent_id, run_id)?;
        let scope = if run["context_json"].as_str().is_some_and(|c| !c.is_empty()) {
            parsed(&run, "context_json")["candidateScope"].clone()
        } else {
            db::get(
                db,
                "SELECT scope_json FROM work WHERE id=?",
                [run["work_id"].as_str()],
            )?
            .map(|row| parsed(&row, "scope_json"))
            .unwrap_or(Value::Null)
        };
        if let Some(scope) = scope.as_object()
            && scope
                .iter()
                .any(|(key, value)| body["candidate"].get(key) != Some(value))
        {
            return Err(fail(
                "ASSIGNMENT_MISMATCH",
                "The candidate does not match this assignment\u{2019}s requested scope.",
            ));
        }
        let client_key = text(&body["clientKey"], 80, "request key")?;
        let input = match bundle_sha {
            Some(sha) => format!(
                r#"{{"candidate":{candidate_json},"bundleSha":{}}}"#,
                json!(sha)
            ),
            None => candidate_json.to_string(),
        };
        let input_hash = digest(input);
        if let Some(previous) = db::get(
            db,
            "SELECT * FROM submissions WHERE run_id=? AND client_key=?",
            [run_id, &client_key],
        )? {
            if previous["input_hash"] != input_hash {
                return Err(fail_status(
                    "REQUEST_CONFLICT",
                    "This submission key has different content.",
                    409,
                ));
            }
            if previous["status"] == "complete" {
                return Ok(Attempt::Done(parsed(&previous, "result_json")));
            }
            if previous["status"] == "pending" {
                return Err(fail_status(
                    "SUBMISSION_PENDING",
                    "This submission is still being checked.",
                    409,
                ));
            }
            let id = previous["id"].as_i64().unwrap_or_default();
            db::run(
                db,
                "UPDATE submissions SET status='pending' WHERE id=?",
                [id],
            )?;
            return Ok(Attempt::Pending(id));
        }
        if !run["finished_at"].is_null() {
            return Err(fail_status(
                "RUN_FINISHED",
                "This research run is finished.",
                409,
            ));
        }
        if count(
            db,
            "SELECT count(*) AS n FROM submissions WHERE run_id=?",
            [run_id],
        )? >= 100
        {
            return Err(fail_status(
                "SUBMISSION_LIMIT",
                "This run has reached its submission limit.",
                429,
            ));
        }
        let id = db::run(
            db,
            "INSERT INTO submissions(run_id,client_key,input_hash,status,created_at) VALUES (?,?,?,'pending',?)",
            params![run_id, client_key, input_hash, now],
        )?;
        Ok(Attempt::Pending(id))
    })
}

/// A candidate with a web proof keeps the bundle's digest (the API's address
/// for it, GET /oracle/evidence/:digest), in its row and its result.
pub fn complete_submission(
    db: &Connection,
    now: i64,
    id: i64,
    result: &Value,
) -> Result<Value, Failure> {
    atomic(db, || {
        let row = db::get(db, "SELECT * FROM submissions WHERE id=?", [id])?.unwrap_or_default();
        let bundle = result["evidence"]["bundleDigest"].as_str();
        db::run(
            db,
            "UPDATE submissions SET status='complete',result_json=?,bundle_digest=? WHERE id=?",
            params![result.to_string(), bundle, id],
        )?;
        let proof = bundle
            .map(|b| format!(" Web proof: bundle {b}."))
            .unwrap_or_default();
        let duplicate = result["duplicate"] == true;
        let message = if duplicate {
            "This source already exists; original attribution is preserved."
        } else {
            "Source candidate received and attributed. Oracle acceptance is pending."
        };
        let reference = result["candidate"]["details"]["canonicalUrl"]
            .as_str()
            .filter(|url| !url.is_empty());
        add_event(
            db,
            now,
            row["run_id"].as_str().unwrap_or_default(),
            "backend",
            if duplicate {
                "candidate_duplicate"
            } else {
                "candidate_received"
            },
            &format!("{message}{proof}"),
            reference,
            None,
        )?;
        Ok(result.clone())
    })
}

pub fn fail_submission(db: &Connection, id: i64) -> Result<(), Failure> {
    db::run(
        db,
        "UPDATE submissions SET status='retryable' WHERE id=? AND status='pending'",
        [id],
    )?;
    Ok(())
}

/// The agent's page in the TUI: its runs and its newest events, oldest first
/// (its own reports, petri's run events, the backend's and the owner's
/// directives). 300 events hold the current run's feed and several before it.
pub fn snapshot(db: &Connection, agent_id: &str) -> Result<Map<String, Value>, Failure> {
    let sessions: Vec<Value> = db::all(
        db,
        "SELECT * FROM sessions WHERE agent_id=? ORDER BY created_at DESC LIMIT 20",
        [agent_id],
    )?
    .iter()
    .map(session_view)
    .collect();
    let runs: Vec<Value> = db::all(
        db,
        "SELECT runs.*,work.title FROM runs JOIN work ON work.id=runs.work_id WHERE agent_id=? ORDER BY created_at DESC LIMIT 100",
        [agent_id],
    )?
    .iter()
    .map(run_view)
    .collect();
    let mut events: Vec<Value> = db::all(
        db,
        "SELECT events.* FROM events JOIN runs ON runs.id=events.run_id WHERE runs.agent_id=? ORDER BY seq DESC LIMIT 300",
        [agent_id],
    )?
    .iter()
    .map(event_view)
    .collect();
    events.reverse();
    let submissions: Vec<Value> = db::all(
        db,
        "SELECT submissions.run_id,submissions.result_json FROM submissions JOIN runs ON runs.id=submissions.run_id WHERE agent_id=? AND result_json IS NOT NULL ORDER BY submissions.id DESC LIMIT 100",
        [agent_id],
    )?
    .iter()
    .map(|row| {
        let mut item = Map::new();
        item.insert("runId".into(), row["run_id"].clone());
        if let Value::Object(result) = parsed(row, "result_json") {
            item.extend(result);
        }
        Value::Object(item)
    })
    .collect();
    let mut out = Map::new();
    out.insert("sessions".into(), json!(sessions));
    out.insert("runs".into(), json!(runs));
    out.insert("events".into(), json!(events));
    out.insert("submissions".into(), json!(submissions));
    Ok(out)
}
