//! Futures connection settings: flag or environment, then the Petri config
//! file, then the profile's defaults. The signing wallet is Petri's attached
//! wallet (`--keypair`, the saved futures keypair, or the Solana CLI config).
//!
//! The profile names the deployment Petri talks to and trusts when nothing
//! else is set: `production` (Amoeba on Solana mainnet-beta, the default of a
//! release build) or `dev` (the dev chain on loopback, the default of a build
//! with the `dev-chain` feature, which `scripts/install-dev.sh` enables).

use std::path::PathBuf;

use clap::ValueEnum;

use crate::{cli::Cli, petri_config, solana_config};

use super::args::RouteArg;

/// The production futures API (`ship/targets.md`, "First mainnet launch":
/// the futures API takes over `api.amoeba.farm`).
pub const PRODUCTION_API_URL: &str = "https://api.amoeba.farm";
/// The production RPC: Solana's public mainnet-beta RPC, a full RPC node.
/// Not the API's `/rpc` gateway, which serves only the web app's methods
/// (petri also reads with getProgramAccounts, simulateTransaction and
/// getVersion: `rpc::METHODS`).
pub const PRODUCTION_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const PRODUCTION_CLUSTER: &str = "mainnet-beta";
/// Solana mainnet-beta's genesis hash.
pub const MAINNET_GENESIS: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
/// The production programs (`ship/targets.md`, "First mainnet launch"). The
/// controller is the SDK's `PRODUCTION_CONTROLLER_PROGRAM_ID` (`8fhNi6…`).
pub const PRODUCTION_ORACLE: &str = "45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS";
pub const PRODUCTION_FUTURES: &str = "Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu";

/// The dev chain: the dev VM's loopback futures API and validator.
pub const DEV_API_URL: &str = "http://127.0.0.1:4500";
pub const DEV_RPC_URL: &str = "http://127.0.0.1:8899";
pub const DEV_CLUSTER: &str = "localnet";
/// The dev chain's genesis, the only chain where a dev-profile deployment may
/// use its own governance controller and the oracle's `localnet-dev` release.
pub const DEV_GENESIS: &str = "A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX";

/// Which deployment Petri talks to and trusts by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Profile {
    /// Amoeba on Solana mainnet-beta: the production API, Solana's public
    /// mainnet-beta RPC, and only the production programs and controller.
    #[value(name = "production", alias = "prod", alias = "mainnet")]
    Production,
    /// The dev chain (localnet): the loopback API and validator, the program
    /// ids the API or a manifest names, and the dev chain's own controller.
    #[value(name = "dev", alias = "localnet")]
    Dev,
}

impl Profile {
    /// This build's profile when none is chosen: `dev` in a `dev-chain` build
    /// (the dev VM's petri), else `production`.
    pub const BUILT_IN: Profile = if cfg!(feature = "dev-chain") {
        Profile::Dev
    } else {
        Profile::Production
    };

    pub fn name(self) -> &'static str {
        match self {
            Profile::Production => "production",
            Profile::Dev => "dev",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        <Self as ValueEnum>::from_str(text.trim(), true).ok()
    }

    pub fn api_url(self) -> &'static str {
        match self {
            Profile::Production => PRODUCTION_API_URL,
            Profile::Dev => DEV_API_URL,
        }
    }

    /// The RPC when none is set.
    pub fn rpc_url(self) -> &'static str {
        match self {
            Profile::Production => PRODUCTION_RPC_URL,
            Profile::Dev => DEV_RPC_URL,
        }
    }

    pub fn cluster(self) -> &'static str {
        match self {
            Profile::Production => PRODUCTION_CLUSTER,
            Profile::Dev => DEV_CLUSTER,
        }
    }
}

/// Where a value came from, for `petri config show`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Flag,
    File,
    SolanaConfig,
    Default,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Flag => "flag/env",
            Source::File => "petri config",
            Source::SolanaConfig => "Solana CLI config",
            Source::Default => "default",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub profile: (Profile, Source),
    pub rpc_url: (String, Source),
    pub api_url: (String, Source),
    pub keypair: (String, Source),
    pub deployment: Option<(PathBuf, Source)>,
    pub json: bool,
    pub yes: bool,
    pub route: RouteArg,
    pub allow_insecure_keypair: bool,
    pub color: bool,
}

