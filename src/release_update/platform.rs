use super::{Platform, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn plain(path: &Path, directory: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "An update path is unavailable.")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Updates cannot follow Windows reparse points.".into());
        }
    }
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(
            "Updates require ordinary application files and directories, not links.".into(),
        );
    }
    Ok(())
}

pub(super) fn child_path(root: &Path, name: &str, allow_missing: bool) -> Result<PathBuf> {
    if name.is_empty()
        || name.split('/').any(|s| {
            s.is_empty()
                || matches!(s, "." | "..")
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    {
        return Err("Unsafe update file name.".into());
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = name.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(_) => plain(&path, index + 1 < parts.len())?,
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("An application file is unavailable for updating.".into()),
        }
    }
    Ok(path)
}

pub(super) fn install_root(platform: Platform) -> Result<PathBuf> {
    let executable =
        std::env::current_exe().map_err(|_| "Could not locate the installed Petri executable.")?;
    plain(&executable, false)?;
    let root = match platform {
        Platform::WindowsX64 if executable.file_name().is_some_and(|s| s == "petri.exe") => {
            executable.parent()
        }
        Platform::MacosArm64 | Platform::MacosX86_64
            if executable.ends_with("Contents/Resources/petri") =>
        {
            executable.ancestors().nth(3)
        }
        _ => None,
    }
    .ok_or("This is not a supported Petri app installation. Install the downloaded app first.")?;
    let root = fs::canonicalize(root).map_err(|_| "Could not resolve the Petri installation.")?;
    validate_root(&root)?;
    Ok(root)
}

pub(super) fn validate_root(root: &Path) -> Result<()> {
    if !root.is_absolute()
        || root.parent().is_none()
        || root.file_name().is_none()
        || root.to_string_lossy().starts_with("\\\\?\\UNC\\")
        || (root.to_string_lossy().starts_with("\\\\")
            && !root.to_string_lossy().starts_with("\\\\?\\"))
    {
        return Err("Updates require a local, user-owned Petri installation.".into());
    }
    owned_directory(root)?;
    owned_directory(root.parent().ok_or("Invalid installation parent.")?)?;
    Ok(())
}

#[cfg(unix)]
pub(super) fn owned_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    plain(path, true)?;
    let metadata =
        fs::metadata(path).map_err(|_| "Could not inspect the application directory.")?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o022 != 0 {
        return Err(
            "Update directories must be owned by you and not writable by other users.".into(),
        );
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn create_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(path)
        .map_err(|_| "Could not create a private update directory.".into())
}

#[cfg(windows)]
mod windows_acl {
    use super::*;
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, LocalFree},
        Security::{Authorization::*, *},
        Storage::FileSystem::{
            CreateDirectoryW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        },
        System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Token {
        handle: windows_sys::Win32::Foundation::HANDLE,
        data: Vec<usize>,
    }
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
    impl Token {
        fn current() -> Result<Self> {
            let mut handle = std::ptr::null_mut();
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
                return Err("Could not verify the update directory owner.".into());
            }
            let mut token = Self {
                handle,
                data: Vec::new(),
            };
            let mut size = 0;
            unsafe {
                GetTokenInformation(handle, TokenUser, std::ptr::null_mut(), 0, &mut size);
            }
            if size == 0 {
                return Err("Could not read the current Windows user.".into());
            }
            token
                .data
                .resize((size as usize).div_ceil(std::mem::size_of::<usize>()), 0);
            if unsafe {
                GetTokenInformation(
                    handle,
                    TokenUser,
                    token.data.as_mut_ptr().cast(),
                    size,
                    &mut size,
                )
            } == 0
            {
                return Err("Could not read the current Windows user.".into());
            }
            Ok(token)
        }
        fn sid(&self) -> PSID {
            unsafe { (*(self.data.as_ptr().cast::<TOKEN_USER>())).User.Sid }
        }
    }

    pub(super) fn owned(path: &Path) -> Result<()> {
        plain(path, true)?;
        let token = Token::current()?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| "Could not inspect the Windows update directory.")?;
        let mut owner = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                directory.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        let result = (|| {
            if status != ERROR_SUCCESS
                || owner.is_null()
                || dacl.is_null()
                || unsafe { EqualSid(owner, token.sid()) } == 0
            {
                return Err("Updates require a Windows directory owned by your account with a restricted ACL.".into());
            }
            let mut trusted = [[0usize; 16]; 2];
            for (buffer, kind) in trusted
                .iter_mut()
                .zip([WinLocalSystemSid, WinBuiltinAdministratorsSid])
            {
                let mut size = (buffer.len() * std::mem::size_of::<usize>()) as u32;
                if unsafe {
                    CreateWellKnownSid(
                        kind,
                        std::ptr::null_mut(),
                        buffer.as_mut_ptr().cast(),
                        &mut size,
                    )
                } == 0
                {
                    return Err("Could not verify Windows update permissions.".into());
                }
            }
            for index in 0..u32::from(unsafe { (*dacl).AceCount }) {
                let mut raw = std::ptr::null_mut();
                if unsafe { GetAce(dacl, index, &mut raw) } == 0 || raw.is_null() {
                    return Err("Invalid update directory ACL.".into());
                }
                let header = unsafe { &*raw.cast::<ACE_HEADER>() };
                if u32::from(header.AceType) == ACCESS_DENIED_ACE_TYPE {
                    continue;
                }
                if u32::from(header.AceType) != ACCESS_ALLOWED_ACE_TYPE {
                    return Err("Unsupported update directory ACL.".into());
                }
                let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
                // Generic write/all, delete, owner/DACL changes, file/directory writes.
                if ace.Mask & 0x500D_0156 == 0 {
                    continue;
                }
                let sid = std::ptr::addr_of!(ace.SidStart).cast_mut().cast();
                if unsafe { EqualSid(sid, token.sid()) } == 0
                    && !trusted
                        .iter()
                        .any(|s| unsafe { EqualSid(sid, s.as_ptr().cast_mut().cast()) } != 0)
                {
                    return Err("The Petri directory is writable by another user. Reinstall Petri in your user account.".into());
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

    pub(super) fn create(path: &Path) -> Result<()> {
        let token = Token::current()?;
        let mut sid_string = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(token.sid(), &mut sid_string) } == 0 {
            return Err("Could not secure the update directory.".into());
        }
        let mut len = 0;
        while unsafe { *sid_string.add(len) } != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_string, len) });
        unsafe {
            LocalFree(sid_string.cast());
        }
        let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})\0").encode_utf16().collect();
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err("Could not create private Windows update permissions.".into());
        }
        let security = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let path_w: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let created = unsafe { CreateDirectoryW(path_w.as_ptr(), &security) };
        unsafe {
            LocalFree(descriptor);
        }
        if created == 0 {
            return Err("Could not create a private Windows update directory.".into());
        }
        Ok(())
    }
}

