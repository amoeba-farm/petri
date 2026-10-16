use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, fs,
    fs::{OpenOptions, Permissions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

const SERVER_NAME: &str = "petri";
const MANAGED_BY_ENV: &str = "PETRI_MCP_MANAGED_BY";
const MANAGED_BY_VALUE: &str = "petri-tui-v1";
const PETRI_BIN_ENV: &str = "PETRI_MCP_PETRI_BIN";
const RETIRED_TRANSACTION_MODE_ENV: &str = "PETRI_MCP_TRANSACTION_MODE";
const SERVER_FILE_NAME: &str = "petri-mcp-server.mjs";
const SERVER_SOURCE: &str = include_str!("../scripts/petri-mcp-server.mjs");
const BRIDGE_FILE_NAME: &str = "petri-action-bridge.mjs";
const BRIDGE_SOURCE: &str = include_str!("../scripts/petri-action-bridge.mjs");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpClientState {
    Disabled,
    Enabled,
    NeedsRepair,
    Conflict,
}

impl McpClientState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::NeedsRepair => "needs_repair",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpSetupStatus {
    pub(crate) codex: McpClientState,
    pub(crate) claude: McpClientState,
    pub(crate) runtime_ready: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpRepairOutcome {
    AlreadyHealthy,
    Repaired,
}

impl McpRepairOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyHealthy => "already_healthy",
            Self::Repaired => "repaired",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpRepairResult {
    pub(crate) status: McpSetupStatus,
    pub(crate) outcome: McpRepairOutcome,
}

impl McpSetupStatus {
    pub(crate) fn is_fully_enabled(&self) -> bool {
        self.codex == McpClientState::Enabled && self.claude == McpClientState::Enabled
    }

    pub(crate) fn is_partially_enabled(&self) -> bool {
        matches!(self.codex, McpClientState::Enabled)
            ^ matches!(self.claude, McpClientState::Enabled)
    }

    pub(crate) fn has_conflict(&self) -> bool {
        matches!(self.codex, McpClientState::Conflict)
            || matches!(self.claude, McpClientState::Conflict)
    }

    #[cfg(test)]
    pub(crate) fn has_enabled_managed_entry(&self) -> bool {
        matches!(
            self.codex,
            McpClientState::Enabled | McpClientState::NeedsRepair
        ) || matches!(
            self.claude,
            McpClientState::Enabled | McpClientState::NeedsRepair
        )
    }

    pub(crate) fn needs_repair(&self) -> bool {
        matches!(self.codex, McpClientState::NeedsRepair)
            || matches!(self.claude, McpClientState::NeedsRepair)
            || (!self.runtime_ready
                && matches!(
                    (self.codex, self.claude),
                    (McpClientState::Enabled, _) | (_, McpClientState::Enabled)
                ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpSetupError {
    message: String,
}

impl McpSetupError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn io(action: &str, path: &Path, error: impl fmt::Display) -> Self {
        Self::new(format!("{action} {}: {error}", path.display()))
    }
}

impl fmt::Display for McpSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for McpSetupError {}

#[derive(Clone, Debug)]
struct SetupLocations {
    codex_config: PathBuf,
    claude_config: PathBuf,
    runtime_script: PathBuf,
}

#[derive(Clone, Debug)]
struct RuntimeSpec {
    locations: SetupLocations,
    node_bin: PathBuf,
    petri_bin: PathBuf,
}

#[derive(Clone, Debug)]
struct FileSnapshot {
    path: PathBuf,
    contents: Option<Vec<u8>>,
    permissions: Option<Permissions>,
}

#[derive(Clone, Debug)]
struct PreparedFile {
    snapshot: FileSnapshot,
    desired: Option<Vec<u8>>,
}

pub(crate) fn status() -> Result<McpSetupStatus, McpSetupError> {
    status_with_locations(&resolve_locations()?)
}

pub(crate) fn enable() -> Result<McpSetupStatus, McpSetupError> {
    let locations = resolve_locations()?;
    let spec = RuntimeSpec {
        node_bin: resolve_node_bin()?,
        petri_bin: resolve_petri_bin()?,
        locations,
    };
    preflight_server(&spec)?;
    enable_with_spec(&spec)
}

pub(crate) fn repair() -> Result<McpRepairResult, McpSetupError> {
    let locations = resolve_locations()?;
    let current = status_with_locations(&locations)?;
    if current.is_fully_enabled() && current.runtime_ready {
        return Ok(McpRepairResult {
            status: current,
            outcome: McpRepairOutcome::AlreadyHealthy,
        });
    }
    ensure_repairable(&current)?;
    let spec = RuntimeSpec {
        node_bin: resolve_node_bin()?,
        petri_bin: resolve_petri_bin()?,
        locations,
    };
    repair_with_spec(&spec, true)
}

pub(crate) fn disable() -> Result<McpSetupStatus, McpSetupError> {
    disable_with_locations(&resolve_locations()?)
}

fn enable_with_spec(spec: &RuntimeSpec) -> Result<McpSetupStatus, McpSetupError> {
    require_file(&spec.node_bin, "Node executable")?;
    require_file(&spec.petri_bin, "Petri executable")?;

    let runtime = capture_file(&spec.locations.runtime_script)?;
    let bridge = capture_file(
        &spec
            .locations
            .runtime_script
            .with_file_name(BRIDGE_FILE_NAME),
    )?;
    let codex = capture_file(&spec.locations.codex_config)?;
    let claude = capture_file(&spec.locations.claude_config)?;

    let mut codex_doc = parse_codex(&codex)?;
    ensure_codex_entry_is_available(&codex_doc)?;
    set_codex_enabled(&mut codex_doc, spec)?;

    let mut claude_doc = parse_claude(&claude)?;
    ensure_claude_entry_is_available(&claude_doc)?;
    set_claude_enabled(&mut claude_doc, spec)?;

    let prepared = vec![
        PreparedFile {
            snapshot: bridge,
            desired: Some(BRIDGE_SOURCE.as_bytes().to_vec()),
        },
        PreparedFile {
            snapshot: runtime,
            desired: Some(SERVER_SOURCE.as_bytes().to_vec()),
        },
        PreparedFile {
            snapshot: codex,
            desired: Some(codex_doc.to_string().into_bytes()),
        },
        PreparedFile {
            snapshot: claude,
            desired: Some(render_json(&claude_doc)?),
        },
    ];
    commit_prepared(&prepared)?;
    status_with_locations(&spec.locations)
}

fn repair_with_spec(
    spec: &RuntimeSpec,
    run_preflight: bool,
) -> Result<McpRepairResult, McpSetupError> {
    let current = status_with_locations(&spec.locations)?;
    if current.is_fully_enabled() && current.runtime_ready {
        return Ok(McpRepairResult {
            status: current,
            outcome: McpRepairOutcome::AlreadyHealthy,
        });
    }
    ensure_repairable(&current)?;

    if run_preflight {
        preflight_server(spec)?;
    }
    let status = enable_with_spec(spec)?;
    if !status.is_fully_enabled() || !status.runtime_ready {
        return Err(McpSetupError::new(
            "Petri rebuilt the owned connection, but its health check still did not pass.",
        ));
    }
    Ok(McpRepairResult {
        status,
        outcome: McpRepairOutcome::Repaired,
    })
}

fn ensure_repairable(status: &McpSetupStatus) -> Result<(), McpSetupError> {
    if status.has_conflict() {
        return Err(McpSetupError::new(
            "Petri found an existing connection it does not own. Repair stopped without changing it.",
        ));
    }
    if !status.needs_repair() && !status.is_partially_enabled() {
        return Err(McpSetupError::new(
            "The Petri connection is not enabled. Use Enable Petri MCP to connect it.",
        ));
    }
    Ok(())
}

fn disable_with_locations(locations: &SetupLocations) -> Result<McpSetupStatus, McpSetupError> {
    let codex = capture_file(&locations.codex_config)?;
    let claude = capture_file(&locations.claude_config)?;

    let mut codex_doc = parse_codex(&codex)?;
    let codex_changed = set_codex_disabled(&mut codex_doc)?;

    let mut claude_doc = parse_claude(&claude)?;
    let claude_changed = remove_claude_entry(&mut claude_doc)?;

    let codex_desired = if codex_changed {
        Some(codex_doc.to_string().into_bytes())
    } else {
        codex.contents.clone()
    };
    let claude_desired = if claude_changed {
        Some(render_json(&claude_doc)?)
    } else {
        claude.contents.clone()
    };

    commit_prepared(&[
        PreparedFile {
            snapshot: codex,
            desired: codex_desired,
        },
        PreparedFile {
            snapshot: claude,
            desired: claude_desired,
        },
    ])?;
    status_with_locations(locations)
}

fn status_with_locations(locations: &SetupLocations) -> Result<McpSetupStatus, McpSetupError> {
    let runtime_ready = fs::read(&locations.runtime_script)
        .map(|contents| contents == SERVER_SOURCE.as_bytes())
        .unwrap_or(false)
        && fs::read(locations.runtime_script.with_file_name(BRIDGE_FILE_NAME))
            .map(|contents| contents == BRIDGE_SOURCE.as_bytes())
            .unwrap_or(false);
    let codex = capture_file(&locations.codex_config)?;
    let claude = capture_file(&locations.claude_config)?;
    let codex_doc = parse_codex(&codex)?;
    let claude_doc = parse_claude(&claude)?;

    let codex_state = codex_state(&codex_doc, locations, runtime_ready)?;
    let claude_state = claude_state(&claude_doc, locations, runtime_ready)?;

    Ok(McpSetupStatus {
        codex: codex_state,
        claude: claude_state,
        runtime_ready,
    })
}

fn codex_state(
    document: &DocumentMut,
    locations: &SetupLocations,
    runtime_ready: bool,
) -> Result<McpClientState, McpSetupError> {
    let Some(item) = codex_entry(document)? else {
        return Ok(McpClientState::Disabled);
    };
    if !codex_entry_owned(item) {
        return Ok(McpClientState::Conflict);
    }
    let Some(table) = item.as_table() else {
        return Ok(McpClientState::NeedsRepair);
    };
    if table
        .get("enabled")
        .and_then(Item::as_bool)
        .is_some_and(|enabled| !enabled)
    {
        return Ok(McpClientState::Disabled);
    }

    let healthy = runtime_ready
        && item_path_exists(table.get("command").and_then(Item::as_str))
        && table
            .get("args")
            .and_then(Item::as_array)
            .is_some_and(|args| {
                args.len() == 1
                    && args
                        .get(0)
                        .and_then(|argument| argument.as_str())
                        .is_some_and(|argument| {
                            same_path_text(argument, &locations.runtime_script)
                                && Path::new(argument).is_file()
                        })
            })
        && table.get("cwd").and_then(Item::as_str).is_some_and(|cwd| {
            Path::new(cwd).is_dir()
                && locations
                    .runtime_script
                    .parent()
                    .is_some_and(|runtime_dir| same_path_text(cwd, runtime_dir))
        })
        && table
            .get("default_tools_approval_mode")
            .and_then(Item::as_str)
            == Some("writes")
        && table
            .get("env")
            .and_then(Item::as_table)
            .and_then(|env_table| env_table.get(PETRI_BIN_ENV))
            .and_then(Item::as_str)
            .is_some_and(|path| Path::new(path).is_file())
        && table
            .get("env")
            .and_then(Item::as_table)
            .is_some_and(|env_table| !env_table.contains_key(RETIRED_TRANSACTION_MODE_ENV));

    Ok(if healthy {
        McpClientState::Enabled
    } else {
        McpClientState::NeedsRepair
    })
}

fn claude_state(
    document: &JsonValue,
    locations: &SetupLocations,
    runtime_ready: bool,
) -> Result<McpClientState, McpSetupError> {
    let Some(entry) = claude_entry(document)? else {
        return Ok(McpClientState::Disabled);
    };
    if !claude_entry_owned(entry) {
        return Ok(McpClientState::Conflict);
    }
    let healthy = runtime_ready
        && entry.get("type").and_then(JsonValue::as_str) == Some("stdio")
        && item_path_exists(entry.get("command").and_then(JsonValue::as_str))
        && entry
            .get("args")
            .and_then(JsonValue::as_array)
            .is_some_and(|args| {
                args.len() == 1
                    && args[0].as_str().is_some_and(|argument| {
                        same_path_text(argument, &locations.runtime_script)
                            && Path::new(argument).is_file()
                    })
            })
        && entry
            .get("env")
            .and_then(JsonValue::as_object)
            .and_then(|env_map| env_map.get(PETRI_BIN_ENV))
            .and_then(JsonValue::as_str)
            .is_some_and(|path| Path::new(path).is_file())
        && entry
            .get("env")
            .and_then(JsonValue::as_object)
            .is_some_and(|env_map| !env_map.contains_key(RETIRED_TRANSACTION_MODE_ENV));

    Ok(if healthy {
        McpClientState::Enabled
    } else {
        McpClientState::NeedsRepair
    })
}

fn set_codex_enabled(document: &mut DocumentMut, spec: &RuntimeSpec) -> Result<(), McpSetupError> {
    let servers = codex_servers_mut(document)?;
    let mut server = Table::new();
    server.insert("command", value(path_text(&spec.node_bin)));
    let mut args = Array::new();
    args.push(path_text(&spec.locations.runtime_script));
    server.insert("args", value(args));
    let runtime_dir = spec
        .locations
        .runtime_script
        .parent()
        .ok_or_else(|| McpSetupError::new("Petri MCP runtime path has no parent directory."))?;
    server.insert("cwd", value(path_text(runtime_dir)));
    server.insert("enabled", value(true));
    server.insert("required", value(false));
    server.insert("default_tools_approval_mode", value("writes"));

    let mut server_env = Table::new();
    server_env.insert(PETRI_BIN_ENV, value(path_text(&spec.petri_bin)));
    server_env.insert(MANAGED_BY_ENV, value(MANAGED_BY_VALUE));
    server.insert("env", Item::Table(server_env));
    servers.insert(SERVER_NAME, Item::Table(server));
    Ok(())
}

fn set_codex_disabled(document: &mut DocumentMut) -> Result<bool, McpSetupError> {
    if codex_entry(document)?.is_some_and(|entry| !codex_entry_owned(entry)) {
        return Ok(false);
    }
    let Some(servers) = codex_servers_mut_if_present(document)? else {
        return Ok(false);
    };
    let Some(entry) = servers.get_mut(SERVER_NAME) else {
        return Ok(false);
    };
    let Some(table) = entry.as_table_mut() else {
        return Err(McpSetupError::new(
            "The existing Codex Petri MCP entry cannot be safely disabled.",
        ));
    };
    if table.get("enabled").and_then(Item::as_bool) == Some(false) {
        return Ok(false);
    }
    table.insert("enabled", value(false));
    Ok(true)
}

fn set_claude_enabled(document: &mut JsonValue, spec: &RuntimeSpec) -> Result<(), McpSetupError> {
    let servers = claude_servers_mut(document)?;
    servers.insert(
        SERVER_NAME.to_string(),
        json!({
            "type": "stdio",
            "command": path_text(&spec.node_bin),
            "args": [path_text(&spec.locations.runtime_script)],
            "env": {
                PETRI_BIN_ENV: path_text(&spec.petri_bin),
                MANAGED_BY_ENV: MANAGED_BY_VALUE,
            }
        }),
    );
    Ok(())
}

fn remove_claude_entry(document: &mut JsonValue) -> Result<bool, McpSetupError> {
    if claude_entry(document)?.is_some_and(|entry| !claude_entry_owned(entry)) {
        return Ok(false);
    }
    let Some(root) = document.as_object_mut() else {
        return Err(McpSetupError::new(
            "Claude Code configuration must contain a JSON object.",
        ));
    };
    let Some(servers) = root.get_mut("mcpServers") else {
        return Ok(false);
    };
    let Some(servers) = servers.as_object_mut() else {
        return Err(McpSetupError::new(
            "Claude Code mcpServers must contain a JSON object.",
        ));
    };
    Ok(servers.remove(SERVER_NAME).is_some())
}

fn preflight_server(spec: &RuntimeSpec) -> Result<(), McpSetupError> {
    require_file(&spec.node_bin, "Node executable")?;
    require_file(&spec.petri_bin, "Petri executable")?;
    let runtime_dir = spec
        .locations
        .runtime_script
        .parent()
        .ok_or_else(|| McpSetupError::new("Petri MCP runtime path has no parent directory."))?;
    ensure_private_directory(runtime_dir)?;
    let (smoke_path, mut smoke_file) =
        create_temporary_file(&spec.locations.runtime_script, "smoke")?;
    // The preflight is self-contained and never overwrites a live companion.
    use base64::Engine as _;
    let bridge_url = base64::engine::general_purpose::STANDARD.encode(BRIDGE_SOURCE);
    let smoke_source = SERVER_SOURCE.replace(
        "import { ResidentPetri } from './petri-action-bridge.mjs';",
        &format!(
            "const {{ ResidentPetri }} = await import('data:text/javascript;base64,{bridge_url}');"
        ),
    );
    smoke_file
        .write_all(smoke_source.as_bytes())
        .and_then(|_| smoke_file.sync_all())
        .map_err(|error| {
            let _ = fs::remove_file(&smoke_path);
            McpSetupError::io(
                "Cannot prepare the Petri MCP self-check at",
                &smoke_path,
                error,
            )
        })?;
    drop(smoke_file);
    set_private_file_permissions(&smoke_path)?;

    let result = run_server_preflight(spec, &smoke_path, runtime_dir);
    let _ = fs::remove_file(&smoke_path);
    result
}

fn run_server_preflight(
    spec: &RuntimeSpec,
    smoke_path: &Path,
    runtime_dir: &Path,
) -> Result<(), McpSetupError> {
    let mut child = Command::new(&spec.node_bin)
        .arg(smoke_path)
        .current_dir(runtime_dir)
        .env(PETRI_BIN_ENV, &spec.petri_bin)
        .env(MANAGED_BY_ENV, MANAGED_BY_VALUE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            McpSetupError::new(format!(
                "Petri MCP could not start with Node.js at {}: {error}",
                spec.node_bin.display()
            ))
        })?;
    let mut stdout_pipe = child.stdout.take().ok_or_else(|| {
        McpSetupError::new("Petri MCP self-check could not open the server output stream.")
    })?;
    let mut stderr_pipe = child.stderr.take().ok_or_else(|| {
        McpSetupError::new("Petri MCP self-check could not open the server error stream.")
    })?;
    let stdout_reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout_pipe.read_to_end(&mut output).map(|_| output)
    });
    let stderr_reader = thread::spawn(move || {
        let mut output = Vec::new();
        stderr_pipe.read_to_end(&mut output).map(|_| output)
    });

    let requests = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"petri-setup-check\",\"version\":\"0.1.0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"petri.mcp_manifest\",\"arguments\":{}}}\n",
    );
    let mut stdin = child.stdin.take().ok_or_else(|| {
        McpSetupError::new("Petri MCP self-check could not open the server input stream.")
    })?;
    if let Err(error) = stdin.write_all(requests.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(McpSetupError::new(format!(
            "Petri MCP self-check could not send a request: {error}"
        )));
    }
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(8);
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(McpSetupError::new(
                    "Petri MCP self-check timed out before the server became ready.",
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(McpSetupError::new(format!(
                    "Petri MCP self-check could not read the server status: {error}"
                )));
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| McpSetupError::new("Petri MCP self-check output reader stopped."))?
        .map_err(|error| {
            McpSetupError::new(format!(
                "Petri MCP self-check could not read the server response: {error}"
            ))
        })?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| McpSetupError::new("Petri MCP self-check error reader stopped."))?
        .map_err(|error| {
            McpSetupError::new(format!(
                "Petri MCP self-check could not read the server error output: {error}"
            ))
        })?;
    if !exit_status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        return Err(McpSetupError::new(format!(
            "Petri MCP self-check failed{}.",
            short_process_detail(&stderr)
        )));
    }
    validate_preflight_output(&stdout)
}

