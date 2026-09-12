use std::{
    env, fs,
    path::{Path, PathBuf},
};

use yaml_rust2::{Yaml, YamlLoader};

use crate::backend::CliError;

#[derive(Clone, Debug, Default)]
pub struct SolanaCliConfig {
    pub path: PathBuf,
    pub keypair_path: Option<String>,
    pub commitment: Option<String>,
}

pub fn expand_tilde(raw: &str) -> String {
    if raw == "~" {
        return user_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        let home = user_home_dir();
        return PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    raw.to_string()
}

pub fn default_solana_config_path() -> PathBuf {
    let home = user_home_dir();
    PathBuf::from(home).join(".config/solana/cli/config.yml")
}

pub fn default_keypair_path() -> String {
    let home = user_home_dir();
    PathBuf::from(home)
        .join(".config/solana/id.json")
        .to_string_lossy()
        .into_owned()
}

pub fn resolve_config_path(config_path: Option<&str>) -> PathBuf {
    config_path
        .map(|path| PathBuf::from(expand_tilde(path.trim())))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(default_solana_config_path)
}

pub fn load_solana_cli_config(
    config_path: Option<&str>,
) -> Result<Option<SolanaCliConfig>, CliError> {
    let path = resolve_config_path(config_path);
    if !path.exists() {
        if config_path.is_some() {
            return Err(CliError::new(format!(
                "Solana config file {} does not exist",
                path.display()
            )));
        }
        return Ok(None);
    }

    let raw = fs::read_to_string(&path).map_err(|error| {
        CliError::new(format!(
            "failed to read Solana config {}: {error}",
            path.display()
        ))
    })?;
    let docs = YamlLoader::load_from_str(&raw).map_err(|error| {
        CliError::new(format!(
            "failed to parse Solana config {}: {error}",
            path.display()
        ))
    })?;
    let parsed = docs
        .first()
        .ok_or_else(|| CliError::new(format!("Solana config {} is empty", path.display())))?;

    Ok(Some(SolanaCliConfig {
        keypair_path: yaml_string(parsed, "keypair_path")
            .map(|keypair_path| resolve_config_keypair_path(&keypair_path, &path)),
        path,
        commitment: yaml_string(parsed, "commitment"),
    }))
}

pub fn resolve_keypair_path(
    explicit_keypair_path: Option<&str>,
    solana_config: Option<&SolanaCliConfig>,
) -> String {
    clean(explicit_keypair_path.map(ToString::to_string))
        .map(|path| expand_tilde(&path))
        .or_else(|| {
            solana_config
                .and_then(|config| config.keypair_path.clone())
                .and_then(|path| clean(Some(path)))
        })
        .unwrap_or_else(default_keypair_path)
}

pub fn resolve_commitment(
    explicit_commitment: Option<&str>,
    solana_config: Option<&SolanaCliConfig>,
) -> Option<String> {
    clean(explicit_commitment.map(ToString::to_string)).or_else(|| {
        solana_config
            .and_then(|config| config.commitment.clone())
            .and_then(|value| clean(Some(value)))
    })
}

pub fn keypair_file_security_label(path: &str) -> String {
    if is_remote_wallet_path(path) {
        return "hardware-wallet".to_string();
    }

    let preopen_label = preopen_keypair_file_security_label(path);
    if preopen_label != "ok" {
        return preopen_label;
    }

    let path = Path::new(path);
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return "missing".to_string(),
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return "symlink-or-reparse-point".to_string();
    }
    if !metadata.is_file() {
        return "not-a-file".to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return "warn:group-or-world-readable".to_string();
        }
    }

    #[cfg(windows)]
    {
        return windows_keypair_file_security_label(path);
    }

    #[cfg(not(windows))]
    {
        "ok".to_string()
    }
}

