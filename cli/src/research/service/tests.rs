//! The workspace service against the Node service's own tests (research/test/,
//! removed with it in 2026-10):
//! workspace.test.mjs, the application tests of discovery.test.mjs, the first
//! two of feed.test.mjs and the server-level ones of transport.test.mjs, with
//! coverage-fixture.mjs as `coverage_fixture`. Each test keeps its Node name
//! and its assertions, against `App::handle` (an in-memory database, a fake
//! candidate store, clock and oracle) or the loopback HTTP server.

use super::{
    app::{App, Options, Request},
    coverage, db, http, load_owner,
    upstream::{self, OracleReader, Upstream},
    vault::Vault,
};
use crate::research::workspace::{Failure, MAX_BUNDLE_BYTES, digest, endpoint, fail_status, token};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicI64, AtomicU16, Ordering},
    },
};

const PAYOUT: &str = "So11111111111111111111111111111111111111112";

/// coverage-fixture.mjs's `coverageNow`: `Date.UTC(2026, 9, 1, 20)`.
const COVERAGE_NOW: i64 = 1_790_884_800_000;

/// `new Date(millis).toISOString()`.
fn iso(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// coverage-fixture.mjs: the oracle's public snapshot and catalog for one
/// RAMX month taking sources, with `count` active sources on one product.
fn coverage_fixture(now: i64, count: usize) -> Value {
    let protocol = json!({ "programId": "canonical-program", "cluster": "testnet", "namespace": "test-namespace" });
    let freshness = json!({ "observedAt": iso(now), "stale": false, "refreshHealthy": true, "readReady": true });
    let fact = json!({ "marketId": "ramx", "expiryId": "RAMX-202610", "oracleMonthAddress": "canonical-month",
        "phase": "SourceSubmission", "sourceCount": count, "onChainAvailable": true, "oracleMonthAvailable": true });
    let sources: Vec<Value> = (0..count)
        .map(|i| {
            json!({ "sourceId": format!("{i:064x}"), "bucketId": "21".repeat(32), "bucketLabel": "Example governed product",
                "status": "Active", "canonicalLocator": { "verified": true, "url": format!("https://source-{i}.example/product") } })
        })
        .collect();
    let link = spread(
        &fact,
        json!({ "usage": "source-display-only", "observedAt": freshness["observedAt"], "sources": sources }),
    );
    json!({
        "snapshot": { "protocol": protocol, "freshness": freshness, "state": { "months": [fact] }, "sourceLinks": [link] },
        "catalog": { "protocol": protocol, "freshness": freshness, "markets": [{ "marketId": "ramx", "onChainAvailable": true,
            "expiries": [{ "expiryId": "RAMX-202610", "settlementUtc": "2026-11-01T00:00:00.000Z" }] }] },
    })
}

/// `{ ...base, ...fields }`.
fn spread(base: &Value, fields: Value) -> Value {
    let mut out = base.clone();
    for (key, value) in fields.as_object().unwrap() {
        out[key.as_str()] = value.clone();
    }
    out
}

fn push(list: &mut Value, item: Value) {
    list.as_array_mut().unwrap().push(item);
}

/// `list.length` (a missing list fails, as JavaScript's would).
#[track_caller]
fn len(list: &Value) -> usize {
    list.as_array().expect("an array").len()
}

/// `list.find(item => item[key] === value)` (a missing item fails, as the
/// property read on it would).
#[track_caller]
fn find<'a>(list: &'a Value, key: &str, value: &Value) -> &'a Value {
    list.as_array()
        .expect("an array")
        .iter()
        .find(|item| &item[key] == value)
        .unwrap_or_else(|| panic!("no item with {key} {value}"))
}

/// `assert.rejects(promise, { code })`.
#[track_caller]
fn rejects(result: Result<Value, Failure>, code: &str) {
    match result {
        Ok(value) => panic!("expected {code}, got {value}"),
        Err(error) => assert_eq!(error.code.as_deref(), Some(code), "{}", error.message),
    }
}

fn run_path(run: &Value, action: &str) -> String {
    format!("/v1/runs/{}/{action}", run["id"].as_str().unwrap())
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A fresh folder in the system's temporary folder, removed afterwards.
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("petri-research-test-{}", &token()[..16]));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("petri-research-test-")
        );
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The tests' clock: `() => now`, moved by hand.
#[derive(Clone)]
struct Clock(Arc<AtomicI64>);
impl Clock {
    fn new(at: i64) -> Self {
        Self(Arc::new(AtomicI64::new(at)))
    }
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
    fn advance(&self, by: i64) {
        self.0.fetch_add(by, Ordering::SeqCst);
    }
    fn set(&self, at: i64) {
        self.0.store(at, Ordering::SeqCst);
    }
    fn boxed(&self) -> Box<dyn Fn() -> i64 + Send + Sync> {
        let clock = self.clone();
        Box::new(move || clock.now())
    }
}

/// An oracle reader that answers with `read`.
struct FakeOracle<F>(F);
impl<F: Fn() -> Result<Value, Failure> + Send + Sync> OracleReader for FakeOracle<F> {
    fn read(&self) -> Result<Value, Failure> {
        (self.0)()
    }
}

fn oracle(
    read: impl Fn() -> Result<Value, Failure> + Send + Sync + 'static,
) -> Arc<dyn OracleReader> {
    Arc::new(FakeOracle(read))
}

/// `async () => { if (unavailable) throw new Error('offline'); return input; }`.
fn shared_oracle(
    input: &Arc<Mutex<Value>>,
    unavailable: &Arc<AtomicBool>,
) -> Arc<dyn OracleReader> {
    let (input, unavailable) = (input.clone(), unavailable.clone());
    oracle(move || {
        if unavailable.load(Ordering::SeqCst) {
            return Err(Failure {
                code: None,
                message: "offline".into(),
                status: None,
            });
        }
        Ok(input.lock().unwrap().clone())
    })
}

/// A method the Node test's upstream object did not have.
fn missing(name: &str) -> Failure {
    Failure {
        code: None,
        message: format!("upstream.{name} is not a function"),
        status: None,
    }
}

/// The candidate store as the discovery, feed and transport tests give it:
/// a registration (as the given agent ID, or a new identity each time) and
/// no candidates.
struct Registrar(Option<String>);
impl Upstream for Registrar {
    fn register(&self, payout_authority: &str) -> Result<Value, Failure> {
        let agent_id = self.0.clone().unwrap_or_else(token);
        Ok(
            json!({ "agent": { "agentId": agent_id, "payoutAuthority": payout_authority }, "credential": token() }),
        )
    }
    fn submit(&self, _: &Value, _: &str) -> Result<Value, Failure> {
        Err(missing("submit"))
    }
    fn upload_evidence(&self, _: &[u8], _: &str) -> Result<Value, Failure> {
        Err(missing("uploadEvidence"))
    }
    fn candidates(&self, _: &str) -> Result<Value, Failure> {
        Ok(json!({ "candidates": [], "complete": true }))
    }
}

fn options(
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    upstream: Arc<dyn Upstream>,
    oracle: Option<Arc<dyn OracleReader>>,
) -> Options {
    Options {
        clock,
        upstream,
        oracle,
        runtime_id: "test".into(),
        petri_command: None,
        seed_directives: true,
        database: ":memory:".into(),
    }
}

/// `createApplication` and the requests the Node tests make to it.
struct Workspace {
    app: Arc<App>,
    owner_token: String,
}

impl Workspace {
    fn new(options: Options) -> Self {
        let owner_token = token();
        let app = App::new(&owner_token, &token(), options).unwrap();
        Self { app, owner_token }
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Value,
        credential: Option<&str>,
    ) -> Result<Value, Failure> {
        self.app.handle(&Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            credential: credential.map(str::to_string),
            body,
            raw: None,
        })
    }

    fn owner(&self, method: &str, route: &str, body: Value) -> Result<Value, Failure> {
        self.request(
            method,
            &format!("/v1/owner/{route}"),
            body,
            Some(&self.owner_token),
        )
    }

    fn snapshot(&self) -> Value {
        self.owner("GET", "snapshot", json!({})).unwrap()
    }

    /// The snapshot with `?agentId=`: that agent's page as `selected`.
    fn selected(&self, agent_id: &str) -> Value {
        self.app
            .handle(&Request {
                method: "GET".into(),
                path: "/v1/owner/snapshot".into(),
                query: vec![("agentId".into(), agent_id.into())],
                credential: Some(self.owner_token.clone()),
                body: json!({}),
                raw: None,
            })
            .unwrap()
    }

    fn refresh(&self) {
        self.owner("POST", "discovery/refresh", json!({})).unwrap();
    }

    /// `discover()`: a refresh, then the first work item.
    fn discover(&self) -> Value {
        self.refresh();
        self.snapshot()["work"][0].clone()
    }

    /// `connect(name)`: the owner's wallet, a login the owner approves, the
    /// agent's credential and a session.
    fn connect(&self, name: &str, harness: &str) -> Agent<'_> {
        self.owner("POST", "settings", json!({ "payoutAuthority": PAYOUT }))
            .unwrap();
        let pairing = self
            .request(
                "POST",
                "/v1/login",
                json!({ "name": name, "harness": harness }),
                None,
            )
            .unwrap();
        self.owner(
            "POST",
            "pairings/approve",
            json!({ "pairingId": pairing["pairingId"] }),
        )
        .unwrap();
        let result = self
            .request(
                "POST",
                "/v1/login/poll",
                json!({ "pairingId": pairing["pairingId"], "secret": pairing["secret"] }),
                None,
            )
            .unwrap();
        let credential = result["credential"].as_str().unwrap().to_string();
        let session = self
            .request("POST", "/v1/sessions", json!({}), Some(&credential))
            .unwrap()["session"]
            .clone();
        Agent {
            workspace: self,
            credential,
            agent: result["agent"].clone(),
            session,
            pairing,
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        self.app.close();
    }
}

/// A connected agent: its identity, credential, session and pairing.
struct Agent<'w> {
    workspace: &'w Workspace,
    credential: String,
    agent: Value,
    session: Value,
    pairing: Value,
}

impl Agent<'_> {
    fn ask(&self, method: &str, path: &str, body: Value) -> Result<Value, Failure> {
        self.workspace
            .request(method, path, body, Some(&self.credential))
    }
    fn id(&self) -> &str {
        self.agent["id"].as_str().unwrap()
    }
    fn session_id(&self) -> &str {
        self.session["id"].as_str().unwrap()
    }
    /// `POST /v1/runs` on `work` with `key`: the run.
    fn start(&self, work: &Value, key: &str) -> Value {
        self.ask(
            "POST",
            "/v1/runs",
            json!({ "workId": work["id"], "sessionId": self.session_id(), "clientKey": key }),
        )
        .unwrap()["run"]
            .clone()
    }
    fn finish(&self, run: &Value, summary: &str) {
        self.ask(
            "POST",
            &run_path(run, "finish"),
            json!({ "sessionId": self.session_id(), "summary": summary }),
        )
        .unwrap();
    }
    fn next(&self) -> Value {
        self.ask(
            "POST",
            "/v1/work/next",
            json!({ "sessionId": self.session_id() }),
        )
        .unwrap()
    }
}

