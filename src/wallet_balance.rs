use std::{str::FromStr, time::Duration};

use serde_json::{Value, json};
use solana_pubkey::Pubkey;

use crate::{
    backend::{CliError, string_at_key, value_at_key},
    catalog::catalog_chain_identity,
    onchain::{OnchainConfig, resolve_rpc_url},
};

pub const DEFAULT_AMBA_MINT: &str = "Hy1LfQLL4zLQihmiQKZm7DQzXm5K8aVSfKtdHFYXbKMm";
pub const DEFAULT_AMBA_VAULT_TOKEN_ACCOUNT: &str = "CawSd1hKG9fBbBnNDQRnhtP5oHMWwy6quWFj4EWxxxMz";

pub fn resolve_wallet_usdc_mint(
    _network: &str,
    explicit_mint: Option<&str>,
) -> Result<String, CliError> {
    if let Some(mint) = explicit_mint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(mint.to_string());
    }
    Ok(catalog_chain_identity()?.quote_mint)
}

pub fn resolve_wallet_amba_mint(explicit_mint: Option<&str>) -> String {
    if let Some(mint) = explicit_mint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return mint.to_string();
    }
    for env_key in ["AMBA_MINT", "DLMM_ORACLE_AMBA_MINT"] {
        if let Ok(mint) = std::env::var(env_key) {
            let mint = mint.trim().to_string();
            if !mint.is_empty() {
                return mint;
            }
        }
    }
    DEFAULT_AMBA_MINT.to_string()
}

pub fn resolve_amba_vault_token_account(explicit_account: Option<&str>) -> String {
    if let Some(account) = explicit_account
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return account.to_string();
    }
    for env_key in [
        "AMBA_VAULT_TOKEN_ACCOUNT",
        "DLMM_ORACLE_AMBA_VAULT_TOKEN_ACCOUNT",
    ] {
        if let Ok(account) = std::env::var(env_key) {
            let account = account.trim().to_string();
            if !account.is_empty() {
                return account;
            }
        }
    }
    DEFAULT_AMBA_VAULT_TOKEN_ACCOUNT.to_string()
}

pub fn read_wallet_balance(
    config: &OnchainConfig,
    owner_pubkey: &str,
    usdc_mint: &str,
    amba_mint: &str,
) -> Result<Value, CliError> {
    Pubkey::from_str(owner_pubkey)
        .map_err(|error| CliError::new(format!("invalid owner pubkey {owner_pubkey}: {error}")))?;
    Pubkey::from_str(usdc_mint)
        .map_err(|error| CliError::new(format!("invalid USDC mint {usdc_mint}: {error}")))?;
    Pubkey::from_str(amba_mint)
        .map_err(|error| CliError::new(format!("invalid AMBA mint {amba_mint}: {error}")))?;
    crate::chain_identity::verify_onchain_config(config)?;
    let verified_quote_mint = catalog_chain_identity()?.quote_mint;
    if usdc_mint != verified_quote_mint {
        return Err(CliError::new(
            "the requested quote mint does not match the verified Amoeba deployment",
        ));
    }
    let rpc_url = resolve_rpc_url(config)?;
    let balance_response = post_solana_rpc(
        &rpc_url,
        "getBalance",
        json!([owner_pubkey, { "commitment": "confirmed" }]),
    )?;
    let lamports = value_at_path(&balance_response, &["result", "value"])
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usdc_response = read_wallet_token_accounts(&rpc_url, owner_pubkey, usdc_mint)?;
    let amba_response = read_wallet_token_accounts(&rpc_url, owner_pubkey, amba_mint)?;
    let usdc_amount = sum_json_parsed_token_accounts(&usdc_response);
    let amba_amount = sum_json_parsed_token_accounts(&amba_response);

    Ok(json!({
        "ownerPubkey": owner_pubkey,
        "solLamports": lamports,
        "solAmount": lamports as f64 / 1_000_000_000.0,
        "usdcMint": usdc_mint,
        "usdcAmount": usdc_amount,
        "ambaMint": amba_mint,
        "ambaAmount": amba_amount,
        "commitment": "confirmed",
    }))
}

