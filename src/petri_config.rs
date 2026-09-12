use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_BACKEND_URL: &str = "https://api.amoeba.farm";
const DEFAULT_BACKEND_HOST: &str = "api.amoeba.farm";
pub const RPC_GATEWAY_PATH: &str = "/rpc";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PetriUserConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    backend_url: Option<String>,
}

fn nonempty_path_env(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn config_path() -> Result<PathBuf, String> {
    if let Some(path) = nonempty_path_env("PETRI_CONFIG_PATH") {
        return Ok(path);
    }

    #[cfg(windows)]
    if let Some(root) = nonempty_path_env("APPDATA").or_else(|| nonempty_path_env("LOCALAPPDATA")) {
        return Ok(root.join("amoeba").join("petri").join("config.json"));
    }

    if let Some(root) = nonempty_path_env("XDG_CONFIG_HOME") {
        return Ok(root.join("amoeba").join("petri").join("config.json"));
    }
    let home = nonempty_path_env("HOME").or_else(|| nonempty_path_env("USERPROFILE"));
    home.map(|root| {
        root.join(".config")
            .join("amoeba")
            .join("petri")
            .join("config.json")
    })
    .ok_or_else(|| "could not resolve the Petri configuration directory".to_string())
}

fn is_loopback(host: &str) -> bool {
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

/// Validates and normalizes the only application origins Petri may contact.
///
/// Remote clients are pinned to the hosted Amoeba API. Loopback remains
/// available for repository-owned integration tests and local development.
/// Credentials, query strings, fragments, and path-bearing URLs are rejected
/// before the value can be stored, displayed, or used for a request.
pub fn normalize_amoeba_backend_url(backend_url: &str) -> Result<String, String> {
    let raw = backend_url.trim();
    let mut parsed = reqwest::Url::parse(raw)
        .map_err(|_| "Amoeba URL must be an absolute HTTP(S) URL".to_string())?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "Amoeba URL must include a host".to_string())?;
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let loopback = is_loopback(&normalized_host);
    let (raw_host, has_explicit_port) = raw_authority_host(raw)
        .ok_or_else(|| "Amoeba URL must include one canonical host".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Amoeba URL must use http:// or https://".to_string());
    }
    if raw_host.ends_with('.') {
        return Err("Amoeba URL must use one canonical host without a trailing dot".to_string());
    }
    if loopback && !is_canonical_loopback_literal(raw_host, &normalized_host) {
        return Err(
            "Loopback development URLs must use localhost, canonical 127/8 dotted decimal, or [::1]"
                .to_string(),
        );
    }
    if parsed.scheme() == "http" && !loopback {
        return Err(
            "Amoeba URL must use HTTPS (HTTP is allowed only for loopback development)".to_string(),
        );
    }
    if !loopback && normalized_host != DEFAULT_BACKEND_HOST {
        return Err(
            "Amoeba URL must target the hosted Amoeba API (or a loopback development server)"
                .to_string(),
        );
    }
    if !loopback
        && (!raw_host.eq_ignore_ascii_case(DEFAULT_BACKEND_HOST)
            || has_explicit_port
            || parsed.port().is_some())
    {
        return Err("Amoeba URL must use the canonical hosted Amoeba API origin".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Amoeba URL must not contain username/password credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Amoeba URL must not contain query parameters or a fragment".to_string());
    }
    if !matches!(parsed.path(), "" | "/") {
        return Err("Amoeba URL must be an API origin without a path".to_string());
    }
    if !normalized_host.contains(':') {
        parsed
            .set_host(Some(&normalized_host))
            .map_err(|_| "Amoeba URL has an invalid host".to_string())?;
    }
    parsed.set_path("/");
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn raw_authority_host(raw: &str) -> Option<(&str, bool)> {
    let (_, remainder) = raw.split_once("://")?;
    let authority = remainder
        .split(['/', '?', '#'])
        .next()?
        .rsplit_once('@')
        .map_or(remainder.split(['/', '?', '#']).next()?, |(_, host)| host);
    if authority.starts_with('[') {
        let bracket = authority.find(']')?;
        let host = authority.get(..=bracket)?;
        let suffix = authority.get(bracket + 1..)?;
        return Some((host, !suffix.is_empty()));
    }
    match authority.rsplit_once(':') {
        Some((host, _)) => Some((host, true)),
        None => Some((authority, false)),
    }
}

fn is_canonical_loopback_literal(raw_host: &str, normalized_host: &str) -> bool {
    if normalized_host.eq_ignore_ascii_case("localhost") {
        return raw_host.eq_ignore_ascii_case("localhost");
    }
    if normalized_host == "::1" {
        return raw_host.eq_ignore_ascii_case("[::1]");
    }
    raw_host
        .parse::<std::net::Ipv4Addr>()
        .is_ok_and(|address| address.is_loopback() && address.to_string() == raw_host)
}

fn load_from_path(path: &Path) -> Result<PetriUserConfig, String> {
    require_regular_or_missing_config(path)?;
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PetriUserConfig::default());
        }
        Err(error) => return Err(format!("failed to read Petri configuration: {error}")),
    };
    serde_json::from_str(&raw)
        .map_err(|error| format!("failed to parse Petri configuration: {error}"))
}