fn pick(flag: Option<&str>, file: Option<&str>, default: &str) -> (String, Source) {
    let clean = |s: &str| s.trim().trim_end_matches('/').to_string();
    match (
        flag.filter(|s| !s.trim().is_empty()),
        file.filter(|s| !s.trim().is_empty()),
    ) {
        (Some(v), _) => (clean(v), Source::Flag),
        (None, Some(v)) => (clean(v), Source::File),
        (None, None) => (default.to_string(), Source::Default),
    }
}

/// The profile: `--profile` (or `PETRI_PROFILE`), else the saved `profile`,
/// else this build's.
pub fn profile(flag: Option<Profile>, file: &petri_config::FuturesConfig) -> (Profile, Source) {
    match (flag, file.profile.as_deref().and_then(Profile::parse)) {
        (Some(p), _) => (p, Source::Flag),
        (None, Some(p)) => (p, Source::File),
        (None, None) => (Profile::BUILT_IN, Source::Default),
    }
}

impl Settings {
    pub fn from_cli(cli: &Cli) -> Result<Self, String> {
        let file = petri_config::futures_config()?;
        Ok(Self::merge(cli, &file))
    }

    pub fn merge(cli: &Cli, file: &petri_config::FuturesConfig) -> Self {
        let profile = profile(cli.profile, file);
        let keypair = match (&cli.keypair, &file.keypair) {
            (Some(path), _) if !path.trim().is_empty() => {
                (solana_config::expand_tilde(path.trim()), Source::Flag)
            }
            // `petri wallet new` on Windows once saved the verbatim `\\?\C:\…`.
            (_, Some(path)) => (
                solana_config::plain_disk_path(&solana_config::expand_tilde(path)),
                Source::File,
            ),
            _ => {
                let config = solana_config::load_solana_cli_config(cli.solana_config.as_deref())
                    .ok()
                    .flatten();
                match config.as_ref().and_then(|c| c.keypair_path.clone()) {
                    Some(path) => (path, Source::SolanaConfig),
                    None => (solana_config::default_keypair_path(), Source::Default),
                }
            }
        };
        let deployment = match (&cli.deployment, &file.deployment) {
            (Some(p), _) => Some((p.clone(), Source::Flag)),
            (None, Some(p)) => Some((PathBuf::from(solana_config::expand_tilde(p)), Source::File)),
            (None, None) => None,
        };
        Self {
            rpc_url: pick(
                cli.rpc.as_deref(),
                file.rpc_url.as_deref(),
                profile.0.rpc_url(),
            ),
            api_url: pick(
                cli.futures_api.as_deref(),
                file.futures_api_url.as_deref(),
                profile.0.api_url(),
            ),
            profile,
            keypair,
            deployment,
            json: cli.resolved_output() == crate::cli::OutputFormat::Json,
            yes: cli.yes,
            route: cli.route,
            allow_insecure_keypair: cli.allow_insecure_keypair,
            color: !cli.no_color,
        }
    }

