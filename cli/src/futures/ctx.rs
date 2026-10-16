//! Per-invocation context for futures commands: settings, clients, the
//! trusted deployment, the market list and the attached wallet.

use std::{cell::OnceCell, str::FromStr};

use solana_program::pubkey::Pubkey;
use solana_signer::Signer;

use crate::{onchain::OnchainConfig, wallet_signer};

use super::{
    api::Api,
    error::{Error, Result},
    market::{self, Deployment},
    rpc::Rpc,
    settings::Settings,
};

pub struct Ctx {
    pub s: Settings,
    pub rpc: Rpc,
    pub api: Api,
    deployment: OnceCell<Deployment>,
    pub markets: OnceCell<Vec<super::api::MarketView>>,
}

impl Ctx {
    pub fn new(s: Settings) -> Self {
        let rpc = Rpc::new(&s.rpc_url.0);
        let api = Api::new(&s.api_url.0);
        Self {
            s,
            rpc,
            api,
            deployment: OnceCell::new(),
            markets: OnceCell::new(),
        }
    }

    /// The manifest given with `--deployment` (or saved), else the futures
    /// API's `/v1/config`, held to the profile (the production ids in the
    /// production profile).
    pub fn deployment(&self) -> Result<&Deployment> {
        if let Some(d) = self.deployment.get() {
            return Ok(d);
        }
        let profile = self.s.profile.0;
        let d = match &self.s.deployment {
            Some((path, _)) => Deployment::from_manifest(path, profile)?,
            None => Deployment::from_api(&self.api, profile)?,
        };
        Ok(self.deployment.get_or_init(|| d))
    }

    fn onchain_config(&self) -> OnchainConfig {
        OnchainConfig {
            keypair_path: Some(self.s.keypair.0.clone()),
            allow_insecure_keypair: self.s.allow_insecure_keypair,
        }
    }

    /// The attached wallet as a signer (file or hardware wallet), with
    /// Petri's keypair-file safety checks.
    pub fn signer(&self) -> Result<Box<dyn Signer>> {
        self.no_wallet()?;
        wallet_signer::load_signer(&self.onchain_config())
            .map_err(|e| Error::msg(format!("can't sign with {}: {e}", self.s.keypair.0)))
    }

    pub fn wallet_pubkey(&self) -> Result<Pubkey> {
        self.no_wallet()?;
        let text = wallet_signer::signer_pubkey_from_path(&self.s.keypair.0).map_err(|e| {
            Error::msg(format!(
                "can't read the wallet at {}: {e}",
                self.s.keypair.0
            ))
        })?;
        market::pubkey(&text, "wallet")
    }

    /// A clear first step when the wallet file is not there at all.
    fn no_wallet(&self) -> Result<()> {
        let path = &self.s.keypair.0;
        if path.starts_with("usb://") || std::path::Path::new(path).exists() {
            return Ok(());
        }
        Err(Error::msg(format!(
            "no wallet yet at {path}; create one with `petri wallet new`, or pass --keypair <PATH>"
        )))
    }

    /// `owner` if given, else the attached wallet's public key.
    pub fn owner(&self, owner: Option<&str>) -> Result<Pubkey> {
        match owner {
            Some(text) => market::pubkey(text, "owner"),
            None => self.wallet_pubkey(),
        }
    }

    pub fn parse_pubkey(&self, text: &str, what: &str) -> Result<Pubkey> {
        Pubkey::from_str(text.trim())
            .map_err(|_| Error::msg(format!("`{text}` is not a valid {what} address")))
    }
}
