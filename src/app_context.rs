//! Resolve shared CLI/TUI/MCP configuration without dispatching or accessing a signer.
use crate::{backend::CliError, cli::Cli, onchain::OnchainConfig, solana_config};

pub(crate) fn build_onchain_config(cli: &Cli) -> Result<OnchainConfig, CliError> {
    let solana_cli_config = solana_config::load_solana_cli_config(cli.solana_config.as_deref())?;
    Ok(OnchainConfig {
        network: cli.cluster.clone(),
        backend_url: cli.backend_url.clone(),
        commitment: solana_config::resolve_commitment(
            cli.commitment.as_deref(),
            solana_cli_config.as_ref(),
        ),
        keypair_path: Some(solana_config::resolve_keypair_path(
            cli.keypair.as_deref(),
            solana_cli_config.as_ref(),
        )),
        allow_insecure_keypair: cli.allow_insecure_keypair,
    })
}
