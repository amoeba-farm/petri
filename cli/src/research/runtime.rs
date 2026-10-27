//! How petri runs its own research processes: the command an agent's shell
//! uses for petri, and the background workspace service's detachment from
//! the caller's pipes on Windows.

#[cfg(windows)]
pub(super) fn isolate_caller_pipes() -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
        Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
        System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
    };
    // The detached workspace service must not keep a shell/agent's output
    // pipe open after this command exits. Command still duplicates the explicit stdio it
    // needs; unrelated inherited pipes must not reach later descendants.
    for stream in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let handle = GetStdHandle(stream);
            if !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && GetFileType(handle) == FILE_TYPE_PIPE
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) == 0
            {
                return Err("Could not isolate the research process output.".into());
            }
        }
    }
    Ok(())
}

/// How an agent's shell runs this petri, for the steps research hands it
/// (`work next`): `PETRI_COMMAND` (`petri-live petri` on a dev PC), else
/// this executable's own path, quoted for the shell.
pub(super) fn petri_command() -> String {
    if let Ok(command) = std::env::var("PETRI_COMMAND")
        && !command.trim().is_empty()
    {
        return command;
    }
    let path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "petri".into());
    if cfg!(windows) {
        format!("& '{}'", path.replace('\'', "''"))
    } else {
        format!("'{}'", path.replace('\'', "'\"'\"'"))
    }
}