fn path_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }

    false
}

fn normalize_config_write_path(path: &Path) -> Result<PathBuf, String> {
    use std::path::Component;

    if path.as_os_str().is_empty() {
        return Err("Petri configuration path is empty".to_string());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("failed to resolve the current directory: {error}"))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(
                    "Petri configuration path must not contain parent traversal".to_string()
                );
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if !normalized.is_absolute() || normalized.file_name().is_none() {
        return Err("Petri configuration path must name a file".to_string());
    }
    Ok(normalized)
}

fn require_regular_or_missing_config(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if path_is_link_or_reparse(&metadata) || !metadata.is_file() => Err(
            "Petri configuration must be a regular file, not a link or reparse point".to_string(),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect Petri configuration: {error}")),
    }
}

#[cfg(unix)]
fn set_private_config_parent_permissions(directory: &File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to restrict Petri configuration directory: {error}"))
}

#[cfg(windows)]
fn set_private_config_parent_permissions(directory: &File) -> Result<(), String> {
    set_private_file_permissions(directory)
}

#[cfg(not(any(unix, windows)))]
fn set_private_config_parent_permissions(_directory: &File) -> Result<(), String> {
    Err("private Petri configuration directories are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn require_private_config_parent_permissions(directory: &File) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = directory
        .metadata()
        .map_err(|error| format!("failed to inspect Petri configuration directory: {error}"))?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
        return Err(
            "pre-existing Petri configuration directory must be owned by the current user and private; its permissions were not changed"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(windows)]
struct OwnedPetriWindowsSid {
    words: Vec<usize>,
}

#[cfg(windows)]
impl OwnedPetriWindowsSid {
    fn as_psid(&self) -> windows_sys::Win32::Security::PSID {
        self.words.as_ptr().cast_mut().cast::<core::ffi::c_void>()
    }
}

#[cfg(windows)]
fn windows_current_process_user_sid() -> Option<OwnedPetriWindowsSid> {
    use windows_sys::{
        Wdk::Storage::FileSystem::NtOpenProcessToken,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::{
                GetLengthSid, GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
            },
        },
    };

    struct TokenHandle(HANDLE);
    impl Drop for TokenHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    let mut raw_token: HANDLE = std::ptr::null_mut();
    let current_process = (-1_isize) as HANDLE;
    if unsafe { NtOpenProcessToken(current_process, TOKEN_QUERY, &mut raw_token) } < 0
        || raw_token.is_null()
    {
        return None;
    }
    let token = TokenHandle(raw_token);
    let mut required = 0_u32;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut required);
    }
    if required < std::mem::size_of::<TOKEN_USER>() as u32 {
        return None;
    }
    let word_size = std::mem::size_of::<usize>();
    let mut token_user_storage = vec![0_usize; (required as usize).div_ceil(word_size)];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_user_storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return None;
    }
    let token_user = unsafe { &*token_user_storage.as_ptr().cast::<TOKEN_USER>() };
    let sid = token_user.User.Sid;
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return None;
    }
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    if sid_length == 0 {
        return None;
    }
    let mut words = vec![0_usize; sid_length.div_ceil(word_size)];
    unsafe {
        std::ptr::copy_nonoverlapping(
            sid.cast::<u8>(),
            words.as_mut_ptr().cast::<u8>(),
            sid_length,
        );
    }
    Some(OwnedPetriWindowsSid { words })
}

#[cfg(windows)]
fn windows_owner_matches_current_user(
    owner: windows_sys::Win32::Security::PSID,
    current_user: windows_sys::Win32::Security::PSID,
) -> bool {
    use windows_sys::Win32::Security::EqualSid;

    !owner.is_null() && !current_user.is_null() && unsafe { EqualSid(owner, current_user) } != 0
}

