//! The deployment to trust (program ids, USDC mint, sponsor) and market name
//! resolution.

use std::{fs, path::Path, str::FromStr};

use chrono::{DateTime, Datelike, Utc};
use serde::Deserialize;
use solana_program::pubkey::Pubkey;

use super::{
    api::{Api, ApiConfig},
    error::{Error, Result, bail},
    governance::{self, GovernanceDeployment},
    settings::{self, Profile},
};

/// Program ids and accounts this client trusts.
#[derive(Clone, Debug)]
pub struct Deployment {
    pub oracle_release_tag: Option<String>,
    pub governance_controller: Option<GovernanceDeployment>,
    /// The oracle program (manifest and API `programs.oracle`).
    pub oracle: Pubkey,
    /// The futures program (manifest `futures.program`, API
    /// `programs.futures`).
    pub futures: Option<Pubkey>,
    pub usdc_mint: Pubkey,
    pub sponsor: Option<Pubkey>,
    pub genesis_hash: Option<String>,
    /// Where the ids came from, for `petri status`.
    pub source: String,
    /// The profile it is trusted under: the dev chain's exceptions (its own
    /// governance controller, the oracle's `localnet-dev` release) apply only
    /// in the dev profile.
    pub profile: Profile,
}

/// The public parts of a deployment manifest. Other fields are ignored,
/// present or absent (the retired clearing deployment's `programs.clearing`,
/// `programs.matcher`, `config` and `markets` among them, and a leftover
/// pre-rename `range` section).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    oracle: Option<OracleProfile>,
    genesis_hash: Option<String>,
    governance_controller: Option<GovernanceDeployment>,
    programs: ManifestPrograms,
    usdc_mint: String,
    #[serde(default)]
    accounts: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    futures: Option<FuturesSection>,
}

#[derive(Deserialize)]
struct FuturesSection {
    program: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleProfile {
    release_tag: String,
}

#[derive(Deserialize)]
struct ManifestPrograms {
    oracle: String,
}

pub fn pubkey(text: &str, what: &str) -> Result<Pubkey> {
    Pubkey::from_str(text.trim())
        .map_err(|_| Error::msg(format!("`{text}` is not a valid {what} address")))
}

impl Deployment {
    /// Reads a deployment manifest (`.localnet/deployment.json`). Only the
    /// public parts are read: program ids, the mint and account keys. A
    /// manifest is the user's own choice of what to trust, so the production
    /// ids are not imposed on it; the dev chain's exceptions still need the
    /// dev profile.
    pub fn from_manifest(path: &Path, profile: Profile) -> Result<Self> {
        let text = fs::read_to_string(path)
            .map_err(|e| Error::msg(format!("cannot read deployment {}: {e}", path.display())))?;
        Self::parse_manifest(&text, &path.display().to_string(), profile)
    }

    fn parse_manifest(text: &str, path: &str, profile: Profile) -> Result<Self> {
        let m: Manifest = serde_json::from_str(text)
            .map_err(|e| Error::msg(format!("{path} is not a deployment manifest: {e}")))?;
        let futures = m
            .futures
            .map(|f| pubkey(&f.program, "futures program"))
            .transpose()?;
        let d = Self {
            oracle_release_tag: m.oracle.map(|o| o.release_tag),
            governance_controller: m.governance_controller,
            oracle: pubkey(&m.programs.oracle, "oracle program")?,
            futures,
            usdc_mint: pubkey(&m.usdc_mint, "USDC mint")?,
            sponsor: m
                .accounts
                .get("sponsor")
                .and_then(|k| Pubkey::from_str(k).ok()),
            genesis_hash: m.genesis_hash,
            source: format!("manifest {path}"),
            profile,
        };
        d.validate()?;
        Ok(d)
    }

