//! What an agent's commands (agent/) and the research workspace service
//! (service/) share, as the Node service's `security.mjs` had it: refusals,
//! the private storage (the same folders, marker and files), addresses,
//! request checks and random identifiers.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// A refusal or failure, as the research commands and the workspace report
/// it: a code, a message, and the HTTP status of a refusal. Without a status
/// it is an unexpected failure, whose message stays inside.
#[derive(Debug)]
pub struct Failure {
    pub code: Option<String>,
    pub message: String,
    pub status: Option<u16>,
}

pub fn fail(code: &str, message: &str) -> Failure {
    fail_status(code, message, 400)
}

pub fn fail_status(code: &str, message: &str, status: u16) -> Failure {
    Failure {
        code: Some(code.into()),
        message: message.into(),
        status: Some(status),
    }
}

/// Marks a folder as Petri research's own (a folder without it must be empty).
const MARKER: &str = ".petri-research-storage";

/// `PETRI_RESEARCH_HOME`, else `PetriResearch` in `%LOCALAPPDATA%`, else in
/// `~/.local/share`.
pub fn state_directory() -> PathBuf {
    let nonempty = |key| std::env::var_os(key).filter(|value| !value.is_empty());
    let base = nonempty("PETRI_RESEARCH_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            nonempty("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::home_dir()
                        .unwrap_or_default()
                        .join(".local")
                        .join("share")
                })
                .join("PetriResearch")
        });
    absolute(&base)
}

pub fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `directory`, created for this user alone if missing. A folder that isn't
/// Petri research's (no marker) is refused unless it is empty.
pub fn private_directory(directory: &Path) -> Result<PathBuf, Failure> {
    let path = absolute(directory);
    let unsafe_storage = |message: &str| fail("UNSAFE_STORAGE", message);
    if path.exists() && !path.join(MARKER).exists() {
        let mut entries = fs::read_dir(&path)
            .map_err(|_| unsafe_storage("Private storage must be a normal directory."))?;
        if entries.next().is_some() {
            return Err(unsafe_storage(
                "Choose an empty directory dedicated to Petri research.",
            ));
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&path)
        .map_err(|_| unsafe_storage("Could not create the research storage."))?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| unsafe_storage("Private storage must be a normal directory."))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_storage(
            "Private storage must be a normal directory.",
        ));
    }
    restrict(&path)?;
    let marker = path.join(MARKER);
    if !marker.exists() {
        match private_file(&marker) {
            Ok(mut file) => file
                .write_all(b"Petri research private storage\n")
                .map_err(|_| unsafe_storage("Could not mark the research storage."))?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(unsafe_storage("Could not mark the research storage.")),
        }
    }
    Ok(path)
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<(), Failure> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
        fail(
            "UNSAFE_STORAGE",
            "Could not make the research storage private.",
        )
    })
}

/// The folder's ACL for this user alone, as the service sets it.
#[cfg(windows)]
fn restrict(path: &Path) -> Result<(), Failure> {
    use std::os::windows::process::CommandExt;
    let failed = || {
        fail(
            "UNSAFE_STORAGE",
            "Could not make the research storage private.",
        )
    };
    let run = |program: &str, args: &[&std::ffi::OsStr]| {
        std::process::Command::new(program)
            .args(args)
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
    };
    let identity = run("whoami.exe", &[]).ok_or_else(failed)?;
    let identity = String::from_utf8_lossy(&identity.stdout).trim().to_string();
    let grant = format!("{identity}:(OI)(CI)F");
    run(
        "icacls.exe",
        &[
            path.as_os_str(),
            "/inheritance:r".as_ref(),
            "/grant:r".as_ref(),
            grant.as_ref(),
        ],
    )
    .ok_or_else(failed)?;
    Ok(())
}

fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Replaces `path` with `value` as JSON, whole: a private temporary file
/// beside it, renamed over it.
pub fn private_write(path: &Path, value: &Value) -> Result<(), Failure> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(fail(
            "UNSAFE_STORAGE",
            "Credential files must not be symbolic links.",
        ));
    }
    let failed = || fail("STORAGE_FAILED", "Could not save the agent profile.");
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.tmp", &token()[..16]));
    let temporary = PathBuf::from(name);
    let mut encoded = serde_json::to_vec_pretty(value).map_err(|_| failed())?;
    encoded.push(b'\n');
    let written = private_file(&temporary).and_then(|mut file| {
        file.write_all(&encoded)?;
        file.sync_all()
    });
    if written.is_err() || fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(failed());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Creates `path` with `value` as JSON, private, only if it does not exist
/// yet (two processes making a workspace at once keep the first one's).
pub fn private_create(path: &Path, value: &Value) -> Result<(), Failure> {
    let mut encoded = serde_json::to_vec_pretty(value)
        .map_err(|_| fail("STORAGE_FAILED", "Could not save the workspace."))?;
    encoded.push(b'\n');
    match private_file(path) {
        Ok(mut file) => file
            .write_all(&encoded)
            .and_then(|_| file.sync_all())
            .map_err(|_| fail("STORAGE_FAILED", "Could not save the workspace.")),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(fail("STORAGE_FAILED", "Could not save the workspace.")),
    }
}