fn validate_preflight_output(stdout: &[u8]) -> Result<(), McpSetupError> {
    let text = std::str::from_utf8(stdout).map_err(|error| {
        McpSetupError::new(format!(
            "Petri MCP self-check returned unreadable output: {error}"
        ))
    })?;
    let responses = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<JsonValue>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            McpSetupError::new(format!(
                "Petri MCP self-check returned invalid JSON: {error}"
            ))
        })?;
    let response = |id: u64| {
        responses
            .iter()
            .find(|response| response.get("id").and_then(JsonValue::as_u64) == Some(id))
            .ok_or_else(|| {
                McpSetupError::new(format!(
                    "Petri MCP self-check did not return response {id}."
                ))
            })
    };
    for id in 1..=3 {
        if response(id)?.get("error").is_some() {
            return Err(McpSetupError::new(format!(
                "Petri MCP self-check response {id} reported an error."
            )));
        }
    }
    if response(1)?
        .pointer("/result/serverInfo/name")
        .and_then(JsonValue::as_str)
        != Some("petri-mcp")
    {
        return Err(McpSetupError::new(
            "Petri MCP self-check did not initialize the Petri server.",
        ));
    }
    if !response(2)?
        .pointer("/result/tools")
        .and_then(JsonValue::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("name").and_then(JsonValue::as_str) == Some("petri.mcp_manifest")
            })
        })
    {
        return Err(McpSetupError::new(
            "Petri MCP self-check could not discover Petri tools.",
        ));
    }
    if response(3)?
        .pointer("/result/structuredContent/protocol/name")
        .and_then(JsonValue::as_str)
        != Some("petri-agent-protocol")
    {
        return Err(McpSetupError::new(
            "Petri MCP self-check could not call the Petri manifest tool.",
        ));
    }
    Ok(())
}