#[cfg(windows)]
fn require_private_config_parent_permissions(directory: &File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{ERROR_SUCCESS, HANDLE, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, ACL,
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            CreateWellKnownSid, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_MAX_SID_SIZE,
            WinBuiltinAdministratorsSid, WinLocalSystemSid,
        },
        System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
    };

    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            directory.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    let result = (|| -> Result<(), String> {
        if status != ERROR_SUCCESS || owner.is_null() || dacl.is_null() {
            return Err(
                "pre-existing Petri configuration directory ACL is unreadable or not private; its ACL was not changed"
                    .to_string(),
                );
        }
        let current_user = windows_current_process_user_sid().ok_or_else(|| {
            "failed to read the current process TokenUser SID; the pre-existing Petri configuration directory was not accepted"
                .to_string()
        })?;
        if !windows_owner_matches_current_user(owner, current_user.as_psid()) {
            return Err(
                "pre-existing Petri configuration directory must be owned by the current process user; its ACL was not changed"
                    .to_string(),
            );
        }
        let mut allowed_sids = [[0_u8; SECURITY_MAX_SID_SIZE as usize]; 2];
        for (buffer, sid_type) in allowed_sids
            .iter_mut()
            .zip([WinLocalSystemSid, WinBuiltinAdministratorsSid])
        {
            let mut size = SECURITY_MAX_SID_SIZE;
            if unsafe {
                CreateWellKnownSid(
                    sid_type,
                    std::ptr::null_mut(),
                    buffer.as_mut_ptr().cast(),
                    &mut size,
                )
            } == 0
            {
                return Err("failed to verify the Petri configuration directory ACL".to_string());
            }
        }
        for index in 0..u32::from(unsafe { (*dacl).AceCount }) {
            let mut raw_ace = std::ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
                return Err("failed to verify the Petri configuration directory ACL".to_string());
            }
            let header = unsafe { &*(raw_ace.cast::<windows_sys::Win32::Security::ACE_HEADER>()) };
            let ace_type = u32::from(header.AceType);
            if ace_type == ACCESS_DENIED_ACE_TYPE {
                continue;
            }
            if ace_type != ACCESS_ALLOWED_ACE_TYPE {
                return Err(
                    "pre-existing Petri configuration directory has an unsupported ACL entry; its ACL was not changed"
                        .to_string(),
                );
            }
            let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
            let ace_sid = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
            let allowed = unsafe { EqualSid(ace_sid, owner) } != 0
                || allowed_sids.iter_mut().any(|sid| unsafe {
                    EqualSid(ace_sid, sid.as_mut_ptr().cast::<core::ffi::c_void>() as PSID)
                } != 0);
            if !allowed {
                return Err(
                    "pre-existing Petri configuration directory grants access outside its owner, SYSTEM, or Administrators; its ACL was not changed"
                        .to_string(),
                );
            }
        }
        Ok(())
    })();
    if !descriptor.is_null() {
        unsafe {
            LocalFree(descriptor);
        }
    }
    result
}

#[cfg(not(any(unix, windows)))]
fn require_private_config_parent_permissions(_directory: &File) -> Result<(), String> {
    Err("private Petri configuration directories are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn set_private_file_permissions(file: &File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to restrict Petri configuration permissions: {error}"))
}

#[cfg(windows)]
fn set_private_file_permissions(file: &File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{ERROR_SUCCESS, HANDLE, LocalFree},
        Security::{
            ACL,
            Authorization::{
                EXPLICIT_ACCESS_W, GetSecurityInfo, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
                SET_ACCESS, SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
                TRUSTEE_W,
            },
            DACL_SECURITY_INFORMATION, NO_INHERITANCE, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
        Storage::FileSystem::FILE_ALL_ACCESS,
    };

    let handle = file.as_raw_handle() as HANDLE;
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let get_status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_status != ERROR_SUCCESS {
        if !descriptor.is_null() {
            unsafe {
                LocalFree(descriptor);
            }
        }
        return Err(format!(
            "failed to read Petri configuration owner: {}",
            io::Error::from_raw_os_error(get_status as i32)
        ));
    }

    let mut private_acl: *mut ACL = std::ptr::null_mut();
    let result = (|| -> Result<(), String> {
        if owner.is_null() {
            return Err("failed to read Petri configuration owner".to_string());
        }

        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: owner.cast(),
            },
        };

        let acl_status = unsafe { SetEntriesInAclW(1, &entry, std::ptr::null(), &mut private_acl) };
        if acl_status != ERROR_SUCCESS {
            return Err(format!(
                "failed to create a private Petri configuration ACL: {}",
                io::Error::from_raw_os_error(acl_status as i32)
            ));
        }

        let set_status = unsafe {
            SetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                private_acl,
                std::ptr::null(),
            )
        };
        if set_status != ERROR_SUCCESS {
            return Err(format!(
                "failed to restrict Petri configuration permissions: {}",
                io::Error::from_raw_os_error(set_status as i32)
            ));
        }
        Ok(())
    })();

    unsafe {
        if !private_acl.is_null() {
            LocalFree(private_acl.cast());
        }
        if !descriptor.is_null() {
            LocalFree(descriptor);
        }
    }
    result
}

