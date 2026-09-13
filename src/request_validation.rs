//! Canonical public request scalars shared by CLI, TUI and MCP adapters.
use crate::backend::CliError;
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub(crate) fn canonical_u64_string(
    raw: &str,
    label: &str,
    allow_zero: bool,
) -> Result<String, CliError> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CliError::new(format!(
            "{label} must be a canonical unsigned decimal"
        )));
    }
    let value = raw
        .parse::<u64>()
        .map_err(|_| CliError::new(format!("{label} is outside the u64 range")))?;
    if !allow_zero && value == 0 {
        return Err(CliError::new(format!("{label} must be greater than zero")));
    }
    Ok(value.to_string())
}

pub(crate) fn canonical_pubkey_string(raw: &str, label: &str) -> Result<String, CliError> {
    let trimmed = raw.trim();
    let pubkey = Pubkey::from_str(trimmed)
        .map_err(|error| CliError::new(format!("invalid {label} public key: {error}")))?;
    if pubkey.to_string() != trimmed {
        return Err(CliError::new(format!(
            "{label} public key is not canonical"
        )));
    }
    Ok(trimmed.to_string())
}
