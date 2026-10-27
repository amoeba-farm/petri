//! An agent's connection to the research workspace: its HTTP client and the
//! agent client, which keeps the agent's identity in a private profile
//! (as `research/client/` did, in the same files).

use super::super::workspace::{
    Failure, MAX_BUNDLE_BYTES, absolute, bundle_bytes, bundle_too_large, endpoint, fail,
    fail_status, not_a_bundle, private_directory, private_write, state_directory, token, uuid,
};
use base64::Engine as _;
use reqwest::{Method, blocking::Client};
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

/// What an agent reports as it works, one short line each: the owner's live
/// feed in the TUI (the workspace's `REPORT_KINDS`).
pub const REPORT_KINDS: [&str; 7] = [
    "search",
    "read",
    "value",
    "submit",
    "note",
    "source_found",
    "blocked",
];

const DEFAULT_WORKSPACE: &str = "http://127.0.0.1:4185";
const RESPONSE_LIMIT: u64 = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
/// A candidate with a web proof is two API calls of up to 15 s each behind
/// the workspace (the bundle's upload, then the candidate).
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(40);

/// One request to the workspace at `base`; the reply's `data`.
fn request(
    http: &Client,
    base: &str,
    method: Method,
    path: &str,
    body: Option<&Value>,
    credential: Option<&str>,
    timeout: Duration,
) -> Result<Value, Failure> {
    let base = endpoint(base)?;
    if !path.starts_with("/v1/") && path != "/health" {
        return Err(fail("INVALID_ROUTE", "Invalid workspace route."));
    }
    let attempts = if method == Method::GET { 2 } else { 1 };
    let mut response = None;
    for _ in 0..attempts {
        let mut builder = http
            .request(method.clone(), format!("{base}{path}"))
            .timeout(timeout)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("cache-control", "no-store");
        if let Some(credential) = credential {
            builder = builder.bearer_auth(credential);
        }
        if let Some(body) = body {
            builder = builder.body(body.to_string());
        }
        // A redirect is refused like a lost connection.
        if let Ok(reply) = builder.send()
            && !reply.status().is_redirection()
        {
            response = Some(reply);
            break;
        }
    }
    let response = response.ok_or_else(|| {
        fail_status(
            "WORKSPACE_UNAVAILABLE",
            "Petri is unavailable. Open the research TUI or start its workspace service.",
            503,
        )
    })?;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail_status("INVALID_RESPONSE", "Invalid workspace response.", 502))?;
    if bytes.len() as u64 > RESPONSE_LIMIT {
        return Err(fail_status(
            "INVALID_RESPONSE",
            "The workspace response is too large.",
            502,
        ));
    }
    let result: Value = serde_json::from_slice(&bytes)
        .map_err(|_| fail_status("INVALID_RESPONSE", "Invalid workspace response.", 502))?;
    if !status.is_success() || result["ok"] != true {
        let error = &result["error"];
        return Err(fail_status(
            error["code"]
                .as_str()
                .filter(|code| !code.is_empty())
                .unwrap_or("WORKSPACE_ERROR"),
            error["message"]
                .as_str()
                .unwrap_or("The workspace rejected this request."),
            status.as_u16(),
        ));
    }
    Ok(result.get("data").cloned().unwrap_or(Value::Null))
}

/// One step an agent reports (`petri_report_progress`, `work report`).
pub struct Report<'a> {
    pub kind: Option<&'a str>,
    pub message: Option<&'a str>,
    pub reference_url: Option<&'a str>,
    pub client_key: Option<&'a str>,
}

/// What the research commands and the MCP server ask of the workspace.
pub trait Agent {
    fn login(&self) -> Result<Value, Failure>;
    fn me(&self) -> Result<Value, Failure>;
    fn open_session(&self) -> Result<Value, Failure>;
    fn close_session(&self) -> Result<Value, Failure>;
    fn list_work(&self) -> Result<Value, Failure>;
    fn next(&self) -> Result<Value, Failure>;
    fn activity(&self) -> Result<Value, Failure>;
    fn rewards(&self) -> Result<Value, Failure>;
    fn start_work(&self, work_id: &str, client_key: Option<&str>) -> Result<Value, Failure>;
    fn report(&self, run_id: &str, report: Report) -> Result<Value, Failure>;
    fn submit(
        &self,
        run_id: &str,
        candidate: &Value,
        client_key: Option<&str>,
        evidence_bundle: Option<&str>,
    ) -> Result<Value, Failure>;
    fn finish(&self, run_id: &str, summary: Option<&str>) -> Result<Value, Failure>;
}

/// An agent's identity in its private profile (`<name>.json`): the
/// workspace it belongs to, its credential once approved, its session.
pub struct AgentClient {
    name: String,
    harness: String,
    base_url: Option<String>,
    path: PathBuf,
    http: Client,
}

