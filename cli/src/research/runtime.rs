//! Immutable application code and a private bootstrap pipe. Research state stays
//! in its existing PetriResearch directory, outside replaceable application code.
use crate::{backend::CliError, petri_config};
use serde::Deserialize;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};
include!(concat!(env!("OUT_DIR"), "/research-bundle.rs"));

#[cfg(windows)]
fn isolate_caller_pipes() -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    // The detached Node service must not keep a shell/agent's output pipe open
    // after this command exits. Command still duplicates the explicit stdio it
    // needs; unrelated inherited pipes must not reach later descendants.
    for stream in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let handle = GetStdHandle(stream);
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && GetFileType(handle) == FILE_TYPE_PIPE
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0
            {
                return Err("Could not isolate the research process output.".into());
            }
        }
    }
    Ok(())
}

const AGENT_LAUNCHER: &[u8] = br#"import { readFileSync } from 'node:fs';
const { bundleId } = JSON.parse(readFileSync(new URL('./current.json', import.meta.url), 'utf8'));
if (!/^[a-f0-9]{64}$/.test(bundleId)) throw new Error('Open Petri to repair the research connection.');
await import(new URL(`./${bundleId}/bin/petri.mjs`, import.meta.url));
"#;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub base_url: String,
    pub owner_token: String,
}

/// How an agent's shell runs this petri, for the steps research hands it
/// (`work next`): `PETRI_COMMAND` (`petri-live petri` on a dev PC), else
/// this executable's own path, quoted for the shell.
fn petri_command() -> String {
    if let Ok(command) = std::env::var("PETRI_COMMAND")
        && !command.trim().is_empty()
    {
        return command;
    }
    let path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "petri".into());
    if cfg!(windows) {
        format!("& '{}'", path.replace('\'', "''"))
    } else {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }
}

pub struct Runtime {
    node: PathBuf,
    directory: PathBuf,
    /// The futures API Petri uses for markets (`Settings::api_url`: `--api`,
    /// `PETRI_API_URL`, the saved `api`, else the profile's).
    api: String,
}

impl Runtime {
    pub fn load(api: &str) -> Result<Self, String> {
        // The research workspace runs on Node 24: `PETRI_RESEARCH_NODE`, else
        // `node` on PATH.
        let node = std::env::var_os("PETRI_RESEARCH_NODE")
            .map(PathBuf::from)
            .unwrap_or_else(|| "node".into());
        let config = petri_config::config_path()?;
        let directory = config
            .parent()
            .ok_or("Petri storage is unavailable")?
            .join("research-runtime")
            .join(BUNDLE_ID);
        let lock = petri_config::open_private_lock_file(&directory.join("extract.lock"))?;
        lock.lock()
            .map_err(|_| "Could not lock the research runtime")?;
        for (relative, bytes) in FILES {
            let path = directory.join(relative);
            if let Ok(metadata) = fs::symlink_metadata(&path) {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("Research runtime contains an unexpected file type".into());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err("Research runtime contains a reparse point".into());
                    }
                }
                if fs::read(&path).map_err(|_| "Could not read research runtime")? != *bytes {
                    return Err("Research runtime differs from the packaged application. Clear only this research-runtime version and retry.".into());
                }
            } else {
                petri_config::write_private_file(&path, bytes)?;
            }
        }
        let root = directory.parent().ok_or("Research runtime has no parent")?;
        petri_config::write_private_file(&root.join("agent.mjs"), AGENT_LAUNCHER)?;
        petri_config::write_private_file(
            &root.join("current.json"),
            serde_json::json!({ "bundleId": BUNDLE_ID })
                .to_string()
                .as_bytes(),
        )?;
        Ok(Self {
            node,
            directory,
            api: api.to_string(),
        })
    }

    pub fn agent_command(&self) -> (String, String) {
        // Agents run on the bundled Node, so their long-lived MCP connections do
        // not lock petri.exe during an update. New sessions resolve current code;
        // existing sessions finish with their already-loaded protocol client.
        (
            self.node.to_string_lossy().into(),
            self.directory
                .parent()
                .unwrap()
                .join("agent.mjs")
                .to_string_lossy()
                .into(),
        )
    }

    fn process(&self, entry: &str) -> Result<Command, String> {
        #[cfg(windows)]
        isolate_caller_pipes()?;
        let mut command = Command::new(&self.node);
        // Candidates and oracle reads both go to the futures API Petri uses
        // for markets, unless the caller pins them explicitly.
        for key in ["PETRI_RESEARCH_UPSTREAM", "PETRI_RESEARCH_ORACLE_URL"] {
            if std::env::var_os(key).is_none() {
                command.env(key, &self.api);
            }
        }
        command
            .arg(self.directory.join("bin").join(entry))
            .env("PETRI_RESEARCH_RUNTIME", BUNDLE_ID)
            .env("PETRI_COMMAND", petri_command())
            .env_remove("PETRI_RESEARCH_OWNER_TOKEN")
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        Ok(command)
    }

    pub fn connect(&self) -> Result<Connection, String> {
        let result = self
            .process("desktop.mjs")?
            .stdin(Stdio::null())
            .output()
            .map_err(
                |_| "Could not start the research workspace. Source builds require Node 24.",
            )?;
        if !result.status.success() {
            return Err(super::model::safe(&String::from_utf8_lossy(&result.stderr)));
        }
        // Bootstrap credentials are read only from this child pipe, never argv,
        // the terminal, a log, or an environment shared with other children.
        serde_json::from_slice(&result.stdout)
            .map_err(|_| "Invalid research workspace bootstrap response".into())
    }

    pub fn command(&self, args: &[String]) -> Result<(), CliError> {
        let status = self
            .process("petri.mjs")
            .map_err(CliError::new)?
            .args(args)
            .status()
            .map_err(|_| {
                CliError::new("Could not start research commands. Source builds require Node 24.")
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(CliError::new("Research command did not complete."))
        }
    }
}
