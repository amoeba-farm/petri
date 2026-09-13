//! Standalone, explicitly confirmed preview updates. No Git, wallet, service
//! configuration, signing keys, publisher-trust overrides, or install scripts.
//! Trust is the fixed GitHub repository over TLS plus its asset digest; these
//! preview packages are NOT publisher-signed. Signed/source installers stay separate.

mod archive;
mod install;
mod platform;

use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

pub type Result<T> = std::result::Result<T, String>;
const REPOSITORY: &str = "amoeba-farm/petri";
const MAX_METADATA: u64 = 1024 * 1024;
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
const MAX_UNPACKED: u64 = 512 * 1024 * 1024;
const HELPER_ARG: &str = "--petri-apply-update";

pub fn enabled() -> bool {
    option_env!("PETRI_UPDATE_CHANNEL") == Some("preview")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    WindowsX64,
    MacosArm64,
    #[serde(rename = "macos-x86_64")]
    MacosX86_64,
}

impl Platform {
    fn current() -> Result<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Ok(Self::WindowsX64),
            ("macos", "aarch64") => Ok(Self::MacosArm64),
            ("macos", "x86_64") => Ok(Self::MacosX86_64),
            _ => Err("Standalone updates are supported on Windows x64 and macOS only.".into()),
        }
    }
    fn stem(self) -> &'static str {
        match self {
            Self::WindowsX64 => "Petri-windows-x64",
            Self::MacosArm64 => "Petri-macos-arm64",
            Self::MacosX86_64 => "Petri-macos-x86_64",
        }
    }
    fn executable(self) -> &'static str {
        match self {
            Self::WindowsX64 => "petri.exe",
            _ => "Contents/Resources/petri",
        }
    }
    fn helper(self) -> &'static str {
        match self {
            Self::WindowsX64 => "petri-update-helper.exe",
            _ => "petri-update-helper",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct GithubAsset {
    id: u64,
    name: String,
    size: u64,
    state: String,
    browser_download_url: String,
    digest: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub platform: Platform,
    pub release_url: String,
    release_id: u64,
    asset_id: u64,
    checksum_id: u64,
    archive_size: u64,
    archive_sha256: String,
    checksum_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub ok: bool,
    pub channel: &'static str,
    pub status: &'static str,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub message: String,
    pub release: Option<Release>,
}

fn report(status: &'static str, release: Option<Release>, message: String) -> Report {
    Report {
        ok: true,
        channel: "preview",
        status,
        current_version: env!("CARGO_PKG_VERSION").into(),
        latest_version: release.as_ref().map(|r| r.version.clone()),
        update_available: status == "available",
        message,
        release,
    }
}

fn guard() -> Result<()> {
    if !enabled() {
        return Err("This build uses the source updater, not preview release updates.".into());
    }
    if std::env::var_os("PETRI_MCP_READ_ONLY").is_some() {
        return Err(
            "Petri updates must be started directly by the user, outside an agent session.".into(),
        );
    }
    Ok(())
}

pub fn check() -> Result<Report> {
    guard()?;
    let platform = Platform::current()?;
    if let Some(message) = install::pending()? {
        return Ok(report("blocked", None, message));
    }
    let release = fetch_release(platform)?;
    let current = version(env!("CARGO_PKG_VERSION"))?;
    if version(&release.version)? <= current {
        return Ok(report(
            "current",
            Some(release),
            "Petri is up to date.".into(),
        ));
    }
    let message = format!(
        "Petri {} is available. Run petri update to review and install it.",
        release.version
    );
    Ok(report("available", Some(release), message))
}

pub fn information() -> serde_json::Value {
    serde_json::json!({"protocol": 1, "channel": if enabled() { "preview" } else { "source" },
        "version": env!("CARGO_PKG_VERSION"), "platform": Platform::current().ok()})
}

/// The caller obtains explicit approval for this exact Release before calling.
pub fn prepare(approved: &Release, restart: bool) -> Result<Report> {
    guard()?;
    if approved.platform != Platform::current()?
        || version(&approved.version)? <= version(env!("CARGO_PKG_VERSION"))?
    {
        return Err("The selected update is not a newer release for this computer.".into());
    }
    if fetch_release(approved.platform)? != *approved {
        return Err("The release changed after review. Nothing was installed; check and approve the update again.".into());
    }
    install::stage_and_launch(approved, restart)?;
    Ok(report("scheduled", Some(approved.clone()),
        "Update verified. Petri will close, install the update, and keep a recovery copy. Wallets and settings are unchanged.".into()))
}

pub fn recover(restart: bool) -> Result<Report> {
    guard()?;
    install::launch_recovery(Platform::current()?, restart)?;
    Ok(report("scheduled", None, "Petri will close and restore the previous installation. Wallets and settings are unchanged.".into()))
}

pub fn helper_entry() -> Option<Result<()>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|arg| arg != HELPER_ARG) {
        return None;
    }
    Some((|| {
        guard()?;
        let recovering = match args.as_slice() {
            [_] => false,
            [_, operation] if operation == "recover" => true,
            _ => return Err("Invalid Petri update helper invocation.".into()),
        };
        install::run_helper(recovering)
    })())
}

