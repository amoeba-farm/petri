//! The deployment identity gate. Before Petri signs anything it checks that
//! the RPC is the cluster the trusted deployment names (genesis hash) and that
//! the deployment's oracle and futures programs are deployed programs there.
//! Oracle writes also check the oracle's config: its own account, current
//! layout, and the deployment's USDC mint as bond currency. A mismatch
//! refuses before anything is signed or sent.

use std::sync::Mutex;

use serde_json::{Value, json};
use solana_program::pubkey::Pubkey;

use crate::futures::{
    ctx::Ctx,
    error::{Result, bail},
};

/// Loaders that own deployed programs.
const PROGRAM_LOADERS: [&str; 3] = [
    "BPFLoaderUpgradeab1e11111111111111111111111",
    "BPFLoader2111111111111111111111111111111111",
    "LoaderV411111111111111111111111111111111111",
];

/// What the gate verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub rpc: String,
    pub genesis_hash: String,
    pub oracle: Pubkey,
    pub futures: Option<Pubkey>,
}

impl Identity {
    pub fn json(&self) -> Value {
        json!({
            "rpc": self.rpc,
            "genesisHash": self.genesis_hash,
            "programs": {
                "oracle": self.oracle.to_string(),
                "futures": self.futures.map(|f| f.to_string()),
            },
        })
    }
}

static VERIFIED: Mutex<Option<Identity>> = Mutex::new(None);

/// Verifies the cluster and the deployment's programs once per process.
pub fn verify(ctx: &Ctx) -> Result<Identity> {
    let dep = ctx.deployment()?;
    if let Some(known) = VERIFIED.lock().ok().and_then(|g| g.clone())
        && known.rpc == ctx.rpc.url()
        && known.oracle == dep.oracle
        && known.futures == dep.futures
        && dep.genesis_hash.as_deref() == Some(known.genesis_hash.as_str())
    {
        crate::futures::governance::validate_deployment(
            dep.governance_controller.as_ref(),
            dep.futures.as_ref(),
            Some(&known.genesis_hash),
            dep.profile == crate::futures::settings::Profile::Dev,
        )?;
        return Ok(known);
    }
    let Some(expected) = dep.genesis_hash.clone() else {
        return bail(format!(
            "refusing to sign: the deployment ({}) names no genesis hash, so the cluster cannot be identified",
            dep.source
        ));
    };
    let actual = ctx.rpc.genesis_hash()?;
    if actual != expected {
        bail(format!(
            "refusing to sign: the RPC {} is cluster {actual}, but the deployment ({}) is on {expected}",
            ctx.rpc.url(),
            dep.source
        ))?;
    }
    crate::futures::governance::validate_deployment(
        dep.governance_controller.as_ref(),
        dep.futures.as_ref(),
        Some(&actual),
        dep.profile == crate::futures::settings::Profile::Dev,
    )?;
    let mut keys = vec![dep.oracle];
    keys.extend(dep.futures);
    let accounts = ctx.rpc.accounts(&keys)?;
    for ((key, account), name) in keys.iter().zip(&accounts).zip(["oracle", "futures"]) {
        let Some(account) = account else {
            return bail(format!(
                "refusing to sign: the {name} program {key} does not exist on this cluster"
            ));
        };
        if !account.executable
            || !PROGRAM_LOADERS
                .iter()
                .any(|loader| account.owner.to_string() == *loader)
        {
            bail(format!(
                "refusing to sign: {key} is not a deployed program (the {name} program of {})",
                dep.source
            ))?;
        }
    }
    let identity = Identity {
        rpc: ctx.rpc.url().to_string(),
        genesis_hash: actual,
        oracle: dep.oracle,
        futures: dep.futures,
    };
    if let Ok(mut slot) = VERIFIED.lock() {
        *slot = Some(identity.clone());
    }
    Ok(identity)
}

/// [`verify`], plus the oracle's config: initialized at its canonical address
/// with the current layout, and bonds in the deployment's USDC mint.
pub fn verify_oracle(ctx: &Ctx) -> Result<(Identity, ameba_oracle::state::OracleConfig)> {
    let identity = verify(ctx)?;
    let oracle = crate::oracle::chain::Oracle::new(ctx)?;
    let config = oracle.config()?;
    let mint = ctx.deployment()?.usdc_mint;
    if config.usdc_mint != mint {
        bail(format!(
            "refusing to sign: the oracle bonds in {} but the deployment's USDC mint is {mint}",
            config.usdc_mint
        ))?;
    }
    Ok((identity, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaders_are_valid_keys() {
        for loader in PROGRAM_LOADERS {
            assert!(loader.parse::<Pubkey>().is_ok(), "{loader}");
        }
    }
}
