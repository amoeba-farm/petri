use std::{collections::BTreeSet, str::FromStr};

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

use crate::backend::CliError;

const CURRENT_CATALOG_SCHEMA_VERSION: u64 = 2;
const CURRENT_MATURITY_SOURCE: &str = "exact_current_versioned_on_chain_state";
const DEVNET_GENESIS_HASH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const DEVNET_QUOTE_MINT: &str = "21Ft8EZpugvFofW9713vLnYDRfSqyVUGUo9wvvUGhTsZ";
const DEVNET_QUOTE_DECIMALS: u8 = 6;

#[derive(Clone, Debug, Deserialize)]
struct CatalogQuoteAsset {
    mint: String,
    decimals: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct CatalogIdentityAccount {
    pub role: String,
    pub address: String,
    pub owner: String,
    pub executable: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogNetworkIdentity {
    #[serde(rename = "genesisHash")]
    genesis_hash: String,
    #[serde(rename = "lightAccounts")]
    light_accounts: Vec<CatalogIdentityAccount>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogRollingIssuancePolicy {
    #[serde(rename = "stateNamespaceSeedUtf8")]
    state_namespace_seed_utf8: String,
    #[serde(rename = "maturitySource")]
    maturity_source: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CurrentIdentityCatalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u64,
    cluster: String,
    #[serde(rename = "programId")]
    program_id: String,
    #[serde(rename = "networkIdentity")]
    network_identity: CatalogNetworkIdentity,
    #[serde(rename = "quoteAsset")]
    quote_asset: CatalogQuoteAsset,
    #[serde(rename = "rollingIssuancePolicy")]
    rolling_issuance_policy: CatalogRollingIssuancePolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogChainIdentity {
    pub cluster: String,
    pub genesis_hash: String,
    pub program_id: String,
    pub quote_mint: String,
    pub quote_decimals: u8,
    pub light_accounts: Vec<CatalogIdentityAccount>,
}

// This embedded file contributes only closed deployment identity. Markets and maturities are
// discovered from exact current on-chain state through the backend; catalog rows are never parsed
// into client-side market addresses or lifecycle fallbacks.
const EMBEDDED_CATALOG_JSON: &str = include_str!("../config/options_catalog.json");

fn load_identity_catalog() -> Result<CurrentIdentityCatalog, CliError> {
    serde_json::from_str(EMBEDDED_CATALOG_JSON)
        .map_err(|_| CliError::new("Petri's embedded deployment identity is invalid"))
}

fn project_chain_identity(
    catalog: CurrentIdentityCatalog,
) -> Result<CatalogChainIdentity, CliError> {
    let namespace = std::str::from_utf8(ameba_sdk::constants::CURRENT_STATE_NAMESPACE_SEED)
        .map_err(|_| CliError::new("Petri's current program namespace is invalid"))?;
    let current_program = ameba_sdk::ID.to_string();
    if catalog.schema_version != CURRENT_CATALOG_SCHEMA_VERSION
        || catalog.cluster != "devnet"
        || catalog.network_identity.genesis_hash != DEVNET_GENESIS_HASH
        || catalog.program_id != current_program
        || catalog.quote_asset.mint != DEVNET_QUOTE_MINT
        || catalog.quote_asset.decimals != DEVNET_QUOTE_DECIMALS
        || catalog.rolling_issuance_policy.state_namespace_seed_utf8 != namespace
        || catalog.rolling_issuance_policy.maturity_source != CURRENT_MATURITY_SOURCE
        || catalog.network_identity.light_accounts.is_empty()
    {
        return Err(CliError::new(
            "Petri's embedded deployment identity is not the current Devnet program",
        ));
    }
    Pubkey::from_str(&catalog.program_id)
        .map_err(|_| CliError::new("Petri's embedded program identity is invalid"))?;
    Pubkey::from_str(&catalog.quote_asset.mint)
        .map_err(|_| CliError::new("Petri's embedded quote mint identity is invalid"))?;
    Pubkey::from_str(&catalog.network_identity.genesis_hash)
        .map_err(|_| CliError::new("Petri's embedded Devnet genesis identity is invalid"))?;

    let mut addresses = BTreeSet::new();
    for account in &catalog.network_identity.light_accounts {
        if account.role.trim().is_empty()
            || account.role != account.role.trim()
            || Pubkey::from_str(&account.address).is_err()
            || Pubkey::from_str(&account.owner).is_err()
            || !addresses.insert(account.address.clone())
        {
            return Err(CliError::new(
                "Petri's embedded Light account identity is invalid",
            ));
        }
    }

    Ok(CatalogChainIdentity {
        cluster: catalog.cluster,
        genesis_hash: catalog.network_identity.genesis_hash,
        program_id: catalog.program_id,
        quote_mint: catalog.quote_asset.mint,
        quote_decimals: catalog.quote_asset.decimals,
        light_accounts: catalog.network_identity.light_accounts,
    })
}

pub fn catalog_chain_identity() -> Result<CatalogChainIdentity, CliError> {
    project_chain_identity(load_identity_catalog()?)
}
