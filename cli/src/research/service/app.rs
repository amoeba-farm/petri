//! The workspace's routes (`/v1/*`): the owner's (`/v1/owner/*`, with the
//! owner token), an agent's login, and an agent's own (with its credential).
//! The workspace never calculates eligibility, grants an oracle acceptance
//! or creates a reward: the futures API's candidate store does that.

use super::{
    auth, coverage, db, directives, ordered,
    upstream::{OracleReader, Upstream},
    vault::Vault,
    work::{self, Attempt},
};
use crate::research::workspace::{
    Failure, MAX_BUNDLE_BYTES, bundle_bytes, bundle_too_large, digest, equal_secret, fail,
    fail_status, object, text,
};
use base64::Engine as _;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The workspace API carries a bundle as base64 of its bytes, next to the
/// candidate, on top of its ordinary 32 KiB request bound.
pub const MAX_BUNDLE_BASE64: usize = 4 * (MAX_BUNDLE_BYTES as usize).div_ceil(3);
pub const MAX_SUBMISSION_BODY: usize = 32768 + MAX_BUNDLE_BASE64;

/// One request, as the HTTP server read it.
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub credential: Option<String>,
    pub body: Value,
    /// The body's text, when it came over HTTP.
    pub raw: Option<String>,
}

impl Request {
    fn query(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// What the application runs with besides its configuration: the clock and
/// the remote services (tests give their own).
pub struct Options {
    pub clock: Box<dyn Fn() -> i64 + Send + Sync>,
    pub upstream: Arc<dyn Upstream>,
    pub oracle: Option<Arc<dyn OracleReader>>,
    /// Which petri runs the service (`/v1/owner/ping`): a new one replaces it.
    pub runtime_id: String,
    /// How an agent's shell runs petri here (`work next` gives it).
    pub petri_command: Option<String>,
    pub seed_directives: bool,
    /// The database file; `:memory:` for tests.
    pub database: PathBuf,
}

#[derive(Default)]
struct Discovery {
    running: bool,
}

pub struct App {
    db: Mutex<Connection>,
    vault: Vault,
    owner_token: String,
    options: Options,
    /// Each agent's candidates at the API, read at most every 5 seconds.
    candidate_reads: Mutex<HashMap<String, (Value, i64)>>,
    discovery: Mutex<Discovery>,
    discovered: Condvar,
    closed: AtomicBool,
    /// The server's drain-and-exit, for `/v1/owner/restart`.
    restart: OnceLock<Box<dyn Fn() -> Result<(), Failure> + Send + Sync>>,
}

impl App {
    pub fn new(
        owner_token: &str,
        master_key: &str,
        options: Options,
    ) -> Result<Arc<Self>, Failure> {
        let db = db::open(&options.database)?;
        let now = (options.clock)();
        if options.seed_directives {
            directives::seed(&db, now)?;
        }
        Ok(Arc::new(Self {
            db: Mutex::new(db),
            vault: Vault::new(master_key)?,
            owner_token: owner_token.into(),
            options,
            candidate_reads: Mutex::default(),
            discovery: Mutex::default(),
            discovered: Condvar::new(),
            closed: AtomicBool::new(false),
            restart: OnceLock::new(),
        }))
    }

    pub fn on_restart(&self, restart: impl Fn() -> Result<(), Failure> + Send + Sync + 'static) {
        let _ = self.restart.set(Box::new(restart));
    }

    fn now(&self) -> i64 {
        (self.options.clock)()
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Stops discovery; requests that still come get the database as it is.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Discovery's first assessment now, then one a minute.
    pub fn start_discovery(self: &Arc<Self>) {
        if self.options.oracle.is_none() {
            return;
        }
        let app = Arc::downgrade(self);
        std::thread::spawn(move || {
            loop {
                let Some(app) = app.upgrade() else { return };
                if app.closed.load(Ordering::SeqCst) {
                    return;
                }
                let _ = app.refresh();
                drop(app);
                std::thread::sleep(Duration::from_secs(60));
            }
        });
    }

    /// A fresh assessment of the oracle's coverage, recorded as work; the
    /// discovery status after it. One whose directives changed while it read
    /// runs again.
    pub fn refresh(&self) -> Result<Value, Failure> {
        let Some(oracle) = self.options.oracle.clone() else {
            return directives::status(&self.db());
        };
        {
            // One at a time: a refresh asked for while another runs reads the
            // oracle again after it, so it reflects what was asked.
            let mut state = self.discovery.lock().unwrap_or_else(|p| p.into_inner());
            while state.running {
                state = self
                    .discovered
                    .wait(state)
                    .unwrap_or_else(|p| p.into_inner());
            }
            state.running = true;
        }
        loop {
            if self.closed.load(Ordering::SeqCst) {
                break;
            }
            let revisions = directives::list(&self.db());
            let observations = oracle.read();
            let db = self.db();
            let current = directives::list(&db);
            let unchanged = match (&revisions, &current) {
                (Ok(before), Ok(now)) => before == now,
                _ => false,
            };
            if unchanged && !self.closed.load(Ordering::SeqCst) {
                let directives_now = current.unwrap_or_default();
                let recorded = observations
                    .and_then(|observations| {
                        coverage::assess_coverage(&observations, &directives_now, self.now())
                    })
                    .and_then(|result| directives::record(&db, self.now(), &result));
                if recorded.is_err() {
                    let _ = directives::failed(&db);
                }
                break;
            }
        }
        let mut state = self.discovery.lock().unwrap_or_else(|p| p.into_inner());
        state.running = false;
        self.discovered.notify_all();
        drop(state);
        directives::status(&self.db())
    }

    /// A refresh in the background (after the owner changed a directive).
    fn refresh_later(self: &Arc<Self>) {
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let _ = app.refresh();
        });
    }

    fn owner(&self, credential: Option<&str>) -> Result<(), Failure> {
        if equal_secret(&self.owner_token, credential) {
            Ok(())
        } else {
            Err(fail_status(
                "OWNER_REQUIRED",
                "This action requires the workspace owner.",
                403,
            ))
        }
    }

    fn read_candidates(&self, agent_id: &str) -> Result<Value, Failure> {
        let now = self.now();
        if let Some((result, time)) = self
            .candidate_reads
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(agent_id)
            && now - time < 5000
        {
            return Ok(result.clone());
        }
        match self.options.upstream.candidates(agent_id) {
            Ok(result) => {
                self.candidate_reads
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(agent_id.into(), (result.clone(), self.now()));
                Ok(result)
            }
            Err(error) => {
                self.candidate_reads
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(agent_id);
                Err(error)
            }
        }
    }

    /// An agent's page: its identity, runs, feed and submissions, and its
    /// candidates at the API (or why they could not be read).
    fn detail(&self, agent_id: &str) -> Result<Value, Failure> {
        let mut data = {
            let db = self.db();
            let agent = auth::find_agent(&db, agent_id)?
                .ok_or_else(|| fail_status("AGENT_NOT_FOUND", "Agent not found.", 404))?;
            let mut data = Map::new();
            data.insert("agent".into(), agent);
            data.extend(work::snapshot(&db, agent_id)?);
            data
        };
        match self.read_candidates(agent_id) {
            Ok(Value::Object(result)) => {
                data.extend(result);
                data.insert("validationError".into(), Value::Null);
            }
            Ok(_) => {
                data.insert("validationError".into(), Value::Null);
            }
            Err(error) => {
                data.insert("candidates".into(), Value::Null);
                data.insert("complete".into(), json!(false));
                data.insert("validationError".into(), json!(error.message));
            }
        }
        Ok(Value::Object(data))
    }

    fn approve(&self, pairing_id: &str) -> Result<Value, Failure> {
        let payout = {
            let db = self.db();
            let payout = auth::setting(&db, "payoutAuthority")?;
            if payout.is_empty() {
                return Err(fail(
                    "PAYOUT_REQUIRED",
                    "Connect a wallet before connecting an agent.",
                ));
            }
            auth::begin_approval(&db, self.now(), pairing_id)?;
            payout
        };
        let completed = self
            .options
            .upstream
            .register(&payout)
            .and_then(|registered| {
                auth::complete_approval(
                    &self.db(),
                    &self.vault,
                    self.now(),
                    pairing_id,
                    &registered,
                    &payout,
                )
            });
        match completed {
            Ok(agent) => Ok(json!({ "agent": agent })),
            Err(error) => {
                let message = if error.status.is_some() {
                    error.message.clone()
                } else {
                    "Approval could not complete. Retry from Connect.".into()
                };
                auth::fail_approval(&self.db(), pairing_id, &message)?;
                Err(error)
            }
        }
    }

    pub fn handle(self: &Arc<Self>, request: &Request) -> Result<Value, Failure> {
        let (method, path, body) = (
            request.method.as_str(),
            request.path.as_str(),
            &request.body,
        );
        let credential = request.credential.as_deref();
        let now = self.now();
        if method == "GET" && path == "/health" {
            return Ok(json!({ "name": "petri-research", "version": "0.1.0" }));
        }
        if method == "POST" && path == "/v1/login" {
            let body = object(body, &["name", "harness", "loginKey"], &["name", "harness"])?;
            return auth::start(
                &self.db(),
                now,
                &body["name"],
                &body["harness"],
                body.get("loginKey"),
            );
        }
        if method == "POST" && path == "/v1/login/poll" {
            let body = object(body, &["pairingId", "secret"], &["pairingId", "secret"])?;
            let id = text(&body["pairingId"], 32, "pairing ID")?;
            let secret = text(&body["secret"], 64, "pairing secret")?;
            return auth::poll(&self.db(), &self.vault, now, &id, &secret);
        }
        if path.starts_with("/v1/owner/") {
            self.owner(credential)?;
            return self.owner_route(request);
        }
        let agent = auth::authenticate(&self.db(), credential)?;
        let agent_id = agent["id"].as_str().unwrap_or_default().to_string();
        match (method, path) {
            ("GET", "/v1/me") => return Ok(json!({ "agent": agent })),
            ("GET", "/v1/work") => {
                let db = self.db();
                return Ok(json!({ "work": work::list_work(&db, now, false, false)?,
                    "directives": directives::list(&db)?, "discovery": directives::status(&db)? }));
            }
            ("POST", "/v1/work/next") => {
                let body = object(body, &["sessionId"], &["sessionId"])?;
                let session = json!(text(&body["sessionId"], 36, "session ID")?);
                let mut next = work::next(&self.db(), now, &agent, &session)?;
                // How the agent runs petri here (`petri-live petri` on a dev PC).
                next["petri"] = json!(self.options.petri_command);
                return Ok(next);
            }
            ("GET", "/v1/activity") => return self.detail(&agent_id),
            ("GET", "/v1/rewards") => {
                let result = self.read_candidates(&agent_id)?;
                let outcomes: Vec<Value> = result["candidates"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|c| json!({ "contentHash": c["contentHash"], "details": c["details"], "observation": c["observation"] }))
                            .collect()
                    })
                    .unwrap_or_default();
                return Ok(
                    json!({ "agentId": agent_id, "complete": result["complete"], "outcomes": outcomes }),
                );
            }
            ("POST", "/v1/sessions") => {
                let body = object(body, &["harness"], &[])?;
                let harness = body.get("harness").cloned().unwrap_or(Value::Null);
                return Ok(
                    json!({ "session": work::open_session(&self.db(), now, &agent, &harness)? }),
                );
            }
            ("POST", "/v1/sessions/heartbeat") => {
                let body = object(body, &["sessionId"], &["sessionId"])?;
                let id = text(&body["sessionId"], 36, "session ID")?;
                return Ok(json!({ "session": work::heartbeat(&self.db(), now, &agent, &id)? }));
            }
            ("POST", "/v1/sessions/close") => {
                let body = object(body, &["sessionId"], &["sessionId"])?;
                let id = text(&body["sessionId"], 36, "session ID")?;
                work::close_session(&self.db(), now, &agent, &id)?;
                return Ok(json!({ "closed": true }));
            }
            ("POST", "/v1/runs") => {
                let body = object(
                    body,
                    &["sessionId", "workId", "clientKey"],
                    &["sessionId", "workId", "clientKey"],
                )?;
                return Ok(json!({ "run": work::start_run(&self.db(), now, &agent, body)? }));
            }
            _ => {}
        }
        let Some((run_id, action)) = run_route(path).filter(|_| method == "POST") else {
            return Err(fail_status("NOT_FOUND", "Route not found.", 404));
        };
        match action {
            "events" => {
                let body = object(
                    body,
                    &["sessionId", "clientKey", "kind", "message", "referenceUrl"],
                    &["sessionId", "clientKey", "kind", "message"],
                )?;
                Ok(json!({ "event": work::report(&self.db(), now, &agent, run_id, body)? }))
            }
            "finish" => {
                let body = object(body, &["sessionId", "summary"], &["sessionId", "summary"])?;
                Ok(json!({ "run": work::finish_run(&self.db(), now, &agent, run_id, body)? }))
            }
            _ => self.submit(request, &agent, run_id),
        }
    }

    fn submit(&self, request: &Request, agent: &Value, run_id: &str) -> Result<Value, Failure> {
        let body = object(
            &request.body,
            &["sessionId", "clientKey", "candidate", "evidenceBundle"],
            &["sessionId", "clientKey", "candidate"],
        )?;
        if !body["candidate"].is_object() {
            return Err(fail("INVALID_CANDIDATE", "Provide the candidate fields."));
        }
        // A page behind the owner's sign-in comes with its web proof bundle.
        // Only its digests are kept here: the bundle itself goes to the API.
        let bundle = body.get("evidenceBundle").map(decode_bundle).transpose()?;
        let candidate_json = request
            .raw
            .as_deref()
            .and_then(|raw| ordered::member(raw, "candidate"))
            .unwrap_or_else(|| body["candidate"].to_string());
        let agent_id = agent["id"].as_str().unwrap_or_default();
        let attempt = work::begin_submission(
            &self.db(),
            self.now(),
            agent,
            run_id,
            body,
            &candidate_json,
            bundle.as_ref().map(digest).as_deref(),
        )?;
        let id = match attempt {
            Attempt::Done(result) => return Ok(result),
            Attempt::Pending(id) => id,
        };
        let submitted = (|| {
            let credential = auth::upstream_credential(&self.db(), &self.vault, agent_id)?;
            let mut evidence = Value::Null;
            let mut candidate = body["candidate"].clone();
            if let Some(bundle) = &bundle {
                // The API verifies the bundle itself; the candidate then names
                // it, and the API checks the proof is for its canonicalUrl.
                let uploaded = self.options.upstream.upload_evidence(bundle, &credential)?;
                let digest = uploaded["bundleDigest"]
                    .as_str()
                    .filter(|d| {
                        d.len() == 64
                            && d.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    .ok_or_else(|| {
                        fail_status("UPSTREAM_INVALID", "Invalid evidence response.", 502)
                    })?;
                let trust = uploaded["trust"].as_str().filter(|t| {
                    (1..=32).contains(&t.len())
                        && t.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                });
                evidence = json!({ "bundleDigest": digest, "trust": trust });
                candidate["authentication"] = json!({ "bundleDigest": digest });
            }
            let mut result = self.options.upstream.submit(&candidate, &credential)?;
            let hash_ok = result["candidate"]["contentHash"]
                .as_str()
                .is_some_and(|h| {
                    h.len() == 64
                        && h.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                });
            let duplicate = result["duplicate"].as_bool();
            if !hash_ok
                || duplicate.is_none()
                || duplicate == Some(false)
                    && result["candidate"]["agentId"].as_str() != Some(agent_id)
            {
                return Err(fail_status(
                    "UPSTREAM_INVALID",
                    "Invalid candidate response or attribution.",
                    502,
                ));
            }
            if !evidence.is_null() {
                result["evidence"] = evidence;
            }
            let saved = work::complete_submission(&self.db(), self.now(), id, &result)?;
            self.candidate_reads
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(agent_id);
            Ok(saved)
        })();
        if submitted.is_err() {
            let _ = work::fail_submission(&self.db(), id);
        }
        submitted
    }

    fn owner_route(self: &Arc<Self>, request: &Request) -> Result<Value, Failure> {
        let (method, path, body) = (
            request.method.as_str(),
            request.path.as_str(),
            &request.body,
        );
        let now = self.now();
        let route = path.trim_start_matches("/v1/owner/");
        match (method, route) {
            ("GET", "ping") => {
                Ok(json!({ "name": "petri-research", "runtime": self.options.runtime_id }))
            }
            ("POST", "restart") => {
                object(body, &[], &[])?;
                let restart = self.restart.get().ok_or_else(|| {
                    fail_status(
                        "RESTART_UNAVAILABLE",
                        "This workspace must be restarted manually.",
                        409,
                    )
                })?;
                restart()?;
                Ok(json!({ "restarting": true }))
            }
            ("GET", "snapshot") => {
                let (agents, pairings, items, activity, policies, discovery, payout) = {
                    let db = self.db();
                    let agents = auth::list(&db)?;
                    // Who is researching what right now (the TUI's "researching").
                    let activity: Vec<Value> = work::active_runs(&db, now)?
                        .into_iter()
                        .filter(|run| {
                            agents
                                .iter()
                                .any(|a| a["id"] == run["agentId"] && a["revokedAt"].is_null())
                        })
                        .collect();
                    let items: Vec<Value> = work::list_work(&db, now, false, true)?
                        .into_iter()
                        .map(|mut item| {
                            let researchers: Vec<Value> = activity
                                .iter()
                                .filter(|run| run["workId"] == item["id"])
                                .map(|run| run["agentId"].clone())
                                .collect();
                            item["researchers"] = json!(researchers);
                            item
                        })
                        .collect();
                    (
                        agents,
                        auth::pending(&db, now)?,
                        items,
                        activity,
                        directives::list(&db)?,
                        directives::status(&db)?,
                        auth::setting(&db, "payoutAuthority")?,
                    )
                };
                let selected = request
                    .query("agentId")
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        agents
                            .first()
                            .and_then(|a| a["id"].as_str().map(str::to_string))
                    });
                let selected = match selected {
                    Some(id) => self.detail(&id)?,
                    None => Value::Null,
                };
                Ok(
                    json!({ "agents": agents, "pairings": pairings, "work": items, "activity": activity,
                    "directives": policies, "discovery": discovery, "settings": { "payoutAuthority": payout },
                    "selected": selected }),
                )
            }
            ("POST", "settings") => {
                let body = object(body, &["payoutAuthority"], &["payoutAuthority"])?;
                let payout = text(&body["payoutAuthority"], 44, "wallet address")?;
                let base58 =
                    |b: u8| b.is_ascii_alphanumeric() && !matches!(b, b'0' | b'O' | b'I' | b'l');
                if !(32..=44).contains(&payout.len()) || !payout.bytes().all(base58) {
                    return Err(fail("INVALID_PAYOUT", "Provide a Solana public address."));
                }
                auth::set_setting(&self.db(), "payoutAuthority", &payout)?;
                Ok(json!({ "payoutAuthority": payout }))
            }
            ("POST", "pairings/approve") => {
                let body = object(body, &["pairingId"], &["pairingId"])?;
                self.approve(&text(&body["pairingId"], 32, "pairing ID")?)
            }
            ("POST", "pairings/reject") => {
                let body = object(body, &["pairingId"], &["pairingId"])?;
                auth::reject(
                    &self.db(),
                    now,
                    &text(&body["pairingId"], 32, "pairing ID")?,
                )?;
                Ok(json!({ "rejected": true }))
            }
            ("POST", "agents/revoke") => {
                let body = object(body, &["agentId"], &["agentId"])?;
                auth::revoke(&self.db(), now, &text(&body["agentId"], 64, "agent ID")?)?;
                Ok(json!({ "revoked": true }))
            }
            ("POST", "work/archive") => {
                let body = object(body, &["workId"], &["workId"])?;
                work::archive_work(
                    &self.db(),
                    now,
                    &text(&body["workId"], 36, "assignment ID")?,
                )?;
                Ok(json!({ "archived": true }))
            }
            ("POST", "directives") => {
                let directive = directives::save(&self.db(), now, body)?;
                self.refresh_later();
                Ok(json!({ "directive": directive }))
            }
            ("POST", "work/focus") => {
                let body = object(body, &["workId", "focused"], &["workId", "focused"])?;
                let id = text(&body["workId"], 36, "work ID")?;
                Ok(json!({ "focused": work::focus_work(&self.db(), now, &id, &body["focused"])? }))
            }
            ("POST", "work/refine") => {
                let body = object(body, &["workId", "instruction"], &["workId", "instruction"])?;
                let db = self.db();
                let guidance = db::atomic(&db, || {
                    let guidance =
                        directives::refine(&db, now, &body["workId"], &body["instruction"])?;
                    work::directed(
                        &db,
                        now,
                        &body["workId"],
                        guidance["instruction"].as_str().unwrap_or_default(),
                    )?;
                    Ok(guidance)
                })?;
                Ok(json!({ "guidance": guidance }))
            }
            ("POST", "discovery/refresh") => {
                object(body, &[], &[])?;
                Ok(json!({ "discovery": self.refresh()? }))
            }
            _ => Err(fail_status("NOT_FOUND", "Route not found.", 404)),
        }
    }
}

/// `/v1/runs/<run id>/<events|finish|candidates>`.
fn run_route(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/v1/runs/")?;
    let (id, action) = rest.split_once('/')?;
    let id_ok = id.len() == 36
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-');
    (id_ok && matches!(action, "events" | "finish" | "candidates")).then_some((id, action))
}

/// A bundle as the workspace API carries it: base64 of its bytes.
fn decode_bundle(value: &Value) -> Result<Vec<u8>, Failure> {
    let text = value
        .as_str()
        .filter(|t| t.chars().map(char::len_utf16).sum::<usize>() <= MAX_BUNDLE_BASE64)
        .ok_or_else(bundle_too_large)?;
    let invalid = || fail("INVALID_BUNDLE", "Invalid evidence bundle encoding.");
    let body = text.trim_end_matches('=');
    let padding = text.len() - body.len();
    if text.is_empty()
        || text.len() % 4 != 0
        || padding > 2
        || !body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return Err(invalid());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| invalid())?;
    bundle_bytes(bytes)
}