/// The origin of an HTTPS address, or of a plain HTTP one on loopback,
/// without credentials, a path, a query or a fragment.
pub fn endpoint(value: &str) -> Result<String, Failure> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        fail(
            "INVALID_ENDPOINT",
            "Provide an HTTPS or loopback API address.",
        )
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !(url.scheme() == "https"
            || url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")))
    {
        return Err(fail(
            "INVALID_ENDPOINT",
            "API addresses require HTTPS, except on loopback, and must not contain credentials or a path.",
        ));
    }
    Ok(url.origin().ascii_serialization())
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes
}

/// 32 random bytes in hex.
pub fn token() -> String {
    hex::encode(random::<32>())
}

/// A random (version 4) UUID.
pub fn uuid() -> String {
    let mut bytes = random::<16>();
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The SHA-256 of `bytes`, in hex.
pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}

/// Whether `a` and `b` are the same secret, compared in constant time
/// (through their digests).
pub fn equal_secret(a: &str, b: Option<&str>) -> bool {
    let Some(b) = b else { return false };
    let (a, b) = (digest(a), digest(b));
    a.bytes()
        .zip(b.bytes())
        .fold(0, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Text a person or agent gave: a non-blank string of at most `maximum`
/// bytes without control or direction-override characters, trimmed.
pub fn text(value: &Value, maximum: usize, label: &str) -> Result<String, Failure> {
    let invalid = || fail("INVALID_INPUT", &format!("Provide a valid {label}."));
    let value = value.as_str().ok_or_else(invalid)?;
    let forbidden = |c: char| matches!(c, '\u{0}'..='\u{8}' | '\u{b}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    if js_trim(value).is_empty() || value.len() > maximum || value.chars().any(forbidden) {
        return Err(invalid());
    }
    Ok(js_trim(value).to_string())
}

/// `String.prototype.trim`: JavaScript's whitespace, at both ends.
pub fn js_trim(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// A request body that is a JSON object with only `keys`, all of `required`.
/// It comes back as the JSON value, where a key it lacks reads as null.
pub fn object<'a>(body: &'a Value, keys: &[&str], required: &[&str]) -> Result<&'a Value, Failure> {
    let invalid = || fail("INVALID_INPUT", "Unexpected or missing request fields.");
    let object = body.as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| !keys.contains(&key.as_str()))
        || required.iter().any(|key| !object.contains_key(*key))
    {
        return Err(invalid());
    }
    Ok(body)
}

// Text in a URL that carries access (a token, a key, a signed link), the
// futures API's candidate rule (SECRET_FRAGMENTS, api/api/src/candidates/
// rules.rs) plus session ids, read percent-decoded and in any case.
const SECRET_FRAGMENTS: [&str; 11] = [
    "access_token=",
    "api_key=",
    "apikey=",
    "authorization=",
    "password=",
    "secret=",
    "x-amz-signature=",
    "token=",
    "sessionid=",
    "session_id=",
    "signature=",
];

/// Whether `value`, percent-decoded, carries access; an undecodable one does.
pub fn secret_in_url(value: &str) -> bool {
    let Some(decoded) = percent_decode(value) else {
        return true;
    };
    let decoded = decoded.to_lowercase();
    SECRET_FRAGMENTS
        .iter()
        .any(|fragment| decoded.contains(fragment))
}

/// `decodeURIComponent`: None for a malformed escape or invalid UTF-8.
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = value.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// USDC atoms (six decimals) as petri takes them: '1000000' is '1',
/// '2500000' is '2.5'.
pub fn usdc(atoms: &Value) -> Option<String> {
    let digits = atoms
        .as_str()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))?;
    let padded = format!("{digits:0>7}");
    let (whole, part) = padded.split_at(padded.len() - 6);
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let part = part.trim_end_matches('0');
    Some(if part.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{part}")
    })
}

/// Web proofs (TLSNotary, `petri evidence prove`): the verifier refuses a
/// larger bundle (evidence/crates/evidence/src/policy.rs), so petri refuses
/// it before uploading.
pub const MAX_BUNDLE_BYTES: u64 = 2 * 1024 * 1024;
const BUNDLE_FORMAT: &str = "amoeba-evidence-bundle-v1";

pub fn not_a_bundle() -> Failure {
    fail(
        "INVALID_BUNDLE",
        "This is not an evidence bundle. Give the file petri evidence prove wrote (its --out file).",
    )
}

pub fn bundle_too_large() -> Failure {
    fail_status(
        "BUNDLE_TOO_LARGE",
        "This evidence bundle is larger than 2 MiB, more than a web proof can verify. Prove an endpoint with a smaller response.",
        413,
    )
}

/// The bytes of a bundle as petri wrote it: a JSON object in the bundle
/// format, at most 2 MiB.
pub fn bundle_bytes(bytes: Vec<u8>) -> Result<Vec<u8>, Failure> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(bundle_too_large());
    }
    let value: Value = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| serde_json::from_str(text).ok())
        .ok_or_else(not_a_bundle)?;
    if value.get("format").and_then(Value::as_str) != Some(BUNDLE_FORMAT) {
        return Err(not_a_bundle());
    }
    Ok(bytes)
}
