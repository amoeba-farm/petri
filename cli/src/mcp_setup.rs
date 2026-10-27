//! Petri connects AI agents (Claude Code and Codex) to Amoeba's hosted MCP
//! server, the futures API's `/mcp` (api/API.md, "MCP"): registered by URL
//! as the `petri` server in each agent's user configuration. Petri writes
//! only its own entry: one it set up (a URL entry for an Amoeba API's `/mcp`,
//! or the local Node server's it set up before, marked in its env, which is
//! replaced along with that server's files). Anyone else's `petri` entry is
//! left alone, and nothing is written when any file would not parse.
use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, fs,
    fs::{OpenOptions, Permissions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map as JsonMap, Value as JsonValue, json};
use toml_edit::{DocumentMut, Item, Table, value};

const SERVER_NAME: &str = "petri";
/// What marked the entries of the local Node server petri set up before
/// the hosted one (until 2026-10-26).
const LEGACY_MANAGED_BY_ENV: &str = "PETRI_MCP_MANAGED_BY";
const LEGACY_MANAGED_BY_VALUE: &str = "petri-tui-v1";
/// That server's files, removed when its entries are replaced.
const LEGACY_RUNTIME_FILES: [&str; 2] = ["petri-mcp-server.mjs", "petri-action-bridge.mjs"];

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
    /// Where the local Node server's files were (`LEGACY_RUNTIME_FILES`).
    legacy_runtime: PathBuf,
}

/// Where the entries go and the server they name.
#[derive(Clone, Debug)]
struct Spec {
    locations: SetupLocations,
    url: String,
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

/// The hosted MCP server of the futures API at `api`.
pub(crate) fn server_url(api: &str) -> String {
    format!("{}/mcp", api.trim_end_matches('/'))
}

fn spec(api: &str) -> Result<Spec, McpSetupError> {
    Ok(Spec {
        locations: resolve_locations()?,
        url: server_url(api),
    })
}

pub(crate) fn status(api: &str) -> Result<McpSetupStatus, McpSetupError> {
    status_with_spec(&spec(api)?)
}

pub(crate) fn enable(api: &str) -> Result<McpSetupStatus, McpSetupError> {
    enable_with_spec(&spec(api)?)
}

pub(crate) fn repair(api: &str) -> Result<McpRepairResult, McpSetupError> {
    repair_with_spec(&spec(api)?)
}

pub(crate) fn disable(api: &str) -> Result<McpSetupStatus, McpSetupError> {
    disable_with_spec(&spec(api)?)
}

fn enable_with_spec(spec: &Spec) -> Result<McpSetupStatus, McpSetupError> {
    let codex = capture_file(&spec.locations.codex_config)?;
    let claude = capture_file(&spec.locations.claude_config)?;

    let mut codex_doc = parse_codex(&codex)?;
    ensure_codex_entry_is_available(&codex_doc, &spec.url)?;
    set_codex_enabled(&mut codex_doc, &spec.url)?;

    let mut claude_doc = parse_claude(&claude)?;
    ensure_claude_entry_is_available(&claude_doc, &spec.url)?;
    set_claude_enabled(&mut claude_doc, &spec.url)?;

    let mut prepared = vec![
        PreparedFile {
            snapshot: codex,
            desired: Some(codex_doc.to_string().into_bytes()),
        },
        PreparedFile {
            snapshot: claude,
            desired: Some(render_json(&claude_doc)?),
        },
    ];
    // The local server's files go with its entries.
    for name in LEGACY_RUNTIME_FILES {
        let snapshot = capture_file(&spec.locations.legacy_runtime.join(name))?;
        if snapshot.contents.is_some() {
            prepared.push(PreparedFile {
                snapshot,
                desired: None,
            });
        }
    }
    commit_prepared(&prepared)?;
    status_with_spec(spec)
}

/// Moves agents off the local Node server petri set up before v0.1.18 onto
/// the hosted one, so updating petri doesn't leave them without it (that
/// server's commands are gone). Only petri's own old entries that are switched
/// on change, each agent keeps whether it had one, and the old server's files
/// go once no entry names them. The TUI runs it at start; true when it moved one.
pub(crate) fn migrate_legacy(api: &str) -> Result<bool, McpSetupError> {
    migrate_legacy_with_spec(&spec(api)?)
}

fn migrate_legacy_with_spec(spec: &Spec) -> Result<bool, McpSetupError> {
    let codex = capture_file(&spec.locations.codex_config)?;
    let claude = capture_file(&spec.locations.claude_config)?;
    let mut codex_doc = parse_codex(&codex)?;
    let mut claude_doc = parse_claude(&claude)?;
    let codex_entry = codex_entry(&codex_doc)?.filter(|item| codex_entry_legacy(item));
    let claude_entry = claude_entry(&claude_doc)?.filter(|entry| claude_entry_legacy(entry));
    let codex_on =
        codex_entry.is_some_and(|item| item.get("enabled").and_then(Item::as_bool) != Some(false));
    let claude_on = claude_entry.is_some();
    if !codex_on && !claude_on {
        return Ok(false);
    }
    let leftover = codex_entry.is_some() && !codex_on;
    let mut prepared = Vec::new();
    if codex_on {
        set_codex_enabled(&mut codex_doc, &spec.url)?;
        prepared.push(PreparedFile {
            snapshot: codex,
            desired: Some(codex_doc.to_string().into_bytes()),
        });
    }
    if claude_on {
        set_claude_enabled(&mut claude_doc, &spec.url)?;
        prepared.push(PreparedFile {
            snapshot: claude,
            desired: Some(render_json(&claude_doc)?),
        });
    }
    if !leftover {
        for name in LEGACY_RUNTIME_FILES {
            let snapshot = capture_file(&spec.locations.legacy_runtime.join(name))?;
            if snapshot.contents.is_some() {
                prepared.push(PreparedFile {
                    snapshot,
                    desired: None,
                });
            }
        }
    }
    commit_prepared(&prepared)?;
    Ok(true)
}

fn repair_with_spec(spec: &Spec) -> Result<McpRepairResult, McpSetupError> {
    let current = status_with_spec(spec)?;
    if current.is_fully_enabled() {
        return Ok(McpRepairResult {
            status: current,
            outcome: McpRepairOutcome::AlreadyHealthy,
        });
    }
    ensure_repairable(&current)?;
    let status = enable_with_spec(spec)?;
    if !status.is_fully_enabled() {
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
            "The Petri connection is not enabled. Run petri mcp enable to connect it.",
        ));
    }
    Ok(())
}