fn short_process_detail(stderr: &str) -> String {
    let detail = stderr.trim().replace(['\r', '\n'], " ");
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {}", detail.chars().take(240).collect::<String>())
    }
}

fn ensure_codex_entry_is_available(document: &DocumentMut) -> Result<(), McpSetupError> {
    if codex_entry(document)?.is_some_and(|item| !codex_entry_owned(item)) {
        return Err(McpSetupError::new(
            "Codex already has a Petri MCP entry that this TUI does not own. It was left unchanged.",
        ));
    }
    Ok(())
}

fn ensure_claude_entry_is_available(document: &JsonValue) -> Result<(), McpSetupError> {
    if claude_entry(document)?.is_some_and(|entry| !claude_entry_owned(entry)) {
        return Err(McpSetupError::new(
            "Claude Code already has a Petri MCP entry that this TUI does not own. It was left unchanged.",
        ));
    }
    Ok(())
}

fn codex_entry(document: &DocumentMut) -> Result<Option<&Item>, McpSetupError> {
    let Some(servers) = document.as_table().get("mcp_servers") else {
        return Ok(None);
    };
    let Some(servers) = servers.as_table() else {
        return Err(McpSetupError::new(
            "Codex mcp_servers must contain a TOML table.",
        ));
    };
    Ok(servers.get(SERVER_NAME))
}