#[cfg(not(any(unix, windows)))]
fn set_private_file_permissions(_file: &File) -> Result<(), String> {
    Err("private Petri configuration files are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn unix_component_name(name: &std::ffi::OsStr) -> Result<std::ffi::CString, String> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(name.as_bytes())
        .map_err(|_| "Petri configuration path contains NUL".to_string())
}

#[cfg(unix)]
fn unix_open_relative_directory(directory: &File, name: &std::ffi::OsStr) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let name = unix_component_name(name)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn open_or_create_config_parent(parent: &Path) -> Result<File, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Component;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut directory = options
        .open(Path::new("/"))
        .map_err(|error| format!("failed to open the filesystem root: {error}"))?;
    let components = parent
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(Ok(name.to_os_string())),
            Component::RootDir | Component::CurDir => None,
            Component::ParentDir | Component::Prefix(_) => Some(Err(
                "Petri configuration directory must be an absolute path without parent traversal"
                    .to_string(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut final_was_created = false;
    for (index, component) in components.iter().enumerate() {
        let final_component = index + 1 == components.len();
        let (child, created) = match unix_open_relative_directory(&directory, component) {
            Ok(child) => (child, false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = unix_component_name(component)?;
                let created =
                    unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
                if !created {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::AlreadyExists {
                        return Err(format!(
                            "failed to create Petri configuration directory component: {error}"
                        ));
                    }
                }
                let child = unix_open_relative_directory(&directory, component).map_err(|error| {
                    format!(
                        "failed to open Petri configuration directory component without following links: {error}"
                    )
                })?;
                (child, created)
            }
            Err(error) => {
                return Err(format!(
                    "every Petri configuration directory component must be a real directory, not a link or reparse point: {error}"
                ));
            }
        };
        if created {
            set_private_config_parent_permissions(&child)?;
        }
        if final_component {
            final_was_created = created;
        }
        directory = child;
    }
    if components.is_empty() || !final_was_created {
        require_private_config_parent_permissions(&directory)?;
    }
    Ok(directory)
}

#[cfg(windows)]
fn windows_relative_file(
    directory: &File,
    name: &std::ffi::OsStr,
    desired_access: u32,
    create_disposition: u32,
    create_options: u32,
    file_attributes: u32,
) -> io::Result<File> {
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    };
    use windows_sys::{
        Wdk::{Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::NtCreateFile},
        Win32::{
            Foundation::{
                HANDLE, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, RtlNtStatusToDosError,
                UNICODE_STRING,
            },
            Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
            System::IO::IO_STATUS_BLOCK,
        },
    };

    let mut wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty()
        || wide
            .iter()
            .any(|value| *value == 0 || *value == u16::from(b'/') || *value == u16::from(b'\\'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Petri configuration component is invalid",
        ));
    }
    let length = wide
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path component is too long"))?;
    let name = UNICODE_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: directory.as_raw_handle() as HANDLE,
        ObjectName: &name,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            file_attributes,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            create_disposition,
            create_options,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        let error = unsafe { RtlNtStatusToDosError(status) };
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

#[cfg(windows)]
fn windows_open_root(
    root: &Path,
    create_children: bool,
    final_parent: bool,
) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE,
        READ_CONTROL, SYNCHRONIZE,
    };

    let mut access = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;
    if create_children {
        access |= FILE_ADD_SUBDIRECTORY;
    }
    if final_parent {
        access |= FILE_ADD_FILE | FILE_DELETE_CHILD | READ_CONTROL;
    }
    let mut options = OpenOptions::new();
    options
        .access_mode(access)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let directory = options
        .open(root)
        .map_err(|error| format!("failed to open Petri configuration path root: {error}"))?;
    let metadata = directory
        .metadata()
        .map_err(|error| format!("failed to inspect Petri configuration path root: {error}"))?;
    if path_is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err(
            "every Petri configuration directory component must be a real directory, not a link or reparse point"
                .to_string(),
        );
    }
    Ok(directory)
}

#[cfg(windows)]
fn windows_open_relative_directory(
    directory: &File,
    name: &std::ffi::OsStr,
    create_children: bool,
    final_parent: bool,
) -> io::Result<File> {
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
        },
        Win32::Storage::FileSystem::{
            FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_LIST_DIRECTORY,
            FILE_READ_ATTRIBUTES, FILE_TRAVERSE, READ_CONTROL, SYNCHRONIZE,
        },
    };

    let mut access = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE;
    if create_children {
        access |= FILE_ADD_SUBDIRECTORY;
    }
    if final_parent {
        access |= FILE_ADD_FILE | FILE_DELETE_CHILD | READ_CONTROL;
    }
    windows_relative_file(
        directory,
        name,
        access,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        0,
    )
}

