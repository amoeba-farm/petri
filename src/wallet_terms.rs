use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::backend::CliError;

pub(crate) const TERMS_VERSION: &str = "amoeba-wallet-terms-v1";
pub(crate) const DEFAULT_TERMS_URL: &str = "https://amoeba.farm/terms";

#[derive(Clone, Debug)]
pub(crate) struct WalletTermsStatus {
    pub accepted: bool,
    pub terms_url: String,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
struct TermsCache {
    accepted_wallets: Vec<WalletTermsAcceptance>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WalletTermsAcceptance {
    wallet_pubkey: String,
    terms_version: String,
    terms_url: String,
    accepted_at_unix_seconds: u64,
}

pub(crate) fn terms_url() -> String {
    env::var("AMEBA_TERMS_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TERMS_URL.to_string())
}

pub(crate) fn status_for_wallet(wallet_pubkey: Option<&str>) -> WalletTermsStatus {
    let terms_url = terms_url();
    let Some(wallet_pubkey) = wallet_pubkey.filter(|value| !value.trim().is_empty()) else {
        return WalletTermsStatus {
            accepted: true,
            terms_url,
            issue: None,
        };
    };
    let Some(path) = cache_path() else {
        return WalletTermsStatus {
            accepted: false,
            terms_url,
            issue: Some("Wallet terms cache path is unavailable.".to_string()),
        };
    };
    match status_for_wallet_at_path(wallet_pubkey, &terms_url, &path) {
        Ok(status) => status,
        Err(error) => WalletTermsStatus {
            accepted: false,
            terms_url,
            issue: Some(error.to_string()),
        },
    }
}

pub(crate) fn accept_wallet_terms(wallet_pubkey: &str) -> Result<WalletTermsStatus, CliError> {
    let terms_url = terms_url();
    let path =
        cache_path().ok_or_else(|| CliError::new("wallet terms cache path is unavailable"))?;
    accept_wallet_terms_at_path(wallet_pubkey, &terms_url, &path)
}

pub(crate) fn open_terms_url(terms_url: &str) -> Result<(), CliError> {
    open_external_url(terms_url).map_err(|error| {
        CliError::new(format!(
            "failed to open Terms page. Visit {terms_url} manually. {error}"
        ))
    })
}

pub(crate) fn open_external_url(url: &str) -> Result<(), io::Error> {
    open_url(url)
}

fn status_for_wallet_at_path(
    wallet_pubkey: &str,
    terms_url: &str,
    path: &Path,
) -> Result<WalletTermsStatus, CliError> {
    let cache = read_cache(path)?;
    let accepted = cache
        .accepted_wallets
        .iter()
        .any(|entry| entry.wallet_pubkey == wallet_pubkey && entry.terms_version == TERMS_VERSION);
    Ok(WalletTermsStatus {
        accepted,
        terms_url: terms_url.to_string(),
        issue: None,
    })
}

fn accept_wallet_terms_at_path(
    wallet_pubkey: &str,
    terms_url: &str,
    path: &Path,
) -> Result<WalletTermsStatus, CliError> {
    let wallet_pubkey = wallet_pubkey.trim();
    if wallet_pubkey.is_empty() {
        return Err(CliError::new(
            "cannot accept Terms without a wallet public key",
        ));
    }
    let mut cache = read_cache(path)?;
    cache.accepted_wallets.retain(|entry| {
        !(entry.wallet_pubkey == wallet_pubkey && entry.terms_version == TERMS_VERSION)
    });
    cache.accepted_wallets.push(WalletTermsAcceptance {
        wallet_pubkey: wallet_pubkey.to_string(),
        terms_version: TERMS_VERSION.to_string(),
        terms_url: terms_url.to_string(),
        accepted_at_unix_seconds: now_unix_seconds(),
    });
    write_cache(path, &cache)?;
    Ok(WalletTermsStatus {
        accepted: true,
        terms_url: terms_url.to_string(),
        issue: None,
    })
}

fn read_cache(path: &Path) -> Result<TermsCache, CliError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| CliError::new(format!("failed to parse wallet terms cache: {error}"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(TermsCache::default()),
        Err(error) => Err(CliError::new(format!(
            "failed to read wallet terms cache: {error}"
        ))),
    }
}

fn write_cache(path: &Path, cache: &TermsCache) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::new(format!(
                "failed to create wallet terms cache directory: {error}"
            ))
        })?;
    }
    let contents = serde_json::to_string_pretty(cache).map_err(|error| {
        CliError::new(format!("failed to serialize wallet terms cache: {error}"))
    })?;
    fs::write(path, contents)
        .map_err(|error| CliError::new(format!("failed to write wallet terms cache: {error}")))
}

fn cache_path() -> Option<PathBuf> {
    env::var_os("AMEBA_WALLET_TERMS_CACHE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("APPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("Amoeba").join("Petri").join("wallet_terms.json"))
        })
        .or_else(|| {
            env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("Amoeba").join("Petri").join("wallet_terms.json"))
        })
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("amoeba").join("petri").join("wallet_terms.json"))
        })
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    path.join(".config")
                        .join("amoeba")
                        .join("petri")
                        .join("wallet_terms.json")
                })
        })
}

fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?
            .wait()?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?.wait()?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(url).spawn()?.wait()?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Ok(())
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