pub(crate) fn preopen_keypair_file_security_label(path: &str) -> String {
    if is_remote_wallet_path(path) {
        return "hardware-wallet".to_string();
    }

    let path = match std::path::absolute(Path::new(path)) {
        Ok(path) => path,
        Err(_) => return "path-unreadable".to_string(),
    };
    #[cfg(windows)]
    match windows_preopen_path_location(&path) {
        WindowsPreopenPathLocation::Local => {}
        WindowsPreopenPathLocation::Remote => return "network-path".to_string(),
        WindowsPreopenPathLocation::UnsafePrefix => {
            return "device-or-verbatim-path".to_string();
        }
        WindowsPreopenPathLocation::Unreadable => {
            return "drive-status-unreadable".to_string();
        }
    }

    use std::path::Component;
    let mut inspected = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        if matches!(component, Component::ParentDir) {
            return "parent-traversal".to_string();
        }
        inspected.push(component.as_os_str());
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&inspected) {
            Ok(metadata) => metadata,
            Err(_) => return "missing".to_string(),
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return "symlink-or-reparse-point".to_string();
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return "ancestor-not-a-directory".to_string();
            }
        } else if !metadata.is_file() {
            return "not-a-file".to_string();
        }
    }
    "ok".to_string()
}

pub(crate) fn opened_keypair_file_security_label(file: &fs::File) -> String {
    #[cfg(windows)]
    match windows_opened_file_location(file) {
        WindowsOpenedFileLocation::Local => {}
        WindowsOpenedFileLocation::Remote => return "network-path".to_string(),
        WindowsOpenedFileLocation::Unreadable => {
            return "remote-file-status-unreadable".to_string();
        }
    }
    let opened_metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return "changed-during-open".to_string(),
    };
    if metadata_is_link_or_reparse_point(&opened_metadata) || !opened_metadata.is_file() {
        return "changed-during-open".to_string();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let filesystem_label = unix_opened_file_filesystem_security_label(file);
        if filesystem_label != "ok" {
            return filesystem_label.to_string();
        }
        if let Some(label) = classify_unix_keypair_owner_and_mode(
            opened_metadata.uid(),
            unsafe { libc::geteuid() },
            opened_metadata.permissions().mode(),
        ) {
            return label.to_string();
        }
    }

    #[cfg(windows)]
    {
        return windows_opened_keypair_file_security_label(file);
    }

    #[cfg(not(windows))]
    {
        "ok".to_string()
    }
}