#[cfg(windows)]
fn windows_create_relative_directory(
    directory: &File,
    name: &std::ffi::OsStr,
    final_parent: bool,
) -> io::Result<File> {
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FILE_CREATE, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
        },
        Win32::Storage::FileSystem::{
            FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ATTRIBUTE_DIRECTORY, FILE_DELETE_CHILD,
            FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE, READ_CONTROL, SYNCHRONIZE,
            WRITE_DAC,
        },
    };

    let mut access = FILE_LIST_DIRECTORY
        | FILE_READ_ATTRIBUTES
        | FILE_TRAVERSE
        | SYNCHRONIZE
        | READ_CONTROL
        | WRITE_DAC;
    if final_parent {
        access |= FILE_ADD_FILE | FILE_DELETE_CHILD;
    } else {
        access |= FILE_ADD_SUBDIRECTORY;
    }
    windows_relative_file(
        directory,
        name,
        access,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        FILE_ATTRIBUTE_DIRECTORY,
    )
}

#[cfg(windows)]
fn open_or_create_config_parent(parent: &Path) -> Result<File, String> {
    use std::path::Component;

    let mut root = PathBuf::new();
    let mut components = Vec::new();
    for component in parent.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => root.push(component.as_os_str()),
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(
                    "Petri configuration directory must not contain parent traversal".to_string(),
                );
            }
        }
    }
    if root.as_os_str().is_empty() {
        return Err("Petri configuration directory must be absolute".to_string());
    }
    let mut directories = vec![windows_open_root(&root, false, components.is_empty())?];
    let mut final_was_created = false;
    for (index, component) in components.iter().enumerate() {
        let final_component = index + 1 == components.len();
        let opened = windows_open_relative_directory(
            directories.last().expect("root directory handle"),
            component,
            false,
            final_component,
        );
        let (child, created) = match opened {
            Ok(child) => (child, false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if directories.len() == 1 {
                    directories[0] = windows_open_root(&root, true, components.is_empty())?;
                } else {
                    let current_index = directories.len() - 1;
                    let reopened = windows_open_relative_directory(
                        &directories[current_index - 1],
                        &components[index - 1],
                        true,
                        false,
                    )
                    .map_err(|error| {
                        format!(
                            "failed to reopen Petri configuration directory for child creation: {error}"
                        )
                    })?;
                    directories[current_index] = reopened;
                }
                let current = directories.last().expect("current directory handle");
                match windows_create_relative_directory(current, component, final_component) {
                    Ok(child) => (child, true),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
                        windows_open_relative_directory(current, component, false, final_component)
                            .map_err(|error| {
                                format!(
                                    "failed to open concurrently-created Petri configuration directory: {error}"
                                )
                            })?,
                        false,
                    ),
                    Err(error) => {
                        return Err(format!(
                            "failed to create Petri configuration directory component: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                return Err(format!(
                    "every Petri configuration directory component must be a real directory, not a link or reparse point: {error}"
                ));
            }
        };
        let metadata = child.metadata().map_err(|error| {
            format!("failed to inspect Petri configuration directory component: {error}")
        })?;
        if path_is_link_or_reparse(&metadata) || !metadata.is_dir() {
            return Err(
                "every Petri configuration directory component must be a real directory, not a link or reparse point"
                    .to_string(),
            );
        }
        if created {
            set_private_config_parent_permissions(&child)?;
        }
        if final_component {
            final_was_created = created;
        }
        directories.push(child);
    }
    let directory = directories.pop().expect("root directory handle");
    if components.is_empty() || !final_was_created {
        require_private_config_parent_permissions(&directory)?;
    }
    Ok(directory)
}

#[cfg(not(any(unix, windows)))]
fn open_or_create_config_parent(_parent: &Path) -> Result<File, String> {
    Err("handle-relative Petri configuration writes are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn require_regular_or_missing_config_at(
    directory: &File,
    name: &std::ffi::OsStr,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let name = unix_component_name(name)?;
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        if metadata.st_mode & libc::S_IFMT != libc::S_IFREG {
            return Err(
                "Petri configuration must be a regular file, not a link or reparse point"
                    .to_string(),
            );
        }
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(format!("failed to inspect Petri configuration: {error}"))
    }
}

#[cfg(windows)]
fn require_regular_or_missing_config_at(
    directory: &File,
    name: &std::ffi::OsStr,
) -> Result<(), String> {
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
            FILE_SYNCHRONOUS_IO_NONALERT,
        },
        Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, SYNCHRONIZE},
    };

    match windows_relative_file(
        directory,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        0,
    ) {
        Ok(file) => {
            let metadata = file
                .metadata()
                .map_err(|error| format!("failed to inspect Petri configuration: {error}"))?;
            if path_is_link_or_reparse(&metadata) || !metadata.is_file() {
                return Err(
                    "Petri configuration must be a regular file, not a link or reparse point"
                        .to_string(),
                );
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Petri configuration must be a regular file, not a link or reparse point: {error}"
        )),
    }
}