/// workspace.test.mjs
mod workspace {
    use super::*;

    fn candidate() -> Value {
        json!({ "marketId": "ramx", "expiryId": "RAMX-202610", "bucketId": "21".repeat(32),
            "canonicalUrl": "https://example.com/ram", "sourceType": "retail product listing", "productDefinition": "32GB DDR5-5600 kit",
            "priceField": "USD unit price", "region": "United States", "currency": "USD", "evidence": "Public offer and matching product specifications." })
    }

    /// What the fake candidate store saw and holds.
    #[derive(Default)]
    struct Ledger {
        registrations: usize,
        submissions: usize,
        /// Each credential's researcher.
        agents: HashMap<String, Value>,
        candidates: Vec<Value>,
        uploads: Vec<Vec<u8>>,
        submitted: Vec<Value>,
        reject_evidence: bool,
    }

    #[derive(Default)]
    struct FakeApi(Mutex<Ledger>);

    impl FakeApi {
        fn ledger(&self) -> MutexGuard<'_, Ledger> {
            self.0.lock().unwrap_or_else(|p| p.into_inner())
        }
    }

    impl Upstream for FakeApi {
        fn register(&self, payout_authority: &str) -> Result<Value, Failure> {
            let mut ledger = self.ledger();
            ledger.registrations += 1;
            let agent = json!({ "agentId": token(), "payoutAuthority": payout_authority });
            let credential = token();
            ledger.agents.insert(credential.clone(), agent.clone());
            Ok(json!({ "agent": agent, "credential": credential }))
        }

        // POST /oracle/evidence: the API verifies the bundle and names it by digest.
        fn upload_evidence(&self, bundle: &[u8], credential: &str) -> Result<Value, Failure> {
            let mut ledger = self.ledger();
            assert!(ledger.agents.contains_key(credential));
            ledger.uploads.push(bundle.to_vec());
            if ledger.reject_evidence {
                return Err(fail_status(
                    "EVIDENCE_REJECTED",
                    "The API could not verify this web proof.",
                    422,
                ));
            }
            Ok(
                json!({ "bundleDigest": digest(bundle), "proofDigest": token(), "observationId": token(),
                "trust": "development", "stored": true, "duplicate": false }),
            )
        }

        fn submit(&self, details: &Value, credential: &str) -> Result<Value, Failure> {
            let mut ledger = self.ledger();
            ledger.submissions += 1;
            ledger.submitted.push(details.clone());
            let agent = ledger
                .agents
                .get(credential)
                .cloned()
                .expect("a registered credential");
            if let Some(existing) = ledger
                .candidates
                .iter()
                .find(|c| c["details"]["canonicalUrl"] == details["canonicalUrl"])
            {
                return Ok(json!({ "duplicate": true, "candidate": existing }));
            }
            let value = json!({ "contentHash": digest(details.to_string()), "agentId": agent["agentId"],
                "details": details, "observation": null });
            ledger.candidates.push(value.clone());
            Ok(json!({ "duplicate": false, "candidate": value }))
        }

        fn candidates(&self, agent_id: &str) -> Result<Value, Failure> {
            let ledger = self.ledger();
            let mine: Vec<&Value> = ledger
                .candidates
                .iter()
                .filter(|c| c["agentId"] == agent_id)
                .collect();
            Ok(json!({ "candidates": mine, "complete": true }))
        }
    }

    struct Fixture {
        workspace: Workspace,
        clock: Clock,
        api: Arc<FakeApi>,
    }

    impl std::ops::Deref for Fixture {
        type Target = Workspace;
        fn deref(&self) -> &Workspace {
            &self.workspace
        }
    }

    fn fixture() -> Fixture {
        fixture_at(":memory:".into())
    }

    fn fixture_at(database: PathBuf) -> Fixture {
        let clock = Clock::new(COVERAGE_NOW);
        let api = Arc::new(FakeApi::default());
        let read = clock.clone();
        let mut options = options(
            clock.boxed(),
            api.clone(),
            Some(oracle(move || Ok(coverage_fixture(read.now(), 1)))),
        );
        options.database = database;
        Fixture {
            workspace: Workspace::new(options),
            clock,
            api,
        }
    }

    #[test]
    fn pairing_requires_the_owner_and_the_unguessable_polling_secret_public_views_exclude_credentials()
     {
        let f = fixture();
        let pending = f
            .request(
                "POST",
                "/v1/login",
                json!({ "name": "cedar", "harness": "Codex" }),
                None,
            )
            .unwrap();
        rejects(
            f.request(
                "POST",
                "/v1/owner/pairings/approve",
                json!({ "pairingId": pending["pairingId"] }),
                None,
            ),
            "OWNER_REQUIRED",
        );
        rejects(
            f.request(
                "POST",
                "/v1/login/poll",
                json!({ "pairingId": pending["pairingId"], "secret": token() }),
                None,
            ),
            "UNAUTHORIZED",
        );
        rejects(
            f.owner(
                "POST",
                "pairings/approve",
                json!({ "pairingId": pending["pairingId"] }),
            ),
            "PAYOUT_REQUIRED",
        );
        f.owner("POST", "settings", json!({ "payoutAuthority": PAYOUT }))
            .unwrap();
        f.owner(
            "POST",
            "pairings/approve",
            json!({ "pairingId": pending["pairingId"] }),
        )
        .unwrap();
        let poll = json!({ "pairingId": pending["pairingId"], "secret": pending["secret"] });
        let connected = f
            .request("POST", "/v1/login/poll", poll.clone(), None)
            .unwrap();
        let snapshot = f.snapshot().to_string();
        assert!(!snapshot.contains(pending["secret"].as_str().unwrap()));
        assert!(!snapshot.contains(connected["credential"].as_str().unwrap()));
        assert_eq!(connected["agent"]["payoutAuthority"], PAYOUT);
        assert_eq!(
            f.request("POST", "/v1/login/poll", poll, None).unwrap()["credential"],
            connected["credential"]
        );
    }

    #[test]
    fn expired_requests_and_revoked_credentials_cannot_reconnect_or_report_work() {
        let f = fixture();
        let a = f.connect("cedar", "Test harness");
        f.owner("POST", "agents/revoke", json!({ "agentId": a.id() }))
            .unwrap();
        rejects(a.ask("GET", "/v1/me", json!({})), "UNAUTHORIZED");
        assert_eq!(
            f.request(
                "POST",
                "/v1/login/poll",
                json!({ "pairingId": a.pairing["pairingId"], "secret": a.pairing["secret"] }),
                None
            )
            .unwrap()["status"],
            "revoked"
        );
        let pending = f
            .request(
                "POST",
                "/v1/login",
                json!({ "name": "other", "harness": "Kimi Code" }),
                None,
            )
            .unwrap();
        f.clock.advance(600_001);
        rejects(
            f.owner(
                "POST",
                "pairings/approve",
                json!({ "pairingId": pending["pairingId"] }),
            ),
            "PAIRING_EXPIRED",
        );
        rejects(
            f.request(
                "POST",
                "/v1/login/poll",
                json!({ "pairingId": pending["pairingId"], "secret": pending["secret"] }),
                None,
            ),
            "PAIRING_EXPIRED",
        );
    }

    #[test]
    fn research_runs_reject_cross_agent_access_and_forged_platform_events() {
        let f = fixture();
        let (a, b) = (
            f.connect("cedar", "Test harness"),
            f.connect("flint", "Test harness"),
        );
        let work = f.discover();
        let input =
            json!({ "workId": work["id"], "sessionId": a.session_id(), "clientKey": "run-1" });
        let run = a.ask("POST", "/v1/runs", input.clone()).unwrap()["run"].clone();
        assert_eq!(
            a.ask("POST", "/v1/runs", input).unwrap()["run"]["id"],
            run["id"]
        );
        let events = run_path(&run, "events");
        let body = json!({ "sessionId": b.session_id(), "clientKey": "event-1", "kind": "note", "message": "Working" });
        rejects(b.ask("POST", &events, body.clone()), "RUN_NOT_FOUND");
        rejects(
            a.ask(
                "POST",
                &events,
                spread(
                    &body,
                    json!({ "sessionId": a.session_id(), "kind": "reward_settled" }),
                ),
            ),
            "INVALID_EVENT",
        );
        rejects(
            a.ask(
                "POST",
                &events,
                spread(
                    &body,
                    json!({ "sessionId": a.session_id(), "actor": "backend" }),
                ),
            ),
            "INVALID_INPUT",
        );
        let good = spread(&body, json!({ "sessionId": a.session_id() }));
        let event = a.ask("POST", &events, good.clone()).unwrap()["event"].clone();
        assert_eq!(event["actor"], "agent");
        assert_eq!(
            a.ask("POST", &events, good.clone()).unwrap()["event"]["sequence"],
            event["sequence"]
        );
        rejects(
            a.ask(
                "POST",
                &events,
                spread(&good, json!({ "message": "Changed" })),
            ),
            "REQUEST_CONFLICT",
        );
        rejects(
            a.ask(
                "POST",
                "/v1/owner/work",
                json!({ "title": "No", "brief": "No" }),
            ),
            "OWNER_REQUIRED",
        );
    }

    #[test]
    fn candidate_submission_is_idempotent_bound_to_its_assignment_and_preserves_duplicate_attribution()
     {
        let f = fixture();
        let (a, b) = (
            f.connect("cedar", "Test harness"),
            f.connect("flint", "Test harness"),
        );
        let work = f.discover();
        let ar = a.start(&work, "start");
        let br = b.start(&work, "start");
        let input =
            json!({ "sessionId": a.session_id(), "clientKey": "source", "candidate": candidate() });
        let submit = run_path(&ar, "candidates");
        rejects(
            a.ask(
                "POST",
                &submit,
                spread(
                    &input,
                    json!({ "candidate": spread(&candidate(), json!({ "marketId": "nandx" })) }),
                ),
            ),
            "ASSIGNMENT_MISMATCH",
        );
        let first = a.ask("POST", &submit, input.clone()).unwrap();
        let replay = a.ask("POST", &submit, input.clone()).unwrap();
        assert_eq!(replay, first);
        assert_eq!(f.api.ledger().submissions, 1);
        let duplicate = b
            .ask(
                "POST",
                &run_path(&br, "candidates"),
                spread(&input, json!({ "sessionId": b.session_id() })),
            )
            .unwrap();
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(duplicate["candidate"]["agentId"], a.agent["id"]);
        assert_eq!(
            b.ask("GET", "/v1/rewards", json!({})).unwrap()["outcomes"],
            json!([])
        );
        let outcomes = a.ask("GET", "/v1/rewards", json!({})).unwrap()["outcomes"].clone();
        assert!(outcomes[0].is_object(), "{outcomes}");
        assert_eq!(outcomes[0]["observation"], Value::Null);
        a.finish(&ar, "Evidence submitted.");
        let outcomes = a.ask("GET", "/v1/rewards", json!({})).unwrap()["outcomes"].clone();
        assert!(outcomes[0].is_object(), "{outcomes}");
        assert_eq!(outcomes[0]["observation"], Value::Null);
    }

    #[test]
    fn a_new_session_resumes_the_same_identity_and_run_without_creating_rewards() {
        let f = fixture();
        let a = f.connect("cedar", "Test harness");
        let w = f.discover();
        let run = a.start(&w, "run");
        a.ask(
            "POST",
            "/v1/sessions/close",
            json!({ "sessionId": a.session_id() }),
        )
        .unwrap();
        let event = |session: &str| json!({ "sessionId": session, "clientKey": "e", "kind": "note", "message": "Resumed." });
        rejects(
            a.ask("POST", &run_path(&run, "events"), event(a.session_id())),
            "SESSION_EXPIRED",
        );
        let next = a
            .ask("POST", "/v1/sessions", json!({ "harness": "Kimi Code" }))
            .unwrap()["session"]
            .clone();
        a.ask(
            "POST",
            &run_path(&run, "events"),
            event(next["id"].as_str().unwrap()),
        )
        .unwrap();
        assert_eq!(
            a.ask("GET", "/v1/activity", json!({})).unwrap()["runs"][0]["id"],
            run["id"]
        );
        assert_eq!(
            a.ask("GET", "/v1/me", json!({})).unwrap()["agent"]["id"],
            a.agent["id"]
        );
    }

    #[test]
    fn private_vault_ciphertext_is_bound_to_its_purpose() {
        let v = Vault::new(&token()).unwrap();
        let secret = token();
        let sealed = v.seal(&secret, "upstream:a");
        assert!(!sealed.contains(&secret));
        assert_eq!(v.open(&sealed, "upstream:a").unwrap(), secret);
        assert!(v.open(&sealed, "upstream:b").is_err());
    }

    #[test]
    fn a_lost_login_response_can_be_recovered_without_creating_another_pairing() {
        let f = fixture();
        let request = json!({ "name": "cedar", "harness": "Codex", "loginKey": token() });
        let first = f
            .request("POST", "/v1/login", request.clone(), None)
            .unwrap();
        let retry = f
            .request("POST", "/v1/login", request.clone(), None)
            .unwrap();
        assert_eq!(retry, first);
        assert_eq!(len(&f.snapshot()["pairings"]), 1);
        rejects(
            f.request(
                "POST",
                "/v1/login",
                spread(&request, json!({ "name": "flint" })),
                None,
            ),
            "REQUEST_CONFLICT",
        );
    }

    #[test]
    fn work_next_starts_the_owners_pick_resumes_it_and_the_owner_sees_who_is_researching_what() {
        let f = fixture();
        let a = f.connect("cedar", "Test harness");
        let work = f.discover();
        f.owner(
            "POST",
            "work/focus",
            json!({ "workId": work["id"], "focused": true }),
        )
        .unwrap();
        let first = a.next();
        assert_eq!(first["status"], "assigned");
        assert_eq!(first["assignment"]["id"], work["id"]);
        assert_eq!(first["resumed"], false);
        let again = a.next();
        assert_eq!(again["run"]["id"], first["run"]["id"]);
        assert_eq!(again["resumed"], true);
        let snapshot = f.snapshot();
        let pairs: Vec<Value> = snapshot["activity"]
            .as_array()
            .unwrap()
            .iter()
            .map(|run| json!([run["agentId"], run["workId"]]))
            .collect();
        assert_eq!(json!(pairs), json!([[a.agent["id"], work["id"]]]));
        assert_eq!(
            find(&snapshot["work"], "id", &work["id"])["researchers"],
            json!([a.agent["id"]])
        );
        a.ask(
            "POST",
            &run_path(&first["run"], "events"),
            json!({ "sessionId": a.session_id(), "clientKey": "e", "kind": "note", "message": "Checked a listing." }),
        )
        .unwrap();
        assert_eq!(
            f.snapshot()["activity"][0]["lastMessage"],
            "Checked a listing."
        );
        // A finished run is not research; the next one starts afresh.
        a.finish(&first["run"], "Done.");
        assert_eq!(f.snapshot()["activity"], json!([]));
        let next = a.next();
        assert_ne!(next["run"]["id"], first["run"]["id"]);
        // An agent that stopped checking in is not researching.
        f.clock.advance(20 * 60_000 + 1);
        let snapshot = f.snapshot();
        assert_eq!(snapshot["activity"], json!([]));
        assert_eq!(
            find(&snapshot["work"], "id", &work["id"])["researchers"],
            json!([])
        );
    }

    // A bundle as petri evidence prove writes it. Its presentation stands in for
    // the TLSNotary proof; the request it records named a cookie the proof hides.
    const SECRETS: [&str; 2] = ["session=hunter2-cookie", "hunter2-password"];
    const NOTE: &str = "Read in the owner\u{2019}s signed-in browser.";

    fn bundle_file(note: &str) -> Vec<u8> {
        let presentation = encode(
            format!(
                "GET /api/prices HTTP/1.1\r\nCookie: {}\r\nX-Password: {}\r\n",
                SECRETS[0], SECRETS[1]
            )
            .as_bytes(),
        );
        let mut bytes = serde_json::to_vec_pretty(&json!({ "format": "amoeba-evidence-bundle-v1", "method": "web_proof",
            "backend": { "name": "tlsnotary", "revision": "5f67389de3f8962227fddffd71d78a852acc9b01", "mode": "mpc" },
            "presentation": presentation,
            "manifest": { "format": "amoeba-extraction-manifest-v1", "name": "Distributor X: DDR5 kit", "evidenceType": "catalog_price",
                "origin": "portal.distributor.example", "path": "/api/prices", "content": "json", "records": "/items",
                "match": { "/sku": "DDR5-32G-5600" }, "fields": { "price": "/unit_price", "currency": "/currency" } },
            "annotations": { "note": note } }))
        .unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn proven() -> Value {
        spread(
            &candidate(),
            json!({ "canonicalUrl": "https://portal.distributor.example/api/prices", "sourceType": "distributor price list" }),
        )
    }

    #[test]
    fn a_candidate_with_a_web_proof_uploads_the_bundle_unchanged_then_names_its_digest_retries_keep_it_idempotent()
     {
        let f = fixture();
        let a = f.connect("cedar", "Test harness");
        let work = f.discover();
        let run = a.start(&work, "start");
        let bundle = bundle_file(NOTE);
        let input = json!({ "sessionId": a.session_id(), "clientKey": "proven", "candidate": proven(), "evidenceBundle": encode(&bundle) });
        let submit = run_path(&run, "candidates");
        // The API refuses the proof: nothing is submitted, and the same key may retry.
        f.api.ledger().reject_evidence = true;
        rejects(a.ask("POST", &submit, input.clone()), "EVIDENCE_REJECTED");
        assert_eq!(f.api.ledger().submissions, 0);
        f.api.ledger().reject_evidence = false;
        let first = a.ask("POST", &submit, input.clone()).unwrap();
        // The API got the file's exact bytes, then the candidate with only the bundle digest.
        {
            let ledger = f.api.ledger();
            assert_eq!(ledger.uploads.len(), 2);
            assert!(ledger.uploads.iter().all(|upload| *upload == bundle));
            assert_eq!(
                ledger.submitted.last().unwrap(),
                &spread(
                    &proven(),
                    json!({ "authentication": { "bundleDigest": digest(&bundle) } })
                )
            );
        }
        assert_eq!(
            first["evidence"],
            json!({ "bundleDigest": digest(&bundle), "trust": "development" })
        );
        assert_eq!(first["duplicate"], false);
        // A lost response: the same key and bundle return the same result, with no new upload.
        assert_eq!(a.ask("POST", &submit, input.clone()).unwrap(), first);
        assert_eq!(f.api.ledger().uploads.len(), 2);
        assert_eq!(f.api.ledger().submissions, 1);
        // The key covers the bundle: another bundle, or none, is another request.
        rejects(
            a.ask(
                "POST",
                &submit,
                spread(
                    &input,
                    json!({ "evidenceBundle": encode(&bundle_file("Another note.")) }),
                ),
            ),
            "REQUEST_CONFLICT",
        );
        let mut plain = input.clone();
        plain.as_object_mut().unwrap().remove("evidenceBundle");
        rejects(a.ask("POST", &submit, plain), "REQUEST_CONFLICT");
        // The feed and the agent's page carry the digest.
        let snapshot = f.selected(a.id());
        let received = find(
            &snapshot["selected"]["events"],
            "kind",
            &json!("candidate_received"),
        );
        assert_eq!(received["actor"], "backend");
        assert_eq!(received["referenceUrl"], proven()["canonicalUrl"]);
        assert!(
            received["message"]
                .as_str()
                .unwrap()
                .contains(&digest(&bundle)),
            "{}",
            received["message"]
        );
        assert_eq!(
            snapshot["selected"]["submissions"][0]["evidence"]["bundleDigest"],
            digest(&bundle)
        );
    }

    #[test]
    fn a_bundle_over_the_verifiers_2_mib_or_a_file_that_is_not_a_bundle_is_refused_before_any_upload()
     {
        let f = fixture();
        let a = f.connect("cedar", "Test harness");
        let work = f.discover();
        let run = a.start(&work, "start");
        let submit = run_path(&run, "candidates");
        let send = |key: &str, bytes: &[u8]| {
            a.ask(
                "POST",
                &submit,
                json!({ "sessionId": a.session_id(), "clientKey": key, "candidate": proven(), "evidenceBundle": encode(bytes) }),
            )
        };
        let padded = |size: usize| {
            let (head, tail) = (
                br#"{"format":"amoeba-evidence-bundle-v1","presentation":""#,
                br#""}"#,
            );
            let mut bytes = head.to_vec();
            bytes.resize(size - tail.len(), b'A');
            bytes.extend_from_slice(tail);
            bytes
        };
        let max = MAX_BUNDLE_BYTES as usize;
        rejects(send("large", &padded(max + 1)), "BUNDLE_TOO_LARGE");
        rejects(
            send("manifest", br#"{"format":"amoeba-extraction-manifest-v1"}"#),
            "INVALID_BUNDLE",
        );
        rejects(send("text", b"not json"), "INVALID_BUNDLE");
        rejects(
            a.ask(
                "POST",
                &submit,
                json!({ "sessionId": a.session_id(), "clientKey": "encoding", "candidate": proven(), "evidenceBundle": "***" }),
            ),
            "INVALID_BUNDLE",
        );
        assert_eq!(f.api.ledger().uploads.len(), 0);
        assert_eq!(f.api.ledger().submissions, 0);
        // Exactly 2 MiB is the verifier's bound, not over it.
        send("largest", &padded(max)).unwrap();
        assert_eq!(f.api.ledger().uploads.len(), 1);
        assert_eq!(f.api.ledger().uploads[0].len(), max);
    }

    #[test]
    fn no_cookie_password_or_proof_bytes_ever_reach_the_stored_events_submissions_or_snapshot() {
        let root = Scratch::new();
        let path = root.0.join("workspace.sqlite");
        let f = fixture_at(path.clone());
        let a = f.connect("cedar", "Test harness");
        let work = f.discover();
        let run = a.start(&work, "start");
        let bundle = bundle_file(NOTE);
        let presentation = serde_json::from_slice::<Value>(&bundle).unwrap()["presentation"]
            .as_str()
            .unwrap()
            .to_string();
        a.ask(
            "POST",
            &run_path(&run, "events"),
            json!({ "sessionId": a.session_id(), "clientKey": "read", "kind": "read", "message": "Distributor X price list (signed in)",
                "referenceUrl": "https://portal.distributor.example/account/prices" }),
        )
        .unwrap();
        a.ask(
            "POST",
            &run_path(&run, "candidates"),
            json!({ "sessionId": a.session_id(), "clientKey": "proven", "candidate": proven(), "evidenceBundle": encode(&bundle) }),
        )
        .unwrap();
        // A reference that carries a sign-in is refused, not stored.
        rejects(
            a.ask(
                "POST",
                &run_path(&run, "events"),
                json!({ "sessionId": a.session_id(), "clientKey": "leak", "kind": "read", "message": "Prices",
                    "referenceUrl": format!("https://portal.distributor.example/prices?token={}", SECRETS[1]) }),
            ),
            "INVALID_REFERENCE",
        );
        let snapshot = f.selected(a.id()).to_string();
        let activity = a.ask("GET", "/v1/activity", json!({})).unwrap().to_string();
        drop(a);
        drop(f);
        let db = rusqlite::Connection::open(&path).unwrap();
        let stored = json!(
            ["events", "submissions", "runs", "work"].map(|table| db::all(
                &db,
                &format!("SELECT * FROM {table}"),
                []
            )
            .unwrap())
        )
        .to_string();
        assert_eq!(
            db::get(&db, "SELECT bundle_digest FROM submissions", [])
                .unwrap()
                .unwrap()["bundle_digest"],
            digest(&bundle)
        );
        drop(db);
        for text in [&stored, &snapshot, &activity] {
            for secret in
                SECRETS
                    .iter()
                    .copied()
                    .chain([presentation.as_str(), "Cookie", "X-Password"])
            {
                assert!(!text.contains(secret), "{secret} leaked");
            }
        }
    }
}

/// discovery.test.mjs: the tests through the application, and the oracle
/// reader against a local HTTP server.
mod discovery {
    use super::*;

    const NOW: i64 = COVERAGE_NOW;
    const DAY: i64 = 86_400_000;

    fn directive() -> Value {
        json!({ "id": "test-directive", "revision": 1, "enabled": true, "marketIds": [], "minActiveSources": 3,
            "minDistinctHosts": 2, "maxObservationAgeMs": 600000 })
    }

    fn observations(count: usize) -> Value {
        coverage_fixture(NOW, count)
    }

    /// The application on `oracle`, its researcher registered with one fixed
    /// agent ID.
    fn application(
        clock: Box<dyn Fn() -> i64 + Send + Sync>,
        oracle: Arc<dyn OracleReader>,
    ) -> Workspace {
        Workspace::new(options(
            clock,
            Arc::new(Registrar(Some(token()))),
            Some(oracle),
        ))
    }

    #[test]
    fn the_oracle_reader_uses_only_the_two_public_get_routes_and_sends_no_workspace_credential() {
        let input = observations(1);
        let data = input.clone();
        let server = remote(move |request| {
            let part = if request.path == "/v1/oracle/catalog" {
                &data["catalog"]
            } else {
                &data["snapshot"]
            };
            json_reply(200, &json!({ "ok": true, "data": part }))
        });
        let observed = upstream::Oracle::new(&server.base).unwrap().read().unwrap();
        assert_eq!(observed, input);
        let seen = server.seen.lock().unwrap();
        let mut paths: Vec<&str> = seen.iter().map(|r| r.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["/v1/oracle/catalog", "/v1/oracle/public-snapshot"]);
        assert!(
            seen.iter()
                .all(|r| r.method == "GET" && r.header("authorization").is_none())
        );
    }

    #[test]
    fn discovery_deduplicates_work_preserves_human_guidance_and_run_context_and_resolves_only_on_fresh_oracle_coverage()
     {
        let time = Clock::new(NOW);
        let input = Arc::new(Mutex::new(observations(1)));
        let unavailable = Arc::new(AtomicBool::new(false));
        let app = application(time.boxed(), shared_oracle(&input, &unavailable));
        let set = |value: Value| *input.lock().unwrap() = value;
        app.refresh();
        let snapshot = app.snapshot();
        assert_eq!(len(&snapshot["directives"]), 1);
        assert_eq!(len(&snapshot["work"]), 1);
        let (work, policy) = (
            snapshot["work"][0].clone(),
            snapshot["directives"][0].clone(),
        );
        app.owner(
            "POST",
            "work/refine",
            json!({ "workId": work["id"], "instruction": "Prioritize manufacturer listings in the required region." }),
        )
        .unwrap();
        // The owner's "Research this" survives a fresh discovery.
        app.owner(
            "POST",
            "work/focus",
            json!({ "workId": work["id"], "focused": true }),
        )
        .unwrap();
        rejects(
            app.owner(
                "POST",
                "work/focus",
                json!({ "workId": work["id"], "focused": "yes" }),
            ),
            "INVALID_INPUT",
        );
        rejects(
            app.owner(
                "POST",
                "work/focus",
                json!({ "workId": "f".repeat(36), "focused": true }),
            ),
            "WORK_NOT_FOUND",
        );
        app.refresh();
        let snapshot = app.snapshot();
        assert_eq!(snapshot["work"][0]["id"], work["id"]);
        assert_eq!(snapshot["work"][0]["guidance"]["authorType"], "human");
        assert_eq!(snapshot["work"][0]["focused"], true);
        let agent = app.connect("researcher", "Test");
        let run = agent.start(&work, "start");
        assert_eq!(run["researchContext"]["objective"]["directiveRevision"], 1);
        assert_eq!(
            run["researchContext"]["guidance"]["instruction"],
            "Prioritize manufacturer listings in the required region."
        );
        agent.finish(&run, "Research done.");
        let listed = || agent.ask("GET", "/v1/work", json!({})).unwrap()["work"].clone();
        assert_eq!(len(&listed()), 1);
        assert_eq!(listed()[0]["focused"], true);
        rejects(
            agent.ask(
                "POST",
                "/v1/owner/work/focus",
                json!({ "workId": work["id"], "focused": false }),
            ),
            "OWNER_REQUIRED",
        );
        app.owner(
            "POST",
            "work/focus",
            json!({ "workId": work["id"], "focused": false }),
        )
        .unwrap();
        assert_eq!(listed()[0]["focused"], false);
        set(observations(3));
        app.refresh();
        assert_eq!(len(&listed()), 0);
        assert_eq!(app.snapshot()["work"][0]["availability"], "resolved");
        set(observations(1));
        app.refresh();
        assert_eq!(listed()[0]["id"], work["id"]);
        unavailable.store(true, Ordering::SeqCst);
        app.refresh();
        assert_eq!(len(&listed()), 0);
        rejects(
            agent.ask(
                "POST",
                "/v1/runs",
                json!({ "workId": work["id"], "sessionId": agent.session_id(), "clientKey": "stale" }),
            ),
            "WORK_NOT_FOUND",
        );
        unavailable.store(false, Ordering::SeqCst);
        app.refresh();
        time.advance(600_001);
        assert_eq!(len(&listed()), 0);
        time.set(NOW);
        app.owner(
            "POST",
            "directives",
            json!({ "id": policy["id"], "marketIds": [], "minActiveSources": 3, "minDistinctHosts": 2, "enabled": false }),
        )
        .unwrap();
        app.refresh();
        assert_eq!(len(&listed()), 0);
        assert_eq!(
            agent.ask("GET", "/v1/activity", json!({})).unwrap()["runs"][0]["researchContext"]["objective"]
                ["directiveRevision"],
            1
        );
        rejects(
            agent.ask(
                "POST",
                "/v1/owner/directives",
                json!({ "marketIds": [], "minActiveSources": 1, "minDistinctHosts": 1 }),
            ),
            "OWNER_REQUIRED",
        );
        rejects(
            app.owner(
                "POST",
                "work/refine",
                json!({ "workId": work["id"], "instruction": "x", "authorType": "farm_ai" }),
            ),
            "INVALID_INPUT",
        );
    }

    #[test]
    fn work_next_hands_out_openings_before_updates_and_an_unpaid_one_only_when_the_owner_picks_it()
    {
        // RAMX-202611 is in Opening with one frozen source; RAMX-202610 in Game with rewards.
        let mut input = observations(1);
        {
            let game = &mut input["snapshot"]["state"]["months"][0];
            game["phase"] = json!("Game");
            game["rewards"] = json!({ "remainingAtomic": "15000000" });
            let links = &mut input["snapshot"]["sourceLinks"][0];
            links["phase"] = json!("Game");
            links["sources"][0]["sourceAddress"] = json!("live-source");
        }
        let opening = spread(
            &input["snapshot"]["state"]["months"][0],
            json!({ "expiryId": "RAMX-202611", "oracleMonthAddress": "opening-month", "phase": "Opening",
                "rewards": { "remainingAtomic": "50000000" },
                "schedule": { "scrambleStartUtc": iso(NOW - 7 * DAY), "listingUtc": iso(NOW + DAY) },
                "skus": [{ "bucketId": "21".repeat(32), "bucketLabel": "Example governed product",
                    "openingBondAtomic": "1000000", "openingBudgetAtomic": "30000000" }] }),
        );
        push(&mut input["snapshot"]["state"]["months"], opening);
        let links = &input["snapshot"]["sourceLinks"][0];
        let frozen = spread(
            &links["sources"][0],
            json!({ "sourceId": "ab".repeat(32), "sourceAddress": "frozen-source", "status": "Frozen", "openingSubmitted": false }),
        );
        let opening_links = spread(
            links,
            json!({ "expiryId": "RAMX-202611", "oracleMonthAddress": "opening-month", "phase": "Opening", "sources": [frozen] }),
        );
        push(&mut input["snapshot"]["sourceLinks"], opening_links);
        push(
            &mut input["catalog"]["markets"][0]["expiries"],
            json!({ "expiryId": "RAMX-202611", "settlementUtc": "2026-12-01T00:00:00.000Z" }),
        );
        let input = Arc::new(Mutex::new(input));
        let app = application(
            Box::new(|| NOW),
            shared_oracle(&input, &Arc::new(AtomicBool::new(false))),
        );
        app.refresh();
        let agent = app.connect("researcher", "Test");
        let mut kinds: Vec<[String; 2]> = app.snapshot()["work"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                let per = if w["candidateScope"]["sourceId"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
                {
                    "per source"
                } else {
                    "per product"
                };
                [
                    w["objective"]["kind"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    per.to_string(),
                ]
            })
            .collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                ["opening".to_string(), "per source".to_string()],
                ["update".to_string(), "per source".to_string()]
            ]
        );
        let next = agent.next();
        assert_eq!(next["assignment"]["objective"]["kind"], "opening");
        assert_eq!(
            next["assignment"]["objective"]["sourceAddress"],
            "frozen-source"
        );
        assert_eq!(
            next["assignment"]["objective"]["openingBondAtomic"],
            "1000000"
        );
        assert_eq!(
            next["assignment"]["title"],
            "RAMX \u{b7} Example governed product \u{b7} opening"
        );
        agent.finish(&next["run"], "Opened.");
        // A product without an opening budget: its opening waits for the owner's pick.
        input.lock().unwrap()["snapshot"]["state"]["months"][1]["skus"][0]["openingBudgetAtomic"] =
            json!("0");
        app.refresh();
        let next = agent.next();
        assert_eq!(next["assignment"]["objective"]["kind"], "update");
        let unpaid = app.snapshot()["work"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["objective"]["kind"] == "opening")
            .unwrap()["id"]
            .clone();
        app.owner(
            "POST",
            "work/focus",
            json!({ "workId": unpaid, "focused": true }),
        )
        .unwrap();
        let next = agent.next();
        assert_eq!(next["assignment"]["id"], unpaid);
    }

    #[test]
    fn a_settled_month_with_frozen_rewards_gives_its_claims_once_to_a_workspace_that_worked_on_it()
    {
        // RAMX-202610 in Game, RAMX-202609 settled and frozen already (nobody here worked on it).
        let mut input = observations(1);
        {
            let game = &mut input["snapshot"]["state"]["months"][0];
            game["phase"] = json!("Game");
            game["rewards"] = json!({ "phase": "Funded", "remainingAtomic": "15000000" });
            let links = &mut input["snapshot"]["sourceLinks"][0];
            links["phase"] = json!("Game");
            links["sources"][0]["sourceAddress"] = json!("live-source");
        }
        let old = spread(
            &input["snapshot"]["state"]["months"][0],
            json!({ "expiryId": "RAMX-202609", "oracleMonthAddress": "old-month", "phase": "Settled",
                "rewards": { "phase": "EntitlementsFinalized", "remainingAtomic": "9000000" } }),
        );
        push(&mut input["snapshot"]["state"]["months"], old.clone());
        push(
            &mut input["catalog"]["markets"][0]["expiries"],
            json!({ "expiryId": "RAMX-202609", "settlementUtc": "2026-10-01T00:00:00.000Z" }),
        );
        let assessed =
            coverage::assess_coverage(&input, &[directive()], NOW).unwrap()["assessments"].clone();
        let rows: Vec<Value> = assessed
            .as_array()
            .unwrap()
            .iter()
            .map(|a| json!([a["expiryId"], a["kind"], a["status"]]))
            .collect();
        assert_eq!(
            json!(rows),
            json!([
                ["RAMX-202610", "update", "gap"],
                ["RAMX-202609", "claims", "gap"]
            ])
        );
        assert_eq!(
            coverage::describe_gap(&assessed[1]).0,
            "RAMX \u{b7} RAMX-202609 \u{b7} claims"
        );
        // Settled but not frozen yet: nothing to do.
        let mut unfrozen = input.clone();
        unfrozen["snapshot"]["state"] =
            json!({ "months": [spread(&old, json!({ "rewards": { "phase": "Funded" } }))] });
        let rows: Vec<Value> =
            coverage::assess_coverage(&unfrozen, &[directive()], NOW).unwrap()["assessments"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|a| a["expiryId"] == old["expiryId"])
                .map(|a| json!([a["kind"], a["status"]]))
                .collect();
        assert_eq!(json!(rows), json!([[null, "inactive"]]));
        let input = Arc::new(Mutex::new(input));
        // `clock: () => time++`
        let time = Arc::new(AtomicI64::new(NOW));
        let app = application(
            Box::new(move || time.fetch_add(1, Ordering::SeqCst)),
            shared_oracle(&input, &Arc::new(AtomicBool::new(false))),
        );
        app.refresh();
        let agent = app.connect("researcher", "Test");
        // RAMX-202609's claims never come up: no run here was on that month.
        let run = agent.next();
        assert_eq!(run["assignment"]["objective"]["kind"], "update");
        agent.finish(&run["run"], "Updated.");
        // RAMX-202610 settles and its rewards freeze: its claims come up first, once.
        {
            let mut input = input.lock().unwrap();
            let game = &mut input["snapshot"]["state"]["months"][0];
            game["phase"] = json!("Settled");
            game["rewards"] =
                json!({ "phase": "EntitlementsFinalized", "remainingAtomic": "15000000" });
            input["snapshot"]["sourceLinks"][0]["phase"] = json!("Settled");
            input["catalog"]["markets"][0]["expiries"][0]["settlementUtc"] = json!(iso(NOW - 1000));
        }
        app.refresh();
        let run = agent.next();
        assert_eq!(run["assignment"]["objective"]["kind"], "claims");
        assert_eq!(
            run["assignment"]["candidateScope"]["expiryId"],
            "RAMX-202610"
        );
        agent.finish(&run["run"], "Claimed.");
        assert_eq!(agent.next()["status"], "idle");
        // The owner's pick brings them back for one more run.
        let claims = app.snapshot()["work"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| {
                w["objective"]["kind"] == "claims"
                    && w["candidateScope"]["expiryId"] == "RAMX-202610"
            })
            .unwrap()["id"]
            .clone();
        app.owner(
            "POST",
            "work/focus",
            json!({ "workId": claims, "focused": true }),
        )
        .unwrap();
        let run = agent.next();
        assert_eq!(run["assignment"]["id"], claims);
        agent.finish(&run["run"], "Nothing left.");
        assert_eq!(agent.next()["status"], "idle");
    }

    #[test]
    fn a_refresh_archives_the_work_of_a_series_the_oracle_stopped_listing_and_only_a_successful_one()
     {
        // RAMX-202610 and RAMX-202611 both take sources: a gap each.
        let listed = || {
            let mut input = observations(1);
            let next = spread(
                &input["snapshot"]["state"]["months"][0],
                json!({ "expiryId": "RAMX-202611", "oracleMonthAddress": "next-month" }),
            );
            let links = spread(
                &input["snapshot"]["sourceLinks"][0],
                json!({ "expiryId": "RAMX-202611", "oracleMonthAddress": "next-month" }),
            );
            push(&mut input["snapshot"]["state"]["months"], next);
            push(&mut input["snapshot"]["sourceLinks"], links);
            push(
                &mut input["catalog"]["markets"][0]["expiries"],
                json!({ "expiryId": "RAMX-202611", "settlementUtc": "2026-12-01T00:00:00.000Z" }),
            );
            input
        };
        let input = Arc::new(Mutex::new(listed()));
        let unavailable = Arc::new(AtomicBool::new(false));
        let app = application(Box::new(|| NOW), shared_oracle(&input, &unavailable));
        let set = |value: Value| *input.lock().unwrap() = value;
        let work = || app.snapshot()["work"].clone();
        let months = || {
            let mut months: Vec<String> = work()
                .as_array()
                .unwrap()
                .iter()
                .map(|w| {
                    w["candidateScope"]["expiryId"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect();
            months.sort();
            months
        };
        let both = ["RAMX-202610", "RAMX-202611"];
        app.refresh();
        assert_eq!(months(), both);
        let later = work()
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["candidateScope"]["expiryId"] == "RAMX-202611")
            .unwrap()["id"]
            .clone();
        app.owner(
            "POST",
            "work/focus",
            json!({ "workId": later, "focused": true }),
        )
        .unwrap();
        // A failed refresh archives nothing: an unreachable oracle, or a read
        // without RAMX-202611 that does not hold together.
        unavailable.store(true, Ordering::SeqCst);
        app.refresh();
        assert_eq!(months(), both);
        unavailable.store(false, Ordering::SeqCst);
        let mut broken = observations(1);
        broken["catalog"]["protocol"]["programId"] = json!("other");
        set(broken);
        app.refresh();
        assert_ne!(app.snapshot()["discovery"].get("error"), Some(&Value::Null));
        assert_eq!(months(), both);
        // A series still listed stays, whatever its assessment: its month not
        // read, past settlement, or left out by every directive. Its work is
        // only withheld from agents.
        let mut unread = listed();
        unread["snapshot"]["state"]["months"][1]["oracleMonthAvailable"] = json!(false);
        set(unread);
        app.refresh();
        assert_eq!(months(), both);
        assert_eq!(find(&work(), "id", &later)["availability"], "unknown");
        let mut settled = listed();
        settled["catalog"]["markets"][0]["expiries"][1]["settlementUtc"] = json!(iso(NOW));
        set(settled);
        app.refresh();
        assert_eq!(
            find(
                &app.snapshot()["discovery"]["assessments"],
                "expiryId",
                &json!("RAMX-202611")
            )["status"],
            "inactive"
        );
        assert_eq!(months(), both);
        let policy = app.snapshot()["directives"][0].clone();
        app.owner(
            "POST",
            "directives",
            json!({ "id": policy["id"], "marketIds": ["nandx"], "minActiveSources": 3, "minDistinctHosts": 2 }),
        )
        .unwrap();
        set(listed());
        app.refresh();
        assert_eq!(app.snapshot()["discovery"]["assessments"], json!([]));
        assert_eq!(months(), both);
        app.owner(
            "POST",
            "directives",
            json!({ "id": policy["id"], "marketIds": [], "minActiveSources": 3, "minDistinctHosts": 2 }),
        )
        .unwrap();
        app.refresh();
        // The catalog drops RAMX-202611: its work is archived, gone from every list.
        set(observations(1));
        app.refresh();
        assert_eq!(months(), ["RAMX-202610"]);
        rejects(
            app.owner(
                "POST",
                "work/focus",
                json!({ "workId": later, "focused": false }),
            ),
            "WORK_NOT_FOUND",
        );
        // Listed again, it comes back as it was.
        set(listed());
        app.refresh();
        let now = work();
        let back = find(&now, "id", &later);
        assert_eq!(back["focused"], true);
        assert_eq!(back["availability"], "available");
        // The owner's own archive stays, listed or not.
        app.owner("POST", "work/archive", json!({ "workId": later }))
            .unwrap();
        set(observations(1));
        app.refresh();
        set(listed());
        app.refresh();
        assert_eq!(months(), ["RAMX-202610"]);
    }
}

/// feed.test.mjs: the first two tests.
mod feed {
    use super::*;

    const PAGE: &str = "https://example.com/ram";

    fn workspace() -> (Workspace, Clock) {
        let clock = Clock::new(COVERAGE_NOW);
        let read = clock.clone();
        let workspace = Workspace::new(options(
            clock.boxed(),
            Arc::new(Registrar(None)),
            Some(oracle(move || Ok(coverage_fixture(read.now(), 1)))),
        ));
        (workspace, clock)
    }

    fn feed(workspace: &Workspace, agent: &Agent) -> Vec<Value> {
        workspace.selected(agent.id())["selected"]["events"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn an_agent_reports_its_searches_pages_values_and_petri_writes_malformed_reports_are_refused() {
        let (f, _clock) = workspace();
        let a = f.connect("cedar", "Codex");
        let work = f.discover();
        let run = a.next()["run"].clone();
        let key = Cell::new(0);
        let events_path = run_path(&run, "events");
        let report = |kind: &str, message: &str, reference: Option<&str>| {
            key.set(key.get() + 1);
            let mut body = json!({ "sessionId": a.session_id(), "clientKey": format!("step-{}", key.get()), "kind": kind, "message": message });
            if let Some(reference) = reference.filter(|r| !r.is_empty()) {
                body["referenceUrl"] = json!(reference);
            }
            a.ask("POST", &events_path, body)
        };
        let steps: [(&str, &str, Option<&str>); 9] = [
            ("search", "DDR5 RDIMM 64GB price", None),
            ("read", "Crucial 64GB DDR5-4800 RDIMM", Some(PAGE)),
            ("value", "289.99 USD, DDR5 RDIMM 64GB", Some(PAGE)),
            ("value", "289.99 USD, DDR5 RDIMM 64GB", None),
            ("submit", "update committed", None),
            (
                "submit",
                "sponsored 30d7e0f9\u{2026}",
                Some("https://example.com/tx"),
            ),
            ("note", "Comparing two listings.", None),
            ("source_found", "DDR5 RDIMM 64GB at 289.99 USD", Some(PAGE)),
            ("blocked", "The page needs a login.", None),
        ];
        for (kind, message, reference) in steps {
            let event = report(kind, message, reference).unwrap()["event"].clone();
            assert_eq!(
                json!([
                    event["actor"],
                    event["kind"],
                    event["message"],
                    event["referenceUrl"]
                ]),
                json!(["agent", kind, message, reference])
            );
        }
        // A read names its page; references stay HTTPS without credentials.
        rejects(report("read", "No page given", None), "INVALID_REFERENCE");
        rejects(
            report("read", "Plain HTTP", Some("http://example.com/ram")),
            "INVALID_REFERENCE",
        );
        rejects(
            report(
                "value",
                "With a password",
                Some("https://user:pw@example.com/ram"),
            ),
            "INVALID_REFERENCE",
        );
        // A page read in the owner's signed-in browser is a reference like any
        // other, as long as its URL carries no token, key or session id.
        let portal = "https://portal.distributor.example/account/prices?region=us";
        assert_eq!(
            report("read", "Distributor X price list (signed in)", Some(portal)).unwrap()["event"]
                ["referenceUrl"],
            portal
        );
        for leak in [
            "https://portal.distributor.example/prices?token=abc",
            "https://portal.distributor.example/prices?access_token=abc",
            "https://portal.distributor.example/prices;jsessionid=ABC123",
            "https://portal.distributor.example/prices#id_token=abc",
            "https://files.example.com/prices.json?X-Amz-Signature=abc",
            "https://portal.distributor.example/prices?API_KEY=abc",
            "https://portal.distributor.example/prices?p=%74oken%3Dabc",
        ] {
            match report("read", "Leaky link", Some(leak)) {
                Ok(value) => panic!("{leak}: expected INVALID_REFERENCE, got {value}"),
                Err(error) => {
                    assert_eq!(error.code.as_deref(), Some("INVALID_REFERENCE"), "{leak}")
                }
            }
        }
        // Only the owner, petri and the backend write their own kinds.
        for kind in [
            "directive",
            "run_started",
            "run_finished",
            "candidate_received",
            "Search",
            "page",
        ] {
            rejects(report(kind, "Forged", None), "INVALID_EVENT");
        }
        // The 4,000-character limit stays as it was.
        rejects(report("search", &"x".repeat(4001), None), "INVALID_INPUT");
        assert_eq!(
            report("search", &"x".repeat(4000), None).unwrap()["event"]["kind"],
            "search"
        );

        // The snapshot: the agent's events oldest first, with its runs and who is on what.
        let snapshot = f.selected(a.id());
        let events = snapshot["selected"]["events"].as_array().unwrap();
        let runs = &snapshot["selected"]["runs"];
        let mut expected = vec![json!(["petri", "run_started"])];
        expected.extend(steps.iter().map(|(kind, _, _)| json!(["agent", kind])));
        expected.extend([json!(["agent", "read"]), json!(["agent", "search"])]);
        assert_eq!(
            events
                .iter()
                .map(|e| json!([e["actor"], e["kind"]]))
                .collect::<Vec<_>>(),
            expected
        );
        let sequences: Vec<i64> = events
            .iter()
            .map(|e| e["sequence"].as_i64().unwrap())
            .collect();
        let mut sorted = sequences.clone();
        sorted.sort();
        assert_eq!(sequences, sorted);
        for event in events {
            let mut keys: Vec<&str> = event
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort();
            assert_eq!(
                keys,
                [
                    "actor",
                    "createdAt",
                    "kind",
                    "message",
                    "referenceUrl",
                    "runId",
                    "sequence"
                ]
            );
            assert_eq!(event["runId"], run["id"]);
            assert_eq!(event["createdAt"], COVERAGE_NOW);
        }
        assert_eq!(events[2]["referenceUrl"], PAGE);
        assert_eq!(
            json!([
                runs[0]["id"],
                runs[0]["workId"],
                runs[0]["createdAt"],
                runs[0]["finishedAt"]
            ]),
            json!([run["id"], work["id"], COVERAGE_NOW, null])
        );
        assert_eq!(
            snapshot["activity"],
            json!([{ "agentId": a.agent["id"], "workId": work["id"], "runId": run["id"], "title": work["title"],
                "startedAt": COVERAGE_NOW, "lastSeenAt": COVERAGE_NOW, "lastMessage": "x".repeat(4000) }])
        );
    }

    #[test]
    fn the_owners_directive_shows_in_the_feed_of_each_agent_on_the_work_as_it_is_set_and_opens_later_runs()
     {
        let (f, clock) = workspace();
        let (a, b, c) = (
            f.connect("cedar", "Codex"),
            f.connect("flint", "Codex"),
            f.connect("birch", "Codex"),
        );
        let work = f.discover();
        let ar = a.start(&work, "a-1");
        let cr = c.start(&work, "c-1");
        c.finish(&cr, "Done.");
        a.ask(
            "POST",
            &run_path(&ar, "events"),
            json!({ "sessionId": a.session_id(), "clientKey": "s", "kind": "search", "message": "DDR5 RDIMM 64GB" }),
        )
        .unwrap();
        clock.advance(1000);
        f.owner(
            "POST",
            "work/refine",
            json!({ "workId": work["id"], "instruction": "Try Newegg first." }),
        )
        .unwrap();
        // a is on it: the directive lands in its run's feed, at the time it was set.
        let directive = feed(&f, &a).last().unwrap().clone();
        assert_eq!(
            directive,
            json!({ "sequence": directive["sequence"], "runId": ar["id"], "actor": "owner", "kind": "directive",
                "message": "Try Newegg first.", "referenceUrl": null, "createdAt": COVERAGE_NOW + 1000 })
        );
        // The agent sees it too; its last report stays its own.
        assert_eq!(
            a.ask("GET", "/v1/activity", json!({})).unwrap()["events"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["kind"],
            "directive"
        );
        assert_eq!(
            find(&f.snapshot()["activity"], "agentId", &a.agent["id"])["lastMessage"],
            "DDR5 RDIMM 64GB"
        );
        // c had finished: nothing new in its feed.
        assert!(!feed(&f, &c).iter().any(|event| event["actor"] == "owner"));
        // b starts it later: its run opens with the directive.
        clock.advance(1000);
        let next = b.next();
        assert_eq!(next["assignment"]["id"], work["id"]);
        assert_eq!(
            next["assignment"]["guidance"]["instruction"],
            "Try Newegg first."
        );
        let title = format!("Started: {}", work["title"].as_str().unwrap());
        assert_eq!(
            feed(&f, &b)
                .iter()
                .map(|e| json!([
                    e["runId"],
                    e["actor"],
                    e["kind"],
                    e["message"],
                    e["createdAt"]
                ]))
                .collect::<Vec<_>>(),
            vec![
                json!([
                    next["run"]["id"],
                    "petri",
                    "run_started",
                    title,
                    COVERAGE_NOW + 2000
                ]),
                json!([
                    next["run"]["id"],
                    "owner",
                    "directive",
                    "Try Newegg first.",
                    COVERAGE_NOW + 2000
                ]),
            ]
        );
        // A changed or cleared directive reaches both open runs; a run started after clearing opens without one.
        clock.advance(1000);
        f.owner(
            "POST",
            "work/refine",
            json!({ "workId": work["id"], "instruction": "Newegg is out of stock: try B&H." }),
        )
        .unwrap();
        f.owner(
            "POST",
            "work/refine",
            json!({ "workId": work["id"], "instruction": "" }),
        )
        .unwrap();
        for agent in [&a, &b] {
            let events = feed(&f, agent);
            let tail: Vec<Value> = events[events.len().saturating_sub(2)..]
                .iter()
                .map(|e| json!([e["actor"], e["kind"], e["message"], e["createdAt"]]))
                .collect();
            assert_eq!(
                tail,
                vec![
                    json!([
                        "owner",
                        "directive",
                        "Newegg is out of stock: try B&H.",
                        COVERAGE_NOW + 3000
                    ]),
                    json!([
                        "owner",
                        "directive",
                        "Directive cleared.",
                        COVERAGE_NOW + 3000
                    ]),
                ]
            );
        }
        let later = c.start(&work, "c-2");
        assert_eq!(
            feed(&f, &c)
                .iter()
                .filter(|e| e["runId"] == later["id"])
                .map(|e| e["kind"].clone())
                .collect::<Vec<_>>(),
            vec![json!("run_started")]
        );
        // Only the owner refines.
        rejects(
            a.ask(
                "POST",
                "/v1/owner/work/refine",
                json!({ "workId": work["id"], "instruction": "No." }),
            ),
            "OWNER_REQUIRED",
        );
    }
}

/// One request a stand-in remote service got.
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A status, extra headers and a body.
type Reply = (u16, Vec<(&'static str, String)>, Vec<u8>);

fn json_reply(status: u16, value: &Value) -> Reply {
    (
        status,
        vec![("Content-Type", "application/json".into())],
        value.to_string().into_bytes(),
    )
}

/// A stand-in remote service on loopback (Node's `http.createServer`):
/// `reply` answers each request, and every request is kept.
struct Remote {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

fn remote<F: Fn(&Seen) -> Reply + Send + Sync + 'static>(reply: F) -> Remote {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (kept, reply) = (seen.clone(), Arc::new(reply));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (kept, reply) = (kept.clone(), reply.clone());
            std::thread::spawn(move || answer(stream, reply.as_ref(), &kept));
        }
    });
    Remote { base, seen }
}

fn answer<F: Fn(&Seen) -> Reply>(mut stream: TcpStream, reply: &F, kept: &Mutex<Vec<Seen>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or_default().to_string(),
        parts.next().unwrap_or_default().to_string(),
    );
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let request = Seen {
        method,
        path,
        headers,
        body,
    };
    let (status, extra, content) = reply(&request);
    kept.lock().unwrap().push(request);
    let mut head = format!(
        "HTTP/1.1 {status} Reply\r\nContent-Length: {}\r\nConnection: close\r\n",
        content.len()
    );
    for (name, value) in extra {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&content);
}

/// transport.test.mjs: the server-level tests.
mod transport {
    use super::*;

    /// The Node tests' `createHttpClient`: one JSON request to the workspace
    /// at `base`, its data or its refusal.
    fn ask(
        base: &str,
        method: &str,
        path: &str,
        body: Option<&Value>,
        credential: Option<&str>,
    ) -> Result<Value, Failure> {
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut builder = client
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                format!("{base}{path}"),
            )
            .header("accept", "application/json")
            .header("content-type", "application/json");
        if let Some(credential) = credential {
            builder = builder.bearer_auth(credential);
        }
        if let Some(body) = body {
            builder = builder.body(body.to_string());
        }
        let response = builder.send().map_err(|_| {
            fail_status(
                "WORKSPACE_UNAVAILABLE",
                "Petri is unavailable. Open the research TUI or start its workspace service.",
                503,
            )
        })?;
        let status = response.status().as_u16();
        let result: Value = response
            .json()
            .map_err(|_| fail_status("INVALID_RESPONSE", "Invalid workspace response.", 502))?;
        if !(200..300).contains(&status) || result["ok"] != true {
            return Err(fail_status(
                result["error"]["code"]
                    .as_str()
                    .unwrap_or("WORKSPACE_ERROR"),
                result["error"]["message"]
                    .as_str()
                    .unwrap_or("The workspace rejected this request."),
                status,
            ));
        }
        Ok(result["data"].clone())
    }

    fn port_of(base: &str) -> u16 {
        base.rsplit(':').next().unwrap().parse().unwrap()
    }

    /// A GET of `path` with exactly `headers` (a Host line among them), as
    /// raw HTTP: the response status and head.
    fn raw_get(port: u16, path: &str, headers: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\n{headers}Connection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let head = response
            .split("\r\n\r\n")
            .next()
            .unwrap_or_default()
            .to_string();
        let status = head
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (status, head)
    }

    /// `start(directory)`: the server for a workspace in `directory`.
    fn start(directory: &Path) -> Arc<http::Server> {
        let mut options = options(
            Box::new(|| COVERAGE_NOW),
            Arc::new(Registrar(None)),
            Some(oracle(|| Ok(coverage_fixture(COVERAGE_NOW, 1)))),
        );
        options.database = directory.join("workspace.sqlite");
        let app = App::new(&token(), &token(), options).unwrap();
        http::start(app, 0).unwrap()
    }

    #[test]
    fn the_local_http_boundary_rejects_foreign_origins_host_rebinding_methods_and_oversized_requests()
     {
        let root = Scratch::new();
        let server = start(&root.0);
        let port = port_of(&server.base_url);
        for (headers, expected) in [
            (
                format!("Host: 127.0.0.1:{port}\r\nOrigin: https://evil.example\r\n"),
                403,
            ),
            ("Host: evil.example\r\n".to_string(), 403),
        ] {
            let (status, head) = raw_get(port, "/health", &headers);
            assert_eq!(status, expected, "{head}");
            assert!(
                head.lines().any(|line| line
                    .split_once(':')
                    .is_some_and(|(name, value)| name.eq_ignore_ascii_case("cache-control")
                        && value.trim() == "no-store")),
                "{head}"
            );
        }
        let client = reqwest::blocking::Client::new();
        let base = &server.base_url;
        assert_eq!(
            client
                .put(format!("{base}/health"))
                .send()
                .unwrap()
                .status()
                .as_u16(),
            405
        );
        // fetch's own type for a string body.
        assert_eq!(
            client
                .post(format!("{base}/v1/login"))
                .header("content-type", "text/plain;charset=UTF-8")
                .body("{}")
                .send()
                .unwrap()
                .status()
                .as_u16(),
            415
        );
        let too_large = client
            .post(format!("{base}/v1/login"))
            .header("content-type", "application/json")
            .body(json!({ "name": "a".repeat(40000), "harness": "X" }).to_string())
            .send()
            .unwrap();
        assert_eq!(too_large.status().as_u16(), 413);
        server.close();
    }

    /// A candidate store whose registration waits until the test releases it.
    #[derive(Default)]
    struct Gate {
        /// Whether a registration came in, and whether it may finish.
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    impl Gate {
        fn lock(&self) -> MutexGuard<'_, (bool, bool)> {
            self.state.lock().unwrap_or_else(|p| p.into_inner())
        }
        fn entered(&self) {
            let mut state = self.lock();
            while !state.0 {
                state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
            }
        }
        fn release(&self) {
            self.lock().1 = true;
            self.changed.notify_all();
        }
    }

    impl Upstream for Gate {
        fn register(&self, payout_authority: &str) -> Result<Value, Failure> {
            let mut state = self.lock();
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
            }
            Ok(
                json!({ "agent": { "agentId": token(), "payoutAuthority": payout_authority }, "credential": token() }),
            )
        }
        fn submit(&self, _: &Value, _: &str) -> Result<Value, Failure> {
            Err(missing("submit"))
        }
        fn upload_evidence(&self, _: &[u8], _: &str) -> Result<Value, Failure> {
            Err(missing("uploadEvidence"))
        }
        fn candidates(&self, _: &str) -> Result<Value, Failure> {
            Err(missing("candidates"))
        }
    }

    /// `finally { release(); }`
    struct Released(Arc<Gate>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    #[test]
    fn service_replacement_requires_its_owner_preserves_state_and_refuses_active_submissions() {
        let root = Scratch::new();
        let (owner_token, master_key) = (token(), token());
        let gate = Arc::new(Gate::default());
        let _release = Released(gate.clone());
        let serve = |runtime: &str, port: u16| {
            let app = App::new(
                &owner_token,
                &master_key,
                Options {
                    clock: Box::new(|| chrono::Utc::now().timestamp_millis()),
                    upstream: gate.clone(),
                    oracle: None,
                    runtime_id: runtime.into(),
                    petri_command: None,
                    seed_directives: false,
                    database: root.0.join("workspace.sqlite"),
                },
            )
            .unwrap();
            http::start(app, port).unwrap()
        };
        let server = serve("before-update", 0);
        let base = server.base_url.clone();
        let owner = |method: &str, route: &str, body: Option<Value>| {
            ask(
                &base,
                method,
                &format!("/v1/owner/{route}"),
                body.as_ref(),
                Some(&owner_token),
            )
        };
        assert_eq!(
            owner("GET", "ping", None).unwrap()["runtime"],
            "before-update"
        );
        rejects(
            ask(
                &base,
                "POST",
                "/v1/owner/restart",
                Some(&json!({})),
                Some(&token()),
            ),
            "OWNER_REQUIRED",
        );
        owner(
            "POST",
            "settings",
            Some(json!({ "payoutAuthority": PAYOUT })),
        )
        .unwrap();
        let pairing = ask(
            &base,
            "POST",
            "/v1/login",
            Some(&json!({ "name": "cedar", "harness": "Codex" })),
            None,
        )
        .unwrap();
        let saving = {
            let (base, owner_token) = (base.clone(), owner_token.clone());
            let body = json!({ "pairingId": pairing["pairingId"] });
            std::thread::spawn(move || {
                ask(
                    &base,
                    "POST",
                    "/v1/owner/pairings/approve",
                    Some(&body),
                    Some(&owner_token),
                )
            })
        };
        gate.entered();
        rejects(owner("POST", "restart", Some(json!({}))), "SERVICE_BUSY");
        assert_eq!(
            owner("GET", "ping", None).unwrap()["runtime"],
            "before-update"
        );
        gate.release();
        let approved = saving.join().unwrap().unwrap();
        let port = port_of(&base);
        assert_eq!(
            owner("POST", "restart", Some(json!({}))).unwrap()["restarting"],
            true
        );
        server.close();
        server.wait();
        let server = serve("after-update", port);
        let snapshot = owner("GET", "snapshot", None).unwrap();
        assert_eq!(
            owner("GET", "ping", None).unwrap()["runtime"],
            "after-update"
        );
        assert_eq!(snapshot["settings"]["payoutAuthority"], PAYOUT);
        assert_eq!(snapshot["agents"][0]["id"], approved["agent"]["id"]);
        server.close();
    }

    #[test]
    fn upstream_credentials_are_never_forwarded_through_redirects_and_remote_http_is_refused() {
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com?key=secret",
        ] {
            assert_eq!(
                endpoint(url).unwrap_err().code.as_deref(),
                Some("INVALID_ENDPOINT"),
                "{url}"
            );
        }
        let receiver = remote(|_| (200, Vec::new(), b"{}".to_vec()));
        let location = format!("http://127.0.0.1:{}/collect", port_of(&receiver.base));
        let redirect = remote(move |_| (307, vec![("Location", location.clone())], Vec::new()));
        let failure = upstream::Api::new(&redirect.base)
            .unwrap()
            .submit(&json!({}), &token())
            .unwrap_err();
        assert_eq!(failure.code.as_deref(), Some("UPSTREAM_UNAVAILABLE"));
        assert_eq!(receiver.seen.lock().unwrap().len(), 0);
    }

    #[test]
    fn an_evidence_upload_sends_the_bundles_exact_bytes_and_never_echoes_the_apis_refusal_text() {
        let status = Arc::new(AtomicU16::new(200));
        let current = status.clone();
        let api = remote(move |_| {
            let status = current.load(Ordering::SeqCst);
            json_reply(
                status,
                &if status == 200 {
                    json!({ "ok": true, "data": { "bundleDigest": "cd".repeat(32), "trust": "development", "stored": true, "duplicate": false } })
                } else {
                    json!({ "ok": false, "code": "EVIDENCE_UNVERIFIED", "message": "secret transport detail", "verification": { "stage": "provenance" } })
                },
            )
        });
        let upstream = upstream::Api::new(&api.base).unwrap();
        let credential = token();
        let bytes = b"{\n  \"format\": \"amoeba-evidence-bundle-v1\",\n  \"manifest\": { \"assume\": { \"unit\": \"1.0\" } }\n}\n";
        assert_eq!(
            upstream.upload_evidence(bytes, &credential).unwrap()["bundleDigest"],
            "cd".repeat(32)
        );
        {
            let seen = api.seen.lock().unwrap();
            let bearer = format!("Bearer {credential}");
            assert_eq!(
                (
                    seen[0].method.as_str(),
                    seen[0].path.as_str(),
                    seen[0].header("content-type"),
                    seen[0].header("authorization")
                ),
                (
                    "POST",
                    "/oracle/evidence",
                    Some("application/json"),
                    Some(bearer.as_str())
                )
            );
            assert_eq!(seen[0].body, bytes);
        }
        status.store(422, Ordering::SeqCst);
        let failure = upstream.upload_evidence(bytes, &credential).unwrap_err();
        assert_eq!(failure.code.as_deref(), Some("EVIDENCE_REJECTED"));
        assert_eq!(failure.status, Some(422));
        assert!(!failure.message.contains("secret transport detail"));
        assert!(
            failure.message.contains("petri evidence verify"),
            "{}",
            failure.message
        );
    }

    /// The candidate store of the near-2 MiB test: it keeps each upload and
    /// submission.
    #[derive(Default)]
    struct Proofs {
        uploads: Mutex<Vec<Vec<u8>>>,
        submissions: Mutex<Vec<Value>>,
    }

    impl Upstream for Proofs {
        fn register(&self, payout_authority: &str) -> Result<Value, Failure> {
            Ok(
                json!({ "agent": { "agentId": "ef".repeat(32), "payoutAuthority": payout_authority }, "credential": token() }),
            )
        }
        fn candidates(&self, _: &str) -> Result<Value, Failure> {
            Ok(json!({ "candidates": [], "complete": true }))
        }
        fn upload_evidence(&self, bundle: &[u8], _: &str) -> Result<Value, Failure> {
            self.uploads.lock().unwrap().push(bundle.to_vec());
            Ok(json!({ "bundleDigest": "12".repeat(32), "trust": "development" }))
        }
        fn submit(&self, details: &Value, _: &str) -> Result<Value, Failure> {
            self.submissions.lock().unwrap().push(details.clone());
            Ok(
                json!({ "duplicate": false, "candidate": { "contentHash": "34".repeat(32), "agentId": "ef".repeat(32), "details": details } }),
            )
        }
    }

    /// Driven over HTTP here, without the Node agent client: its login, its
    /// run and its submission as the requests it makes.
    #[test]
    fn a_near_2_mib_bundle_goes_from_the_agents_file_through_the_loopback_workspace_to_the_api() {
        let root = Scratch::new();
        let proofs = Arc::new(Proofs::default());
        let owner_token = token();
        let mut options = options(
            Box::new(|| COVERAGE_NOW),
            proofs.clone(),
            Some(oracle(|| Ok(coverage_fixture(COVERAGE_NOW, 1)))),
        );
        options.database = root.0.join("workspace.sqlite");
        let server = http::start(App::new(&owner_token, &token(), options).unwrap(), 0).unwrap();
        let base = server.base_url.clone();
        let owner = |method: &str, route: &str, body: Option<Value>| {
            ask(
                &base,
                method,
                &format!("/v1/owner/{route}"),
                body.as_ref(),
                Some(&owner_token),
            )
        };
        // client.login(): a pairing the owner approves, then the credential and a session.
        let pairing = ask(
            &base,
            "POST",
            "/v1/login",
            Some(&json!({ "name": "cedar", "harness": "Codex", "loginKey": token() })),
            None,
        )
        .unwrap();
        owner(
            "POST",
            "settings",
            Some(json!({ "payoutAuthority": PAYOUT })),
        )
        .unwrap();
        let pending = owner("GET", "snapshot", None).unwrap()["pairings"][0]["id"].clone();
        owner(
            "POST",
            "pairings/approve",
            Some(json!({ "pairingId": pending })),
        )
        .unwrap();
        let login = ask(
            &base,
            "POST",
            "/v1/login/poll",
            Some(&json!({ "pairingId": pairing["pairingId"], "secret": pairing["secret"] })),
            None,
        )
        .unwrap();
        let credential = login["credential"].as_str().unwrap().to_string();
        let agent = |method: &str, path: &str, body: Option<Value>| {
            ask(&base, method, path, body.as_ref(), Some(&credential))
        };
        let session = agent("POST", "/v1/sessions", Some(json!({}))).unwrap()["session"].clone();
        owner("POST", "discovery/refresh", Some(json!({}))).unwrap();
        let work = owner("GET", "snapshot", None).unwrap()["work"][0].clone();
        let run = agent(
            "POST",
            "/v1/runs",
            Some(json!({ "workId": work["id"], "sessionId": session["id"], "clientKey": "run-1" })),
        )
        .unwrap()["run"]
            .clone();
        let (head, tail) = (
            br#"{"format":"amoeba-evidence-bundle-v1","presentation":""#,
            b"\"}\n",
        );
        let mut bytes = head.to_vec();
        bytes.resize(2 * 1024 * 1024 - tail.len(), b'A');
        bytes.extend_from_slice(tail);
        let candidate = spread(
            &work["candidateScope"],
            json!({ "canonicalUrl": "https://portal.distributor.example/api/prices", "sourceType": "distributor price list",
                "productDefinition": "32GB DDR5-5600 kit", "priceField": "USD unit price", "region": "United States", "currency": "USD",
                "evidence": "Signed-in distributor price list, proven with a web proof." }),
        );
        let submit = |key: &str, bundle: &[u8]| {
            agent(
                "POST",
                &run_path(&run, "candidates"),
                Some(
                    json!({ "sessionId": session["id"], "clientKey": key, "candidate": candidate, "evidenceBundle": encode(bundle) }),
                ),
            )
        };
        let result = submit("proven", &bytes).unwrap();
        assert_eq!(result["evidence"]["bundleDigest"], "12".repeat(32));
        assert_eq!(proofs.uploads.lock().unwrap().len(), 1);
        assert!(proofs.uploads.lock().unwrap()[0] == bytes);
        assert_eq!(
            proofs.submissions.lock().unwrap()[0]["authentication"],
            json!({ "bundleDigest": "12".repeat(32) })
        );
        // One byte more is refused before any upload (the Node agent client
        // refuses it in its own process; over HTTP, the workspace does).
        let mut over = bytes.clone();
        over.push(b' ');
        rejects(submit("too-large", &over), "BUNDLE_TOO_LARGE");
        assert_eq!(proofs.uploads.lock().unwrap().len(), 1);
        // Other routes keep the ordinary 32 KiB bound.
        let too_large = agent(
            "POST",
            "/v1/sessions",
            Some(json!({ "harness": "a".repeat(40000) })),
        )
        .unwrap_err();
        assert_eq!(too_large.status, Some(413));
        server.close();
    }

    #[test]
    fn an_existing_workspace_cannot_redirect_its_stored_researcher_credentials_to_another_backend()
    {
        let root = Scratch::new();
        let state = root.0.join("state");
        load_owner(&state, Some("http://127.0.0.1:4500"), None, None).unwrap();
        assert_eq!(
            load_owner(&state, Some("https://different.example"), None, None)
                .err()
                .unwrap()
                .code
                .as_deref(),
            Some("UPSTREAM_MISMATCH")
        );
        assert_eq!(
            load_owner(&state, None, None, None).unwrap().upstream,
            "http://127.0.0.1:4500"
        );
    }

    #[test]
    fn a_workspace_bound_to_the_retired_candidate_api_is_rebound_to_the_futures_api() {
        let root = Scratch::new();
        let state = root.0.join("state");
        load_owner(&state, None, None, None).unwrap();
        let path = state.join("owner.json");
        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let retired = spread(
            &stored,
            json!({ "upstream": "http://127.0.0.1:4184", "oracleReadUrl": "http://127.0.0.1:4184" }),
        );
        fs::write(&path, retired.to_string()).unwrap();
        let config = load_owner(
            &state,
            Some("http://127.0.0.1:4500"),
            None,
            Some("http://127.0.0.1:4500"),
        )
        .unwrap();
        assert_eq!(config.upstream, "http://127.0.0.1:4500");
        assert_eq!(config.oracle_read_url, "http://127.0.0.1:4500");
        assert_eq!(
            load_owner(&state, Some("https://different.example"), None, None)
                .err()
                .unwrap()
                .code
                .as_deref(),
            Some("UPSTREAM_MISMATCH")
        );
    }

    #[test]
    fn the_production_futures_api_is_a_workspace_endpoint_like_any_other_never_rebound() {
        let root = Scratch::new();
        let state = root.0.join("state");
        let created = load_owner(&state, None, None, None).unwrap();
        assert_eq!(created.upstream, "https://api.amoeba.farm");
        assert_eq!(created.oracle_read_url, "https://api.amoeba.farm");
        let config = load_owner(
            &state,
            Some("https://api.amoeba.farm"),
            None,
            Some("https://api.amoeba.farm"),
        )
        .unwrap();
        assert_eq!(config.upstream, "https://api.amoeba.farm");
        assert_eq!(config.oracle_read_url, "https://api.amoeba.farm");
        assert_eq!(
            load_owner(&state, Some("http://127.0.0.1:4500"), None, None)
                .err()
                .unwrap()
                .code
                .as_deref(),
            Some("UPSTREAM_MISMATCH")
        );
    }
}

/// Petri's own agent client and MCP server (research/agent/) against this
/// service over HTTP: what an agent's harness runs, from pairing to a
/// finished run.
mod agent_end_to_end {
    use super::*;
    use crate::research::agent::{
        Flags,
        client::{Agent as _, AgentClient},
        mcp,
    };

    #[test]
    fn an_agent_pairs_researches_and_reports_through_petris_client_and_mcp_server() {
        let root = Scratch::new();
        let owner = token();
        let mut options = options(
            Box::new(|| COVERAGE_NOW),
            Arc::new(Registrar(None)),
            Some(oracle(|| Ok(coverage_fixture(COVERAGE_NOW, 1)))),
        );
        options.database = root.0.join("workspace.sqlite");
        options.petri_command = Some("'petri'".into());
        let app = App::new(&owner, &token(), options).unwrap();
        let server = http::start(app.clone(), 0).unwrap();
        let as_owner = |method: &str, route: &str, body: Value| {
            app.handle(&Request {
                method: method.into(),
                path: format!("/v1/owner/{route}"),
                query: vec![],
                credential: Some(owner.clone()),
                body,
                raw: None,
            })
        };
        as_owner("POST", "settings", json!({ "payoutAuthority": PAYOUT })).unwrap();

        let profiles = root.0.join("profiles");
        let agent = AgentClient::new(
            "cedar",
            "Codex",
            Some(&server.base_url),
            Some(profiles.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(agent.login().unwrap()["status"], "pending");
        let pairing = as_owner("GET", "snapshot", json!({})).unwrap()["pairings"][0]["id"].clone();
        as_owner("POST", "pairings/approve", json!({ "pairingId": pairing })).unwrap();
        assert_eq!(agent.login().unwrap()["status"], "connected");
        as_owner("POST", "discovery/refresh", json!({})).unwrap();

        let flags = Flags::default();
        let mut handler = mcp::Handler::new(&agent, &flags, "'petri'".into());
        let init = handler
            .handle(&json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {} }))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "petri-research");
        let mut call = |name: &str, arguments: Value| {
            let reply = handler
                .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": arguments } }))
                .unwrap();
            assert_eq!(reply["result"]["isError"], false, "{name}: {reply}");
            reply["result"]["structuredContent"].clone()
        };
        let next = call("petri_work_next", json!({}));
        assert_eq!(next["status"], "assigned");
        assert_eq!(next["petri"], "'petri'");
        let run = next["run"]["id"].as_str().unwrap().to_string();
        assert!(
            next["nextStep"]
                .as_str()
                .unwrap()
                .contains(&format!("work report {run}"))
        );
        call(
            "petri_report_progress",
            json!({ "runId": run, "clientKey": "read-1", "kind": "read",
            "message": "Read the listing.", "referenceUrl": "https://example.com/ram" }),
        );
        call(
            "petri_run_finish",
            json!({ "runId": run, "summary": "Read one listing." }),
        );
        let activity = call("petri_activity", json!({}));
        let messages: Vec<&str> = activity["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["message"].as_str())
            .collect();
        assert!(
            messages.contains(&"Read the listing.") && messages.contains(&"Read one listing."),
            "{messages:?}"
        );
        assert_eq!(call("petri_rewards", json!({}))["outcomes"], json!([]));
        call("petri_session_close", json!({}));
        server.close();
    }
}
