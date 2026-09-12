//! Fail-closed boundary for retired generic transaction submission.
//!
//! Petri reads chain state only through Amoeba's read gateway. Product
//! mutations must use an operation-specific Amoeba prepare/submit/status route,
//! so this module intentionally owns no JSON-RPC transport.

use crate::{backend::CliError, onchain::OnchainConfig};

// This explicit fail-closed seam is intentionally callable by regression tests
// even though no current production dispatch reaches generic submission.
#[allow(dead_code)]
pub fn submit_and_confirm_signed_transaction(
    _config: &OnchainConfig,
    _signed_transaction_base64: &str,
) -> Result<String, CliError> {
    crate::current_release::require_current_write_release()?;
    Err(CliError::new(
        "Generic transaction submission is not_wired; use a supported typed operation.",
    ))
}