type Profile = Map<String, Value>;

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=40).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

fn hex_of(value: &Value, length: usize) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == length
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A path segment, percent-encoded as `encodeURIComponent` does.
fn segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl AgentClient {
    pub fn new(
        name: &str,
        harness: &str,
        base_url: Option<&str>,
        profile_directory: Option<&str>,
    ) -> Result<Self, Failure> {
        if !valid_name(name) {
            return Err(fail(
                "INVALID_NAME",
                "Use lowercase letters, digits, hyphens or underscores for the agent name.",
            ));
        }
        let directory = match profile_directory {
            Some(directory) => private_directory(Path::new(directory))?,
            None => private_directory(&private_directory(&state_directory())?.join("profiles"))?,
        };
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                fail(
                    "WORKSPACE_UNAVAILABLE",
                    "Could not start the workspace client.",
                )
            })?;
        Ok(Self {
            name: name.into(),
            harness: harness.into(),
            base_url: base_url.map(str::to_string),
            path: directory.join(format!("{name}.json")),
            http,
        })
    }

    fn read(&self) -> Result<Profile, Failure> {
        if !self.path.exists() {
            let base = endpoint(self.base_url.as_deref().unwrap_or(DEFAULT_WORKSPACE))?;
            let mut profile = Profile::new();
            profile.insert("version".into(), json!(1));
            profile.insert("name".into(), json!(self.name));
            profile.insert("baseUrl".into(), json!(base));
            return Ok(profile);
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(fail(
                "UNSAFE_STORAGE",
                "Agent profiles must not be symbolic links.",
            ));
        }
        let value: Value = fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or_else(|| fail("INVALID_PROFILE", "This agent profile could not be read."))?;
        let Value::Object(profile) = value else {
            return Err(fail(
                "INVALID_PROFILE",
                "This agent profile could not be read.",
            ));
        };
        let mismatch = profile.get("version") != Some(&json!(1))
            || profile.get("name").and_then(Value::as_str) != Some(&self.name)
            || match &self.base_url {
                Some(base) => {
                    profile.get("baseUrl").and_then(Value::as_str) != Some(&endpoint(base)?)
                }
                None => false,
            };
        if mismatch {
            return Err(fail(
                "PROFILE_MISMATCH",
                "This agent profile belongs to another workspace. Use a separate profile directory.",
            ));
        }
        endpoint(
            profile
                .get("baseUrl")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )?;
        Ok(profile)
    }

    fn save(&self, profile: &Profile) -> Result<(), Failure> {
        private_write(&self.path, &Value::Object(profile.clone()))
    }

    fn base(profile: &Profile) -> &str {
        profile
            .get("baseUrl")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    fn call(
        &self,
        base: &str,
        method: Method,
        route: &str,
        body: Option<&Value>,
        credential: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        request(&self.http, base, method, route, body, credential, timeout)
    }

    fn authorized(
        &self,
        method: Method,
        route: &str,
        body: Option<&Value>,
    ) -> Result<Value, Failure> {
        self.authorized_for(method, route, body, TIMEOUT)
    }

    fn authorized_for(
        &self,
        method: Method,
        route: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, Failure> {
        let profile = self.read()?;
        let credential = profile
            .get("credential")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| {
                fail_status("LOGIN_REQUIRED", "Log this agent into Petri first.", 401)
            })?;
        self.call(
            Self::base(&profile),
            method,
            route,
            body,
            Some(credential),
            timeout,
        )
    }

    /// The agent's open session (a heartbeat keeps it), else a new one.
    fn session(&self, force: bool) -> Result<Value, Failure> {
        let profile = self.read()?;
        if let Some(session_id) = profile
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|_| !force)
        {
            match self.authorized(
                Method::POST,
                "/v1/sessions/heartbeat",
                Some(&json!({ "sessionId": session_id })),
            ) {
                Ok(result) => return Ok(result["session"].clone()),
                Err(error) if error.code.as_deref() == Some("SESSION_EXPIRED") => {}
                Err(error) => return Err(error),
            }
        }
        let result = self.authorized(
            Method::POST,
            "/v1/sessions",
            Some(&json!({ "harness": self.harness })),
        )?;
        let mut profile = self.read()?;
        profile.insert("sessionId".into(), result["session"]["id"].clone());
        self.save(&profile)?;
        Ok(result["session"].clone())
    }

    fn session_id(&self) -> Result<Value, Failure> {
        Ok(self.session(false)?["id"].clone())
    }
}

/// `object` with `key` set to `value` when the reply had one.
fn copy(object: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        object.insert(key.into(), value.clone());
    }
}