fn read_wallet_token_accounts(
    rpc_url: &str,
    owner_pubkey: &str,
    mint: &str,
) -> Result<Value, CliError> {
    post_solana_rpc(
        rpc_url,
        "getTokenAccountsByOwner",
        json!([
            owner_pubkey,
            { "mint": mint },
            { "encoding": "jsonParsed", "commitment": "confirmed" }
        ]),
    )
}

fn post_solana_rpc(rpc_url: &str, method: &str, params: Value) -> Result<Value, CliError> {
    let http = crate::backend::pinned_blocking_http_client_builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| CliError::new(format!("failed to build RPC client: {error}")))?;
    let response = http
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": format!("petri-cli:{method}"),
            "method": method,
            "params": params,
        }))
        .send()
        .map_err(|_| CliError::new(format!("Amoeba chain read {method} failed")))?;
    let status = response.status();
    let payload = response.json::<Value>().map_err(|error| {
        CliError::new(format!(
            "failed to parse Amoeba chain read {method} response: {error}"
        ))
    })?;
    if !status.is_success() {
        return Err(CliError::new(format!(
            "Amoeba chain read {method} returned HTTP {status}"
        )));
    }
    if let Some(error) = value_at_key(&payload, &["error"]) {
        let message = string_at_key(error, &["message"]).unwrap_or_else(|| "RPC error".to_string());
        return Err(CliError::new(format!(
            "Amoeba chain read {method} failed: {message}"
        )));
    }
    Ok(payload)
}

pub fn sum_json_parsed_token_accounts(payload: &Value) -> f64 {
    let accounts = value_at_path(payload, &["result", "value"])
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut sum = 0.0;
    for account in accounts {
        let token_amount = value_at_path(
            &account,
            &["account", "data", "parsed", "info", "tokenAmount"],
        );
        if let Some(amount) = token_amount.and_then(|value| value_at_key(value, &["uiAmount"])) {
            if let Some(value) = amount.as_f64() {
                sum += value;
                continue;
            }
        }
        if let Some(amount) =
            token_amount.and_then(|value| string_at_key(value, &["uiAmountString"]))
        {
            if let Ok(value) = amount.parse::<f64>() {
                if value.is_finite() {
                    sum += value;
                }
            }
        }
    }
    (sum * 1_000_000.0).round() / 1_000_000.0
}

pub fn render_wallet_balance(payload: &Value) -> String {
    let owner = string_at_key(payload, &["ownerPubkey"]).unwrap_or_else(|| "-".to_string());
    let sol = value_at_key(payload, &["solAmount"])
        .and_then(Value::as_f64)
        .map(format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    let usdc = value_at_key(payload, &["usdcAmount"])
        .and_then(Value::as_f64)
        .map(format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    let amba = value_at_key(payload, &["ambaAmount"])
        .and_then(Value::as_f64)
        .map(format_wallet_number)
        .unwrap_or_else(|| "-".to_string());
    let usdc_mint = string_at_key(payload, &["usdcMint"]).unwrap_or_else(|| "-".to_string());
    let amba_mint = string_at_key(payload, &["ambaMint"]).unwrap_or_else(|| "-".to_string());
    let commitment = string_at_key(payload, &["commitment"]).unwrap_or_else(|| "-".to_string());

    [
        format!("wallet={owner}"),
        format!(
            "sol={sol} | usdc={usdc} | amba={amba} | usdcMint={usdc_mint} | ambaMint={amba_mint} | commitment={commitment}"
        ),
    ]
    .join("\n")
}

pub fn format_wallet_number(value: f64) -> String {
    if !value.is_finite() {
        return "-".to_string();
    }
    if value.fract().abs() < 0.000_001 {
        format!("{value:.0}")
    } else {
        let mut text = format!("{value:.6}");
        while text.contains('.') && text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        text
    }
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}