fn codex_entry_owned(item: &Item) -> bool {
    item.as_table()
        .and_then(|table| table.get("env"))
        .and_then(Item::as_table)
        .and_then(|server_env| server_env.get(MANAGED_BY_ENV))
        .and_then(Item::as_str)
        == Some(MANAGED_BY_VALUE)
}

fn codex_servers_mut(document: &mut DocumentMut) -> Result<&mut Table, McpSetupError> {
    if document.as_table().get("mcp_servers").is_none() {
        document
            .as_table_mut()
            .insert("mcp_servers", Item::Table(Table::new()));
    }
    document
        .as_table_mut()
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| McpSetupError::new("Codex mcp_servers must contain a TOML table."))
}

fn codex_servers_mut_if_present(
    document: &mut DocumentMut,
) -> Result<Option<&mut Table>, McpSetupError> {
    let Some(servers) = document.as_table_mut().get_mut("mcp_servers") else {
        return Ok(None);
    };
    servers
        .as_table_mut()
        .map(Some)
        .ok_or_else(|| McpSetupError::new("Codex mcp_servers must contain a TOML table."))
}

fn claude_entry(document: &JsonValue) -> Result<Option<&JsonValue>, McpSetupError> {
    let Some(root) = document.as_object() else {
        return Err(McpSetupError::new(
            "Claude Code configuration must contain a JSON object.",
        ));
    };
    let Some(servers) = root.get("mcpServers") else {
        return Ok(None);
    };
    let Some(servers) = servers.as_object() else {
        return Err(McpSetupError::new(
            "Claude Code mcpServers must contain a JSON object.",
        ));
    };
    Ok(servers.get(SERVER_NAME))
}

fn claude_entry_owned(entry: &JsonValue) -> bool {
    entry
        .get("env")
        .and_then(JsonValue::as_object)
        .and_then(|server_env| server_env.get(MANAGED_BY_ENV))
        .and_then(JsonValue::as_str)
        == Some(MANAGED_BY_VALUE)
}

fn claude_servers_mut(
    document: &mut JsonValue,
) -> Result<&mut JsonMap<String, JsonValue>, McpSetupError> {
    let Some(root) = document.as_object_mut() else {
        return Err(McpSetupError::new(
            "Claude Code configuration must contain a JSON object.",
        ));
    };
    if !root.contains_key("mcpServers") {
        root.insert("mcpServers".to_string(), json!({}));
    }
    root.get_mut("mcpServers")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| McpSetupError::new("Claude Code mcpServers must contain a JSON object."))
}

fn parse_codex(snapshot: &FileSnapshot) -> Result<DocumentMut, McpSetupError> {
    let Some(contents) = snapshot.contents.as_deref() else {
        return Ok(DocumentMut::new());
    };
    let text = std::str::from_utf8(contents).map_err(|error| {
        McpSetupError::io("Cannot read Codex configuration at", &snapshot.path, error)
    })?;
    text.parse::<DocumentMut>().map_err(|error| {
        McpSetupError::io("Cannot parse Codex configuration at", &snapshot.path, error)
    })
}

fn parse_claude(snapshot: &FileSnapshot) -> Result<JsonValue, McpSetupError> {
    let Some(contents) = snapshot.contents.as_deref() else {
        return Ok(json!({}));
    };
    serde_json::from_slice(contents).map_err(|error| {
        McpSetupError::io(
            "Cannot parse Claude Code configuration at",
            &snapshot.path,
            error,
        )
    })
}

