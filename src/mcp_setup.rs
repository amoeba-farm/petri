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
    smoke_file
        .write_all(SERVER_SOURCE.as_bytes())
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
    let mut combined = OsString::from(drive);
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
