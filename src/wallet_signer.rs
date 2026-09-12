use solana_derivation_path::DerivationPath;
use solana_keypair::read_keypair;
use solana_pubkey::Pubkey;
use solana_remote_wallet::{
    locator::Locator,
    remote_keypair::generate_remote_keypair,
    remote_wallet::{RemoteWalletError, initialize_wallet_manager},
};
use solana_signer::Signer;
use uriparse::URIReference;

use crate::{backend::CliError, onchain::OnchainConfig, solana_config};

const HARDWARE_WALLET_CONNECT_HINT: &str =
    "connect and unlock your hardware wallet, open the Solana app, close Ledger Live, then retry";

fn mcp_signer_access_blocked() -> bool {
    crate::mcp_actions::identity_blocked()
}

#[cfg(unix)]
fn open_local_keypair_file(path: &str) -> std::io::Result<std::fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::{Component, Path},
    };

    let absolute = std::path::absolute(Path::new(path))?;
    let mut components = absolute.components().peekable();
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "keypair path must resolve beneath a filesystem root",
        ));
    }
    let mut parent = std::fs::File::open("/")?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "keypair path contains unsafe traversal",
            ));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "keypair path contains NUL",
            )
        })?;
        let is_leaf = components.peek().is_none();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if is_leaf { 0 } else { libc::O_DIRECTORY };
        let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let opened = unsafe { std::fs::File::from_raw_fd(descriptor) };
        let metadata = opened.metadata()?;
        if (is_leaf && !metadata.is_file()) || (!is_leaf && !metadata.is_dir()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                if is_leaf {
                    "keypair is not a regular file"
                } else {
                    "keypair ancestor is not a directory"
                },
            ));
        }
        if is_leaf {
            return Ok(opened);
        }
        parent = opened;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "keypair path has no file name",
    ))
}

