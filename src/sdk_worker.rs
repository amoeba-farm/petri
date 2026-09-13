//! Pinned SDK runtime transport. No secrets in argv/environment, no signing in JS.
use crate::content_hash::sha256_hex as hash;
use crate::{backend::CliError, petri_config};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/petri-sdk-runtime.bin"));
include!(concat!(env!("OUT_DIR"), "/sdk-script-digests.rs"));
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[path = "sdk_archive.rs"]
mod archive;
// Only immutable embedded bytes are memoized. Every use below still verifies
// the files in the mutable extracted runtime under its private lock.
static RUNTIME_ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash(BUNDLE));
pub fn warmup() -> Result<(), CliError> {
    runtime().map(|_| ())
}
fn unavailable(message: impl Into<String>) -> CliError {
    CliError::coded("SDK_RUNTIME_UNAVAILABLE", "unavailable", message, false)
}
fn runtime() -> Result<PathBuf, CliError> {
    if BUNDLE.len() < 8 {
        return Err(unavailable(
            "This source build needs its SDK runtime. Build scripts/build-sdk-runtime.mjs, then rebuild Petri. Existing native trading is unaffected.",
        ));
    }
    let archive = archive::Archive::parse(BUNDLE)?;
    let manifest = &archive.manifest;
    if manifest["candidate"] == true
        || manifest["sdkCommit"] != crate::current_release::SDK_PACKAGE_COMMIT
    {
        return Err(unavailable("SDK archive identity differs from Petri"));
    }
    let platform = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        std::env::consts::ARCH
    };
    if manifest["platform"] != platform || manifest["arch"] != arch {
        return Err(unavailable(
            "SDK runtime does not match this operating system and architecture",
        ));
    }
    let config = petri_config::config_path().map_err(CliError::new)?;
    let directory = config
        .parent()
        .ok_or_else(|| unavailable("Petri storage is unavailable"))?
        .join("sdk-runtime")
        .join(RUNTIME_ID.as_str());
    let lock = petri_config::open_private_lock_file(&directory.join("extract.lock"))
        .map_err(CliError::new)?;
    lock.try_lock().map_err(|_| {
        unavailable("Another Petri context is preparing the SDK runtime. Retry after it finishes.")
    })?;
    let entries = manifest["files"]
        .as_array()
        .filter(|v| v.len() <= 40000)
        .ok_or_else(|| unavailable("SDK archive file inventory invalid"))?;
    let mut worker = false;
    let mut paths = std::collections::HashSet::new();
    let mut expanded_bytes = 0u64;
    let mut decoded_block: Option<(usize, Vec<u8>)> = None;
    for (block, member) in archive
        .blocks
        .iter()
        .flat_map(|block| block.files.iter().map(move |member| (block, member)))
    {
        let entry = &entries[member.index];
        let name = entry["path"]
            .as_str()
            .ok_or_else(|| unavailable("SDK archive path invalid"))?;
        if !paths.insert(name)
            || name.contains('\\')
            || name.contains(':')
            || name.contains('\0')
            || name
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(unavailable("SDK archive path escapes its directory"));
        }
        let path = directory.join(name);
        let expected = entry["sha256"]
            .as_str()
            .ok_or_else(|| unavailable("SDK file digest missing"))?;
        if name == "worker.mjs" {
            worker = expected == EXPECTED_SDK_SCRIPTS[0].1;
            if !worker {
                return Err(unavailable(
                    "SDK worker changed; rebuild the SDK runtime before rebuilding Petri",
                ));
            }
        }
        for &(companion, digest) in &EXPECTED_SDK_SCRIPTS[1..] {
            if name == companion && expected != digest {
                return Err(unavailable(
                    "SDK companion changed; rebuild the private runtime",
                ));
            }
        }
        let size = entry["bytes"]
            .as_u64()
            .filter(|n| *n <= 128 * 1024 * 1024)
            .ok_or_else(|| unavailable("SDK file size invalid"))?;
        expanded_bytes = expanded_bytes
            .checked_add(size)
            .filter(|total| *total <= 2 * 1024 * 1024 * 1024)
            .ok_or_else(|| unavailable("SDK archive expanded size invalid"))?;
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(unavailable("SDK runtime contains an unexpected file type"));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(unavailable("SDK runtime contains a reparse point"));
                }
            }
            if metadata.len() == size
                && hash(&fs::read(&path).map_err(|_| unavailable("Could not read SDK runtime"))?)
                    == expected
            {
                continue;
            }
            return Err(unavailable(
                "SDK runtime cache differs from the packaged bytes. Clear only this SDK runtime cache version, then retry.",
            ));
        }
        if decoded_block
            .as_ref()
            .is_none_or(|(offset, _)| *offset != block.offset)
        {
            // Release the previous block before allocating the next one. A
            // verified mutable cache hit above never decompresses a block.
            drop(decoded_block.take());
            decoded_block = Some((block.offset, archive.decode(block)?));
        }
        let decoded =
            &decoded_block.as_ref().unwrap().1[member.offset..member.offset + size as usize];
        if decoded.len() as u64 != size || hash(decoded) != expected {
            return Err(unavailable("SDK archive content digest mismatch"));
        }
        petri_config::write_private_file(&path, decoded).map_err(CliError::new)?;
        #[cfg(unix)]
        if name == "node" || name == "compressed-verifier" {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|_| unavailable("SDK executable permissions failed"))?;
        }
    }
    if !worker {
        return Err(unavailable("SDK worker missing"));
    }
    Ok(directory)
}