#[cfg(unix)]
fn classify_unix_keypair_owner_and_mode(
    file_uid: u32,
    effective_uid: u32,
    mode: u32,
) -> Option<&'static str> {
    if file_uid != effective_uid {
        // Ownership is not an overrideable permissions warning. A different
        // account can replace the contents without Petri's signer controlling
        // that change, even when the visible mode bits are 0600.
        return Some("foreign-owner");
    }
    if mode & 0o077 != 0 {
        return Some("warn:group-or-world-readable");
    }
    None
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnixOpenedFilesystemLocation {
    Local,
    NetworkOrUserspace,
    Unrecognized,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn classify_linux_filesystem_magic(magic: u64) -> UnixOpenedFilesystemLocation {
    // Linux exposes f_type as a signed word on some architectures. Masking to
    // 32 bits preserves constants such as CIFS_MAGIC_NUMBER on both widths.
    let magic = magic & 0xffff_ffff;
    match magic {
        0x0000_3434 // NILFS_SUPER_MAGIC
        | 0x0000_ef53 // EXT2/3/4_SUPER_MAGIC
        | 0x0102_1994 // TMPFS_MAGIC
        | 0x2405_1905 // UBIFS_SUPER_MAGIC
        | 0x2fc1_2fc1 // ZFS_SUPER_MAGIC
        | 0x3153_464a // JFS_SUPER_MAGIC
        | 0x5265_4973 // REISERFS_SUPER_MAGIC
        | 0x5846_5342 // XFS_SUPER_MAGIC
        | 0x794c_7630 // OVERLAYFS_SUPER_MAGIC
        | 0x8584_58f6 // RAMFS_MAGIC
        | 0x9123_683e // BTRFS_SUPER_MAGIC
        | 0xca45_1a4e // BCACHEFS_SUPER_MAGIC
        | 0xf2f5_2010 => UnixOpenedFilesystemLocation::Local, // F2FS_SUPER_MAGIC
        0x0000_517b // SMB_SUPER_MAGIC
        | 0x0000_564c // NCP_SUPER_MAGIC
        | 0x0000_6969 // NFS_SUPER_MAGIC
        | 0x0bd0_0bd0 // LUSTRE_SUPER_MAGIC
        | 0x00c3_6400 // CEPH_SUPER_MAGIC
        | 0x0102_1997 // V9FS_MAGIC
        | 0x0116_1970 // GFS2_MAGIC
        | 0x4750_4653 // GPFS_SUPER_MAGIC
        | 0x5346_414f // AFS_SUPER_MAGIC
        | 0x6573_5546 // FUSE_SUPER_MAGIC
        | 0x7461_636f // OCFS2_SUPER_MAGIC
        | 0x7375_7245 // CODA_SUPER_MAGIC
        | 0xaad7_aaea // PANFS_SUPER_MAGIC
        | 0xff53_4d42 => UnixOpenedFilesystemLocation::NetworkOrUserspace, // CIFS_MAGIC_NUMBER
        _ => UnixOpenedFilesystemLocation::Unrecognized,
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd"
))]
fn classify_named_unix_filesystem(name: &[u8]) -> UnixOpenedFilesystemLocation {
    let name = String::from_utf8_lossy(name).to_ascii_lowercase();
    let name = name.trim_end_matches('\0');
    if matches!(
        name,
        "9p" | "afs"
            | "ceph"
            | "cifs"
            | "coda"
            | "davfs"
            | "macfuse"
            | "ncpfs"
            | "nfs"
            | "nfs4"
            | "osxfuse"
            | "smbfs"
            | "sshfs"
            | "webdav"
    ) || name.starts_with("fuse")
    {
        UnixOpenedFilesystemLocation::NetworkOrUserspace
    } else if matches!(
        name,
        "apfs"
            | "bcachefs"
            | "btrfs"
            | "devfs"
            | "ext2"
            | "ext3"
            | "ext4"
            | "f2fs"
            | "hfs"
            | "hfs+"
            | "nilfs"
            | "nullfs"
            | "tmpfs"
            | "ufs"
            | "unionfs"
            | "xfs"
            | "zfs"
    ) {
        UnixOpenedFilesystemLocation::Local
    } else {
        UnixOpenedFilesystemLocation::Unrecognized
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn unix_opened_file_filesystem_security_label(file: &fs::File) -> &'static str {
    use std::os::fd::AsRawFd;

    let mut status = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), status.as_mut_ptr()) } != 0 {
        return "filesystem-status-unreadable";
    }
    let status = unsafe { status.assume_init() };
    match classify_linux_filesystem_magic(status.f_type as u64) {
        UnixOpenedFilesystemLocation::Local => "ok",
        UnixOpenedFilesystemLocation::NetworkOrUserspace => "network-or-userspace-filesystem",
        UnixOpenedFilesystemLocation::Unrecognized => "filesystem-type-unrecognized",
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd"
))]
fn unix_opened_file_filesystem_security_label(file: &fs::File) -> &'static str {
    use std::os::fd::AsRawFd;

    let mut status = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), status.as_mut_ptr()) } != 0 {
        return "filesystem-status-unreadable";
    }
    let status = unsafe { status.assume_init() };
    let filesystem_name = status
        .f_fstypename
        .iter()
        .map(|byte| *byte as u8)
        .take_while(|byte| *byte != 0)
        .collect::<Vec<_>>();
    match classify_named_unix_filesystem(&filesystem_name) {
        UnixOpenedFilesystemLocation::Local => "ok",
        UnixOpenedFilesystemLocation::NetworkOrUserspace => "network-or-userspace-filesystem",
        UnixOpenedFilesystemLocation::Unrecognized => "filesystem-type-unrecognized",
    }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd"
    ))
))]
fn unix_opened_file_filesystem_security_label(_file: &fs::File) -> &'static str {
    // Unknown Unix statfs layouts are not accepted without a handle-bound
    // filesystem classifier.
    "filesystem-status-unavailable"
}

fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn is_windows_network_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
    )
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsPreopenPathLocation {
    Local,
    Remote,
    UnsafePrefix,
    Unreadable,
}

#[cfg(windows)]
fn windows_preopen_path_location(path: &Path) -> WindowsPreopenPathLocation {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    use windows_sys::Win32::{
        Storage::FileSystem::GetDriveTypeW,
        System::WindowsProgramming::{DRIVE_NO_ROOT_DIR, DRIVE_REMOTE, DRIVE_UNKNOWN},
    };

    if is_windows_network_path(path) {
        return WindowsPreopenPathLocation::Remote;
    }
    let drive = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) => drive,
            _ => return WindowsPreopenPathLocation::UnsafePrefix,
        },
        _ => return WindowsPreopenPathLocation::Unreadable,
    };
    let root = PathBuf::from(format!("{}:\\", char::from(drive)))
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    match unsafe { GetDriveTypeW(root.as_ptr()) } {
        DRIVE_REMOTE => WindowsPreopenPathLocation::Remote,
        DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => WindowsPreopenPathLocation::Unreadable,
        _ => WindowsPreopenPathLocation::Local,
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsOpenedFileLocation {
    Local,
    Remote,
    Unreadable,
}

#[cfg(windows)]
fn initialized_windows_remote_protocol_info()
-> windows_sys::Win32::Storage::FileSystem::FILE_REMOTE_PROTOCOL_INFO {
    use windows_sys::Win32::Storage::FileSystem::FILE_REMOTE_PROTOCOL_INFO;

    let mut remote_info = FILE_REMOTE_PROTOCOL_INFO::default();
    // The Win32 contract requires both input fields. A zeroed structure makes
    // the resulting ERROR_INVALID_PARAMETER indistinguishable from the local-
    // file result and would therefore make the remote probe fail open.
    remote_info.StructureVersion = 2;
    remote_info.StructureSize = std::mem::size_of::<FILE_REMOTE_PROTOCOL_INFO>()
        .try_into()
        .expect("FILE_REMOTE_PROTOCOL_INFO size must fit in u16");
    remote_info
}

#[cfg(windows)]
fn classify_windows_remote_protocol_probe(
    succeeded: bool,
    error: windows_sys::Win32::Foundation::WIN32_ERROR,
) -> WindowsOpenedFileLocation {
    use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;

    if succeeded {
        WindowsOpenedFileLocation::Remote
    } else if error == ERROR_INVALID_PARAMETER {
        // FileRemoteProtocolInfo returns ERROR_INVALID_PARAMETER for a local
        // file. Every other failure is ambiguous and therefore fails closed.
        WindowsOpenedFileLocation::Local
    } else {
        WindowsOpenedFileLocation::Unreadable
    }
}

#[cfg(windows)]
fn windows_opened_file_location(file: &fs::File) -> WindowsOpenedFileLocation {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{GetLastError, HANDLE},
        Storage::FileSystem::{
            FILE_REMOTE_PROTOCOL_INFO, FileRemoteProtocolInfo, GetFileInformationByHandleEx,
        },
    };

    let mut remote_info = initialized_windows_remote_protocol_info();
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileRemoteProtocolInfo,
            std::ptr::addr_of_mut!(remote_info).cast(),
            std::mem::size_of::<FILE_REMOTE_PROTOCOL_INFO>() as u32,
        )
    } != 0;
    let error = if succeeded {
        windows_sys::Win32::Foundation::ERROR_SUCCESS
    } else {
        unsafe { GetLastError() }
    };
    classify_windows_remote_protocol_probe(succeeded, error)
}