fn render_json(document: &JsonValue) -> Result<Vec<u8>, McpSetupError> {
    let mut contents = serde_json::to_vec_pretty(document).map_err(|error| {
        McpSetupError::new(format!(
            "Cannot serialize Claude Code configuration: {error}"
        ))
    })?;
    contents.push(b'\n');
    Ok(contents)
}

fn capture_file(path: &Path) -> Result<FileSnapshot, McpSetupError> {
    let path = resolve_write_path(path)?;
    match fs::read(&path) {
        Ok(contents) => {
            let metadata = fs::metadata(&path)
                .map_err(|error| McpSetupError::io("Cannot inspect", &path, error))?;
            Ok(FileSnapshot {
                path,
                contents: Some(contents),
                permissions: Some(metadata.permissions()),
            })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileSnapshot {
            path,
            contents: None,
            permissions: None,
        }),
        Err(error) => Err(McpSetupError::io("Cannot read", &path, error)),
    }
}

fn resolve_write_path(path: &Path) -> Result<PathBuf, McpSetupError> {
    let mut current = path.to_path_buf();
    for _ in 0..16 {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&current)
                    .map_err(|error| McpSetupError::io("Cannot read symlink", &current, error))?;
                current = if target.is_absolute() {
                    target
                } else {
                    current
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(target)
                };
            }
            Ok(_) => return Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(error) => return Err(McpSetupError::io("Cannot inspect", &current, error)),
        }
    }
    Err(McpSetupError::new(format!(
        "Cannot update {} because its symlink chain is too deep.",
        path.display()
    )))
}

fn commit_prepared(files: &[PreparedFile]) -> Result<(), McpSetupError> {
    let mut changed = Vec::new();
    for (index, file) in files.iter().enumerate() {
        if file.snapshot.contents == file.desired {
            continue;
        }
        if let Err(error) = apply_prepared(file) {
            let mut rollback_indexes = changed.clone();
            rollback_indexes.push(index);
            let mut rollback_issues = Vec::new();
            for rollback_index in rollback_indexes.into_iter().rev() {
                if let Err(rollback_error) = restore_snapshot(&files[rollback_index].snapshot) {
                    rollback_issues.push(rollback_error.to_string());
                }
            }
            let rollback_note = if rollback_issues.is_empty() {
                String::new()
            } else {
                format!(" Rollback issue: {}", rollback_issues.join("; "))
            };
            return Err(McpSetupError::new(format!("{error}{rollback_note}")));
        }
        changed.push(index);
    }
    Ok(())
}

fn apply_prepared(file: &PreparedFile) -> Result<(), McpSetupError> {
    match file.desired.as_deref() {
        Some(contents) => atomic_write(
            &file.snapshot.path,
            contents,
            file.snapshot.permissions.as_ref(),
        ),
        None => remove_file_if_present(&file.snapshot.path),
    }
}

fn restore_snapshot(snapshot: &FileSnapshot) -> Result<(), McpSetupError> {
    match snapshot.contents.as_deref() {
        Some(contents) => atomic_write(&snapshot.path, contents, snapshot.permissions.as_ref()),
        None => remove_file_if_present(&snapshot.path),
    }
}

fn atomic_write(
    path: &Path,
    contents: &[u8],
    previous_permissions: Option<&Permissions>,
) -> Result<(), McpSetupError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_private_directory(parent)?;
    let (temporary_path, mut temporary_file) = create_temporary_file(path, "tmp")?;
    if let Err(error) = temporary_file
        .write_all(contents)
        .and_then(|_| temporary_file.sync_all())
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(McpSetupError::io("Cannot write", path, error));
    }
    drop(temporary_file);

    if let Some(permissions) = previous_permissions {
        fs::set_permissions(&temporary_path, permissions.clone()).map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            McpSetupError::io("Cannot preserve permissions for", path, error)
        })?;
    } else {
        set_private_file_permissions(&temporary_path)?;
    }

    match fs::rename(&temporary_path, path) {
        Ok(()) => sync_directory(parent),
        Err(_first_error) if path.exists() => {
            let backup_path = unique_sibling_path(path, "swap", 0);
            fs::rename(path, &backup_path).map_err(|error| {
                let _ = fs::remove_file(&temporary_path);
                McpSetupError::io("Cannot replace", path, error)
            })?;
            if let Err(error) = fs::rename(&temporary_path, path) {
                let restore_error = fs::rename(&backup_path, path).err();
                let _ = fs::remove_file(&temporary_path);
                return Err(McpSetupError::new(format!(
                    "Cannot replace {}: {error}.{}",
                    path.display(),
                    restore_error
                        .map(|restore| format!(
                            " The original also could not be restored: {restore}"
                        ))
                        .unwrap_or_default()
                )));
            }
            let _ = fs::remove_file(&backup_path);
            sync_directory(parent)
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary_path);
            Err(McpSetupError::io("Cannot install", path, error))
        }
    }
}

fn create_temporary_file(path: &Path, label: &str) -> Result<(PathBuf, fs::File), McpSetupError> {
    for attempt in 0..16 {
        let temporary_path = unique_sibling_path(path, label, attempt);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary_path) {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(McpSetupError::io("Cannot create", &temporary_path, error)),
        }
    }
    Err(McpSetupError::new(format!(
        "Cannot create a temporary file beside {}.",
        path.display()
    )))
}

fn unique_sibling_path(path: &Path, label: &str, attempt: usize) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("petri-config");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    parent.join(format!(
        ".{file_stem}.petri-{label}-{}-{nonce}-{attempt}{extension}",
        std::process::id(),
    ))
}

fn remove_file_if_present(path: &Path) -> Result<(), McpSetupError> {
    match fs::remove_file(path) {
        Ok(()) => sync_directory(path.parent().unwrap_or_else(|| Path::new("."))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(McpSetupError::io("Cannot remove", path, error)),
    }
}

fn ensure_private_directory(path: &Path) -> Result<(), McpSetupError> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path)
        .map_err(|error| McpSetupError::io("Cannot create directory", path, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, Permissions::from_mode(0o700))
            .map_err(|error| McpSetupError::io("Cannot secure directory", path, error))?;
    }
    Ok(())
}

fn set_private_file_permissions(_path: &Path) -> Result<(), McpSetupError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(_path, Permissions::from_mode(0o600))
            .map_err(|error| McpSetupError::io("Cannot secure", _path, error))?;
    }
    Ok(())
}

