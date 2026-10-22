//! Petri's evidence folder: the bundles made or imported here, and what
//! happened to each (uploads, submissions), next to the config (mode 0600:
//! a response body can hold account details).
//!
//! ```text
//! <config dir>/evidence/bundles/<bundle digest>.json
//! <config dir>/evidence/index.json
//! ```
//!
//! The index is a convenience for `list` and the TUI; the bundle files are
//! the evidence, and any of them re-verifies on its own.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    futures::error::{Error, Result},
    petri_config,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub digest: String,
    pub file: PathBuf,
    /// `proved` here or `imported`.
    pub origin: String,
    pub added_at: u64,
    pub locator: String,
    #[serde(default)]
    pub price: String,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

/// Something done with a bundle: `uploaded`, `candidate`, `update`, with
/// the API's or the chain's own answer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub kind: String,
    pub at: u64,
    pub summary: String,
    #[serde(default)]
    pub detail: Value,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn dir() -> Result<PathBuf> {
    let config = petri_config::config_path().map_err(Error::msg)?;
    Ok(config
        .parent()
        .ok_or_else(|| Error::msg("no Petri config directory"))?
        .join("evidence"))
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join("index.json")
}

pub fn load_index(dir: &Path) -> Result<BTreeMap<String, Entry>> {
    match std::fs::read(index_path(dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| Error::msg(format!("{} is unreadable: {e}", index_path(dir).display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(Error::msg(format!("cannot read the evidence index: {e}"))),
    }
}

fn save_index(dir: &Path, index: &BTreeMap<String, Entry>) -> Result<()> {
    petri_config::create_private_dir_all(dir).map_err(|e| Error::msg(e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(index).expect("index encodes");
    petri_config::write_private_file(&index_path(dir), &bytes).map_err(Error::msg)
}

/// Writes the bundle (at `out`, or in the folder) and indexes it.
pub fn save_in(dir: &Path, mut entry: Entry, json: &str, out: Option<&Path>) -> Result<Entry> {
    let file = match out {
        Some(p) => p.to_path_buf(),
        None => dir.join("bundles").join(format!("{}.json", entry.digest)),
    };
    if let Some(parent) = file.parent() {
        petri_config::create_private_dir_all(parent).map_err(|e| Error::msg(e.to_string()))?;
    }
    petri_config::write_private_file(&file, json.as_bytes()).map_err(Error::msg)?;
    entry.file = file;
    let mut index = load_index(dir)?;
    if let Some(old) = index.get(&entry.digest) {
        entry.events = old.events.clone();
    }
    index.insert(entry.digest.clone(), entry.clone());
    save_index(dir, &index)?;
    Ok(entry)
}

pub fn record_in(dir: &Path, digest: &str, event: Event) -> Result<()> {
    let mut index = load_index(dir)?;
    if let Some(entry) = index.get_mut(digest) {
        entry.events.push(event);
        save_index(dir, &index)?;
    }
    Ok(())
}

/// A bundle by file path, or by digest (or a unique prefix of at least 8
/// characters) in the folder: its path and text.
pub fn resolve_in(dir: &Path, arg: &str) -> Result<(PathBuf, String)> {
    let path = Path::new(arg);
    if path.is_file() {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::msg(format!("cannot read {}: {e}", path.display())))?;
        return Ok((path.to_path_buf(), text));
    }
    let lower = arg.to_ascii_lowercase();
    if lower.len() >= 8 && lower.bytes().all(|b| b.is_ascii_hexdigit()) {
        let index = load_index(dir)?;
        let hits: Vec<&Entry> = index
            .values()
            .filter(|e| e.digest.starts_with(&lower))
            .collect();
        match hits.as_slice() {
            [e] => {
                let text = std::fs::read_to_string(&e.file).map_err(|err| {
                    Error::msg(format!("cannot read {}: {err}", e.file.display()))
                })?;
                return Ok((e.file.clone(), text));
            }
            [] => {}
            _ => {
                return Err(Error::msg(format!(
                    "`{arg}` matches several proofs; give more of the digest"
                )));
            }
        }
    }
    Err(Error::msg(format!(
        "no bundle file or proof `{arg}`; see `petri evidence list`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_resolve_record() {
        let dir = std::env::temp_dir().join(format!("petri-evidence-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let digest = "ab".repeat(32);
        let entry = Entry {
            digest: digest.clone(),
            origin: "proved".into(),
            locator: "https://a.example/api".into(),
            ..Default::default()
        };
        let saved = save_in(&dir, entry, "{\"x\":1}", None).unwrap();
        assert!(saved.file.ends_with(format!("{digest}.json")));
        let (path, text) = resolve_in(&dir, &digest[..10]).unwrap();
        assert_eq!((path, text.as_str()), (saved.file.clone(), "{\"x\":1}"));
        record_in(
            &dir,
            &digest,
            Event {
                kind: "uploaded".into(),
                at: 1,
                summary: "stored".into(),
                detail: Value::Null,
            },
        )
        .unwrap();
        assert_eq!(load_index(&dir).unwrap()[&digest].events.len(), 1);
        assert!(resolve_in(&dir, "zzzz").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&saved.file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