#[cfg(windows)]
fn windows_keypair_file_security_label(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        ACL,
        Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    windows_security_descriptor_label(status, dacl, owner, descriptor)
}

#[cfg(windows)]
fn windows_opened_keypair_file_security_label(file: &fs::File) -> String {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Security::{
            ACL,
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
    };

    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    windows_security_descriptor_label(status, dacl, owner, descriptor)
}

#[cfg(windows)]
fn windows_security_descriptor_label(
    status: windows_sys::Win32::Foundation::WIN32_ERROR,
    dacl: *mut windows_sys::Win32::Security::ACL,
    owner: windows_sys::Win32::Security::PSID,
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
) -> String {
    use windows_sys::Win32::{
        Foundation::{ERROR_SUCCESS, LocalFree},
        Security::{
            ACCESS_ALLOWED_ACE, CreateWellKnownSid, EqualSid, GetAce, PSID, SECURITY_MAX_SID_SIZE,
            WinBuiltinAdministratorsSid, WinLocalSystemSid,
        },
        System::SystemServices::ACCESS_DENIED_ACE_TYPE,
    };

    let result = if status != ERROR_SUCCESS {
        "warn:windows-acl-unreadable".to_string()
    } else {
        (|| {
            if dacl.is_null() || owner.is_null() {
                return "warn:windows-null-dacl".to_string();
            }
            let current_user_sid = match windows_current_process_user_sid() {
                Some(sid) => sid,
                None => return "windows-current-user-unreadable".to_string(),
            };
            if unsafe { EqualSid(owner, current_user_sid.as_psid()) } == 0 {
                // Unlike a broad mode/ACE warning, foreign ownership is not
                // overrideable: that owner can replace the signing material.
                return "windows-owner-not-current-user".to_string();
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
                    return "warn:windows-acl-unreadable".to_string();
                }
            }

            let ace_count = unsafe { (*dacl).AceCount };
            let mut broad_read_access = false;
            for index in 0..u32::from(ace_count) {
                let mut raw_ace = std::ptr::null_mut();
                if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 || raw_ace.is_null() {
                    return "warn:windows-acl-unreadable".to_string();
                }
                let header =
                    unsafe { &*(raw_ace.cast::<windows_sys::Win32::Security::ACE_HEADER>()) };
                let ace_type = u32::from(header.AceType);
                if ace_type == ACCESS_DENIED_ACE_TYPE {
                    continue;
                }
                if !windows_dacl_allow_ace_is_fully_parsed(ace_type) {
                    // Object/callback ACE layouts need different SID offsets.
                    // Treating an unparsed allow ACE as overrideable could hide
                    // a foreign write grant, so it is a hard failure.
                    return "windows-unparsed-dacl-ace".to_string();
                }
                let ace = unsafe { &*(raw_ace.cast::<ACCESS_ALLOWED_ACE>()) };
                let ace_sid = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
                let principal_allowed = windows_ace_sid_is_allowed(
                    ace_sid,
                    current_user_sid.as_psid(),
                    allowed_sids[0]
                        .as_ptr()
                        .cast_mut()
                        .cast::<core::ffi::c_void>() as PSID,
                    allowed_sids[1]
                        .as_ptr()
                        .cast_mut()
                        .cast::<core::ffi::c_void>() as PSID,
                );
                if !principal_allowed && windows_access_mask_grants_file_mutation(ace.Mask) {
                    return "windows-broad-write-access".to_string();
                }
                if !principal_allowed && windows_access_mask_grants_file_read(ace.Mask) {
                    broad_read_access = true;
                }
            }
            if broad_read_access {
                return "warn:windows-broad-read-access".to_string();
            }
            "ok".to_string()
        })()
    };
    if !descriptor.is_null() {
        unsafe {
            LocalFree(descriptor);
        }
    }
    result
}