impl Agent for AgentClient {
    /// Connects this agent: the first login asks the workspace for a pairing
    /// (the owner approves its code in Connect), later ones poll it until
    /// approved, and a connected agent gets its session.
    fn login(&self) -> Result<Value, Failure> {
        let mut profile = self.read()?;
        let base = Self::base(&profile).to_string();
        if profile
            .get("credential")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty())
        {
            let result = self.authorized(Method::GET, "/v1/me", None)?;
            return Ok(
                json!({ "status": "connected", "agent": result["agent"], "session": self.session(false)? }),
            );
        }
        if let Some(pending) = profile.get("pending").cloned().filter(|p| !p.is_null()) {
            let poll = json!({ "pairingId": pending["pairingId"], "secret": pending["secret"] });
            let result = match self.call(
                &base,
                Method::POST,
                "/v1/login/poll",
                Some(&poll),
                None,
                TIMEOUT,
            ) {
                Ok(result) => result,
                Err(error) => {
                    if error.code.as_deref() == Some("PAIRING_EXPIRED") {
                        profile.remove("pending");
                        self.save(&profile)?;
                    }
                    return Err(error);
                }
            };
            if result["status"] == "approved" {
                if !hex_of(&result["credential"], 64)
                    || result["agent"]["name"].as_str() != Some(&self.name)
                {
                    return Err(fail_status(
                        "INVALID_RESPONSE",
                        "Invalid login response.",
                        502,
                    ));
                }
                profile.remove("pending");
                profile.remove("loginAttempt");
                profile.insert("credential".into(), result["credential"].clone());
                profile.insert("agentId".into(), result["agent"]["id"].clone());
                self.save(&profile)?;
                return Ok(
                    json!({ "status": "connected", "agent": result["agent"], "session": self.session(false)? }),
                );
            }
            let mut reply = Map::new();
            copy(&mut reply, "status", result.get("status"));
            copy(&mut reply, "userCode", pending.get("userCode"));
            copy(&mut reply, "expiresAt", result.get("expiresAt"));
            reply.insert(
                "nextStep".into(),
                json!(if result["status"] == "rejected" {
                    "The workspace owner rejected this connection."
                } else {
                    "Ask the owner to approve this code in Petri\u{2019}s Connect view."
                }),
            );
            let error = result
                .get("error")
                .filter(|e| !is_falsy(e))
                .cloned()
                .unwrap_or(Value::Null);
            reply.insert("error".into(), error);
            return Ok(Value::Object(reply));
        }
        let attempt = match profile.get("loginAttempt").filter(|a| !a.is_null()) {
            Some(attempt) => attempt.clone(),
            None => {
                let attempt =
                    json!({ "name": self.name, "harness": self.harness, "loginKey": token() });
                profile.insert("loginAttempt".into(), attempt.clone());
                self.save(&profile)?;
                attempt
            }
        };
        let pending = match self.call(
            &base,
            Method::POST,
            "/v1/login",
            Some(&attempt),
            None,
            TIMEOUT,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                if error.code.as_deref() == Some("PAIRING_EXPIRED") {
                    profile.remove("loginAttempt");
                    self.save(&profile)?;
                }
                return Err(error);
            }
        };
        if !hex_of(&pending["secret"], 64) || !hex_of(&pending["pairingId"], 32) {
            return Err(fail_status(
                "INVALID_RESPONSE",
                "Invalid login response.",
                502,
            ));
        }
        profile.remove("loginAttempt");
        profile.insert("pending".into(), pending.clone());
        self.save(&profile)?;
        let mut reply = Map::new();
        reply.insert("status".into(), json!("pending"));
        copy(&mut reply, "userCode", pending.get("userCode"));
        copy(&mut reply, "expiresAt", pending.get("expiresAt"));
        reply.insert(
            "nextStep".into(),
            json!("Ask the owner to approve this code in Petri\u{2019}s Connect view, then call login again."),
        );
        Ok(Value::Object(reply))
    }

    fn me(&self) -> Result<Value, Failure> {
        self.authorized(Method::GET, "/v1/me", None)
    }

    fn open_session(&self) -> Result<Value, Failure> {
        self.session(true)
    }

    fn close_session(&self) -> Result<Value, Failure> {
        let mut profile = self.read()?;
        let Some(session_id) = profile.get("sessionId").cloned().filter(|s| !is_falsy(s)) else {
            return Ok(json!({ "closed": true }));
        };
        let result = self.authorized(
            Method::POST,
            "/v1/sessions/close",
            Some(&json!({ "sessionId": session_id })),
        )?;
        profile.remove("sessionId");
        self.save(&profile)?;
        Ok(result)
    }

    fn list_work(&self) -> Result<Value, Failure> {
        self.authorized(Method::GET, "/v1/work", None)
    }

    fn next(&self) -> Result<Value, Failure> {
        let body = json!({ "sessionId": self.session_id()? });
        self.authorized(Method::POST, "/v1/work/next", Some(&body))
    }

    fn activity(&self) -> Result<Value, Failure> {
        self.authorized(Method::GET, "/v1/activity", None)
    }

    fn rewards(&self) -> Result<Value, Failure> {
        self.authorized(Method::GET, "/v1/rewards", None)
    }

    fn start_work(&self, work_id: &str, client_key: Option<&str>) -> Result<Value, Failure> {
        let client_key = client_key.map(str::to_string).unwrap_or_else(uuid);
        let body =
            json!({ "workId": work_id, "clientKey": client_key, "sessionId": self.session_id()? });
        self.authorized(Method::POST, "/v1/runs", Some(&body))
    }

    /// One step of the agent's work, for the owner's live feed.
    fn report(&self, run_id: &str, report: Report) -> Result<Value, Failure> {
        let kind = report.kind.unwrap_or("note");
        if !REPORT_KINDS.contains(&kind) {
            return Err(fail(
                "INVALID_EVENT",
                &format!("Report one of: {}.", REPORT_KINDS.join(", ")),
            ));
        }
        let client_key = report.client_key.map(str::to_string).unwrap_or_else(uuid);
        let mut body = Map::new();
        body.insert("sessionId".into(), self.session_id()?);
        body.insert("clientKey".into(), json!(client_key));
        body.insert("kind".into(), json!(kind));
        if let Some(message) = report.message {
            body.insert("message".into(), json!(message));
        }
        if let Some(url) = report.reference_url.filter(|url| !url.is_empty()) {
            body.insert("referenceUrl".into(), json!(url));
        }
        self.authorized(
            Method::POST,
            &format!("/v1/runs/{}/events", segment(run_id)),
            Some(&Value::Object(body)),
        )
    }

    /// A candidate, with the web proof bundle of a page behind the owner's
    /// sign-in when there is one (the file `petri evidence prove` wrote). The
    /// bundle is read here, in the agent's own process, and goes to the
    /// workspace as base64; the workspace uploads it to the API for
    /// verification, then submits the candidate.
    fn submit(
        &self,
        run_id: &str,
        candidate: &Value,
        client_key: Option<&str>,
        evidence_bundle: Option<&str>,
    ) -> Result<Value, Failure> {
        let client_key = client_key.map(str::to_string).unwrap_or_else(uuid);
        let bundle = evidence_bundle
            .map(|path| {
                read_bundle_file(path)
                    .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            })
            .transpose()?;
        let mut body = Map::new();
        body.insert("sessionId".into(), self.session_id()?);
        body.insert("clientKey".into(), json!(client_key));
        body.insert("candidate".into(), candidate.clone());
        if let Some(bundle) = &bundle {
            body.insert("evidenceBundle".into(), json!(bundle));
        }
        self.authorized_for(
            Method::POST,
            &format!("/v1/runs/{}/candidates", segment(run_id)),
            Some(&Value::Object(body)),
            if bundle.is_some() {
                BUNDLE_TIMEOUT
            } else {
                TIMEOUT
            },
        )
    }

    fn finish(&self, run_id: &str, summary: Option<&str>) -> Result<Value, Failure> {
        let mut body = Map::new();
        body.insert("sessionId".into(), self.session_id()?);
        if let Some(summary) = summary {
            body.insert("summary".into(), json!(summary));
        }
        self.authorized(
            Method::POST,
            &format!("/v1/runs/{}/finish", segment(run_id)),
            Some(&Value::Object(body)),
        )
    }
}

/// JavaScript's falsy JSON values: what `||` passes over.
fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

/// The bytes of the bundle petri wrote at `path`: a JSON object in the
/// bundle format, at most 2 MiB. Reads at most one byte past the bound,
/// whatever the file claims its size is.
pub fn read_bundle_file(path: &str) -> Result<Vec<u8>, Failure> {
    if path.trim().is_empty() {
        return Err(not_a_bundle());
    }
    let file = fs::File::open(absolute(Path::new(path))).map_err(|_| {
        fail(
            "BUNDLE_NOT_FOUND",
            "Petri could not open that evidence bundle. Give the path petri evidence prove wrote (its --out file).",
        )
    })?;
    let metadata = file.metadata().map_err(|_| not_a_bundle())?;
    if !metadata.is_file() {
        return Err(not_a_bundle());
    }
    if metadata.len() > MAX_BUNDLE_BYTES {
        return Err(bundle_too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BUNDLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| not_a_bundle())?;
    bundle_bytes(bytes)
}