fn sync_directory(_path: &Path) -> Result<(), McpSetupError> {
    #[cfg(unix)]
    {
        let directory = fs::File::open(_path)
            .map_err(|error| McpSetupError::io("Cannot open directory", _path, error))?;
        directory
            .sync_all()
            .map_err(|error| McpSetupError::io("Cannot sync directory", _path, error))?;
    }
    Ok(())
}

fn resolve_locations() -> Result<SetupLocations, McpSetupError> {
    let home = home_dir()?;
    let codex_home = nonempty_env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let runtime_root = nonempty_env_path("APPDATA")
        .or_else(|| nonempty_env_path("LOCALAPPDATA"))
        .map(|path| path.join("Amoeba").join("Petri"))
        .or_else(|| {
            nonempty_env_path("XDG_CONFIG_HOME").map(|path| path.join("amoeba").join("petri"))
        })
        .unwrap_or_else(|| home.join(".config").join("amoeba").join("petri"));

    Ok(SetupLocations {
        codex_config: absolute_path(codex_home.join("config.toml"))?,
        claude_config: absolute_path(home.join(".claude.json"))?,
        runtime_script: absolute_path(runtime_root.join("mcp").join(SERVER_FILE_NAME))?,
    })
}

fn resolve_node_bin() -> Result<PathBuf, McpSetupError> {
    if let Some(path) = nonempty_env_path("PETRI_MCP_NODE_BIN") {
        return canonical_file(path, "Node executable");
    }
    let path = env::var_os("PATH").ok_or_else(|| {
        McpSetupError::new("Node could not be found because PATH is not available.")
    })?;
    let executable_names: &[&str] = if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node"]
    };
    for directory in env::split_paths(&path) {
        for executable_name in executable_names {
            let candidate = directory.join(executable_name);
            if candidate.is_file() {
                return canonical_file(candidate, "Node executable");
            }
        }
    }
    Err(McpSetupError::new(
        "Petri MCP needs Node.js, but no Node executable was found on this computer.",
    ))
}

fn resolve_petri_bin() -> Result<PathBuf, McpSetupError> {
    if let Some(path) = nonempty_env_path(PETRI_BIN_ENV) {
        return canonical_file(path, "Petri executable");
    }
    let path = env::current_exe().map_err(|error| {
        McpSetupError::new(format!("Cannot locate the Petri executable: {error}"))
    })?;
    canonical_file(path, "Petri executable")
}

fn home_dir() -> Result<PathBuf, McpSetupError> {
    #[cfg(windows)]
    let home = nonempty_env_path("USERPROFILE")
        .or_else(home_drive_path)
        .or_else(|| nonempty_env_path("HOME"));

    #[cfg(not(windows))]
    let home = nonempty_env_path("HOME")
        .or_else(|| nonempty_env_path("USERPROFILE"))
        .or_else(home_drive_path);

    home.ok_or_else(|| McpSetupError::new("Cannot find the current user's home directory."))
}

fn home_drive_path() -> Option<PathBuf> {
    let drive = nonempty_env_os("HOMEDRIVE")?;
    let path = nonempty_env_os("HOMEPATH")?;
    let mut combined = drive;
    combined.push(path);
    Some(PathBuf::from(combined))
}

fn nonempty_env_path(name: &str) -> Option<PathBuf> {
    nonempty_env_os(name).map(PathBuf::from)
}

fn nonempty_env_os(name: &str) -> Option<OsString> {
    env::var_os(name).filter(|value| !value.is_empty())
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, McpSetupError> {
    if path.is_absolute() {
        return Ok(path);
    }
    env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| McpSetupError::new(format!("Cannot resolve an absolute path: {error}")))
}

fn canonical_file(path: PathBuf, label: &str) -> Result<PathBuf, McpSetupError> {
    require_file(&path, label)?;
    fs::canonicalize(&path)
        .map_err(|error| McpSetupError::io(&format!("Cannot resolve {label} at"), &path, error))
}

fn require_file(path: &Path, label: &str) -> Result<(), McpSetupError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(McpSetupError::new(format!(
            "{label} is unavailable at {}.",
            path.display()
        )))
    }
}

fn item_path_exists(path: Option<&str>) -> bool {
    path.is_some_and(|path| Path::new(path).is_file())
}