    /// The deployment the futures API's `/v1/config` serves. In the
    /// production profile it must be the production deployment
    /// (`pin_production`).
    pub fn from_api(api: &Api, profile: Profile) -> Result<Self> {
        let c: ApiConfig = api.config().map_err(|e| {
            Error::msg(format!(
                "{e}\nPetri needs the deployment's program ids: start the API or pass --deployment <manifest>"
            ))
        })?;
        Self::from_api_config(c, &format!("API {}/v1/config", api.base()), profile)
    }

    fn from_api_config(c: ApiConfig, source: &str, profile: Profile) -> Result<Self> {
        let futures = c
            .programs
            .futures
            .as_deref()
            .map(|f| pubkey(f, "futures program"))
            .transpose()?;
        let mut d = Self {
            oracle_release_tag: c.oracle_release_tag,
            governance_controller: c.governance_controller,
            oracle: pubkey(&c.programs.oracle, "oracle program")?,
            futures,
            usdc_mint: pubkey(&c.usdc_mint, "USDC mint")?,
            sponsor: Pubkey::from_str(&c.sponsor).ok(),
            genesis_hash: Some(c.genesis_hash).filter(|g| !g.is_empty()),
            source: source.to_string(),
            profile,
        };
        if profile == Profile::Production {
            d = d.pin_production()?;
        }
        d.validate()?;
        Ok(d)
    }

    fn validate(&self) -> Result<()> {
        governance::validate_deployment(
            self.governance_controller.as_ref(),
            self.futures.as_ref(),
            self.genesis_hash.as_deref(),
            self.profile == Profile::Dev,
        )
    }

    /// The production profile trusts only the production deployment: Solana
    /// mainnet-beta, the production oracle and futures programs and the
    /// production governance controller. What the API names must be those;
    /// what it leaves out is filled in with them (the controller gating both
    /// programs). Anything else is refused, so a wrong or compromised API can
    /// never point a production Petri at other programs.
    fn pin_production(mut self) -> Result<Self> {
        let refuse = |what: &str, got: &str, want: &str| {
            Err(Error::msg(format!(
                "refusing {}: it names {what} {got}, but the production profile trusts only {want}; \
                 for another deployment use --profile dev (or `petri config set profile dev`)",
                self.source
            )))
        };
        match self.genesis_hash.as_deref() {
            Some(g) if g != settings::MAINNET_GENESIS => {
                return refuse("cluster genesis", g, settings::MAINNET_GENESIS);
            }
            Some(_) => {}
            None => self.genesis_hash = Some(settings::MAINNET_GENESIS.to_string()),
        }
        let oracle = pubkey(settings::PRODUCTION_ORACLE, "oracle program")?;
        if self.oracle != oracle {
            return refuse(
                "the oracle program",
                &self.oracle.to_string(),
                settings::PRODUCTION_ORACLE,
            );
        }
        let futures = pubkey(settings::PRODUCTION_FUTURES, "futures program")?;
        match self.futures {
            Some(f) if f != futures => {
                return refuse(
                    "the futures program",
                    &f.to_string(),
                    settings::PRODUCTION_FUTURES,
                );
            }
            Some(_) => {}
            None => self.futures = Some(futures),
        }
        let controller = ameba_sdk_futures::futures::governance::PRODUCTION_CONTROLLER_PROGRAM_ID;
        match &self.governance_controller {
            Some(g) if g.program.trim() != controller.to_string() => {
                return refuse(
                    "the governance controller",
                    &g.program,
                    &controller.to_string(),
                );
            }
            Some(_) => {}
            None => {
                self.governance_controller = Some(GovernanceDeployment {
                    program: controller.to_string(),
                    targets: vec![futures.to_string(), oracle.to_string()],
                });
            }
        }
        Ok(self)
    }

