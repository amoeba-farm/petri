//! Private reveal material for sealed update claims. A commit (tag 166)
//! stores only a hash on chain; the reveal (tag 198) needs the values and the
//! secret salt behind it. Petri keeps them in a private file next to its
//! config (mode 0600) and never prints the salt.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    futures::error::{Error, Result},
    petri_config,
};

const FILE: &str = "oracle-update-claims.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub oracle: String,
    pub month: String,
    pub source: String,
    pub claimant: String,
    pub claim_id: String,
    pub prior_state: u64,
    pub new_state: u64,
    pub source_time: u64,
    pub evidence_hash: String,
    pub archive_url: String,
    pub secret_salt: String,
    #[serde(default)]
    pub revealed: bool,
}

fn path() -> Result<PathBuf> {
    let config = petri_config::config_path().map_err(Error::msg)?;
    Ok(config
        .parent()
        .ok_or_else(|| Error::msg("no Petri config directory"))?
        .join(FILE))
}

fn load_from(path: &Path) -> Result<BTreeMap<String, Claim>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| Error::msg(format!("{} is unreadable: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(Error::msg(format!("cannot read {}: {e}", path.display()))),
    }
}

fn save_to(path: &Path, claims: &BTreeMap<String, Claim>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(claims).expect("claims encode");
    petri_config::write_private_file(path, &bytes).map_err(Error::msg)
}

/// Saves the reveal material of the claim at `address` (before the commit is sent).
pub fn save(address: &str, claim: Claim) -> Result<PathBuf> {
    let path = path()?;
    let mut claims = load_from(&path)?;
    claims.insert(address.to_string(), claim);
    save_to(&path, &claims)?;
    Ok(path)
}

pub fn load(address: &str) -> Result<Claim> {
    let path = path()?;
    load_from(&path)?.remove(address).ok_or_else(|| {
        Error::msg(format!(
            "no saved reveal material for update claim {address} in {}; only the wallet that committed it from this Petri can reveal it",
            path.display()
        ))
    })
}

pub fn mark_revealed(address: &str) -> Result<()> {
    let path = path()?;
    let mut claims = load_from(&path)?;
    if let Some(claim) = claims.get_mut(address) {
        claim.revealed = true;
    }
    save_to(&path, &claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_round_trip_through_a_private_file() {
        let dir = std::env::temp_dir().join(format!("petri-claims-{}", std::process::id()));
        // write_private_file creates the missing directory itself, privately.
        let path = dir.join(FILE);
        let claim = Claim {
            oracle: "o".into(),
            month: "m".into(),
            source: "s".into(),
            claimant: "c".into(),
            claim_id: "00".repeat(32),
            prior_state: 1,
            new_state: 2,
            source_time: 3,
            evidence_hash: "11".repeat(32),
            archive_url: "https://web.archive.org/web/20261013000000/https://example.com".into(),
            secret_salt: "22".repeat(32),
            revealed: false,
        };
        let mut all = BTreeMap::new();
        all.insert("claim".to_string(), claim.clone());
        save_to(&path, &all).unwrap();
        assert_eq!(load_from(&path).unwrap()["claim"], claim);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