fn version(input: &str) -> Result<[u64; 3]> {
    let parts: Vec<_> = input.split('.').collect();
    if parts.len() != 3 {
        return Err("The release has an unsupported version.".into());
    }
    let mut result = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 9
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err("The release has an invalid version.".into());
        }
        result[index] = part.parse().map_err(|_| "Invalid release version.")?;
    }
    Ok(result)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest(value: &Option<String>) -> Result<String> {
    value
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| valid_hash(s))
        .map(str::to_owned)
        .ok_or_else(|| {
            "The release has no valid GitHub SHA-256 digest. Nothing was installed.".into()
        })
}
fn download_url(release: &Release, checksum: bool) -> String {
    format!(
        "https://github.com/{REPOSITORY}/releases/download/v{}/{}.zip{}",
        release.version,
        release.platform.stem(),
        if checksum { ".sha256" } else { "" }
    )
}

fn parse_release(bytes: &[u8], platform: Platform) -> Result<Release> {
    let data: GithubRelease =
        serde_json::from_slice(bytes).map_err(|_| "Petri release information is invalid.")?;
    if data.id == 0 || data.draft || data.prerelease || data.assets.len() > 32 {
        return Err("The release is not a supported published preview.".into());
    }
    let number = data
        .tag_name
        .strip_prefix('v')
        .ok_or("Invalid release tag.")?;
    version(number)?;
    if data.html_url != format!("https://github.com/{REPOSITORY}/releases/tag/v{number}") {
        return Err("The release does not belong to the trusted Petri repository.".into());
    }
    let find = |name: &str| -> Result<&GithubAsset> {
        let mut matches = data.assets.iter().filter(|asset| asset.name == name);
        let asset = matches
            .next()
            .ok_or("The release does not include this platform's complete download.")?;
        if matches.next().is_some() || asset.id == 0 || asset.state != "uploaded" || asset.size == 0
        {
            return Err("The release has duplicate or incomplete assets.".into());
        }
        Ok(asset)
    };
    let asset = find(&format!("{}.zip", platform.stem()))?;
    let checksum = find(&format!("{}.zip.sha256", platform.stem()))?;
    if asset.size > MAX_ARCHIVE || checksum.size > 256 {
        return Err("The update exceeds the supported download size.".into());
    }
    let release = Release {
        version: number.into(),
        platform,
        release_url: data.html_url.clone(),
        release_id: data.id,
        asset_id: asset.id,
        checksum_id: checksum.id,
        archive_size: asset.size,
        archive_sha256: digest(&asset.digest)?,
        checksum_sha256: digest(&checksum.digest)?,
    };
    if asset.browser_download_url != download_url(&release, false)
        || checksum.browser_download_url != download_url(&release, true)
    {
        return Err("The download does not belong to the trusted Petri release.".into());
    }
    Ok(release)
}

fn safe_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some("github.com" | "release-assets.githubusercontent.com")
        )
}

fn client(download: bool) -> Result<Client> {
    Client::builder()
        .user_agent(concat!("Petri/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(if download { 180 } else { 15 }))
        .redirect(if download {
            Policy::custom(|attempt| {
                if attempt.previous().len() < 4 && safe_redirect(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("Untrusted Petri update redirect")
                }
            })
        } else {
            Policy::none()
        })
        .build()
        .map_err(|_| "Could not initialize the secure update connection.".into())
}

fn fetch_release(platform: Platform) -> Result<Release> {
    let response = client(false)?
        .get(format!(
            "https://api.github.com/repos/{REPOSITORY}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|_| "Could not reach Petri releases. Try again later.")?;
    if !response.status().is_success() {
        return Err("Petri release checks are temporarily unavailable. Try again later.".into());
    }
    parse_release(&read_bounded(response, MAX_METADATA)?, platform)
}

fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The update download was interrupted.")?;
    if bytes.len() as u64 > limit {
        return Err("The update data exceeds its size limit.".into());
    }
    Ok(bytes)
}

fn download(release: &Release, destination: &Path) -> Result<()> {
    let client = client(true)?;
    let checksum = client
        .get(download_url(release, true))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| "Could not download the release checksum.")?;
    let bytes = read_bounded(checksum, 256)?;
    if format!("{:x}", Sha256::digest(&bytes)) != release.checksum_sha256 {
        return Err("The checksum asset differs from the approved GitHub release.".into());
    }
    let expected = format!(
        "{}  {}.zip",
        release.archive_sha256,
        release.platform.stem()
    );
    if std::str::from_utf8(&bytes).ok().map(str::trim) != Some(expected.as_str()) {
        return Err("The release checksums disagree. Nothing was installed.".into());
    }
    let response = client
        .get(download_url(release, false))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| "Could not download the Petri update.")?;
    let mut reader = response.take(release.archive_size + 1);
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not stage the update.")?;
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| "The update download was interrupted.")?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > release.archive_size {
            return Err("The update is larger than the approved release.".into());
        }
        file.write_all(&buffer[..read])
            .map_err(|_| "Could not save the update. Check free disk space.")?;
        hash.update(&buffer[..read]);
    }
    file.sync_all()
        .map_err(|_| "Could not finish saving the update.")?;
    if count != release.archive_size || format!("{:x}", hash.finalize()) != release.archive_sha256 {
        return Err(
            "The update is incomplete or its SHA-256 does not match. Nothing was installed.".into(),
        );
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|_| "Could not read an update file.")?;
    if file
        .metadata()
        .map_err(|_| "Could not inspect an update file.")?
        .len()
        > MAX_UNPACKED
    {
        return Err("An update file exceeds its size limit.".into());
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|_| "Could not verify an update file.")?;
    Ok(format!("{:x}", hash.finalize()))
}
