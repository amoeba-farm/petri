//! An agent's research commands (`petri research agent|session|work|
//! activity|rewards|mcp`), in petri itself: what an agent's harness runs,
//! from its shell or as an MCP server. They talk to the local research
//! workspace over HTTP. Its service and the owner's commands (`serve`,
//! `owner`) still run on the bundled service (runtime.rs).

pub(super) mod client;
pub(super) mod mcp;
mod steps;
#[cfg(test)]
mod tests;

use super::workspace::{self as storage, Failure, fail};
use client::{Agent, AgentClient, Report};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The options of a research command: `--name value` pairs and switches.
#[derive(Default)]
pub(super) struct Flags {
    values: BTreeMap<String, String>,
    switches: BTreeSet<String>,
}

impl Flags {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
    fn on(&self, key: &str) -> bool {
        self.switches.contains(key)
    }
}

const SWITCHES: [&str; 3] = ["json", "help", "wait"];
const OPTIONS: [&str; 16] = [
    "state-dir",
    "upstream",
    "oracle-url",
    "port",
    "agent",
    "name",
    "harness",
    "url",
    "profile-dir",
    "file",
    "key",
    "message",
    "kind",
    "source",
    "summary",
    "evidence-bundle",
];

pub(super) fn parse(args: &[String]) -> Result<(Flags, Vec<String>), Failure> {
    let mut flags = Flags::default();
    let mut words = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        let Some(key) = arg.strip_prefix("--") else {
            words.push(arg.clone());
            continue;
        };
        if SWITCHES.contains(&key) {
            flags.switches.insert(key.into());
        } else if let Some(value) = args
            .get(index)
            .filter(|value| OPTIONS.contains(&key) && !value.is_empty() && !value.starts_with("--"))
        {
            flags.values.insert(key.into(), value.clone());
            index += 1;
        } else {
            return Err(fail(
                "INVALID_OPTION",
                &format!("Unknown option or missing value: {arg}"),
            ));
        }
    }
    Ok((flags, words))
}

const HELP: &str = "Petri \u{2014} oracle research and data submission

  petri tui                             Open the clickable TUI
  petri research serve                   Run the local workspace service
  petri research agent login --name cedar --harness codex
  petri research agent status --name cedar
  petri research work next --agent cedar   Start (or resume) the next research run
  petri research work list --agent cedar
  petri research work start <work-id> --agent cedar --key <request-key>
  petri research work report <run-id> --agent cedar --kind search|read|value|submit|note --message <text> [--source <https URL>]
  petri research work submit <run-id> --agent cedar --file candidate.json [--evidence-bundle <bundle file>]
  petri research work finish <run-id> --agent cedar --summary <text>
  petri research rewards --agent cedar
  petri research mcp --agent cedar --harness codex

Owner controls (also available in the TUI):
  petri research owner address <public-solana-address>
  petri research owner directive save --file directive.json
  petri research owner discover
  petri research owner refine <work-id> --message <research-guidance>
  petri research owner approve <pairing-id>
  petri research owner reject <pairing-id>
  petri research owner revoke <agent-id>

Options: --json, --state-dir <path>, --profile-dir <path>, --url <workspace-url>
Service options: --upstream <candidate-api-origin>, --oracle-url <oracle-read-origin>, --port <loopback-port>
First login returns a pairing code; approve it in Connect, then log in again.
Use --wait to wait for that approval. No model API key or wallet key is needed.
A page behind a sign-in: the owner signs in to petri evidence browser themselves,
petri evidence prove writes a bundle, and work submit --evidence-bundle sends it.
";

/// A candidate file: JSON, at most 32 KiB.
fn json_file(path: Option<&str>) -> Result<Value, Failure> {
    let path = path.ok_or_else(|| fail("FILE_REQUIRED", "Provide --file with a JSON file."))?;
    let bytes = std::fs::read(storage::absolute(std::path::Path::new(path)))
        .map_err(|_| fail("FILE_NOT_FOUND", "Petri could not open that file."))?;
    if bytes.len() > 32768 {
        return Err(fail("FILE_TOO_LARGE", "The JSON file is too large."));
    }
    serde_json::from_slice(&bytes).map_err(|_| fail("INVALID_JSON", "The file is not valid JSON."))
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_millis() as f64)
}

enum Outcome {
    /// A result for stdout.
    Value(Value),
    /// The command wrote its own output (help, the MCP server).
    Done,
}