    /// Whether the RPC is the profile's own (no flag, environment or config
    /// set one): oracle reads then take their program scans from the futures
    /// API (`oracle::chain::Oracle::reads`).
    pub fn default_rpc(&self) -> bool {
        self.rpc_url.1 == Source::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn settings(args: &[&str], file: petri_config::FuturesConfig) -> Settings {
        Settings::merge(&Cli::parse_from(args), &file)
    }

    #[test]
    fn production_ids_are_the_reserved_ones() {
        use solana_program::pubkey::Pubkey;
        for id in [
            PRODUCTION_ORACLE,
            PRODUCTION_FUTURES,
            MAINNET_GENESIS,
            DEV_GENESIS,
        ] {
            assert!(id.parse::<Pubkey>().is_ok(), "{id}");
        }
        // The production oracle build binds its address in source.
        assert_eq!(ameba_oracle::ID.to_string(), PRODUCTION_ORACLE);
        assert_eq!(
            ameba_sdk_futures::futures::governance::PRODUCTION_CONTROLLER_PROGRAM_ID.to_string(),
            "8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx"
        );
    }

    #[test]
    fn only_the_profiles_own_rpc_is_the_default() {
        assert!(settings(&["petri"], Default::default()).default_rpc());
        assert!(
            !settings(
                &["petri", "--rpc", "https://rpc.example"],
                Default::default()
            )
            .default_rpc()
        );
        let file = petri_config::FuturesConfig {
            rpc_url: Some("https://rpc.example".into()),
            ..Default::default()
        };
        assert!(!settings(&["petri"], file).default_rpc());
    }

    /// A release build (no `dev-chain`) with no config talks to production.
    #[cfg(not(feature = "dev-chain"))]
    #[test]
    fn a_release_build_defaults_to_production() {
        assert_eq!(Profile::BUILT_IN, Profile::Production);
        let s = settings(&["petri"], Default::default());
        assert_eq!(s.profile, (Profile::Production, Source::Default));
        assert_eq!(
            s.api_url,
            ("https://api.amoeba.farm".to_string(), Source::Default)
        );
        assert_eq!(
            s.rpc_url,
            (
                "https://api.mainnet-beta.solana.com".to_string(),
                Source::Default
            )
        );
        assert_eq!(s.profile.0.cluster(), "mainnet-beta");
    }

    /// The dev VM's petri (`--features dev-chain`, scripts/install-dev.sh)
    /// with no config talks to the dev chain.
    #[cfg(feature = "dev-chain")]
    #[test]
    fn a_dev_build_defaults_to_the_dev_chain() {
        assert_eq!(Profile::BUILT_IN, Profile::Dev);
        let s = settings(&["petri"], Default::default());
        assert_eq!(s.profile, (Profile::Dev, Source::Default));
        assert_eq!(
            s.api_url,
            ("http://127.0.0.1:4500".to_string(), Source::Default)
        );
        assert_eq!(
            s.rpc_url,
            ("http://127.0.0.1:8899".to_string(), Source::Default)
        );
        assert_eq!(s.profile.0.cluster(), "localnet");
    }

    #[test]
    fn the_profile_comes_from_the_flag_then_the_file() {
        let s = settings(&["petri", "--profile", "dev"], Default::default());
        assert_eq!(s.profile, (Profile::Dev, Source::Flag));
        assert_eq!(s.api_url.0, DEV_API_URL);
        assert_eq!(s.rpc_url.0, DEV_RPC_URL);

        let file = petri_config::FuturesConfig {
            profile: Some("localnet".into()),
            ..Default::default()
        };
        let s = settings(&["petri"], file.clone());
        assert_eq!(s.profile, (Profile::Dev, Source::File));
        assert_eq!(s.api_url.0, DEV_API_URL);
        let s = settings(&["petri", "--profile", "production"], file);
        assert_eq!(s.profile, (Profile::Production, Source::Flag));
        assert_eq!(s.api_url.0, PRODUCTION_API_URL);
        assert_eq!(s.rpc_url.0, PRODUCTION_RPC_URL);
    }

    #[test]
    fn explicit_urls_win_over_the_profile() {
        let s = settings(
            &[
                "petri",
                "--profile",
                "production",
                "--rpc",
                "https://rpc.example/",
                "--api",
                "https://api.example",
            ],
            Default::default(),
        );
        assert_eq!(s.rpc_url, ("https://rpc.example".into(), Source::Flag));
        assert_eq!(s.api_url, ("https://api.example".into(), Source::Flag));
        let file = petri_config::FuturesConfig {
            rpc_url: Some("https://saved.example".into()),
            ..Default::default()
        };
        let s = settings(&["petri", "--profile", "production"], file);
        assert_eq!(s.rpc_url, ("https://saved.example".into(), Source::File));
    }

    #[test]
    fn profile_names_parse() {
        for (text, p) in [
            ("production", Profile::Production),
            ("prod", Profile::Production),
            ("mainnet", Profile::Production),
            ("dev", Profile::Dev),
            ("localnet", Profile::Dev),
            (" DEV ", Profile::Dev),
        ] {
            assert_eq!(Profile::parse(text), Some(p), "{text}");
        }
        assert_eq!(Profile::parse("devnet"), None);
    }
}
