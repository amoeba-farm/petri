//! One error type for the CLI, and readable names for program errors.

use std::fmt;

use ameba_sdk_futures::futures;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub struct Error {
    pub message: String,
    /// Program logs worth showing (the failing program's lines).
    pub logs: Vec<String>,
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            logs: Vec::new(),
        }
    }

    pub fn with_logs(mut self, logs: Vec<String>) -> Self {
        self.logs = logs;
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<super::units::UnitError> for Error {
    fn from(e: super::units::UnitError) -> Self {
        Error::msg(e.0)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::msg(e.to_string())
    }
}

pub fn bail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::msg(message))
}

/// SPL Token program error names (`TokenError` order).
const TOKEN_ERRORS: [&str; 20] = [
    "NotRentExempt",
    "InsufficientFunds",
    "InvalidMint",
    "MintMismatch",
    "OwnerMismatch",
    "FixedSupply",
    "AlreadyInUse",
    "InvalidNumberOfProvidedSigners",
    "InvalidNumberOfRequiredSigners",
    "UninitializedState",
    "NativeNotSupported",
    "NonNativeHasBalance",
    "InvalidInstruction",
    "InvalidState",
    "Overflow",
    "AuthorityTypeNotSupported",
    "MintCannotFreeze",
    "AccountFrozen",
    "MintDecimalsMismatch",
    "NonNativeNotSupported",
];

/// Which program raised a custom error, as far as the logs tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raiser {
    Futures,
    Token,
    Unknown,
}

/// A readable name for a custom program error code: governance verifier
/// codes `3100..`, futures codes `0x46520000 + n`, token codes by name.
pub fn custom_error_name(code: u32, raiser: Raiser) -> String {
    if raiser != Raiser::Token
        && let Some(name) = futures::governance::error_name(code)
    {
        return format!("{name} (governance error {code})");
    }
    let base = futures::ERROR_BASE;
    if raiser != Raiser::Token
        && code >= base
        && code < base + 0x100
        && let Some(name) = futures::error_name(code)
    {
        return format!("{name} (futures error {code:#x})");
    }
    if raiser == Raiser::Token
        && let Some(name) = TOKEN_ERRORS.get(code as usize)
    {
        return format!("Token{name} (token error {code})");
    }
    format!("custom program error {code:#x}")
}

/// Plain-language hints for the errors a trader is most likely to hit.
pub fn hint(name: &str) -> Option<&'static str> {
    let name = name.split(' ').next().unwrap_or(name);
    Some(match name {
        // DEV-ONLY: REMOVE FOR PROD (the dev build's `petri faucet`).
        "InsufficientFunds" | "TokenInsufficientFunds" if cfg!(feature = "dev-faucet") => {
            "not enough USDC; run `petri faucet` (dev) or `petri deposit`"
        }
        "InsufficientFunds" | "TokenInsufficientFunds" => "not enough USDC; run `petri deposit`",
        "MinFill" => "nothing filled inside the limit; quote again or raise --slippage",
        "WouldCross" => "a post-only order would fill now; leave out --post to trade",
        "NotTradable" => {
            "the market is not trading now (its oracle period is not live, or it expired)"
        }
        "BadProposal" | "NotBestPrice" | "NotMaxFill" | "FifoViolation" | "Unbalanced" => {
            "the book changed while sending; try again"
        }
        "ClaimReserved" => "a resting close order holds part of this position; cancel it first",
        "ClaimNotFound" | "OrderNotFound" => "it changed or is gone; see `petri positions`",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn futures_governance_and_token_codes_have_names() {
        let base = futures::ERROR_BASE;
        assert_eq!(
            custom_error_name(base + 4, Raiser::Futures),
            "NotTradable (futures error 0x46520004)"
        );
        assert_eq!(
            custom_error_name(base + 18, Raiser::Unknown),
            "MinFill (futures error 0x46520012)"
        );
        assert_eq!(
            custom_error_name(1, Raiser::Token),
            "TokenInsufficientFunds (token error 1)"
        );
        assert_eq!(
            custom_error_name(999, Raiser::Unknown),
            "custom program error 0x3e7"
        );
        assert_eq!(
            custom_error_name(3108, Raiser::Futures),
            "GovernanceGateFrozen (governance error 3108)"
        );
        assert_eq!(
            custom_error_name(3109, Raiser::Token),
            "custom program error 0xc25"
        );
        // The retired clearing program's codes have no names any more.
        assert_eq!(
            custom_error_name(0x4643_0008, Raiser::Unknown),
            "custom program error 0x46430008"
        );
        assert!(hint("MinFill (futures error 0x46520012)").is_some());
    }
}
