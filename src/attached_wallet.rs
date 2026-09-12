use crate::{cli::Cli, solana_config, wallet_signer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WalletPathSource {
    Explicit,
    SolanaConfig,
    Default,
}

impl WalletPathSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Explicit => "--keypair / SOLANA_KEYPAIR",
            Self::SolanaConfig => "Solana CLI config",
            Self::Default => "default Solana keypair",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttachedWallet {
    pub(crate) pubkey: Option<String>,
    pub(crate) keypair_path: String,
    pub(crate) path_source: WalletPathSource,
    pub(crate) keypair_file: String,
    pub(crate) issue: Option<String>,
}

impl AttachedWallet {
    pub(crate) fn is_attached(&self) -> bool {
        self.pubkey.is_some()
    }

    pub(crate) fn account_label(&self) -> &str {
        self.pubkey.as_deref().unwrap_or("not attached")
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if self.is_attached() {
            "attached"
        } else {
            "not attached"
        }
    }
}

pub(crate) fn inspect_attached_wallet(cli: &Cli) -> AttachedWallet {
    let (solana_cli_config, config_issue) =
        match solana_config::load_solana_cli_config(cli.solana_config.as_deref()) {
            Ok(config) => (config, None),
            Err(error) => (None, Some(format!("Solana config unavailable: {error}"))),
        };
    let keypair_path =
        solana_config::resolve_keypair_path(cli.keypair.as_deref(), solana_cli_config.as_ref());
    let path_source = wallet_path_source(cli, solana_cli_config.as_ref());
    let keypair_file = solana_config::keypair_file_security_label(&keypair_path);

    match wallet_signer::signer_pubkey_from_path(&keypair_path) {
        Ok(pubkey) => AttachedWallet {
            pubkey: Some(pubkey),
            keypair_path,
            path_source,
            keypair_file,
            issue: config_issue,
        },
        Err(error) => AttachedWallet {
            pubkey: None,
            keypair_path,
            path_source,
            keypair_file,
            issue: Some(
                config_issue
                    .map(|issue| format!("{issue}; wallet unavailable: {error}"))
                    .unwrap_or_else(|| format!("wallet unavailable: {error}")),
            ),
        },
    }
}

pub(crate) fn inspect_keypair_path(keypair_path: String) -> AttachedWallet {
    let keypair_file = solana_config::keypair_file_security_label(&keypair_path);
    match wallet_signer::signer_pubkey_from_path(&keypair_path) {
        Ok(pubkey) => AttachedWallet {
            pubkey: Some(pubkey),
            keypair_path,
            path_source: WalletPathSource::Explicit,
            keypair_file,
            issue: None,
        },
        Err(error) => AttachedWallet {
            pubkey: None,
            keypair_path,
            path_source: WalletPathSource::Explicit,
            keypair_file,
            issue: Some(format!("wallet unavailable: {error}")),
        },
    }
}

fn wallet_path_source(
    cli: &Cli,
    solana_cli_config: Option<&solana_config::SolanaCliConfig>,
) -> WalletPathSource {
    if cli
        .keypair
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return WalletPathSource::Explicit;
    }
    if solana_cli_config
        .and_then(|config| config.keypair_path.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return WalletPathSource::SolanaConfig;
    }
    WalletPathSource::Default
}