#[cfg(windows)]
fn open_local_keypair_file(path: &str) -> std::io::Result<std::fs::File> {
    use std::{
        os::windows::{
            ffi::OsStrExt,
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle},
        },
        path::{Component, Path, Prefix},
    };
    use windows_sys::{
        Wdk::{
            Foundation::OBJECT_ATTRIBUTES,
            Storage::FileSystem::{
                FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
                FILE_SYNCHRONOUS_IO_NONALERT, NtCreateFile,
            },
        },
        Win32::{
            Foundation::{HANDLE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING},
            Storage::FileSystem::{
                FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
                FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            },
            System::IO::IO_STATUS_BLOCK,
        },
    };

    let absolute = std::path::absolute(Path::new(path))?;
    let mut components = absolute.components().peekable();
    let drive = match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) => drive,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "keypair path has an unsafe Windows prefix",
                ));
            }
        },
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "keypair path has no local drive",
            ));
        }
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "keypair path has no drive root",
        ));
    }

    let root = format!("{}:\\", char::from(drive));
    let mut root_options = std::fs::OpenOptions::new();
    root_options
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let mut parent = root_options.open(root)?;

    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "keypair path contains unsafe traversal",
            ));
        };
        let mut wide_name = name.encode_wide().collect::<Vec<_>>();
        let name_bytes = wide_name
            .len()
            .checked_mul(std::mem::size_of::<u16>())
            .and_then(|length| u16::try_from(length).ok())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "keypair path component is too long",
                )
            })?;
        let unicode_name = UNICODE_STRING {
            Length: name_bytes,
            MaximumLength: name_bytes,
            Buffer: wide_name.as_mut_ptr(),
        };
        let attributes = OBJECT_ATTRIBUTES {
            Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
            RootDirectory: parent.as_raw_handle() as HANDLE,
            ObjectName: &unicode_name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: std::ptr::null(),
            SecurityQualityOfService: std::ptr::null(),
        };
        let is_leaf = components.peek().is_none();
        let mut handle: HANDLE = std::ptr::null_mut();
        let mut status_block = IO_STATUS_BLOCK::default();
        let status = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_GENERIC_READ,
                &attributes,
                &mut status_block,
                std::ptr::null(),
                FILE_ATTRIBUTE_NORMAL,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                FILE_OPEN_REPARSE_POINT
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | if is_leaf {
                        FILE_NON_DIRECTORY_FILE
                    } else {
                        FILE_DIRECTORY_FILE
                    },
                std::ptr::null(),
                0,
            )
        };
        if status < 0 {
            let error = unsafe { RtlNtStatusToDosError(status) };
            return Err(std::io::Error::from_raw_os_error(error as i32));
        }
        let opened = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
        let metadata = opened.metadata()?;
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
            || (is_leaf && !metadata.is_file())
            || (!is_leaf && !metadata.is_dir())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "keypair path contains a reparse point or unexpected file type",
            ));
        }
        if is_leaf {
            return Ok(opened);
        }
        parent = opened;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "keypair path has no file name",
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_local_keypair_file(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

pub(crate) fn signer_pubkey(config: &OnchainConfig) -> Result<String, CliError> {
    if mcp_signer_access_blocked() {
        return Err(CliError::new(
            "wallet identity resolution is unavailable inside the Petri MCP read-only process",
        ));
    }
    let signer_path = config
        .keypair_path
        .clone()
        .unwrap_or_else(solana_config::default_keypair_path);
    signer_pubkey_from_path(&signer_path)
}

pub(crate) fn require_admitted_signer(
    signer: &dyn Signer,
    expected_signer: &Pubkey,
) -> Result<(), CliError> {
    let actual_signer = signer
        .try_pubkey()
        .map_err(|_| CliError::new("The attached wallet public key could not be read."))?;
    if actual_signer != *expected_signer {
        return Err(CliError::new(
            "The attached wallet does not match the admitted signer. Nothing was signed or sent.",
        ));
    }
    Ok(())
}

pub(crate) fn signer_pubkey_from_path(path: &str) -> Result<String, CliError> {
    let owner = read_signer_pubkey_from_path(path)?;
    crate::mcp_actions::check_owner(&owner)?;
    Ok(owner)
}

fn read_signer_pubkey_from_path(path: &str) -> Result<String, CliError> {
    if mcp_signer_access_blocked() {
        return Err(CliError::new(
            "wallet identity resolution is unavailable inside the Petri MCP read-only process",
        ));
    }

    if solana_config::is_remote_wallet_path(path) {
        let signer = load_remote_signer(path)?;
        return signer
            .try_pubkey()
            .map(|pubkey| pubkey.to_string())
            .map_err(|error| {
                CliError::new(format!("failed to read hardware wallet address: {error}"))
            });
    }

    let preopen_label = solana_config::preopen_keypair_file_security_label(path);
    if preopen_label != "ok" {
        return Err(CliError::new(format!(
            "refusing to read an unsafe configured keypair ({preopen_label})"
        )));
    }
    let mut file = open_local_keypair_file(path).map_err(|error| {
        CliError::new(format!("failed to open configured keypair file: {error}"))
    })?;
    let opened_label = solana_config::opened_keypair_file_security_label(&file);
    if opened_label != "ok" && !opened_label.starts_with("warn:") {
        return Err(CliError::new(format!(
            "refusing to read an unsafe configured keypair ({opened_label})"
        )));
    }
    read_keypair(&mut file)
        .map(|keypair| keypair.pubkey().to_string())
        .map_err(|error| CliError::new(format!("failed to read configured keypair file: {error}")))
}

pub(crate) fn load_signer(config: &OnchainConfig) -> Result<Box<dyn Signer>, CliError> {
    if crate::mcp_actions::signing_blocked() {
        return Err(CliError::new(
            "MCP signing requires explicit authorization for one exact reviewed operation",
        ));
    }

    let signer_path = config
        .keypair_path
        .clone()
        .unwrap_or_else(solana_config::default_keypair_path);

    if solana_config::is_remote_wallet_path(&signer_path) {
        return load_remote_signer(&signer_path);
    }

    let preopen_label = solana_config::preopen_keypair_file_security_label(&signer_path);
    if preopen_label != "ok" {
        return Err(CliError::new(format!(
            "refusing to sign with unsafe keypair path {signer_path} ({preopen_label}); use a local regular file with private permissions"
        )));
    }
    let mut keypair_file = open_local_keypair_file(&signer_path).map_err(|error| {
        CliError::new(format!(
            "failed to open keypair file {signer_path} for signing: {error}"
        ))
    })?;
    let security_label = solana_config::opened_keypair_file_security_label(&keypair_file);
    match security_label.as_str() {
        "ok" => {}
        label if label.starts_with("warn:") && config.allow_insecure_keypair => {}
        label if label.starts_with("warn:") => {
            return Err(CliError::new(format!(
                "refusing to sign with insecure keypair file {signer_path} ({security_label}); tighten file permissions or pass --allow-insecure-keypair"
            )));
        }
        _ => {
            return Err(CliError::new(format!(
                "refusing to sign with unsafe keypair path {signer_path} ({security_label}); use a local regular file with private permissions"
            )));
        }
    }
    read_keypair(&mut keypair_file)
        .map(|keypair| Box::new(keypair) as Box<dyn Signer>)
        .map_err(|error| {
            CliError::new(format!(
                "failed to read keypair file {signer_path}: {error}"
            ))
        })
}

fn load_remote_signer(path: &str) -> Result<Box<dyn Signer>, CliError> {
    let (locator, derivation_path) = validated_remote_wallet(path)?;
    let wallet_manager = initialize_wallet_manager().map_err(remote_wallet_cli_error)?;
    wallet_manager
        .update_devices()
        .map_err(remote_wallet_cli_error)?;
    let signer = generate_remote_keypair(
        locator,
        derivation_path,
        wallet_manager.as_ref(),
        false,
        "Petri wallet",
    )
    .map_err(remote_wallet_cli_error)?;

    Ok(Box::new(signer))
}

fn validated_remote_wallet(path: &str) -> Result<(Locator, DerivationPath), CliError> {
    if path.is_empty() || path.trim() != path {
        return Err(invalid_hardware_wallet_url(
            path,
            "surrounding whitespace is not allowed",
        ));
    }
    if !path.is_ascii() || path.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(invalid_hardware_wallet_url(
            path,
            "only printable ASCII is allowed",
        ));
    }
    if path.contains('%') {
        return Err(invalid_hardware_wallet_url(
            path,
            "percent-encoding is not allowed",
        ));
    }

    let uri = URIReference::try_from(path)
        .map_err(|error| invalid_hardware_wallet_url(path, &error.to_string()))?;
    if uri.scheme().map(|scheme| scheme.as_str()) != Some("usb") {
        return Err(invalid_hardware_wallet_url(
            path,
            "scheme must be exactly usb",
        ));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| invalid_hardware_wallet_url(path, "authority must be exactly ledger"))?;
    if authority.username().is_some() || authority.password().is_some() {
        return Err(invalid_hardware_wallet_url(path, "userinfo is not allowed"));
    }
    if authority.port().is_some() {
        return Err(invalid_hardware_wallet_url(path, "ports are not allowed"));
    }
    if authority.to_string() != "ledger" {
        return Err(invalid_hardware_wallet_url(
            path,
            "authority must be exactly ledger",
        ));
    }
    if uri.fragment().is_some() {
        return Err(invalid_hardware_wallet_url(
            path,
            "fragments are not allowed",
        ));
    }

    let locator_path = uri.path().to_string();
    if locator_path != "" && locator_path != "/" {
        let public_key = locator_path.strip_prefix('/').ok_or_else(|| {
            invalid_hardware_wallet_url(path, "locator path must begin with one slash")
        })?;
        if public_key.is_empty() || public_key.contains('/') {
            return Err(invalid_hardware_wallet_url(
                path,
                "locator path may contain only one exact public key",
            ));
        }
    }

    if let Some(query) = uri.query() {
        let query = query.as_str();
        let derivation = query.strip_prefix("key=").ok_or_else(|| {
            invalid_hardware_wallet_url(path, "query must be exactly key=<account>[/<change>]")
        })?;
        if derivation.is_empty()
            || query.matches('=').count() != 1
            || !canonical_derivation_query(derivation)
        {
            return Err(invalid_hardware_wallet_url(
                path,
                "query must be exactly key=<account>[/<change>] using canonical decimal indexes",
            ));
        }
    }

    let locator = Locator::new_from_uri(&uri)
        .map_err(|error| invalid_hardware_wallet_url(path, &error.to_string()))?;
    let derivation_path = DerivationPath::from_uri_key_query(&uri)
        .map(|derivation_path| derivation_path.unwrap_or_default())
        .map_err(|error| {
            CliError::new(format!(
                "invalid hardware wallet derivation path in {path}: {error}"
            ))
        })?;
    Ok((locator, derivation_path))
}