fn agent_command(flags: &Flags, words: &[String]) -> Result<Outcome, Failure> {
    let word = |index: usize| words.get(index).map(String::as_str);
    let command = word(0).unwrap_or("help");
    if flags.on("help") || command == "help" {
        let _ = std::io::stdout().write_all(HELP.as_bytes());
        return Ok(Outcome::Done);
    }
    if command == "tui" {
        return Err(fail(
            "INVALID_COMMAND",
            "Open the workspace with petri tui.",
        ));
    }
    let client = AgentClient::new(
        flags
            .get("agent")
            .or_else(|| flags.get("name"))
            .unwrap_or("researcher"),
        flags.get("harness").unwrap_or("Custom harness"),
        flags.get("url"),
        flags.get("profile-dir"),
    )?;
    let own = super::runtime::petri_command();
    if command == "mcp" {
        let stdin = std::io::stdin();
        mcp::serve(&client, flags, own, stdin.lock(), std::io::stdout())
            .map_err(|_| fail("MCP_TRANSPORT_CLOSED", "The MCP connection closed."))?;
        return Ok(Outcome::Done);
    }
    let id = |index: usize| {
        word(index).ok_or_else(|| fail("INVALID_INPUT", "Provide the ID this command needs."))
    };
    let value = match (command, word(1)) {
        ("agent", Some("login")) => {
            let mut value = client.login()?;
            let finished = |value: &Value| {
                ["connected", "rejected", "revoked"]
                    .contains(&value["status"].as_str().unwrap_or_default())
            };
            if flags.on("wait") && value["status"] != "connected" {
                let mut stderr = std::io::stderr();
                let _ = writeln!(
                    stderr,
                    "Approve code {} in Petri\u{2019}s Connect view.",
                    steps_text(&value["userCode"])
                );
                while !finished(&value)
                    && value["expiresAt"]
                        .as_f64()
                        .is_some_and(|until| now_ms() < until)
                {
                    std::thread::sleep(Duration::from_secs(2));
                    value = client.login()?;
                }
            }
            if value["status"] == "connected" {
                value["nextStep"] = json!(format!(
                    "Connected. Start researching: run {} and follow what it prints.",
                    steps::self_command(&own, "work next", flags)
                ));
            }
            value
        }
        ("agent", Some("status")) => client.me()?,
        ("session", Some("open")) => client.open_session()?,
        ("session", Some("close")) => client.close_session()?,
        ("work", Some("list")) => client.list_work()?,
        ("work", Some("next")) => steps::assignment_steps(client.next()?, flags, &own),
        ("work", Some("start")) => client.start_work(id(2)?, flags.get("key"))?,
        ("work", Some("report")) => client.report(
            id(2)?,
            Report {
                kind: flags.get("kind"),
                message: flags.get("message"),
                reference_url: flags.get("source"),
                client_key: flags.get("key"),
            },
        )?,
        ("work", Some("submit")) => client.submit(
            id(2)?,
            &json_file(flags.get("file"))?,
            flags.get("key"),
            flags.get("evidence-bundle"),
        )?,
        ("work", Some("finish")) => client.finish(id(2)?, flags.get("summary"))?,
        ("activity", _) => client.activity()?,
        ("rewards", _) => client.rewards()?,
        _ => return Err(fail("INVALID_COMMAND", "Unknown command. Use --help.")),
    };
    Ok(Outcome::Value(value))
}

fn steps_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

/// Text safe for a terminal: no control or bidirectional-override characters.
fn printable(message: &str) -> String {
    message
        .chars()
        .map(|c| {
            if c.is_control()
                || ('\u{202a}'..='\u{202e}').contains(&c)
                || ('\u{2066}'..='\u{2069}').contains(&c)
            {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Runs `petri research <args>` when it is an agent's command, writing its
/// JSON result to stdout (or its error to stderr): whether it succeeded.
/// None: a command of the workspace service or its owner (`serve`,
/// `owner`), for the bundled service to run.
pub fn run(args: &[String]) -> Option<bool> {
    let parsed = parse(args);
    if let Ok((_, words)) = &parsed
        && matches!(words.first().map(String::as_str), Some("serve" | "owner"))
    {
        return None;
    }
    let result = parsed.and_then(|(flags, words)| agent_command(&flags, &words));
    Some(match result {
        Ok(Outcome::Value(value)) => {
            let output = json!({ "ok": true, "data": value });
            let pretty = serde_json::to_string_pretty(&output).unwrap_or_default();
            let _ = writeln!(std::io::stdout(), "{pretty}");
            true
        }
        Ok(Outcome::Done) => true,
        Err(failure) => {
            report_failure(failure);
            false
        }
    })
}

/// A research command's failure, as the one JSON line on stderr agents read.
pub(super) fn report_failure(failure: Failure) {
    let error = json!({ "ok": false, "error": {
        "code": failure.code.unwrap_or_else(|| "PETRI_ERROR".into()),
        "message": printable(&failure.message),
    } });
    let _ = writeln!(std::io::stderr(), "{error}");
}
