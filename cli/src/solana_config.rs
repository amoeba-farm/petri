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
    }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        env::temp_dir().join(format!("petri-cli-{name}-{unique}.yml"))
    }

    #[test]
    fn parses_only_the_signer_from_solana_cli_config() {
        let path = temp_config_path("solana-config");
        {
            let mut file = fs::File::create(&path).expect("create temp config");
            writeln!(
                file,
                "json_rpc_url: https://ignored.invalid\nkeypair_path: /tmp/ameba-test-id.json\ncommitment: confirmed"
            )
            .expect("write temp config");
        }

        let config =
            load_solana_cli_config(Some(path.to_str().expect("utf8 path"))).expect("load config");
        let config = config.expect("explicit config should load");

        assert_eq!(
            config.keypair_path.as_deref(),
            Some("/tmp/ameba-test-id.json")
        );

        fs::remove_file(path).ok();
    }

    #[test]
    fn config_relative_keypair_path_resolves_from_config_directory() {
        let path = temp_config_path("relative-keypair-config");
        let keypair_path = path
            .parent()
            .expect("temp config dir")
            .join("petri-relative-keypair.json");
        fs::File::create(&keypair_path).expect("create dummy keypair path");
        {
            let mut file = fs::File::create(&path).expect("create temp config");
            writeln!(file, "keypair_path: petri-relative-keypair.json").expect("write temp config");
        }

        let config =
            load_solana_cli_config(Some(path.to_str().expect("utf8 path"))).expect("load config");
        let config = config.expect("explicit config should load");

        assert_eq!(
            config.keypair_path.as_deref(),
            Some(keypair_path.to_string_lossy().as_ref())
        );

        fs::remove_file(path).ok();
        fs::remove_file(keypair_path).ok();
    }

    #[test]
    fn remote_wallet_urls_are_not_treated_as_missing_files() {
        assert!(is_remote_wallet_path("usb://ledger?key=0"));
        assert!(is_remote_wallet_path(
            " USB://ledger/Wallet111111111111111111111111111111111?key=0/0 "
        ));
        assert!(!is_remote_wallet_path("/tmp/id.json"));
        assert_eq!(
            keypair_file_security_label("usb://ledger?key=0"),
            "hardware-wallet"
        );
    }

    #[test]
    fn preopen_keypair_check_rejects_non_files_before_opening_them() {
        let path = temp_config_path("preopen-directory");
        fs::create_dir_all(&path).expect("create directory fixture");
        assert_eq!(
            preopen_keypair_file_security_label(path.to_str().expect("utf8 path")),
            "not-a-file"
        );
        fs::remove_dir_all(path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn unix_owner_and_mode_classifier_keeps_foreign_ownership_non_overrideable() {
        assert_eq!(
            classify_unix_keypair_owner_and_mode(1001, 1000, 0o600),
            Some("foreign-owner")
        );
        assert_eq!(
            classify_unix_keypair_owner_and_mode(1001, 1000, 0o644),
            Some("foreign-owner")
        );
        assert_eq!(
            classify_unix_keypair_owner_and_mode(1000, 1000, 0o644),
            Some("warn:group-or-world-readable")
        );
        assert_eq!(
            classify_unix_keypair_owner_and_mode(1000, 1000, 0o600),
            None
        );
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn unix_filesystem_magic_classifier_rejects_network_and_fuse_types() {
        for magic in [
            0x0000_517b_u64,
            0x0000_6969,
            0x0102_1997,
            0x6573_5546,
            0x0bd0_0bd0,
            0x4750_4653,
            0xaad7_aaea,
            0xff53_4d42,
        ] {
            assert_eq!(
                classify_linux_filesystem_magic(magic),
                UnixOpenedFilesystemLocation::NetworkOrUserspace
            );
        }
        for magic in [
            0x0000_ef53_u64,
            0x0102_1994,
            0x2fc1_2fc1,
            0x5846_5342,
            0x794c_7630,
            0x9123_683e,
        ] {
            assert_eq!(
                classify_linux_filesystem_magic(magic),
                UnixOpenedFilesystemLocation::Local
            );
        }
        assert_eq!(
            classify_linux_filesystem_magic(0xdead_beef),
            UnixOpenedFilesystemLocation::Unrecognized
        );
        // Signed 32-bit f_type values must classify identically after widening.
        assert_eq!(
            classify_linux_filesystem_magic(0xffff_ffff_ff53_4d42),
            UnixOpenedFilesystemLocation::NetworkOrUserspace
        );
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd"
    ))]
    #[test]
    fn unix_filesystem_name_classifier_rejects_network_and_fuse_types() {
        for name in [
            &b"nfs"[..],
            &b"smbfs\0\0"[..],
            &b"fuse.sshfs"[..],
            &b"macfuse"[..],
            &b"9p"[..],
        ] {
            assert_eq!(
                classify_named_unix_filesystem(name),
                UnixOpenedFilesystemLocation::NetworkOrUserspace
            );
        }
        assert_eq!(
            classify_named_unix_filesystem(b"apfs"),
            UnixOpenedFilesystemLocation::Local
        );
        assert_eq!(
            classify_named_unix_filesystem(b"surprisefs"),
            UnixOpenedFilesystemLocation::Unrecognized
        );
    }

    #[cfg(unix)]
    #[test]
    fn preopen_keypair_check_rejects_symlinked_ancestors() {
        use std::os::unix::fs::symlink;

        let root = temp_config_path("preopen-ancestor-root");
        let real = root.join("real");
        let link = root.join("link");
        fs::create_dir_all(&real).expect("create real directory");
        fs::write(real.join("id.json"), b"[]").expect("write keypair fixture");
        symlink(&real, &link).expect("create ancestor symlink");
        assert_eq!(
            preopen_keypair_file_security_label(link.join("id.json").to_str().expect("utf8 path")),
            "symlink-or-reparse-point"
        );
        fs::remove_file(link).ok();
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn local_keypair_symlinks_fail_closed_even_when_target_is_private() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let target = temp_config_path("private-keypair-target");
        let link = temp_config_path("private-keypair-link");
        fs::write(&target, b"[]").expect("write private keypair fixture");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
            .expect("secure keypair fixture");
        symlink(&target, &link).expect("create keypair symlink");

        assert_eq!(
            keypair_file_security_label(link.to_str().expect("utf8 link")),
            "symlink-or-reparse-point"
        );
        fs::remove_file(link).ok();
        fs::remove_file(target).ok();
    }

    #[cfg(unix)]
    #[test]
    fn local_keypair_permission_label_uses_the_file_it_will_sign_with() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_config_path("keypair-mode");
        fs::write(&path, b"[]").expect("write keypair fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("set broad keypair mode");
        assert_eq!(
            keypair_file_security_label(path.to_str().expect("utf8 path")),
            "warn:group-or-world-readable"
        );
        let opened = fs::File::open(&path).expect("open broad keypair fixture");
        assert_eq!(
            opened_keypair_file_security_label(&opened),
            "warn:group-or-world-readable"
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("set private keypair mode");
        assert_eq!(
            keypair_file_security_label(path.to_str().expect("utf8 path")),
            "ok"
        );
        assert_eq!(opened_keypair_file_security_label(&opened), "ok");

        fs::remove_file(path).ok();
    }

    #[cfg(windows)]
    #[test]
    fn windows_network_keypair_paths_fail_closed_before_file_access() {
        assert_eq!(
            keypair_file_security_label(r"\\server\share\id.json"),
            "network-path"
        );
        assert_eq!(
            keypair_file_security_label(r"\\?\UNC\server\share\id.json"),
            "network-path"
        );
        assert_eq!(
            preopen_keypair_file_security_label(r"\\server\share\id.json"),
            "network-path"
        );
        assert_eq!(
            preopen_keypair_file_security_label(r"\\.\pipe\petri-keypair"),
            "device-or-verbatim-path"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_remote_protocol_probe_classification_is_fail_closed() {
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_SUCCESS,
        };

        assert_eq!(
            classify_windows_remote_protocol_probe(true, ERROR_SUCCESS),
            WindowsOpenedFileLocation::Remote
        );
        assert_eq!(
            classify_windows_remote_protocol_probe(true, ERROR_ACCESS_DENIED),
            WindowsOpenedFileLocation::Remote
        );
        assert_eq!(
            classify_windows_remote_protocol_probe(false, ERROR_INVALID_PARAMETER),
            WindowsOpenedFileLocation::Local
        );
        assert_eq!(
            classify_windows_remote_protocol_probe(false, ERROR_ACCESS_DENIED),
            WindowsOpenedFileLocation::Unreadable
        );

        let initialized = initialized_windows_remote_protocol_info();
        assert_eq!(initialized.StructureVersion, 2);
        assert_eq!(
            usize::from(initialized.StructureSize),
            std::mem::size_of::<windows_sys::Win32::Storage::FileSystem::FILE_REMOTE_PROTOCOL_INFO>(
            )
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_local_open_handle_supports_remote_and_acl_security_queries() {
        use windows_sys::Win32::Security::IsValidSid;

        let path = temp_config_path("windows-open-handle-security");
        fs::write(&path, b"[]").expect("write local keypair fixture");
        let opened = fs::File::open(&path).expect("open local keypair fixture");

        let current_user_sid =
            windows_current_process_user_sid().expect("read current process TokenUser SID");
        assert_ne!(unsafe { IsValidSid(current_user_sid.as_psid()) }, 0);

        assert_eq!(
            windows_opened_file_location(&opened),
            WindowsOpenedFileLocation::Local
        );
        let label = windows_opened_keypair_file_security_label(&opened);
        assert_ne!(label, "warn:windows-acl-unreadable");
        assert_ne!(label, "windows-current-user-unreadable");
        assert_ne!(label, "windows-owner-not-current-user");

        drop(opened);
        fs::remove_file(path).ok();
    }

    #[cfg(windows)]
    #[test]
    fn windows_foreign_write_ace_is_a_hard_failure() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::{ERROR_SUCCESS, LocalFree},
            Security::{
                ACL,
                Authorization::{
                    BuildTrusteeWithSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
                    SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW,
                },
                CreateWellKnownSid, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
                PSECURITY_DESCRIPTOR, PSID, SECURITY_MAX_SID_SIZE, WinWorldSid,
            },
        };

        let path = temp_config_path("windows-foreign-write-ace");
        fs::write(&path, b"[]").expect("write ACL fixture");
        let wide_path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();

        let mut old_dacl: *mut ACL = std::ptr::null_mut();
        let mut old_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let query_status = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut old_dacl,
                std::ptr::null_mut(),
                &mut old_descriptor,
            )
        };
        assert_eq!(query_status, ERROR_SUCCESS);
        assert!(!old_dacl.is_null());

        let mut everyone = [0_u8; SECURITY_MAX_SID_SIZE as usize];
        let mut everyone_size = SECURITY_MAX_SID_SIZE;
        assert_ne!(
            unsafe {
                CreateWellKnownSid(
                    WinWorldSid,
                    std::ptr::null_mut(),
                    everyone.as_mut_ptr().cast(),
                    &mut everyone_size,
                )
            },
            0
        );
        let everyone_sid = everyone.as_mut_ptr().cast::<core::ffi::c_void>() as PSID;
        let mut entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: 0x0000_0002, // FILE_WRITE_DATA
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            ..EXPLICIT_ACCESS_W::default()
        };
        unsafe {
            BuildTrusteeWithSidW(&mut entry.Trustee, everyone_sid);
        }
        let mut modified_dacl: *mut ACL = std::ptr::null_mut();
        let build_status = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut modified_dacl) };
        assert_eq!(build_status, ERROR_SUCCESS);
        assert!(!modified_dacl.is_null());

        let set_status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                modified_dacl,
                std::ptr::null_mut(),
            )
        };
        let label = if set_status == ERROR_SUCCESS {
            let opened = fs::File::open(&path).expect("open ACL fixture after adding write ACE");
            windows_opened_keypair_file_security_label(&opened)
        } else {
            String::new()
        };
        let restore_status = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                old_dacl,
                std::ptr::null_mut(),
            )
        };
        unsafe {
            LocalFree(modified_dacl.cast());
            LocalFree(old_descriptor);
        }
        fs::remove_file(path).ok();

        assert_eq!(set_status, ERROR_SUCCESS);
        assert_eq!(restore_status, ERROR_SUCCESS);
        assert_eq!(label, "windows-broad-write-access");
    }

    #[cfg(windows)]
    #[test]
    fn windows_acl_read_masks_fail_closed() {
        use windows_sys::Win32::{
            Security::{
                CreateWellKnownSid, SECURITY_MAX_SID_SIZE, WELL_KNOWN_SID_TYPE,
                WinBuiltinAdministratorsSid, WinLocalSystemSid, WinWorldSid,
            },
            System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_ALLOWED_CALLBACK_ACE_TYPE},
        };

        fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> [u8; SECURITY_MAX_SID_SIZE as usize] {
            let mut sid = [0_u8; SECURITY_MAX_SID_SIZE as usize];
            let mut size = SECURITY_MAX_SID_SIZE;
            assert_ne!(
                unsafe {
                    CreateWellKnownSid(
                        kind,
                        std::ptr::null_mut(),
                        sid.as_mut_ptr().cast(),
                        &mut size,
                    )
                },
                0
            );
            sid
        }

        assert!(windows_access_mask_grants_file_read(0x0000_0001));
        assert!(windows_access_mask_grants_file_read(0x8000_0000));
        assert!(windows_access_mask_grants_file_read(0x1000_0000));
        assert!(!windows_access_mask_grants_file_read(0x0000_0002));
        for mutation_mask in [
            0x0000_0002_u32,
            0x0000_0004,
            0x0001_0000,
            0x0004_0000,
            0x0008_0000,
            0x1000_0000,
            0x4000_0000,
        ] {
            assert!(windows_access_mask_grants_file_mutation(mutation_mask));
        }
        assert!(!windows_access_mask_grants_file_mutation(0x0000_0001));
        assert!(windows_dacl_allow_ace_is_fully_parsed(
            ACCESS_ALLOWED_ACE_TYPE
        ));
        assert!(!windows_dacl_allow_ace_is_fully_parsed(
            ACCESS_ALLOWED_CALLBACK_ACE_TYPE
        ));

        let current_user =
            windows_current_process_user_sid().expect("read current process TokenUser SID");
        let local_system = well_known_sid(WinLocalSystemSid);
        let administrators = well_known_sid(WinBuiltinAdministratorsSid);
        let everyone = well_known_sid(WinWorldSid);
        let sid_ptr = |sid: &[u8; SECURITY_MAX_SID_SIZE as usize]| {
            sid.as_ptr().cast_mut().cast::<core::ffi::c_void>()
        };
        assert!(windows_ace_sid_is_allowed(
            current_user.as_psid(),
            current_user.as_psid(),
            sid_ptr(&local_system),
            sid_ptr(&administrators),
        ));
        assert!(windows_ace_sid_is_allowed(
            sid_ptr(&local_system),
            current_user.as_psid(),
            sid_ptr(&local_system),
            sid_ptr(&administrators),
        ));
        assert!(windows_ace_sid_is_allowed(
            sid_ptr(&administrators),
            current_user.as_psid(),
            sid_ptr(&local_system),
            sid_ptr(&administrators),
        ));
        assert!(!windows_ace_sid_is_allowed(
            sid_ptr(&everyone),
            current_user.as_psid(),
            sid_ptr(&local_system),
            sid_ptr(&administrators),
        ));
    }

    #[test]
    fn missing_default_config_is_not_an_error() {
        let loaded = load_solana_cli_config(None).expect("default config lookup");
        if let Some(config) = loaded {
            assert!(config.path.ends_with(".config/solana/cli/config.yml"));
        }
    }

    #[test]
    fn explicit_missing_config_is_an_error() {
        let path = temp_config_path("missing");
        let error =
            load_solana_cli_config(Some(path.to_str().expect("utf8 path"))).expect_err("missing");
        assert!(error.to_string().contains("does not exist"));
    }
}