fn canonical_derivation_query(derivation: &str) -> bool {
    let indexes = derivation.split('/').collect::<Vec<_>>();
    (1..=2).contains(&indexes.len())
        && indexes.iter().all(|index| {
            let digits = index.strip_suffix('\'').unwrap_or(index);
            !digits.is_empty()
                && digits.bytes().all(|byte| byte.is_ascii_digit())
                && (digits == "0" || !digits.starts_with('0'))
                && digits.parse::<u32>().is_ok()
        })
}

fn invalid_hardware_wallet_url(path: &str, reason: &str) -> CliError {
    CliError::new(format!("invalid hardware wallet URL {path}: {reason}"))
}

fn remote_wallet_cli_error(error: RemoteWalletError) -> CliError {
    let message = match error {
        RemoteWalletError::NoDeviceFound => {
            format!("hardware wallet not found; {HARDWARE_WALLET_CONNECT_HINT}")
        }
        RemoteWalletError::UserCancel => {
            "hardware wallet rejected signing; no transaction was submitted".to_string()
        }
        RemoteWalletError::DeviceTypeMismatch => {
            "the connected hardware wallet does not match the configured signer URL".to_string()
        }
        RemoteWalletError::InvalidDevice => {
            "the connected hardware wallet is not supported for Solana signing".to_string()
        }
        RemoteWalletError::LedgerError(error) => {
            format!("hardware wallet could not sign this transaction: {error}")
        }
        RemoteWalletError::Hid(error) => {
            format!("hardware wallet connection failed: {error}; {HARDWARE_WALLET_CONNECT_HINT}")
        }
        other => format!("hardware wallet error: {other}"),
    };
    CliError::new(message)
}