#[cfg(not(any(unix, windows)))]
fn require_regular_or_missing_config_at(
    _directory: &File,
    _name: &std::ffi::OsStr,
) -> Result<(), String> {
    Err("handle-relative Petri configuration writes are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn open_private_temp_candidate(directory: &File, candidate: &std::ffi::OsStr) -> io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let candidate = unix_component_name(candidate)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            candidate.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(windows)]
fn open_private_temp_candidate(directory: &File, candidate: &std::ffi::OsStr) -> io::Result<File> {
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FILE_CREATE, FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT,
            FILE_SYNCHRONOUS_IO_NONALERT,
        },
        Win32::Storage::FileSystem::{
            DELETE, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE, READ_CONTROL, WRITE_DAC,
        },
    };

    windows_relative_file(
        directory,
        candidate,
        FILE_GENERIC_WRITE | READ_CONTROL | WRITE_DAC | DELETE,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        FILE_ATTRIBUTE_NORMAL,
    )
}

#[cfg(not(any(unix, windows)))]
fn open_private_temp_candidate(
    _directory: &File,
    _candidate: &std::ffi::OsStr,
) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "handle-relative Petri configuration writes are unsupported on this platform",
    ))
}

fn allocate_private_temp(directory: &File) -> Result<(std::ffi::OsString, File), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..16 {
        let candidate = std::ffi::OsString::from(format!(
            ".config.json.{}.{nonce}.{attempt}.tmp",
            std::process::id()
        ));
        match open_private_temp_candidate(directory, &candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to create temporary Petri configuration: {error}"
                ));
            }
        }
    }
    Err("failed to allocate a unique temporary Petri configuration".to_string())
}

#[cfg(target_os = "linux")]
fn install_private_file(
    file: &File,
    temporary_name: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
    directory: &File,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let destination = std::ffi::CString::new(destination.as_bytes())
        .map_err(|_| "Petri configuration name contains NUL".to_string())?;
    let descriptor_path = std::ffi::CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .map_err(|_| "failed to encode the temporary Petri file handle".to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..16 {
        let staging = std::ffi::CString::new(format!(
            ".config.json.install.{}.{nonce}.{attempt}.tmp",
            std::process::id()
        ))
        .map_err(|_| "failed to encode the Petri install staging name".to_string())?;
        let linked = unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                descriptor_path.as_ptr(),
                directory.as_raw_fd(),
                staging.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        };
        if linked != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                continue;
            }
            return Err(format!(
                "failed to bind the temporary Petri file handle for installation: {error}"
            ));
        }
        if let Err(error) = unlink_private_temp_if_same(file, temporary_name, directory) {
            unsafe {
                libc::unlinkat(directory.as_raw_fd(), staging.as_ptr(), 0);
            }
            return Err(error);
        }
        let renamed = unsafe {
            libc::renameat(
                directory.as_raw_fd(),
                staging.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
            )
        };
        if renamed != 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::unlinkat(directory.as_raw_fd(), staging.as_ptr(), 0);
            }
            return Err(format!("failed to install Petri configuration: {error}"));
        }
        return Ok(());
    }
    Err("failed to allocate a unique Petri install staging link".to_string())
}