pub fn validate(
    config: &crate::onchain::OnchainConfig,
    family: &str,
    owner: &str,
    request: &Value,
    prepared: &Value,
    expected_accounts: Option<&Value>,
) -> Result<Value, CliError> {
    crate::chain_identity::verify_onchain_config_fresh(config)?;
    let directory = runtime()?;
    let input = json!({"schemaVersion":1,"family":family,"owner":owner,"request":request,"prepared":prepared,"backend":config.backend_url,"expectedAccounts":expected_accounts});
    let bytes =
        serde_json::to_vec(&input).map_err(|_| unavailable("SDK request encoding failed"))?;
    if bytes.len() > MAX_BYTES {
        return Err(CliError::new("SDK operation exceeds its size bound"));
    }
    let mut command = Command::new(directory.join(if cfg!(windows) { "node.exe" } else { "node" }));
    command
        .arg(directory.join("worker.mjs"))
        .current_dir(&directory)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| unavailable("Could not start the packaged SDK runtime"))?;
    let mut input_pipe = child.stdin.take().unwrap();
    let writer = thread::spawn(move || {
        let result = input_pipe.write_all(&bytes);
        drop(input_pipe);
        result
    });
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let reader = thread::spawn(move || {
        let mut result = Vec::new();
        out.take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut result)
            .map(|_| result)
    });
    let errors = thread::spawn(move || {
        let _ = std::io::copy(&mut err.take(64 * 1024), &mut std::io::sink());
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable("SDK runtime status unavailable"));
            }
        }
        if started.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unavailable(
                "SDK validation timed out; this validation did not submit anything",
            ));
        }
        thread::sleep(Duration::from_millis(25));
    };
    let _ = writer.join();
    let _ = errors.join();
    let output = reader
        .join()
        .map_err(|_| unavailable("SDK response interrupted"))?
        .map_err(|_| unavailable("SDK response unreadable"))?;
    if output.len() > MAX_BYTES {
        return Err(unavailable("SDK response exceeds its bound"));
    }
    let payload: Value = serde_json::from_slice(&output)
        .map_err(|_| unavailable("SDK returned no qualified result"))?;
    if !status.success() || payload["ok"] != true {
        let mut message = payload
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("SDK rejected the current operation")
            .to_string();
        if request.get("secretSaltHex").is_some() {
            // Provider errors can echo encoded instructions, not just the raw
            // salt. Do not propagate secret-bearing validation diagnostics.
            message = "Current SDK validation rejected this private Oracle action. Private material is retained. This validation submitted nothing.".into();
        }
        return Err(CliError::coded(
            "SDK_OPERATION_REJECTED",
            "denied",
            message,
            false,
        ));
    }
    if payload["sdkCommit"] != crate::current_release::SDK_PACKAGE_COMMIT
        || payload["owner"] != owner
    {
        return Err(CliError::new("SDK response identity mismatch"));
    }
    Ok(payload)
}
