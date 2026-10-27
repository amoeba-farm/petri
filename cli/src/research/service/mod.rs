//! The research workspace service, in petri itself (it ran on Node 24 until
//! 2026-10): pairing, revocable agent credentials, sessions, standing
//! research directives, discovered work, and a durable history of actions,
//! in the same files as before (owner.json, workspace.sqlite). It runs as
//! `petri research serve` in the background; the TUI starts it, and replaces
//! an earlier one (another petri version, or the Node service) on its port.

mod app;
mod auth;
mod coverage;
mod db;
mod directives;
mod http;
mod ordered;
#[cfg(test)]
mod tests;
mod upstream;
mod vault;
mod work;

use super::agent::{Flags, parse};
use super::workspace::{
    Failure, endpoint, fail, fail_status, private_directory, private_write, state_directory, token,
};
use reqwest::{Method, blocking::Client};
use serde_json::{Map, Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// The futures API Petri reads by default; a workspace binds to the API it
/// was made with.
const DEFAULT_FUTURES_API: &str = "https://api.amoeba.farm";
/// The retired local candidate API. A workspace still bound to it is
/// rebound to the futures API; nothing else rebinds silently.
const RETIRED_ENDPOINTS: [&str; 1] = ["http://127.0.0.1:4184"];
const DEFAULT_PORT: i64 = 4185;

/// Which service answers on the port: this petri build's (a TUI of another
/// build, or the Node service, replaces it).
pub fn runtime_id() -> String {
    format!(
        "petri-{}-{}",
        env!("CARGO_PKG_VERSION"),
        env!("PETRI_BUILD_COMMIT")
    )
}

/// The owner's workspace configuration (owner.json): its secrets, the API it
/// is bound to and its loopback port.
pub struct Config {
    pub directory: PathBuf,
    pub owner_token: String,
    master_key: String,
    pub upstream: String,
    pub oracle_read_url: String,
    pub port: u16,
    pub base_url: String,
}

fn hex64(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn bound(stored: Option<&str>, requested: Option<&str>) -> Result<String, Failure> {
    let current = stored.filter(|s| !s.is_empty()).map(endpoint).transpose()?;
    match current {
        Some(current) if !RETIRED_ENDPOINTS.contains(&current.as_str()) => Ok(current),
        _ => endpoint(requested.unwrap_or(DEFAULT_FUTURES_API)),
    }
}

/// The workspace in `directory`, made on first use: bound to `upstream`
/// (the futures API) and `oracle_read_url`, on `port`.
pub fn load_owner(
    directory: &Path,
    upstream: Option<&str>,
    port: Option<&str>,
    oracle_read_url: Option<&str>,
) -> Result<Config, Failure> {
    let root = private_directory(directory)?;
    let path = root.join("owner.json");
    let invalid_port = || fail("INVALID_PORT", "Choose a port from 1024 to 65535.");
    let port: Option<i64> = port
        .map(|p| p.trim().parse().map_err(|_| invalid_port()))
        .transpose()?;
    if !path.exists() {
        let initial = json!({ "version": 1, "ownerToken": token(), "masterKey": token(),
            "upstream": endpoint(upstream.unwrap_or(DEFAULT_FUTURES_API))?, "port": port.unwrap_or(DEFAULT_PORT) });
        super::workspace::private_create(&path, &initial)?;
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(fail("UNSAFE_STORAGE", "Invalid owner configuration."));
    }
    let invalid = || fail("INVALID_CONFIG", "Invalid workspace configuration.");
    let mut config: Map<String, Value> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(invalid)?;
    if config.get("version") != Some(&json!(1))
        || !config.get("ownerToken").is_some_and(hex64)
        || !config.get("masterKey").is_some_and(hex64)
    {
        return Err(invalid());
    }
    let original_upstream = bound(config.get("upstream").and_then(Value::as_str), upstream)?;
    if let Some(upstream) = upstream
        && endpoint(upstream)? != original_upstream
    {
        return Err(fail(
            "UPSTREAM_MISMATCH",
            "This workspace is bound to another candidate API. Use a separate state directory.",
        ));
    }
    config.insert("upstream".into(), json!(original_upstream));
    let original_oracle = bound(
        config.get("oracleReadUrl").and_then(Value::as_str),
        Some(oracle_read_url.unwrap_or(&original_upstream)),
    )?;
    if let Some(oracle) = oracle_read_url
        && endpoint(oracle)? != original_oracle
    {
        return Err(fail(
            "ORACLE_MISMATCH",
            "Use a separate workspace for a different oracle read endpoint.",
        ));
    }
    config.insert("oracleReadUrl".into(), json!(original_oracle));
    let port = match port {
        Some(port) => port,
        None => config
            .get("port")
            .and_then(Value::as_i64)
            .ok_or_else(invalid_port)?,
    };
    if !(1024..=65535).contains(&port) {
        return Err(invalid_port());
    }
    config.insert("port".into(), json!(port));
    private_write(&path, &Value::Object(config.clone()))?;
    let text = |key: &str| config[key].as_str().unwrap_or_default().to_string();
    Ok(Config {
        directory: root,
        owner_token: text("ownerToken"),
        master_key: text("masterKey"),
        upstream: original_upstream,
        oracle_read_url: original_oracle,
        port: port as u16,
        base_url: format!("http://127.0.0.1:{port}"),
    })
}

/// Runs the workspace service for `config` until a newer one replaces it
/// (`/v1/owner/restart`). `announce` gets its address once it listens.
pub fn serve(config: &Config, announce: impl FnOnce(&str)) -> Result<(), Failure> {
    let options = app::Options {
        clock: Box::new(|| chrono::Utc::now().timestamp_millis()),
        upstream: Arc::new(upstream::Api::new(&config.upstream)?),
        oracle: Some(Arc::new(upstream::Oracle::new(&config.oracle_read_url)?)),
        runtime_id: runtime_id(),
        // How an agent's shell runs petri here: what the petri that started
        // this service said (`petri-live petri` on a dev PC).
        petri_command: Some(super::runtime::petri_command()),
        seed_directives: true,
        database: config.directory.join("workspace.sqlite"),
    };
    let app = app::App::new(&config.owner_token, &config.master_key, options)?;
    let server = http::start(app, config.port)?;
    announce(&server.base_url);
    server.wait();
    Ok(())
}

/// The workspace's answer to the owner's ping.
enum Ping {
    Up(Value),
    /// Draining for a restart: answering, but not for long.
    Draining,
    /// Nothing listens on the port.
    Down,
    Refused(Failure),
}

/// One owner request to the workspace at `config`.
fn owner_request(
    http: &Client,
    config: &Config,
    method: Method,
    route: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<Value, (bool, Failure)> {
    let mut builder = http
        .request(method, format!("{}/v1/owner/{route}", config.base_url))
        .timeout(timeout)
        .bearer_auth(&config.owner_token)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("cache-control", "no-store");
    if let Some(body) = body {
        builder = builder.body(body.to_string());
    }
    let unavailable = || {
        fail_status(
            "WORKSPACE_UNAVAILABLE",
            "Petri is unavailable. Open the research TUI or start its workspace service.",
            503,
        )
    };
    let response = builder.send().map_err(|_| (true, unavailable()))?;
    let status = response.status();
    let result: Value = response.json().map_err(|_| {
        (
            false,
            fail_status("INVALID_RESPONSE", "Invalid workspace response.", 502),
        )
    })?;
    if !status.is_success() || result["ok"] != true {
        let error = &result["error"];
        return Err((
            false,
            fail_status(
                error["code"].as_str().unwrap_or("WORKSPACE_ERROR"),
                error["message"]
                    .as_str()
                    .unwrap_or("The workspace rejected this request."),
                status.as_u16(),
            ),
        ));
    }
    Ok(result["data"].clone())
}

fn ping(http: &Client, config: &Config) -> Ping {
    match owner_request(
        http,
        config,
        Method::GET,
        "ping",
        None,
        Duration::from_secs(1),
    ) {
        Ok(value) => Ping::Up(value),
        Err((true, _)) => Ping::Down,
        Err((false, error)) if error.code.as_deref() == Some("WORKSPACE_UNAVAILABLE") => {
            Ping::Draining
        }
        Err((false, error)) => Ping::Refused(error),
    }
}

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("an HTTP client")
}

/// Starts `petri research serve` for `config` in the background, apart from
/// this process: it outlives the TUI, so connected agents keep working.
fn spawn(config: &Config) -> Result<std::process::Child, Failure> {
    let failed = || {
        fail_status(
            "SERVICE_START_FAILED",
            "The research service could not start.",
            503,
        )
    };
    let exe = std::env::current_exe().map_err(|_| failed())?;
    let mut command = std::process::Command::new(exe);
    command
        .args(["research", "serve", "--state-dir"])
        .arg(&config.directory)
        .args([
            "--upstream",
            &config.upstream,
            "--oracle-url",
            &config.oracle_read_url,
            "--port",
        ])
        .arg(config.port.to_string())
        .env("PETRI_COMMAND", super::runtime::petri_command())
        .env_remove("PETRI_RESEARCH_OWNER_TOKEN")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own session, as a daemon: no terminal's hangup or Ctrl+C reaches it.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The service must not keep a shell's or agent's output pipe open.
        super::runtime::isolate_caller_pipes().map_err(|_| failed())?;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: no console, no Ctrl+C.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().map_err(|_| failed())
}

/// The workspace service for `config`, running this petri's build: an
/// earlier one on its port (another version, or the Node service) drains
/// and hands over, keeping the workspace's files.
pub fn ensure_service(config: &Config) -> Result<(), Failure> {
    let http = client();
    let ours = runtime_id();
    match ping(&http, config) {
        Ping::Up(current) if current["runtime"] == ours.as_str() => return Ok(()),
        Ping::Up(current) => {
            if current["runtime"].as_str().is_none_or(str::is_empty) {
                return Err(fail_status(
                    "SERVICE_RESTART_REQUIRED",
                    "Close the earlier standalone research service before opening Petri. Your workspace is preserved.",
                    409,
                ));
            }
            // The owner-authenticated drain stops new work, completes in-flight
            // requests, closes the database, then releases the port.
            let mut attempt = 0;
            loop {
                match owner_request(
                    &http,
                    config,
                    Method::POST,
                    "restart",
                    Some(&json!({})),
                    Duration::from_secs(20),
                ) {
                    Ok(_) => break,
                    Err((_, error))
                        if error.code.as_deref() == Some("SERVICE_BUSY") && attempt < 59 =>
                    {
                        attempt += 1;
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Err((_, error)) => return Err(error),
                }
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                std::thread::sleep(Duration::from_millis(100));
                match ping(&http, config) {
                    Ping::Down => break,
                    Ping::Refused(error) => return Err(error),
                    _ if Instant::now() >= deadline => {
                        return Err(fail_status(
                            "SERVICE_BUSY",
                            "The previous workspace is still finishing requests. Try opening Petri again shortly.",
                            409,
                        ));
                    }
                    _ => {}
                }
            }
        }
        Ping::Draining => {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !matches!(ping(&http, config), Ping::Down) {
                if Instant::now() >= deadline {
                    return Err(fail_status(
                        "SERVICE_BUSY",
                        "The previous workspace is still finishing requests. Try opening Petri again shortly.",
                        409,
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ping::Down => {}
        Ping::Refused(error) => return Err(error),
    }
    let mut child = spawn(config)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        match ping(&http, config) {
            Ping::Up(current) if current["runtime"] == ours.as_str() => return Ok(()),
            Ping::Up(_) => {
                return Err(fail_status(
                    "SERVICE_VERSION",
                    "Another version of the research workspace is using this port.",
                    409,
                ));
            }
            Ping::Refused(error) => return Err(error),
            Ping::Down | Ping::Draining => {}
        }
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
    }
    Err(fail_status(
        "SERVICE_START_FAILED",
        "The research workspace could not start. Run petri research serve to see the error.",
        503,
    ))
}

/// The workspace for this petri, running: the TUI's connection to it.
pub fn connect(api: &str) -> Result<(String, String), String> {
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let config = load_owner(
        &state_directory(),
        Some(env("PETRI_RESEARCH_UPSTREAM").as_deref().unwrap_or(api)),
        env("PETRI_RESEARCH_PORT").as_deref(),
        Some(env("PETRI_RESEARCH_ORACLE_URL").as_deref().unwrap_or(api)),
    )
    .and_then(|config| ensure_service(&config).map(|()| config))
    .map_err(|error| super::model::safe(&error.message))?;
    Ok((config.base_url, config.owner_token))
}

fn json_file(path: Option<&str>) -> Result<Value, Failure> {
    let path = path.ok_or_else(|| fail("FILE_REQUIRED", "Provide --file with a JSON file."))?;
    let bytes = std::fs::read(path)
        .map_err(|_| fail("FILE_NOT_FOUND", "Petri could not open that file."))?;
    if bytes.len() > 32768 {
        return Err(fail("FILE_TOO_LARGE", "The JSON file is too large."));
    }
    serde_json::from_slice(&bytes).map_err(|_| fail("INVALID_JSON", "The file is not valid JSON."))
}

fn configured(flags: &Flags, api: &str) -> Result<Config, Failure> {
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
    let directory = flags
        .get("state-dir")
        .map(PathBuf::from)
        .unwrap_or_else(state_directory);
    let upstream = flags
        .get("upstream")
        .map(str::to_string)
        .or_else(|| env("PETRI_RESEARCH_UPSTREAM"))
        .unwrap_or_else(|| api.to_string());
    let oracle = flags
        .get("oracle-url")
        .map(str::to_string)
        .or_else(|| env("PETRI_RESEARCH_ORACLE_URL"))
        .unwrap_or_else(|| api.to_string());
    let port = flags
        .get("port")
        .map(str::to_string)
        .or_else(|| env("PETRI_RESEARCH_PORT"));
    load_owner(&directory, Some(&upstream), port.as_deref(), Some(&oracle))
}

fn owner_command(flags: &Flags, words: &[String], api: &str) -> Result<Option<Value>, Failure> {
    let config = configured(flags, api)?;
    if words.first().map(String::as_str) == Some("serve") {
        serve(&config, |base| {
            let _ = writeln!(std::io::stdout(), "Petri research workspace: {base}");
        })?;
        return Ok(None);
    }
    ensure_service(&config)?;
    let word = |index: usize| words.get(index).map(String::as_str);
    let id = word(2);
    let mut body = Map::new();
    let mut put = |key: &str, value: Option<&str>| {
        if let Some(value) = value {
            body.insert(key.into(), json!(value));
        }
    };
    let (method, route) = match (word(1), id) {
        (Some("address"), _) => {
            put("payoutAuthority", id);
            (Method::POST, "settings")
        }
        (Some("directive"), Some("save")) => {
            let directive = json_file(flags.get("file"))?;
            let http = client();
            return owner_request(
                &http,
                &config,
                Method::POST,
                "directives",
                Some(&directive),
                Duration::from_secs(20),
            )
            .map(Some)
            .map_err(|(_, error)| error);
        }
        (Some("discover"), _) => (Method::POST, "discovery/refresh"),
        (Some("refine"), _) => {
            put("workId", id);
            put("instruction", flags.get("message"));
            (Method::POST, "work/refine")
        }
        (Some("approve"), _) => {
            put("pairingId", id);
            (Method::POST, "pairings/approve")
        }
        (Some("reject"), _) => {
            put("pairingId", id);
            (Method::POST, "pairings/reject")
        }
        (Some("revoke"), _) => {
            put("agentId", id);
            (Method::POST, "agents/revoke")
        }
        (Some("snapshot"), _) => (Method::GET, "snapshot"),
        _ => {
            return Err(fail(
                "INVALID_COMMAND",
                "Unknown owner command. Use --help.",
            ));
        }
    };
    let body = (method == Method::POST).then_some(Value::Object(body));
    let http = client();
    owner_request(
        &http,
        &config,
        method,
        route,
        body.as_ref(),
        Duration::from_secs(20),
    )
    .map(Some)
    .map_err(|(_, error)| error)
}

/// `petri research serve` and `petri research owner ...`, against the
/// futures API at `api` unless the workspace or its flags say otherwise:
/// whether it succeeded (its result on stdout, its error on stderr).
pub fn run(args: &[String], api: &str) -> bool {
    let result = parse(args).and_then(|(flags, words)| owner_command(&flags, &words, api));
    match result {
        Ok(Some(value)) => {
            let output = json!({ "ok": true, "data": value });
            let pretty = serde_json::to_string_pretty(&output).unwrap_or_default();
            let _ = writeln!(std::io::stdout(), "{pretty}");
            true
        }
        Ok(None) => true,
        Err(failure) => {
            super::agent::report_failure(failure);
            false
        }
    }
}