#[cfg(target_os = "macos")]
fn copy_private_file_to_staging(
    file: &File,
    directory: &File,
) -> Result<(std::ffi::OsString, File), String> {
    use std::os::unix::fs::FileExt;

    let (staging_name, staging_file) = allocate_private_temp(directory)?;
    let result = (|| -> Result<(), String> {
        set_private_file_permissions(&staging_file)?;
        let source_length = file
            .metadata()
            .map_err(|error| format!("failed to inspect temporary Petri configuration: {error}"))?
            .len();
        let mut offset = 0_u64;
        let mut buffer = [0_u8; 8 * 1024];
        while offset < source_length {
            let remaining = usize::try_from((source_length - offset).min(buffer.len() as u64))
                .expect("bounded Petri configuration copy length");
            let read = file
                .read_at(&mut buffer[..remaining], offset)
                .map_err(|error| {
                    format!("failed to read temporary Petri configuration by handle: {error}")
                })?;
            if read == 0 {
                return Err(
                    "temporary Petri configuration changed while preparing installation"
                        .to_string(),
                );
            }
            let mut written = 0_usize;
            while written < read {
                let count = staging_file
                    .write_at(&buffer[written..read], offset + written as u64)
                    .map_err(|error| {
                        format!("failed to stage temporary Petri configuration by handle: {error}")
                    })?;
                if count == 0 {
                    return Err(
                        "failed to stage the complete temporary Petri configuration".to_string()
                    );
                }
                written += count;
            }
            offset += read as u64;
        }
        if file
            .metadata()
            .map_err(|error| format!("failed to recheck temporary Petri configuration: {error}"))?
            .len()
            != source_length
        {
            return Err(
                "temporary Petri configuration changed while preparing installation".to_string(),
            );
        }
        staging_file
            .set_len(source_length)
            .map_err(|error| format!("failed to size staged Petri configuration: {error}"))?;
        staging_file
            .sync_all()
            .map_err(|error| format!("failed to flush staged Petri configuration: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = unlink_private_temp_if_same(&staging_file, &staging_name, directory);
        return Err(error);
    }
    Ok((staging_name, staging_file))
}

#[cfg(target_os = "macos")]
fn install_private_file(
    file: &File,
    temporary_name: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
    directory: &File,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let destination = unix_component_name(destination)?;
    let (staging_name, staging_file) = copy_private_file_to_staging(file, directory)?;
    let result = (|| -> Result<(), String> {
        let staging = checked_private_temp_name(&staging_file, &staging_name, directory)?;
        unlink_private_temp_if_same(file, temporary_name, directory)?;
        if unsafe {
            libc::renameat(
                directory.as_raw_fd(),
                staging.as_ptr(),
                directory.as_raw_fd(),
                destination.as_ptr(),
            )
        } != 0
        {
            return Err(format!(
                "failed to install Petri configuration: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = unlink_private_temp_if_same(&staging_file, &staging_name, directory);
    }
    result
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn install_private_file(
    _file: &File,
    _temporary_name: &std::ffi::OsStr,
    _destination: &std::ffi::OsStr,
    _directory: &File,
) -> Result<(), String> {
    Err(
        "handle-bound Petri configuration installation is unsupported on this Unix platform"
            .to_string(),
    )
}

#[cfg(windows)]
fn install_private_file(
    file: &File,
    _temporary_name: &std::ffi::OsStr,
    destination: &std::ffi::OsStr,
    directory: &File,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE, RtlNtStatusToDosError},
        System::IO::IO_STATUS_BLOCK,
    };

    let destination = destination.encode_wide().collect::<Vec<_>>();
    let name_bytes = destination
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| "Petri configuration destination is too long".to_string())?;
    let header_bytes = std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName);
    let buffer_bytes = header_bytes
        .checked_add(name_bytes as usize)
        .and_then(|length| length.checked_add(std::mem::size_of::<u16>()))
        .ok_or_else(|| "Petri configuration destination is too long".to_string())?;
    let buffer_words = buffer_bytes.div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; buffer_words];
    let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        (*rename).Anonymous.ReplaceIfExists = true;
        (*rename).RootDirectory = directory.as_raw_handle() as HANDLE;
        (*rename).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            destination.as_ptr(),
            std::ptr::addr_of_mut!((*rename).FileName).cast::<u16>(),
            destination.len(),
        );
    }
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle() as HANDLE,
            &mut status_block,
            rename.cast(),
            buffer_bytes as u32,
            FileRenameInformation,
        )
    };
    if status < 0 {
        let error = unsafe { RtlNtStatusToDosError(status) };
        return Err(format!(
            "failed to install Petri configuration: {}",
            io::Error::from_raw_os_error(error as i32)
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn install_private_file(
    _file: &File,
    _temporary_name: &std::ffi::OsStr,
    _destination: &std::ffi::OsStr,
    _directory: &File,
) -> Result<(), String> {
    Err("handle-bound Petri configuration installation is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn checked_private_temp_name(
    file: &File,
    temporary_name: &std::ffi::OsStr,
    directory: &File,
) -> Result<std::ffi::CString, String> {
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};

    let temporary_name = unix_component_name(temporary_name)?;
    let mut named = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            temporary_name.as_ptr(),
            &mut named,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(format!(
            "failed to inspect temporary Petri configuration through its parent handle: {}",
            io::Error::last_os_error()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!("failed to inspect opened temporary Petri configuration: {error}")
    })?;
    if named.st_dev as u64 != opened.dev()
        || named.st_ino != opened.ino()
        || named.st_mode & libc::S_IFMT != libc::S_IFREG
    {
        return Err(
            "temporary Petri configuration changed before installation; nothing was replaced"
                .to_string(),
        );
    }
    Ok(temporary_name)
}

#[cfg(unix)]
fn unlink_private_temp_if_same(
    file: &File,
    temporary_name: &std::ffi::OsStr,
    directory: &File,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    let temporary_name = checked_private_temp_name(file, temporary_name, directory)?;
    if unsafe { libc::unlinkat(directory.as_raw_fd(), temporary_name.as_ptr(), 0) } != 0 {
        return Err(format!(
            "failed to remove temporary Petri configuration through its parent handle: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn unlink_private_temp_if_same(
    file: &File,
    _temporary_name: &std::ffi::OsStr,
    _directory: &File,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::{
        Wdk::Storage::FileSystem::{
            FILE_DISPOSITION_INFORMATION, FileDispositionInformation, NtSetInformationFile,
        },
        Win32::{
            Foundation::{HANDLE, RtlNtStatusToDosError},
            System::IO::IO_STATUS_BLOCK,
        },
    };

    let disposition = FILE_DISPOSITION_INFORMATION { DeleteFile: true };
    let mut status_block = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle() as HANDLE,
            &mut status_block,
            std::ptr::addr_of!(disposition).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFORMATION>() as u32,
            FileDispositionInformation,
        )
    };
    if status < 0 {
        let error = unsafe { RtlNtStatusToDosError(status) };
        return Err(format!(
            "failed to remove the opened temporary Petri configuration: {}",
            io::Error::from_raw_os_error(error as i32)
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn unlink_private_temp_if_same(
    _file: &File,
    _temporary_name: &std::ffi::OsStr,
    _directory: &File,
) -> Result<(), String> {
    Err("handle-bound Petri configuration cleanup is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn sync_config_parent(directory: &File) -> Result<(), String> {
    match directory.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(format!(
            "failed to flush Petri configuration directory: {error}"
        )),
    }
}

#[cfg(not(unix))]
fn sync_config_parent(_directory: &File) -> Result<(), String> {
    Ok(())
}

/// Locks are stable files, never renamed or removed while another process may
/// hold them. Kernel locking is released automatically after a process crash.
pub(crate) fn open_private_lock_file(path: &Path) -> Result<File, String> {
    let path = normalize_config_write_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Lock path has no parent".to_string())?;
    let _directory = open_or_create_config_parent(parent)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(&path)
        .map_err(|_| "Could not open operation lock".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not inspect operation lock".to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Operation lock is not a regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Operation lock is a reparse point".into());
        }
    }
    set_private_file_permissions(&file)?;
    Ok(file)
}

pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let path = normalize_config_write_path(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Petri configuration path has no parent directory".to_string())?;
    let destination = path
        .file_name()
        .ok_or_else(|| "Petri configuration path must name a file".to_string())?;
    let directory = open_or_create_config_parent(parent)?;
    require_regular_or_missing_config_at(&directory, destination)?;

    let (temporary_name, mut file) = allocate_private_temp(&directory)?;
    let result = (|| -> Result<(), String> {
        set_private_file_permissions(&file)?;
        file.write_all(bytes)
            .map_err(|error| format!("failed to write Petri configuration: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to flush Petri configuration: {error}"))?;
        require_regular_or_missing_config_at(&directory, destination)?;
        install_private_file(&file, &temporary_name, destination, &directory)?;
        file.sync_all()
            .map_err(|error| format!("failed to flush installed Petri configuration: {error}"))?;
        sync_config_parent(&directory)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = unlink_private_temp_if_same(&file, &temporary_name, &directory);
    }
    result
}

fn save_to_path(path: &Path, config: &PetriUserConfig) -> Result<(), String> {
    let mut encoded = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("failed to encode Petri configuration: {error}"))?;
    encoded.push(b'\n');
    write_private_file(path, &encoded)
}

pub fn resolve_backend_url(explicit: Option<&str>) -> Result<String, String> {
    resolve_backend_url_from_path(explicit, &config_path()?)
}

fn resolve_backend_url_from_path(explicit: Option<&str>, path: &Path) -> Result<String, String> {
    if let Some(value) = explicit.filter(|value| !value.trim().is_empty()) {
        return normalize_amoeba_backend_url(value);
    }
    if let Some(value) = load_from_path(path)?.backend_url {
        return normalize_amoeba_backend_url(&value);
    }
    Ok(DEFAULT_BACKEND_URL.to_string())
}

pub fn set_backend_url(raw: &str) -> Result<PathBuf, String> {
    let path = config_path()?;
    set_backend_url_at(&path, raw)?;
    Ok(path)
}

fn set_backend_url_at(path: &Path, raw: &str) -> Result<(), String> {
    let normalized = normalize_amoeba_backend_url(raw)?;
    let mut config = load_from_path(path)?;
    config.backend_url = Some(normalized);
    save_to_path(path, &config)
}

pub fn reset_backend_url() -> Result<PathBuf, String> {
    let path = config_path()?;
    reset_backend_url_at(&path)?;
    Ok(path)
}

fn reset_backend_url_at(path: &Path) -> Result<(), String> {
    save_to_path(path, &PetriUserConfig::default())
}

/// Derives the only chain-read endpoint Petri may use from the Amoeba API base.
pub fn rpc_gateway_url(backend_url: &str) -> Result<String, String> {
    let backend_url = normalize_amoeba_backend_url(backend_url)?;
    Ok(format!("{}{}", backend_url, RPC_GATEWAY_PATH))
}