    /// The dev chain under the dev profile: the only place a custom governance
    /// controller and the oracle's `localnet-dev` release are accepted.
    pub fn dev_chain(&self) -> bool {
        self.profile == Profile::Dev && self.genesis_hash.as_deref() == Some(settings::DEV_GENESIS)
    }
}

/// `"RAMX Oct 26"`: the month that ends at the expiry.
pub fn label(symbol: &str, expiry_ts: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let at = DateTime::<Utc>::from_timestamp(expiry_ts as i64 - 1, 0).unwrap_or_default();
    format!(
        "{symbol} {} {:02}",
        MONTHS[at.month0() as usize],
        at.year().rem_euclid(100)
    )
}

/// One market as far as naming needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketRef {
    pub address: Pubkey,
    pub symbol: String,
    pub expiry_ts: u64,
    pub label: String,
    pub status: String,
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Expiry spellings a market answers to: `OCT26`, `OCT`, `202610` (also
/// written `2026-10`) and the unix seconds. The month is the one the label
/// names.
fn expiry_keys(m: &MarketRef) -> Vec<String> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let at = DateTime::<Utc>::from_timestamp(m.expiry_ts as i64 - 1, 0).unwrap_or_default();
    let mon = MONTHS[at.month0() as usize];
    vec![
        m.expiry_ts.to_string(),
        format!("{mon}{:02}", at.year().rem_euclid(100)),
        mon.to_string(),
        format!("{}{:02}", at.year(), at.month()),
    ]
}

/// Resolves a market name against the known markets. A bare symbol picks the
/// open market with the nearest expiry unless `expiry` narrows it.
pub fn resolve<'a>(
    query: &str,
    expiry: Option<&str>,
    markets: &'a [MarketRef],
) -> Result<&'a MarketRef> {
    if let Ok(address) = Pubkey::from_str(query.trim()) {
        return markets
            .iter()
            .find(|m| m.address == address)
            .ok_or_else(|| Error::msg(format!("{address} is not a market of this deployment")));
    }
    let q = squash(query);
    if q.is_empty() {
        bail("name a market, for example RAMX or \"RAMX Nov 26\"")?;
    }
    let wanted_expiry = expiry.map(squash);
    let mut candidates: Vec<&MarketRef> = markets
        .iter()
        .filter(|m| {
            let symbol = squash(&m.symbol);
            let by_name = q == symbol
                || q == squash(&m.label)
                || expiry_keys(m).iter().any(|k| q == format!("{symbol}{k}"));
            let by_expiry = wanted_expiry
                .as_ref()
                .is_none_or(|e| expiry_keys(m).iter().any(|k| k == e));
            by_name && by_expiry
        })
        .collect();
    if candidates.is_empty() {
        let known: Vec<&str> = markets.iter().map(|m| m.label.as_str()).collect();
        return Err(Error::msg(format!(
            "no market matches `{query}`{}; markets: {}",
            expiry.map(|e| format!(" expiring {e}")).unwrap_or_default(),
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        )));
    }
    candidates.sort_by_key(|m| (m.status != "open", m.expiry_ts));
    Ok(candidates[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(symbol: &str, expiry: u64, status: &str, n: u8) -> MarketRef {
        MarketRef {
            address: Pubkey::new_from_array([n; 32]),
            symbol: symbol.into(),
            expiry_ts: expiry,
            label: label(symbol, expiry),
            status: status.into(),
        }
    }

    #[test]
    fn manifests_parse_with_or_without_the_retired_clearing_fields() {
        let key = |n: u8| Pubkey::new_from_array([n; 32]).to_string();
        let with = serde_json::json!({
            "genesisHash": "g", "usdcMint": key(4),
            "programs": { "clearing": key(1), "matcher": key(2), "oracle": key(3) },
            "config": key(9), "markets": [{ "instrument": key(8) }],
            "futures": { "program": key(5), "treasury": key(6) },
            "accounts": { "sponsor": key(7) },
        });
        let without = serde_json::json!({
            "genesisHash": "g", "usdcMint": key(4),
            "programs": { "oracle": key(3) },
            "futures": { "program": key(5) },
            "accounts": { "sponsor": key(7) },
        });
        for manifest in [with, without] {
            let d =
                Deployment::parse_manifest(&manifest.to_string(), "fixture", Profile::Dev).unwrap();
            assert_eq!(d.oracle.to_string(), key(3));
            assert_eq!(d.futures.map(|f| f.to_string()), Some(key(5)));
            assert_eq!(d.usdc_mint.to_string(), key(4));
            assert_eq!(d.sponsor.map(|s| s.to_string()), Some(key(7)));
        }
        // A leftover pre-rename `range` section is ignored.
        let old = serde_json::json!({
            "genesisHash": "g", "usdcMint": key(4),
            "programs": { "oracle": key(3) },
            "range": { "program": key(5) },
        });
        let d = Deployment::parse_manifest(&old.to_string(), "fixture", Profile::Dev).unwrap();
        assert_eq!(d.futures, None);
        // Governance must list the futures program it gates.
        let governed = |targets: Vec<String>| {
            serde_json::json!({
                "genesisHash": "A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX", "usdcMint": key(4),
                "programs": { "oracle": key(3) }, "futures": { "program": key(5) },
                "governanceController": {
                    "program": ameba_sdk_futures::futures::governance::LOCALNET_CONTROLLER_PROGRAM_ID.to_string(),
                    "targets": targets,
                },
            })
            .to_string()
        };
        assert!(
            Deployment::parse_manifest(&governed(vec![key(5)]), "fixture", Profile::Dev).is_ok()
        );
        assert!(
            Deployment::parse_manifest(&governed(vec![key(1), key(2)]), "fixture", Profile::Dev)
                .is_err()
        );
        // The dev chain's own controller needs the dev profile.
        let refused =
            Deployment::parse_manifest(&governed(vec![key(5)]), "fixture", Profile::Production)
                .unwrap_err();
        assert!(
            refused.message.contains("--profile dev"),
            "{}",
            refused.message
        );
    }

    fn api_config(genesis: &str, oracle: &str, futures: Option<&str>) -> ApiConfig {
        serde_json::from_value(serde_json::json!({
            "cluster": "mainnet-beta", "rpcUrl": null, "genesisHash": genesis,
            "programs": { "oracle": oracle, "futures": futures },
            "usdcMint": Pubkey::new_from_array([4; 32]).to_string(), "usdcDecimals": 6,
            "sponsor": Pubkey::new_from_array([7; 32]).to_string(),
        }))
        .unwrap()
    }

    #[test]
    fn the_production_profile_trusts_only_the_production_deployment() {
        use settings::{DEV_GENESIS, MAINNET_GENESIS, PRODUCTION_FUTURES, PRODUCTION_ORACLE};
        let controller = ameba_sdk_futures::futures::governance::PRODUCTION_CONTROLLER_PROGRAM_ID;
        let production = |c| Deployment::from_api_config(c, "API fixture", Profile::Production);

        // The production API: taken as served; what it leaves out is the
        // production ids, with the production controller gating both programs.
        let d = production(api_config(MAINNET_GENESIS, PRODUCTION_ORACLE, None)).unwrap();
        assert_eq!(d.genesis_hash.as_deref(), Some(MAINNET_GENESIS));
        assert_eq!(d.oracle.to_string(), PRODUCTION_ORACLE);
        assert_eq!(
            d.futures.map(|f| f.to_string()).as_deref(),
            Some(PRODUCTION_FUTURES)
        );
        let g = d.governance_controller.as_ref().unwrap();
        assert_eq!(g.program, controller.to_string());
        assert_eq!(g.targets, vec![PRODUCTION_FUTURES, PRODUCTION_ORACLE]);
        assert!(!d.dev_chain());
        let mut served = api_config("", PRODUCTION_ORACLE, Some(PRODUCTION_FUTURES));
        served.governance_controller = Some(GovernanceDeployment {
            program: controller.to_string(),
            targets: vec![PRODUCTION_FUTURES.to_string()],
        });
        let d = production(served).unwrap();
        assert_eq!(d.genesis_hash.as_deref(), Some(MAINNET_GENESIS));
        assert_eq!(
            d.governance_controller.unwrap().targets,
            vec![PRODUCTION_FUTURES]
        );

        // Anything else is refused: another cluster, other programs, another
        // controller.
        let other = Pubkey::new_from_array([9; 32]).to_string();
        for c in [
            api_config(DEV_GENESIS, PRODUCTION_ORACLE, Some(PRODUCTION_FUTURES)),
            api_config(MAINNET_GENESIS, &other, Some(PRODUCTION_FUTURES)),
            api_config(MAINNET_GENESIS, PRODUCTION_ORACLE, Some(&other)),
        ] {
            let e = production(c).unwrap_err();
            assert!(e.message.contains("--profile dev"), "{}", e.message);
        }
        let mut served = api_config(MAINNET_GENESIS, PRODUCTION_ORACLE, Some(PRODUCTION_FUTURES));
        served.governance_controller = Some(GovernanceDeployment {
            program: other.clone(),
            targets: vec![PRODUCTION_FUTURES.to_string()],
        });
        assert!(production(served).is_err());

        // The dev profile takes the dev chain's API as served.
        let dev_controller = ameba_sdk_futures::futures::governance::LOCALNET_CONTROLLER_PROGRAM_ID;
        let mut served = api_config(DEV_GENESIS, &other, Some(&other));
        served.governance_controller = Some(GovernanceDeployment {
            program: dev_controller.to_string(),
            targets: vec![other.clone()],
        });
        let d = Deployment::from_api_config(served.clone(), "API fixture", Profile::Dev).unwrap();
        assert_eq!(d.oracle.to_string(), other);
        assert!(d.dev_chain());
        assert!(Deployment::from_api_config(served, "API fixture", Profile::Production).is_err());
    }

    #[test]
    fn labels_name_the_month_ending_at_expiry() {
        assert_eq!(label("RAMX", 1_793_491_200), "RAMX Oct 26");
        assert_eq!(label("NANDX", 1_796_083_200), "NANDX Nov 26");
    }

    #[test]
    fn names_resolve_to_markets() {
        let markets = vec![
            m("RAMX", 1_796_083_200, "open", 1),
            m("RAMX", 1_793_491_200, "open", 2),
            m("NANDX", 1_793_491_200, "open", 3),
        ];
        assert_eq!(
            resolve("ramx", None, &markets).unwrap().expiry_ts,
            1_793_491_200
        );
        assert_eq!(
            resolve("RAMX Nov 26", None, &markets).unwrap().address,
            markets[0].address
        );
        assert_eq!(
            resolve("ramx-nov26", None, &markets).unwrap().address,
            markets[0].address
        );
        assert_eq!(
            resolve("RAMX", Some("nov26"), &markets).unwrap().address,
            markets[0].address
        );
        assert_eq!(
            resolve("RAMX", Some("2026-11"), &markets).unwrap().address,
            markets[0].address
        );
        assert_eq!(
            resolve("RAMX", Some("1796083200"), &markets)
                .unwrap()
                .address,
            markets[0].address
        );
        assert_eq!(
            resolve("RAMX", Some("oct"), &markets).unwrap().address,
            markets[1].address
        );
        assert_eq!(
            resolve(&markets[2].address.to_string(), None, &markets)
                .unwrap()
                .symbol,
            "NANDX"
        );
        assert!(resolve("NOSUCH", None, &markets).is_err());
        assert!(resolve("NANDX", Some("nov26"), &markets).is_err());
        assert!(resolve(&Pubkey::new_from_array([9; 32]).to_string(), None, &markets).is_err());
    }

    #[test]
    fn bare_symbols_prefer_open_markets() {
        let markets = vec![
            m("RAMX", 1_793_491_200, "settled", 1),
            m("RAMX", 1_796_083_200, "open", 2),
        ];
        assert_eq!(
            resolve("RAMX", None, &markets).unwrap().address,
            markets[1].address
        );
    }
}
