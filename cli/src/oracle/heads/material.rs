//! Private reveal material for heads claims. A commit (tag 210/15) puts only a hash on chain;
//! the reveal (210/16) needs the value, the source time, the origin, the LTA terms, the
//! evidence and the secret salt behind it. Petri keeps them in a private file next to its config
//! (mode 0600, as the v1 update claims') and never prints the salt.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ameba_oracle::instruction::heads::{HeadsLtaTerms, RevealClaimParams};
use serde::{Deserialize, Serialize};
use solana_program::pubkey::Pubkey;

use crate::{
    futures::error::{Error, Result},
    oracle::skus::{hex, parse_hex32},
    petri_config,
};

const FILE: &str = "oracle-heads-claims.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub oracle: String,
    pub product: String,
    pub head: u32,
    pub seq: u32,
    pub fetcher: String,
    pub value: u64,
    pub source_ts: u64,
    pub origin_id: String,
    #[serde(default)]
    pub effective_from_ts: u64,
    #[serde(default)]
    pub next_reset_ts: u64,
    #[serde(default)]
    pub floor: u64,
    #[serde(default)]
    pub ceiling: u64,
    pub salt: String,
    pub evidence_kind: u8,
    pub evidence_commitment: String,
    /// The sealed object the evidence is, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_object: Option<String>,
    /// The web proof cited (its bundle digest), published once the claim is revealed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_bundle: Option<String>,
    #[serde(default)]
    pub revealed: bool,
}

impl Material {
    /// The reveal's payload.
    pub fn params(&self) -> Result<RevealClaimParams> {
        Ok(RevealClaimParams {
            value: self.value,
            source_ts: self.source_ts,
            origin_id: parse_hex32(&self.origin_id, "origin id")?,
            lta: HeadsLtaTerms {
                effective_from_ts: self.effective_from_ts,
                next_reset_ts: self.next_reset_ts,
                floor: self.floor,
                ceiling: self.ceiling,
            },
            salt: parse_hex32(&self.salt, "salt")?,
            evidence_kind: self.evidence_kind,
            evidence_commitment: parse_hex32(&self.evidence_commitment, "evidence commitment")?,
        })
    }

    pub fn evidence_object(&self) -> Result<Option<Pubkey>> {
        self.evidence_object
            .as_deref()
            .map(|k| {
                k.parse::<Pubkey>()
                    .map_err(|_| Error::msg(format!("{k} is not an evidence object address")))
            })
            .transpose()
    }

    /// The material of `params`.
    #[allow(clippy::too_many_arguments)]
    pub fn of(
        oracle: &Pubkey,
        product: &str,
        head: u32,
        seq: u32,
        fetcher: &Pubkey,
        params: &RevealClaimParams,
        object: Option<&Pubkey>,
        bundle: Option<String>,
    ) -> Self {
        Self {
            oracle: oracle.to_string(),
            product: product.to_string(),
            head,
            seq,
            fetcher: fetcher.to_string(),
            value: params.value,
            source_ts: params.source_ts,
            origin_id: hex(&params.origin_id),
            effective_from_ts: params.lta.effective_from_ts,
            next_reset_ts: params.lta.next_reset_ts,
            floor: params.lta.floor,
            ceiling: params.lta.ceiling,
            salt: hex(&params.salt),
            evidence_kind: params.evidence_kind,
            evidence_commitment: hex(&params.evidence_commitment),
            evidence_object: object.map(|o| o.to_string()),
            evidence_bundle: bundle,
            revealed: false,
        }
    }
}

pub fn path() -> Result<PathBuf> {
    let config = petri_config::config_path().map_err(Error::msg)?;
    Ok(config
        .parent()
        .ok_or_else(|| Error::msg("no Petri config directory"))?
        .join(FILE))
}

fn load_from(path: &Path) -> Result<BTreeMap<String, Material>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| Error::msg(format!("{} is unreadable: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(Error::msg(format!("cannot read {}: {e}", path.display()))),
    }
}

fn save_to(path: &Path, all: &BTreeMap<String, Material>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(all).expect("claims encode");
    petri_config::write_private_file(path, &bytes).map_err(Error::msg)
}

/// Saves the material of the claim at `address`, before its commit is sent.
pub fn save(address: &Pubkey, material: Material) -> Result<PathBuf> {
    let path = path()?;
    let mut all = load_from(&path)?;
    all.insert(address.to_string(), material);
    save_to(&path, &all)?;
    Ok(path)
}

pub fn load(address: &Pubkey, head: u32, seq: u32) -> Result<Material> {
    let path = path()?;
    load_from(&path)?.remove(&address.to_string()).ok_or_else(|| {
        Error::msg(format!(
            "Petri has no reveal material for claim {seq} of head {head} in {}: only the Petri that committed it can reveal it.",
            path.display()
        ))
    })
}

pub fn mark_revealed(address: &Pubkey) -> Result<()> {
    let path = path()?;
    let mut all = load_from(&path)?;
    if let Some(m) = all.get_mut(&address.to_string()) {
        m.revealed = true;
    }
    save_to(&path, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_round_trips_through_a_private_file() {
        let dir = std::env::temp_dir().join(format!("petri-heads-claims-{}", std::process::id()));
        let path = dir.join(FILE);
        let params = RevealClaimParams {
            value: 4_850_000,
            source_ts: 1_793_178_000,
            origin_id: [7; 32],
            lta: HeadsLtaTerms {
                effective_from_ts: 1,
                next_reset_ts: 2,
                floor: 3,
                ceiling: 4,
            },
            salt: [9; 32],
            evidence_kind: 3,
            evidence_commitment: [5; 32],
        };
        let object = Pubkey::new_unique();
        let m = Material::of(
            &Pubkey::new_unique(),
            "RAMX",
            4,
            2,
            &Pubkey::new_unique(),
            &params,
            Some(&object),
            None,
        );
        assert_eq!(m.params().unwrap(), params);
        assert_eq!(m.evidence_object().unwrap(), Some(object));
        let mut all = BTreeMap::new();
        all.insert("claim".to_string(), m.clone());
        save_to(&path, &all).unwrap();
        assert_eq!(load_from(&path).unwrap()["claim"], m);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