fn same_path_text(text: &str, path: &Path) -> bool {
    text == path_text(path)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default();
            let path = env::temp_dir().join(format!(
                "petri-mcp-setup-{label}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn join(&self, path: impl AsRef<Path>) -> PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_spec(root: &TestDir) -> RuntimeSpec {
        let node_bin = root.join("runtime with spaces/node executable");
        let petri_bin = root.join("runtime with spaces/petri executable");
        write_test_file(&node_bin, b"node");
        write_test_file(&petri_bin, b"petri");
        RuntimeSpec {
            locations: SetupLocations {
                codex_config: root.join("codex home/config.toml"),
                claude_config: root.join("claude home/.claude.json"),
                runtime_script: root.join("Petri MCP Runtime/petri-mcp-server.mjs"),
            },
            node_bin,
            petri_bin,
        }
    }

    fn write_test_file(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().expect("file parent")).expect("create file parent");
        fs::write(path, contents).expect("write test file");
    }

    #[test]
    fn enable_preserves_other_config_and_is_idempotent() {
        let root = TestDir::new("preserve");
        let spec = test_spec(&root);
        write_test_file(
            &spec.locations.codex_config,
            br#"# keep this comment
model = "gpt-test"

[mcp_servers.other]
command = "/other/server"
"#,
        );
        write_test_file(
            &spec.locations.claude_config,
            br#"{
  "projects": {"/keep/me": {"allowedTools": ["Read"]}},
  "mcpServers": {"other": {"type": "stdio", "command": "/other/server"}}
}
"#,
        );

        let status = enable_with_spec(&spec).expect("enable Petri MCP");
        assert!(status.is_fully_enabled());
        assert!(status.runtime_ready);

        let codex_text = fs::read_to_string(&spec.locations.codex_config).expect("Codex config");
        assert!(codex_text.contains("# keep this comment"));
        assert!(codex_text.contains("[mcp_servers.other]"));
        assert!(codex_text.contains("[mcp_servers.petri]"));
        assert!(codex_text.contains(MANAGED_BY_VALUE));

        let claude: JsonValue = serde_json::from_slice(
            &fs::read(&spec.locations.claude_config).expect("Claude config"),
        )
        .expect("parse Claude config");
        assert!(claude.pointer("/projects/~1keep~1me").is_some());
        assert!(claude.pointer("/mcpServers/other").is_some());
        assert_eq!(
            claude.pointer("/mcpServers/petri/env/PETRI_MCP_MANAGED_BY"),
            Some(&json!(MANAGED_BY_VALUE))
        );
        assert!(
            claude
                .pointer("/mcpServers/petri/env/PETRI_MCP_TRANSACTION_MODE")
                .is_none()
        );

        let codex_once = fs::read(&spec.locations.codex_config).expect("Codex once");
        let claude_once = fs::read(&spec.locations.claude_config).expect("Claude once");
        enable_with_spec(&spec).expect("idempotent enable");
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("Codex twice"),
            codex_once
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("Claude twice"),
            claude_once
        );
    }

    #[test]
    fn repair_reports_a_healthy_connection_without_rewriting_it() {
        let root = TestDir::new("repair healthy");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");
        let runtime_before = fs::read(&spec.locations.runtime_script).expect("runtime before");
        let codex_before = fs::read(&spec.locations.codex_config).expect("Codex before");
        let claude_before = fs::read(&spec.locations.claude_config).expect("Claude before");

        let result = repair_with_spec(&spec, false).expect("diagnose healthy connection");

        assert_eq!(result.outcome, McpRepairOutcome::AlreadyHealthy);
        assert!(result.status.is_fully_enabled());
        assert_eq!(
            fs::read(&spec.locations.runtime_script).expect("runtime after"),
            runtime_before
        );
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("Codex after"),
            codex_before
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("Claude after"),
            claude_before
        );
    }

    #[test]
    fn repair_rebuilds_an_owned_broken_connection_in_place() {
        let root = TestDir::new("repair owned");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");
        write_test_file(&spec.locations.runtime_script, b"outdated runtime");

        let broken = status_with_locations(&spec.locations).expect("broken status");
        assert!(broken.needs_repair());
        assert!(broken.has_enabled_managed_entry());

        let result = repair_with_spec(&spec, false).expect("repair owned connection");

        assert_eq!(result.outcome, McpRepairOutcome::Repaired);
        assert!(result.status.is_fully_enabled());
        assert!(result.status.runtime_ready);
        assert_eq!(
            fs::read(&spec.locations.runtime_script).expect("repaired runtime"),
            SERVER_SOURCE.as_bytes()
        );
    }

    #[test]
    fn repair_refuses_an_unowned_connection_without_writing_anything() {
        let root = TestDir::new("repair conflict");
        let spec = test_spec(&root);
        let codex_original = br#"[mcp_servers.petri]
command = "/someone/elses/server"
"#;
        let claude_original = br#"{"mcpServers":{"other":{"command":"keep"}}}"#;
        write_test_file(&spec.locations.codex_config, codex_original);
        write_test_file(&spec.locations.claude_config, claude_original);

        let error = repair_with_spec(&spec, false).expect_err("unowned repair must fail");

        assert!(error.to_string().contains("does not own"));
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("unchanged Codex"),
            codex_original
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged Claude"),
            claude_original
        );
        assert!(!spec.locations.runtime_script.exists());
    }

    #[test]
    fn retired_transaction_mode_requires_repair_and_is_removed() {
        let root = TestDir::new("retired transaction mode");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");

        let mut codex =
            parse_codex(&capture_file(&spec.locations.codex_config).expect("capture Codex config"))
                .expect("parse Codex config");
        codex_servers_mut_if_present(&mut codex)
            .expect("Codex servers")
            .expect("Codex server table")
            .get_mut(SERVER_NAME)
            .and_then(Item::as_table_mut)
            .and_then(|table| table.get_mut("env"))
            .and_then(Item::as_table_mut)
            .expect("Codex Petri env")
            .insert(RETIRED_TRANSACTION_MODE_ENV, value("multi_step"));
        write_test_file(&spec.locations.codex_config, codex.to_string().as_bytes());

        let claude = capture_file(&spec.locations.claude_config).expect("capture Claude config");
        let mut claude_doc = parse_claude(&claude).expect("parse Claude config");
        claude_doc
            .pointer_mut("/mcpServers/petri/env")
            .and_then(JsonValue::as_object_mut)
            .expect("Claude Petri env")
            .insert(
                RETIRED_TRANSACTION_MODE_ENV.to_string(),
                json!("multi_step"),
            );
        write_test_file(
            &spec.locations.claude_config,
            &render_json(&claude_doc).expect("render retired Claude config"),
        );

        let status = status_with_locations(&spec.locations).expect("read retired status");
        assert_eq!(status.codex, McpClientState::NeedsRepair);
        assert_eq!(status.claude, McpClientState::NeedsRepair);
        assert!(status.needs_repair());

        let repaired = repair_with_spec(&spec, false).expect("repair retired mode");
        assert_eq!(repaired.outcome, McpRepairOutcome::Repaired);
        assert!(repaired.status.is_fully_enabled());
        let codex_text = fs::read_to_string(&spec.locations.codex_config).expect("Codex repaired");
        let claude_text =
            fs::read_to_string(&spec.locations.claude_config).expect("Claude repaired");
        assert!(!codex_text.contains(RETIRED_TRANSACTION_MODE_ENV));
        assert!(!claude_text.contains(RETIRED_TRANSACTION_MODE_ENV));
    }

    #[test]
    fn disable_keeps_codex_registration_and_removes_only_owned_claude_entry() {
        let root = TestDir::new("disable");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");

        let status = disable_with_locations(&spec.locations).expect("disable Petri MCP");
        assert_eq!(status.codex, McpClientState::Disabled);
        assert_eq!(status.claude, McpClientState::Disabled);
        assert!(!status.is_fully_enabled());

        let codex =
            parse_codex(&capture_file(&spec.locations.codex_config).expect("capture Codex config"))
                .expect("parse Codex config");
        let petri = codex_entry(&codex)
            .expect("read Codex entry")
            .expect("Codex entry remains")
            .as_table()
            .expect("Codex Petri table");
        assert_eq!(petri.get("enabled").and_then(Item::as_bool), Some(false));

        let claude = parse_claude(
            &capture_file(&spec.locations.claude_config).expect("capture Claude config"),
        )
        .expect("parse Claude config");
        assert!(claude_entry(&claude).expect("read Claude entry").is_none());

        let enabled_again = enable_with_spec(&spec).expect("re-enable Petri MCP");
        assert!(enabled_again.is_fully_enabled());
    }

    #[test]
    fn disable_owned_entry_even_when_other_client_has_an_unowned_conflict() {
        let root = TestDir::new("disable with conflict");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");
        let unowned_claude =
            br#"{"mcpServers":{"petri":{"type":"stdio","command":"someone-else"}}}"#;
        write_test_file(&spec.locations.claude_config, unowned_claude);

        let status = disable_with_locations(&spec.locations).expect("disable owned Codex entry");

        assert_eq!(status.codex, McpClientState::Disabled);
        assert_eq!(status.claude, McpClientState::Conflict);
        assert!(!status.has_enabled_managed_entry());
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged unowned Claude entry"),
            unowned_claude
        );
    }

    #[cfg(unix)]
    #[test]
    fn enable_and_disable_preserve_symlinked_config_files() {
        use std::os::unix::fs::symlink;

        let root = TestDir::new("symlink configs");
        let mut spec = test_spec(&root);
        let target_dir = root.join("dotfiles");
        let link_dir = root.join("home");
        let codex_target = target_dir.join("codex.toml");
        let claude_target = target_dir.join("claude.json");
        write_test_file(&codex_target, b"model = \"keep-me\"\n");
        write_test_file(&claude_target, br#"{"keep":true}"#);
        fs::create_dir_all(&link_dir).expect("create symlink directory");
        spec.locations.codex_config = link_dir.join("config.toml");
        spec.locations.claude_config = link_dir.join(".claude.json");
        symlink(
            Path::new("../dotfiles/codex.toml"),
            &spec.locations.codex_config,
        )
        .expect("create Codex config symlink");
        symlink(
            Path::new("../dotfiles/claude.json"),
            &spec.locations.claude_config,
        )
        .expect("create Claude config symlink");

        enable_with_spec(&spec).expect("enable through config symlinks");
        assert!(
            fs::symlink_metadata(&spec.locations.codex_config)
                .expect("Codex symlink metadata")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(&spec.locations.claude_config)
                .expect("Claude symlink metadata")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(&codex_target)
                .expect("updated Codex target")
                .contains("[mcp_servers.petri]")
        );
        assert!(
            fs::read_to_string(&claude_target)
                .expect("updated Claude target")
                .contains("\"petri\"")
        );

        disable_with_locations(&spec.locations).expect("disable through config symlinks");
        assert!(
            fs::symlink_metadata(&spec.locations.codex_config)
                .expect("Codex symlink remains")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(&spec.locations.claude_config)
                .expect("Claude symlink remains")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn malformed_config_causes_no_writes() {
        let root = TestDir::new("malformed");
        let spec = test_spec(&root);
        let malformed = b"[mcp_servers.petri\ncommand =";
        let claude_original = br#"{"projects":{"keep":true}}"#;
        write_test_file(&spec.locations.codex_config, malformed);
        write_test_file(&spec.locations.claude_config, claude_original);

        let error = enable_with_spec(&spec).expect_err("malformed TOML should fail");
        assert!(
            error
                .to_string()
                .contains("Cannot parse Codex configuration")
        );
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("unchanged Codex"),
            malformed
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged Claude"),
            claude_original
        );
        assert!(!spec.locations.runtime_script.exists());
    }

    #[test]
    fn unowned_petri_entry_refuses_the_entire_enable() {
        let root = TestDir::new("conflict");
        let spec = test_spec(&root);
        let codex_original = br#"[mcp_servers.petri]
command = "/someone/elses/server"
"#;
        let claude_original = br#"{"mcpServers":{"other":{"command":"keep"}}}"#;
        write_test_file(&spec.locations.codex_config, codex_original);
        write_test_file(&spec.locations.claude_config, claude_original);

        let error = enable_with_spec(&spec).expect_err("unowned entry should conflict");
        assert!(error.to_string().contains("does not own"));
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("unchanged Codex"),
            codex_original
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged Claude"),
            claude_original
        );
        assert!(!spec.locations.runtime_script.exists());
    }

    #[test]
    fn paths_with_spaces_round_trip_through_both_formats() {
        let root = TestDir::new("path spaces");
        let spec = test_spec(&root);
        let status = enable_with_spec(&spec).expect("enable with spaced paths");
        assert!(status.is_fully_enabled());

        let codex =
            parse_codex(&capture_file(&spec.locations.codex_config).expect("capture Codex config"))
                .expect("parse Codex config");
        let codex_petri = codex_entry(&codex)
            .expect("Codex entry lookup")
            .expect("Codex Petri entry")
            .as_table()
            .expect("Codex Petri table");
        assert_eq!(
            codex_petri.get("command").and_then(Item::as_str),
            Some(path_text(&spec.node_bin).as_str())
        );
        assert_eq!(
            codex_petri
                .get("args")
                .and_then(Item::as_array)
                .and_then(|args| args.get(0))
                .and_then(|argument| argument.as_str()),
            Some(path_text(&spec.locations.runtime_script).as_str())
        );

        let claude: JsonValue = serde_json::from_slice(
            &fs::read(&spec.locations.claude_config).expect("Claude config"),
        )
        .expect("parse Claude config");
        assert_eq!(
            claude.pointer("/mcpServers/petri/command"),
            Some(&json!(path_text(&spec.node_bin)))
        );
        assert_eq!(
            claude.pointer("/mcpServers/petri/args/0"),
            Some(&json!(path_text(&spec.locations.runtime_script)))
        );
    }

    #[test]
    fn enabled_registration_with_missing_binary_needs_repair() {
        let root = TestDir::new("repair");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");
        fs::remove_file(&spec.node_bin).expect("remove Node test binary");

        let status = status_with_locations(&spec.locations).expect("read repair status");
        assert_eq!(status.codex, McpClientState::NeedsRepair);
        assert_eq!(status.claude, McpClientState::NeedsRepair);
        assert!(status.needs_repair());
    }

    #[test]
    fn protocol_preflight_requires_initialize_tools_and_manifest_results() {
        let valid = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"petri-mcp\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"petri.mcp_manifest\"}]}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"structuredContent\":{\"protocol\":{\"name\":\"petri-agent-protocol\"}}}}\n",
        );
        validate_preflight_output(valid.as_bytes()).expect("valid protocol preflight");

        let missing_tools = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"petri-mcp\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"structuredContent\":{\"protocol\":{\"name\":\"petri-agent-protocol\"}}}}\n",
        );
        assert!(validate_preflight_output(missing_tools.as_bytes()).is_err());
    }

    #[test]
    fn later_write_failure_rolls_back_an_earlier_file() {
        let root = TestDir::new("rollback");
        let first_path = root.join("first/config.toml");
        write_test_file(&first_path, b"before");
        let first = capture_file(&first_path).expect("capture first file");

        let blocked_parent = root.join("blocked-parent");
        write_test_file(&blocked_parent, b"not a directory");
        let blocked_path = blocked_parent.join("config.json");
        let blocked = FileSnapshot {
            path: blocked_path,
            contents: None,
            permissions: None,
        };

        let result = commit_prepared(&[
            PreparedFile {
                snapshot: first,
                desired: Some(b"after".to_vec()),
            },
            PreparedFile {
                snapshot: blocked,
                desired: Some(b"write must fail".to_vec()),
            },
        ]);
        assert!(result.is_err());
        assert_eq!(
            fs::read(first_path).expect("rolled-back first file"),
            b"before"
        );
    }
}
