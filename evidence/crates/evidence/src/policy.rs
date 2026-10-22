//! Verifier policy: which proof signers and certificate roots a verifier
//! accepts.
//!
//! This is cryptographic trust, not source curation. It names no publisher,
//! website or price: any HTTPS source can be proven. It names the notaries
//! whose attestations count, because a signature from a key the prover
//! chose proves nothing (the prover could run that notary itself). A
//! notary sees only encrypted traffic in MPC mode; what it vouches for is
//! that the transcript came from a TLS session with the named server.
//!
//! Policies are versioned files compiled into every verifier
//! (`policy/*.json`), so changing one is a reviewed release, never an
//! operator's switch. Anyone can run a notary (`amoeba-notary`); adding its
//! key to a policy is the governed step.

use crate::hash::tagged;

pub const POLICY_FORMAT: &str = "amoeba-webproof-verifier-policy-v1";
/// The TLSNotary revision this crate's verifier is built from. Proofs from
/// another revision may not deserialize; the policy names the one it
/// accepts.
pub const TLSN_REVISION: &str = "5f67389de3f8962227fddffd71d78a852acc9b01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// A notary or root a production verifier accepts.
    Production,
    /// A development notary or test root: never evidence on production.
    Development,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notary {
    pub name: String,
    /// `secp256k1` (compressed SEC1 public key).
    pub alg: String,
    pub key: Vec<u8>,
    pub trust: Trust,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_bundle_bytes: usize,
    pub max_presentation_bytes: usize,
    pub max_sent: usize,
    pub max_recv: usize,
}