#[cfg(windows)]
struct OwnedWindowsSid {
    // usize storage keeps the SID naturally aligned while retaining ownership
    // after the process-token query buffer is released.
    words: Vec<usize>,
}

#[cfg(windows)]
impl OwnedWindowsSid {
    fn as_psid(&self) -> windows_sys::Win32::Security::PSID {
        self.words.as_ptr().cast_mut().cast::<core::ffi::c_void>()
    }
}

#[cfg(windows)]
fn windows_current_process_user_sid() -> Option<OwnedWindowsSid> {
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
    // -1 is the documented current-process pseudo handle. NtOpenProcessToken
    // avoids resolving the account by name, which is ambiguous for domain and
    // local accounts with the same display name.
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
    let word_count = (required as usize).div_ceil(word_size);
    let mut token_user_storage = vec![0_usize; word_count];
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
    Some(OwnedWindowsSid { words })
}

#[cfg(windows)]
fn windows_dacl_allow_ace_is_fully_parsed(ace_type: u32) -> bool {
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
    ace_type == ACCESS_ALLOWED_ACE_TYPE
}

#[cfg(windows)]
fn windows_ace_sid_is_allowed(
    ace_sid: windows_sys::Win32::Security::PSID,
    current_user_sid: windows_sys::Win32::Security::PSID,
    local_system_sid: windows_sys::Win32::Security::PSID,
    administrators_sid: windows_sys::Win32::Security::PSID,
) -> bool {
    use windows_sys::Win32::Security::EqualSid;

    [current_user_sid, local_system_sid, administrators_sid]
        .into_iter()
        .any(|allowed_sid| unsafe { EqualSid(ace_sid, allowed_sid) } != 0)
}

#[cfg(windows)]
fn windows_access_mask_grants_file_read(mask: u32) -> bool {
    const FILE_READ_DATA: u32 = 0x0000_0001;
    const GENERIC_ALL: u32 = 0x1000_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    mask & (FILE_READ_DATA | GENERIC_ALL | GENERIC_READ) != 0
}

#[cfg(windows)]
fn windows_access_mask_grants_file_mutation(mask: u32) -> bool {
    const FILE_WRITE_DATA: u32 = 0x0000_0002;
    const FILE_APPEND_DATA: u32 = 0x0000_0004;
    const DELETE: u32 = 0x0001_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    mask & (FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_ALL
        | GENERIC_WRITE)
        != 0
}

pub fn is_remote_wallet_path(path: &str) -> bool {
    path.trim_start().to_ascii_lowercase().starts_with("usb://")
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn user_home_dir() -> String {
    clean(env::var("HOME").ok())
        .or_else(|| clean(env::var("USERPROFILE").ok()))
        .or_else(|| {
            let drive = clean(env::var("HOMEDRIVE").ok())?;
            let path = clean(env::var("HOMEPATH").ok())?;
            clean(Some(format!("{drive}{path}")))
        })
        .unwrap_or_else(|| ".".to_string())
}

fn resolve_config_keypair_path(raw: &str, config_path: &Path) -> String {
    let expanded = expand_tilde(raw);
    let expanded_path = PathBuf::from(&expanded);
    if expanded_path.is_absolute() {
        return expanded;
    }

    if let Some(config_dir) = config_path.parent() {
        let candidate = config_dir.join(&expanded_path);
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }

    let home_candidate = PathBuf::from(user_home_dir()).join(&expanded_path);
    if home_candidate.exists() {
        return home_candidate.to_string_lossy().into_owned();
    }

    expanded
}

fn yaml_string(root: &Yaml, key: &str) -> Option<String> {
    match &root[key] {
        Yaml::String(value) => clean(Some(value.clone())),
        Yaml::Real(value) => clean(Some(value.clone())),
        Yaml::Integer(value) => Some(value.to_string()),
        _ => None,
    }
}