fn disable_with_spec(spec: &Spec) -> Result<McpSetupStatus, McpSetupError> {
    let codex = capture_file(&spec.locations.codex_config)?;
    let claude = capture_file(&spec.locations.claude_config)?;

    let mut codex_doc = parse_codex(&codex)?;
    let codex_changed = set_codex_disabled(&mut codex_doc, &spec.url)?;

    let mut claude_doc = parse_claude(&claude)?;
    let claude_changed = remove_claude_entry(&mut claude_doc, &spec.url)?;

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
    status_with_spec(spec)
}

fn status_with_spec(spec: &Spec) -> Result<McpSetupStatus, McpSetupError> {
    let codex = capture_file(&spec.locations.codex_config)?;
    let claude = capture_file(&spec.locations.claude_config)?;
    let codex_doc = parse_codex(&codex)?;
    let claude_doc = parse_claude(&claude)?;
    Ok(McpSetupStatus {
        codex: codex_state(&codex_doc, &spec.url)?,
        claude: claude_state(&claude_doc, &spec.url)?,
    })
}

fn codex_state(document: &DocumentMut, url: &str) -> Result<McpClientState, McpSetupError> {
    let Some(item) = codex_entry(document)? else {
        return Ok(McpClientState::Disabled);
    };
    if !codex_entry_owned(item, url) {
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
    // The hosted server for this petri's API, with nothing of a local one.
    let healthy = table.get("url").and_then(Item::as_str) == Some(url)
        && !table.contains_key("command")
        && !table.contains_key("env")
        && table
            .get("default_tools_approval_mode")
            .and_then(Item::as_str)
            == Some("writes");
    Ok(if healthy {
        McpClientState::Enabled
    } else {
        McpClientState::NeedsRepair
    })
}

fn claude_state(document: &JsonValue, url: &str) -> Result<McpClientState, McpSetupError> {
    let Some(entry) = claude_entry(document)? else {
        return Ok(McpClientState::Disabled);
    };
    if !claude_entry_owned(entry, url) {
        return Ok(McpClientState::Conflict);
    }
    let healthy = entry.get("type").and_then(JsonValue::as_str) == Some("http")
        && entry.get("url").and_then(JsonValue::as_str) == Some(url)
        && entry.get("command").is_none()
        && entry.get("env").is_none();
    Ok(if healthy {
        McpClientState::Enabled
    } else {
        McpClientState::NeedsRepair
    })
}

fn set_codex_enabled(document: &mut DocumentMut, url: &str) -> Result<(), McpSetupError> {
    let servers = codex_servers_mut(document)?;
    let mut server = Table::new();
    server.insert("url", value(url));
    server.insert("enabled", value(true));
    server.insert("required", value(false));
    // Codex asks before a tool that is not read-only (the relays).
    server.insert("default_tools_approval_mode", value("writes"));
    servers.insert(SERVER_NAME, Item::Table(server));
    Ok(())
}

fn set_codex_disabled(document: &mut DocumentMut, url: &str) -> Result<bool, McpSetupError> {
    if codex_entry(document)?.is_some_and(|entry| !codex_entry_owned(entry, url)) {
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

fn set_claude_enabled(document: &mut JsonValue, url: &str) -> Result<(), McpSetupError> {
    let servers = claude_servers_mut(document)?;
    servers.insert(
        SERVER_NAME.to_string(),
        json!({ "type": "http", "url": url }),
    );
    Ok(())
}

fn remove_claude_entry(document: &mut JsonValue, url: &str) -> Result<bool, McpSetupError> {
    if claude_entry(document)?.is_some_and(|entry| !claude_entry_owned(entry, url)) {
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

fn ensure_codex_entry_is_available(document: &DocumentMut, url: &str) -> Result<(), McpSetupError> {
    if codex_entry(document)?.is_some_and(|item| !codex_entry_owned(item, url)) {
        return Err(McpSetupError::new(
            "Codex already has a Petri MCP entry that this TUI does not own. It was left unchanged.",
        ));
    }
    Ok(())
}

fn ensure_claude_entry_is_available(document: &JsonValue, url: &str) -> Result<(), McpSetupError> {
    if claude_entry(document)?.is_some_and(|entry| !claude_entry_owned(entry, url)) {
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

/// The local Node server's entry, marked in its env.
fn codex_entry_legacy(item: &Item) -> bool {
    item.as_table()
        .and_then(|table| table.get("env"))
        .and_then(Item::as_table)
        .and_then(|server_env| server_env.get(LEGACY_MANAGED_BY_ENV))
        .and_then(Item::as_str)
        == Some(LEGACY_MANAGED_BY_VALUE)
}

fn codex_entry_owned(item: &Item, url: &str) -> bool {
    let Some(table) = item.as_table() else {
        return false;
    };
    codex_entry_legacy(item)
        || table
            .get("url")
            .and_then(Item::as_str)
            .is_some_and(|entry| amoeba_url(entry, url))
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

fn claude_entry_legacy(entry: &JsonValue) -> bool {
    entry
        .get("env")
        .and_then(JsonValue::as_object)
        .and_then(|server_env| server_env.get(LEGACY_MANAGED_BY_ENV))
        .and_then(JsonValue::as_str)
        == Some(LEGACY_MANAGED_BY_VALUE)
}

fn claude_entry_owned(entry: &JsonValue, url: &str) -> bool {
    claude_entry_legacy(entry)
        || entry
            .get("url")
            .and_then(JsonValue::as_str)
            .is_some_and(|entry| amoeba_url(entry, url))
}

/// Whether `entry` is an Amoeba futures API's MCP server: `url` (this
/// petri's), the production API's, or a loopback API's `/mcp` (a dev one).
fn amoeba_url(entry: &str, url: &str) -> bool {
    if entry == url {
        return true;
    }
    let Ok(parsed) = reqwest::Url::parse(entry) else {
        return false;
    };
    parsed.path() == "/mcp"
        && match parsed.host_str() {
            Some("api.amoeba.farm") => parsed.scheme() == "https",
            Some("127.0.0.1" | "localhost" | "[::1]") => true,
            _ => false,
        }
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
    // Every folder it makes is private: the runtime's parent is Petri's own
    // config folder, which Petri refuses once others can read it.
    crate::petri_config::create_private_dir_all(path)
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
        legacy_runtime: absolute_path(runtime_root.join("mcp"))?,
    })
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

    const URL: &str = "https://api.amoeba.farm/mcp";

    fn test_spec(root: &TestDir) -> Spec {
        Spec {
            locations: SetupLocations {
                codex_config: root.join("codex home/config.toml"),
                claude_config: root.join("claude home/.claude.json"),
                legacy_runtime: root.join("Petri MCP Runtime"),
            },
            url: URL.into(),
        }
    }

    fn write_test_file(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().expect("file parent")).expect("create file parent");
        fs::write(path, contents).expect("write test file");
    }

    fn claude(spec: &Spec) -> JsonValue {
        serde_json::from_slice(&fs::read(&spec.locations.claude_config).expect("Claude config"))
            .expect("parse Claude config")
    }

    fn codex_petri(spec: &Spec) -> Table {
        let codex =
            parse_codex(&capture_file(&spec.locations.codex_config).expect("capture Codex"))
                .expect("parse Codex config");
        codex_entry(&codex)
            .expect("Codex entry lookup")
            .expect("Codex Petri entry")
            .as_table()
            .expect("Codex Petri table")
            .clone()
    }

    #[test]
    fn the_hosted_server_is_the_api_s_mcp_path() {
        assert_eq!(server_url("https://api.amoeba.farm"), URL);
        assert_eq!(
            server_url("http://127.0.0.1:4500/"),
            "http://127.0.0.1:4500/mcp"
        );
    }

    #[test]
    fn enable_registers_the_hosted_server_by_url_preserves_other_config_and_is_idempotent() {
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

        let codex_text = fs::read_to_string(&spec.locations.codex_config).expect("Codex config");
        assert!(codex_text.contains("# keep this comment"));
        assert!(codex_text.contains("[mcp_servers.other]"));
        let petri = codex_petri(&spec);
        assert_eq!(petri.get("url").and_then(Item::as_str), Some(URL));
        assert_eq!(
            petri
                .get("default_tools_approval_mode")
                .and_then(Item::as_str),
            Some("writes")
        );
        assert!(petri.get("command").is_none());

        let claude = claude(&spec);
        assert!(claude.pointer("/projects/~1keep~1me").is_some());
        assert!(claude.pointer("/mcpServers/other").is_some());
        assert_eq!(
            claude.pointer("/mcpServers/petri"),
            Some(&json!({ "type": "http", "url": URL }))
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
        let codex_before = fs::read(&spec.locations.codex_config).expect("Codex before");
        let claude_before = fs::read(&spec.locations.claude_config).expect("Claude before");

        let result = repair_with_spec(&spec).expect("diagnose healthy connection");

        assert_eq!(result.outcome, McpRepairOutcome::AlreadyHealthy);
        assert!(result.status.is_fully_enabled());
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
    fn the_local_node_servers_entries_are_replaced_by_the_hosted_one_and_its_files_removed() {
        let root = TestDir::new("legacy");
        let spec = test_spec(&root);
        let script = spec.locations.legacy_runtime.join("petri-mcp-server.mjs");
        let bridge = spec
            .locations
            .legacy_runtime
            .join("petri-action-bridge.mjs");
        write_test_file(&script, b"// the old server");
        write_test_file(&bridge, b"// its bridge");
        let unrelated = spec.locations.legacy_runtime.join("notes.txt");
        write_test_file(&unrelated, b"keep");
        write_test_file(
            &spec.locations.codex_config,
            format!(
                "[mcp_servers.petri]\ncommand = \"/usr/bin/node\"\nargs = [\"{}\"]\nenabled = true\ndefault_tools_approval_mode = \"writes\"\n\n[mcp_servers.petri.env]\nPETRI_MCP_PETRI_BIN = \"/opt/petri\"\nPETRI_MCP_MANAGED_BY = \"petri-tui-v1\"\n",
                script.display()
            )
            .as_bytes(),
        );
        write_test_file(
            &spec.locations.claude_config,
            serde_json::to_vec(&json!({ "mcpServers": { "petri": { "type": "stdio", "command": "/usr/bin/node",
                "args": [script.display().to_string()], "env": { "PETRI_MCP_PETRI_BIN": "/opt/petri", "PETRI_MCP_MANAGED_BY": "petri-tui-v1" } },
                "other": { "command": "keep" } } }))
            .unwrap()
            .as_slice(),
        );

        let before = status_with_spec(&spec).expect("legacy status");
        assert_eq!(
            (before.codex, before.claude),
            (McpClientState::NeedsRepair, McpClientState::NeedsRepair)
        );
        assert!(before.has_enabled_managed_entry());

        let repaired = repair_with_spec(&spec).expect("repair the legacy connection");
        assert_eq!(repaired.outcome, McpRepairOutcome::Repaired);
        assert!(repaired.status.is_fully_enabled());
        let petri = codex_petri(&spec);
        assert_eq!(petri.get("url").and_then(Item::as_str), Some(URL));
        assert!(petri.get("command").is_none() && petri.get("env").is_none());
        let claude = claude(&spec);
        assert_eq!(
            claude.pointer("/mcpServers/petri"),
            Some(&json!({ "type": "http", "url": URL }))
        );
        assert_eq!(
            claude.pointer("/mcpServers/other/command"),
            Some(&json!("keep"))
        );
        assert!(!script.exists() && !bridge.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn an_update_moves_only_the_old_node_entries_and_leaves_everything_else() {
        let root = TestDir::new("migrate");
        let spec = test_spec(&root);
        let script = spec.locations.legacy_runtime.join("petri-mcp-server.mjs");
        write_test_file(&script, b"// the old server");
        // Claude Code had the old server; Codex had nothing.
        write_test_file(
            &spec.locations.claude_config,
            serde_json::to_vec(&json!({ "mcpServers": { "petri": { "type": "stdio", "command": "/usr/bin/node",
                "args": [script.display().to_string()], "env": { "PETRI_MCP_MANAGED_BY": "petri-tui-v1" } },
                "other": { "command": "keep" } } }))
            .unwrap()
            .as_slice(),
        );
        assert!(migrate_legacy_with_spec(&spec).expect("migrate"));
        let claude = claude(&spec);
        assert_eq!(
            claude.pointer("/mcpServers/petri"),
            Some(&json!({ "type": "http", "url": URL }))
        );
        assert_eq!(
            claude.pointer("/mcpServers/other/command"),
            Some(&json!("keep"))
        );
        assert!(
            !spec.locations.codex_config.exists(),
            "no Codex entry was made"
        );
        assert!(!script.exists());
        // Run again: nothing old is left, nothing changes.
        let before = fs::read(&spec.locations.claude_config).unwrap();
        assert!(!migrate_legacy_with_spec(&spec).expect("again"));
        assert_eq!(fs::read(&spec.locations.claude_config).unwrap(), before);

        // A hosted entry for a dev API and someone else's entry stay as they are.
        let other = TestDir::new("migrate other");
        let spec = test_spec(&other);
        let dev = b"[mcp_servers.petri]\nurl = \"http://127.0.0.1:4500/mcp\"\nenabled = true\n";
        write_test_file(&spec.locations.codex_config, dev);
        write_test_file(
            &spec.locations.claude_config,
            serde_json::to_vec(
                &json!({ "mcpServers": { "petri": { "command": "someone-elses" } } }),
            )
            .unwrap()
            .as_slice(),
        );
        assert!(!migrate_legacy_with_spec(&spec).expect("nothing to move"));
        assert_eq!(fs::read(&spec.locations.codex_config).unwrap(), dev);
    }

    #[test]
    fn an_entry_for_another_amoeba_api_is_owned_and_repaired_to_this_one() {
        let root = TestDir::new("other api");
        let spec = test_spec(&root);
        let dev = Spec {
            url: "http://127.0.0.1:4500/mcp".into(),
            ..test_spec(&root)
        };
        enable_with_spec(&dev).expect("enable for the dev API");
        assert!(
            status_with_spec(&dev)
                .expect("dev status")
                .is_fully_enabled()
        );

        let status = status_with_spec(&spec).expect("status for production");
        assert_eq!(
            (status.codex, status.claude),
            (McpClientState::NeedsRepair, McpClientState::NeedsRepair)
        );
        let repaired = repair_with_spec(&spec).expect("repair to production");
        assert!(repaired.status.is_fully_enabled());
        assert_eq!(
            claude(&spec).pointer("/mcpServers/petri/url"),
            Some(&json!(URL))
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

        let error = repair_with_spec(&spec).expect_err("unowned repair must fail");

        assert!(error.to_string().contains("does not own"));
        assert_eq!(
            fs::read(&spec.locations.codex_config).expect("unchanged Codex"),
            codex_original
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged Claude"),
            claude_original
        );
    }

    #[test]
    fn someone_elses_url_entry_is_theirs() {
        let root = TestDir::new("their url");
        let spec = test_spec(&root);
        let claude_original =
            br#"{"mcpServers":{"petri":{"type":"http","url":"https://petri.example/mcp"}}}"#;
        write_test_file(&spec.locations.claude_config, claude_original);
        assert_eq!(
            status_with_spec(&spec).expect("status").claude,
            McpClientState::Conflict
        );
        assert!(
            enable_with_spec(&spec)
                .expect_err("their entry")
                .to_string()
                .contains("does not own")
        );
        assert_eq!(
            fs::read(&spec.locations.claude_config).expect("unchanged"),
            claude_original
        );
        for (url, owned) in [
            (URL, true),
            ("https://api.amoeba.farm/mcp", true),
            ("http://api.amoeba.farm/mcp", false),
            ("http://localhost:4500/mcp", true),
            ("https://api.amoeba.farm/other", false),
            ("https://api.amoeba.farm.example/mcp", false),
        ] {
            assert_eq!(
                amoeba_url(url, "https://staging.example/mcp"),
                owned,
                "{url}"
            );
        }
        assert!(amoeba_url(
            "https://staging.example/mcp",
            "https://staging.example/mcp"
        ));
    }

    #[test]
    fn disable_keeps_codex_registration_and_removes_only_owned_claude_entry() {
        let root = TestDir::new("disable");
        let spec = test_spec(&root);
        enable_with_spec(&spec).expect("enable Petri MCP");

        let status = disable_with_spec(&spec).expect("disable Petri MCP");
        assert_eq!(status.codex, McpClientState::Disabled);
        assert_eq!(status.claude, McpClientState::Disabled);
        assert!(!status.is_fully_enabled());
        assert_eq!(
            codex_petri(&spec).get("enabled").and_then(Item::as_bool),
            Some(false)
        );
        let claude =
            parse_claude(&capture_file(&spec.locations.claude_config).expect("capture Claude"))
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

        let status = disable_with_spec(&spec).expect("disable owned Codex entry");

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
        .expect("Codex symlink");
        symlink(
            Path::new("../dotfiles/claude.json"),
            &spec.locations.claude_config,
        )
        .expect("Claude symlink");

        enable_with_spec(&spec).expect("enable through config symlinks");
        for link in [&spec.locations.codex_config, &spec.locations.claude_config] {
            assert!(
                fs::symlink_metadata(link)
                    .expect("symlink metadata")
                    .file_type()
                    .is_symlink()
            );
        }
        assert!(
            fs::read_to_string(&codex_target)
                .expect("Codex target")
                .contains("[mcp_servers.petri]")
        );
        assert!(
            fs::read_to_string(&claude_target)
                .expect("Claude target")
                .contains("\"petri\"")
        );

        disable_with_spec(&spec).expect("disable through config symlinks");
        for link in [&spec.locations.codex_config, &spec.locations.claude_config] {
            assert!(
                fs::symlink_metadata(link)
                    .expect("symlink remains")
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[test]
    fn malformed_config_causes_no_writes() {
        let root = TestDir::new("malformed");
        let spec = test_spec(&root);
        let malformed = b"[mcp_servers.petri\ncommand =";
        let claude_original = br#"{"projects":{"keep":true}}"#;
        write_test_file(&spec.locations.codex_config, malformed);
        write_test_file(&spec.locations.claude_config, claude_original);
        let script = spec.locations.legacy_runtime.join("petri-mcp-server.mjs");
        write_test_file(&script, b"// the old server");

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
        assert!(script.exists());
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