/// Bounds on hostile input (prompt v2, section 9: "enforce size/time
/// limits"). `max_sent`/`max_recv` are the most a verifier (and the
/// notary) accepts; the prover asks for less by default
/// (`amoeba-webproof`'s `DEFAULT_MAX_SENT`/`DEFAULT_MAX_RECV`) and up to
/// these with `--max-response`.
pub const DEFAULT_LIMITS: Limits = Limits {
    max_bundle_bytes: 2 * 1024 * 1024,
    max_presentation_bytes: 1024 * 1024,
    max_sent: 8 * 1024,
    max_recv: 64 * 1024,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// `<format>/<profile>`, recorded in every verification.
    pub id: String,
    pub notaries: Vec<Notary>,
    /// Development roots (DER) tried after the Mozilla set; a chain to one
    /// of them is development trust.
    pub development_roots: Vec<Vec<u8>>,
    /// DKIM keys this verifier accepts for email evidence, by domain and
    /// selector, with where each binding comes from.
    pub dkim_keys: Vec<crate::email::PinnedKey>,
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid verifier policy: {0}")]
pub struct PolicyError(pub String);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PolicyFile {
    format: String,
    profile: String,
    backend: BackendFile,
    notaries: Vec<NotaryFile>,
    #[serde(default)]
    dkim_keys: Vec<DkimKeyFile>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DkimKeyFile {
    domain: String,
    selector: String,
    record: String,
    trust: Trust,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BackendFile {
    name: String,
    revision: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NotaryFile {
    name: String,
    alg: String,
    key: String,
    trust: Trust,
}

impl Policy {
    pub fn from_json(text: &str) -> Result<Self, PolicyError> {
        let f: PolicyFile = serde_json::from_str(text).map_err(|e| PolicyError(e.to_string()))?;
        if f.format != POLICY_FORMAT {
            return Err(PolicyError(format!("format must be {POLICY_FORMAT}")));
        }
        if f.backend.name != "tlsnotary" || f.backend.revision != TLSN_REVISION {
            return Err(PolicyError(format!(
                "this verifier is built for tlsnotary {TLSN_REVISION}"
            )));
        }
        let mut notaries = Vec::new();
        for n in f.notaries {
            if n.alg != "secp256k1" {
                return Err(PolicyError(format!(
                    "notary {}: only secp256k1 keys",
                    n.name
                )));
            }
            let key = hex::decode(&n.key)
                .map_err(|_| PolicyError(format!("notary {}: key is not hex", n.name)))?;
            if key.len() != 33 || !matches!(key[0], 2 | 3) {
                return Err(PolicyError(format!(
                    "notary {}: key must be a compressed secp256k1 point",
                    n.name
                )));
            }
            notaries.push(Notary {
                name: n.name,
                alg: n.alg,
                key,
                trust: n.trust,
            });
        }
        let mut dkim_keys = Vec::new();
        for k in f.dkim_keys {
            crate::dkim::parse_key_record(&k.record).map_err(|e| {
                PolicyError(format!(
                    "DKIM key {}._domainkey.{}: {e}",
                    k.selector, k.domain
                ))
            })?;
            dkim_keys.push(crate::email::PinnedKey {
                provenance: format!("pinned in the verifier policy {}", f.profile),
                domain: k.domain.to_ascii_lowercase(),
                selector: k.selector.to_ascii_lowercase(),
                record: k.record,
                trust: k.trust,
            });
        }
        Ok(Self {
            id: format!("{POLICY_FORMAT}/{}", f.profile),
            notaries,
            development_roots: Vec::new(),
            dkim_keys,
            limits: DEFAULT_LIMITS,
        })
    }

    /// The production policy: the notaries production evidence may use.
    pub fn production() -> Self {
        Self::from_json(include_str!("../../../policy/production.json"))
            .expect("compiled-in production policy")
    }

    /// The dev chain's policy (dev.amoeba.farm): its notary is development
    /// trust.
    pub fn dev() -> Self {
        Self::from_json(include_str!("../../../policy/dev.json")).expect("compiled-in dev policy")
    }

    /// A test policy: one development notary and development roots. Every
    /// result it produces says development trust.
    pub fn development(name: &str, notary_key: Vec<u8>, roots: Vec<Vec<u8>>) -> Self {
        Self {
            id: format!("{POLICY_FORMAT}/test"),
            notaries: vec![Notary {
                name: name.into(),
                alg: "secp256k1".into(),
                key: notary_key,
                trust: Trust::Development,
            }],
            development_roots: roots,
            dkim_keys: Vec::new(),
            limits: DEFAULT_LIMITS,
        }
    }

    pub fn notary(&self, alg: &str, key: &[u8]) -> Option<&Notary> {
        self.notaries.iter().find(|n| n.alg == alg && n.key == key)
    }

    /// A digest of what this policy accepts (recorded with results).
    pub fn digest(&self) -> [u8; 32] {
        let mut fields: Vec<Vec<u8>> = vec![
            self.id.clone().into_bytes(),
            TLSN_REVISION.as_bytes().to_vec(),
        ];
        for n in &self.notaries {
            fields.push(format!("{}:{}:{:?}", n.alg, hex::encode(&n.key), n.trust).into_bytes());
        }
        for r in &self.development_roots {
            fields.push(r.clone());
        }
        for k in &self.dkim_keys {
            fields.push(
                format!(
                    "dkim:{}:{}:{}:{:?}",
                    k.domain, k.selector, k.record, k.trust
                )
                .into_bytes(),
            );
        }
        let refs: Vec<&[u8]> = fields.iter().map(|f| f.as_slice()).collect();
        tagged(POLICY_FORMAT, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_policies_load() {
        let prod = Policy::production();
        assert!(prod.notaries.iter().all(|n| n.trust == Trust::Production));
        let dev = Policy::dev();
        assert!(dev.notaries.iter().all(|n| n.trust == Trust::Development));
    }

    #[test]
    fn refuses_other_backends_and_keys() {
        let ok = format!(
            r#"{{"format":"{POLICY_FORMAT}","profile":"x","backend":{{"name":"tlsnotary","revision":"{TLSN_REVISION}"}},"notaries":[]}}"#
        );
        assert!(Policy::from_json(&ok).is_ok());
        assert!(Policy::from_json(&ok.replace("tlsnotary", "other")).is_err());
        let bad_key = ok.replace(
            "\"notaries\":[]",
            r#""notaries":[{"name":"n","alg":"secp256k1","key":"04ab","trust":"production"}]"#,
        );
        assert!(Policy::from_json(&bad_key).is_err());
    }
}