#[cfg(windows)]
pub(super) fn owned_directory(path: &Path) -> Result<()> {
    windows_acl::owned(path)
}
#[cfg(windows)]
pub(super) fn create_private_directory(path: &Path) -> Result<()> {
    windows_acl::create(path)
}
#[cfg(not(any(windows, unix)))]
pub(super) fn owned_directory(_: &Path) -> Result<()> {
    Err("Unsupported update platform.".into())
}
#[cfg(not(any(windows, unix)))]
pub(super) fn create_private_directory(_: &Path) -> Result<()> {
    Err("Unsupported update platform.".into())
}

pub(super) fn set_executable(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )
        .map_err(|_| "Could not set update file permissions.")?;
    }
    #[cfg(not(unix))]
    let _ = (path, executable);
    Ok(())
}

pub(super) fn hidden_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: helper never steals focus.
    }
    command
}

pub(super) fn wait_child(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| "Could not observe the update process.")?
        {
            return Ok(status);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill(); // Only our own bounded startup probe, never another Petri instance.
            let _ = child.wait();
            return Err("The updated executable did not respond in time.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn smoke(executable: &Path, version: &str) -> Result<()> {
    let mut child = hidden_command(executable)
        .arg("--version")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "The downloaded Petri executable could not start.")?;
    if !wait_child(&mut child, Duration::from_secs(15))?.success() {
        return Err("The downloaded Petri executable failed its startup check.".into());
    }
    let bytes = super::read_bounded(child.stdout.take().ok_or("Missing version output.")?, 256)?;
    if String::from_utf8_lossy(&bytes).trim() != format!("petri {version}") {
        return Err("The downloaded executable has the wrong version.".into());
    }
    Ok(())
}

pub(super) fn verify_app(package_root: &Path, platform: Platform) -> Result<()> {
    #[cfg(target_os = "macos")]
    if platform != Platform::WindowsX64 {
        let mut child = hidden_command(Path::new("/usr/bin/codesign"))
            .args(["--verify", "--deep", "--strict"])
            .arg(package_root)
            .spawn()
            .map_err(|_| "Could not verify the Mac application's local signature.")?;
        if !wait_child(&mut child, Duration::from_secs(30))?.success() {
            return Err(
                "The Mac application's local signature is invalid. Nothing was installed.".into(),
            );
        }
    }
    let _ = (package_root, platform);
    Ok(())
}

pub(super) fn wait_parent(parent: u32) -> Result<()> {
    if parent == 0 || parent == std::process::id() {
        return Err("Invalid update parent process.".into());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent) };
        if handle.is_null() {
            return if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                Ok(())
            } else {
                Err("Could not wait for Petri to close.".into())
            };
        }
        let result = unsafe { WaitForSingleObject(handle, 120_000) };
        unsafe {
            CloseHandle(handle);
        }
        if result != WAIT_OBJECT_0 {
            return Err("Petri did not close in time. Close its other windows and retry.".into());
        }
    }
    #[cfg(unix)]
    {
        let start = Instant::now();
        while unsafe { libc::getppid() } as u32 == parent {
            if start.elapsed() > Duration::from_secs(120) {
                return Err("Petri did not close in time.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if unsafe { libc::kill(parent as i32, 0) } == 0 {
            return Err("The update was not started by the expected Petri process.".into());
        }
    }
    Ok(())
}

pub(super) fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
        }
        let alive = unsafe { WaitForSingleObject(handle, 0) } == WAIT_TIMEOUT;
        unsafe {
            CloseHandle(handle);
        }
        alive
    }
    #[cfg(unix)]
    {
        pid > 0 && unsafe { libc::kill(pid as i32, 0) } == 0
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = pid;
        true
    }
}

pub(super) fn restart(root: &Path, platform: Platform) -> Result<()> {
    if platform == Platform::WindowsX64 {
        let mut command = Command::new(root.join(platform.executable()));
        command.arg("tui").current_dir(root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0000_0010); // User-requested interactive restart in a new console.
        }
        command
            .spawn()
            .map_err(|_| "Update installed; open Petri to start the new version.")?;
    } else {
        Command::new("/usr/bin/open")
            .args(["-a", "Terminal"])
            .arg(root.join("Contents/Resources/petri.command"))
            .spawn()
            .map_err(|_| "Update installed; open Petri to start the new version.")?;
    }
    Ok(())
}
