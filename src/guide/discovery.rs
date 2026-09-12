//! Bounded local discovery. Never search a project, source a shell profile, or
//! run a package manager (which could install software during a status check).

use super::{GuideProviderKind, dedupe_paths, scrub_guide_environment, user_home_dirs};
use std::{
    env,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

const MAX_LAYOUT_ENTRIES: usize = 64;
const MAX_CANDIDATES: usize = 64;

pub(super) fn candidates(kind: GuideProviderKind) -> Vec<PathBuf> {
    let (name, override_key) = match kind {
        GuideProviderKind::Codex => ("codex", "PETRI_CODEX_CLI"),
        GuideProviderKind::ClaudeCode => ("claude", "PETRI_CLAUDE_CLI"),
        GuideProviderKind::GeminiCli => ("gemini", "PETRI_GEMINI_CLI"),
        GuideProviderKind::GrokBuild => ("grok", "PETRI_GROK_CLI"),
    };
    // An explicit choice is authoritative. Do not silently use another account
    // or installation when the chosen executable is broken.
    if let Some(value) = env::var_os(override_key).filter(|value| !value.is_empty()) {
        let path = expand_home(PathBuf::from(value));
        let paths = if path.is_absolute() {
            vec![path]
        } else if path.components().count() == 1 {
            search_dirs()
                .iter()
                .flat_map(|dir| {
                    if path.extension().is_none() {
                        names_in(dir, path.to_str().unwrap_or(name))
                    } else {
                        vec![dir.join(&path)]
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        return resolve_candidates(paths, kind);
    }
    let mut paths = Vec::new();
    for dir in search_dirs() {
        paths.extend(names_in(&dir, name));
    }
    for home in user_home_dirs() {
        match kind {
            GuideProviderKind::Codex => {
                paths.extend(names_in(
                    &home.join(".codex/plugins/.plugin-appserver"),
                    name,
                ));
                for app in ["Codex.app", "ChatGPT.app"] {
                    paths.extend(names_in(
                        &home
                            .join("Applications")
                            .join(app)
                            .join("Contents/Resources"),
                        name,
                    ));
                }
            }
            GuideProviderKind::ClaudeCode => {
                paths.extend(names_in(&home.join(".claude/local"), name))
            }
            GuideProviderKind::GrokBuild => paths.extend(names_in(&home.join(".grok/bin"), name)),
            GuideProviderKind::GeminiCli => {}
        }
    }
    match kind {
        GuideProviderKind::Codex => {
            if let Some(home) = env_path("CODEX_HOME") {
                paths.extend(names_in(&home.join("plugins/.plugin-appserver"), name));
            }
            #[cfg(target_os = "macos")]
            for app in ["Codex.app", "ChatGPT.app"] {
                paths.extend(names_in(
                    &PathBuf::from("/Applications")
                        .join(app)
                        .join("Contents/Resources"),
                    name,
                ));
            }
            #[cfg(windows)]
            if let Some(local) = env_path("LOCALAPPDATA") {
                let root = local.join("OpenAI/Codex/bin");
                paths.extend(names_in(&root, name));
                for version in children(&root) {
                    paths.extend(names_in(&version, name));
                }
                for version in children(&local.join("Programs/Codex")) {
                    paths.extend(names_in(&version.join("resources"), name));
                }
            }
        }
        GuideProviderKind::ClaudeCode => {
            if let Some(home) = env_path("CLAUDE_CONFIG_DIR") {
                paths.extend(names_in(&home.join("local"), name));
            }
        }
        GuideProviderKind::GrokBuild => {
            if let Some(dir) = env_path("GROK_BIN_DIR") {
                paths.extend(names_in(&dir, name));
            }
            if let Some(home) = env_path("GROK_HOME") {
                paths.extend(names_in(&home.join("bin"), name));
            }
        }
        GuideProviderKind::GeminiCli => {}
    }
    #[cfg(windows)]
    for hive in [
        windows_sys::Win32::System::Registry::HKEY_CURRENT_USER,
        windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE,
    ] {
        let key = format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{name}.exe");
        if let Some(path) = registry_string(hive, &key, "") {
            paths.push(PathBuf::from(path.to_string_lossy().trim_matches('"')));
        }
    }
    resolve_candidates(paths, kind)
}

fn expand_home(path: PathBuf) -> PathBuf {
    if let Ok(suffix) = path.strip_prefix("~")
        && let Some(home) = user_home_dirs().into_iter().next()
    {
        return home.join(suffix);
    }
    path
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    #[cfg(windows)]
    for (hive, key) in [
        (
            windows_sys::Win32::System::Registry::HKEY_CURRENT_USER,
            "Environment",
        ),
        (
            windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ),
    ] {
        if let Some(path) = registry_string(hive, key, "Path") {
            dirs.extend(env::split_paths(&path));
        }
    }
    for key in [
        "NPM_CONFIG_PREFIX",
        "npm_config_prefix",
        "PNPM_HOME",
        "NVM_BIN",
        "NVM_SYMLINK",
        "CONDA_PREFIX",
        "GROK_BIN_DIR",
    ] {
        if let Some(dir) = env_path(key) {
            dirs.push(dir.clone());
            dirs.push(dir.join("bin"));
        }
    }
    for key in ["VOLTA_HOME", "BUN_INSTALL", "HOMEBREW_PREFIX"] {
        if let Some(dir) = env_path(key) {
            dirs.push(dir.join("bin"));
        }
    }
    if let Some(dir) = env_path("XDG_DATA_HOME") {
        dirs.push(dir.join("pnpm"));
    }
    let mut layouts = Vec::new();
    for home in user_home_dirs() {
        for suffix in [
            ".local/bin",
            "bin",
            ".npm-global",
            ".npm-global/bin",
            ".npm/bin",
            ".yarn/bin",
            ".volta/bin",
            ".bun/bin",
            ".local/share/pnpm",
            ".asdf/shims",
            ".local/share/mise/shims",
            ".nix-profile/bin",
            "scoop/shims",
        ] {
            dirs.push(home.join(suffix));
        }
        for suffix in [
            ".nvm/versions/node",
            ".asdf/installs/nodejs",
            ".local/share/mise/installs/node",
        ] {
            layouts.push((home.join(suffix), "bin"));
        }
        layouts.push((
            home.join(".local/share/fnm/node-versions"),
            "installation/bin",
        ));
    }
    if let Some(root) = env_path("NVM_DIR") {
        layouts.push((root.join("versions/node"), "bin"));
    }
    if let Some(root) = env_path("NVM_HOME") {
        layouts.push((root, ""));
    }
    if let Some(root) = env_path("FNM_DIR") {
        layouts.push((
            root.join("node-versions"),
            if cfg!(windows) {
                "installation"
            } else {
                "installation/bin"
            },
        ));
    }
    if let Some(root) = env_path("MISE_DATA_DIR") {
        layouts.push((root.join("installs/node"), "bin"));
    }
    if let Some(root) = env_path("ASDF_DATA_DIR") {
        layouts.push((root.join("installs/nodejs"), "bin"));
    }
    #[cfg(windows)]
    {
        if let Some(roaming) = env_path("APPDATA") {
            dirs.push(roaming.join("npm"));
        }
        if let Some(local) = env_path("LOCALAPPDATA") {
            for suffix in [
                "Microsoft/WindowsApps",
                "Microsoft/WinGet/Links",
                "pnpm",
                "Volta/bin",
                "Programs/nodejs",
            ] {
                dirs.push(local.join(suffix));
            }
            layouts.push((local.join("fnm/node-versions"), "installation"));
            layouts.push((local.join("nvm"), ""));
        }
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = env_path(key) {
                dirs.push(dir.join("nodejs"));
            }
        }
    }
    #[cfg(unix)]
    for dir in [
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
        "/snap/bin",
        "/run/current-system/sw/bin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    for (root, suffix) in layouts {
        for version in children(&root) {
            dirs.push(version.join(suffix));
        }
    }
    dedupe_paths(dirs.into_iter().filter(|dir| dir.is_absolute()).collect())
}

fn children(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths = entries
        .take(MAX_LAYOUT_ENTRIES)
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    // Newest app/runtime first, without embedding a version or username.
    paths.sort_by_cached_key(|path| {
        std::cmp::Reverse(fs::metadata(path).and_then(|m| m.modified()).ok())
    });
    paths
}

fn names_in(dir: &Path, name: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    let names = [
        format!("{name}.exe"),
        format!("{name}.cmd"),
        format!("{name}.bat"),
        format!("{name}.ps1"),
        name.to_string(),
    ];
    #[cfg(not(windows))]
    let names = [name.to_string()];
    names.into_iter().map(|name| dir.join(name)).collect()
}

fn package_name(kind: GuideProviderKind) -> Option<&'static str> {
    match kind {
        GuideProviderKind::Codex => Some("@openai/codex"),
        GuideProviderKind::ClaudeCode => Some("@anthropic-ai/claude-code"),
        GuideProviderKind::GeminiCli => Some("@google/gemini-cli"),
        GuideProviderKind::GrokBuild => None,
    }
}

fn resolve_candidates(paths: Vec<PathBuf>, kind: GuideProviderKind) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| path.is_absolute() && path.is_file())
        .filter_map(|path| {
            let resolved = resolve_npm_shim(&path, kind).unwrap_or(path);
            #[cfg(windows)]
            if resolved.extension().is_none() {
                return None;
            } // POSIX npm wrapper, not a Windows program.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&resolved).ok()?.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            let canonical = fs::canonicalize(&resolved).unwrap_or_else(|_| resolved.clone());
            let key = if cfg!(windows) {
                canonical.to_string_lossy().to_lowercase()
            } else {
                canonical.to_string_lossy().into_owned()
            };
            if !seen.insert(key) {
                return None;
            }
            // Unix npm/Homebrew commands are commonly symlinks to JS. Use the
            // corresponding Node runtime even when a GUI's PATH omits Node.
            #[cfg(unix)]
            if canonical
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| matches!(extension, "js" | "mjs" | "cjs"))
            {
                return Some(canonical);
            }
            Some(resolved)
        })
        .take(MAX_CANDIDATES)
        .collect()
}

// Standard npm shims route to the package-declared entry point. Resolve them
// without cmd.exe/PowerShell quoting or a shell's expansion of prompt content.
fn resolve_npm_shim(path: &Path, kind: GuideProviderKind) -> Option<PathBuf> {
    if !cfg!(windows)
        || path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let name = match kind {
        GuideProviderKind::Codex => "codex",
        GuideProviderKind::ClaudeCode => "claude",
        GuideProviderKind::GeminiCli => "gemini",
        GuideProviderKind::GrokBuild => "grok",
    };
    if !path.file_stem()?.eq_ignore_ascii_case(name)
        || !path.extension().is_none_or(|extension| {
            ["cmd", "bat", "ps1"]
                .iter()
                .any(|name| extension.eq_ignore_ascii_case(name))
        })
    {
        return None;
    }
    // Do not replace an unrelated custom wrapper just because it happens to
    // live beside an npm installation.
    let mut shim = String::new();
    fs::File::open(path)
        .ok()?
        .take(16 * 1024)
        .read_to_string(&mut shim)
        .ok()?;
    let package = package_name(kind)?;
    if !shim
        .replace('\\', "/")
        .contains(&format!("node_modules/{package}/"))
    {
        return None;
    }
    let root = path.parent()?.join("node_modules").join(package);
    let mut bytes = Vec::new();
    fs::File::open(root.join("package.json"))
        .ok()?
        .take(128 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if manifest.get("name")?.as_str()? != package {
        return None;
    }
    let bin = manifest.get("bin")?;
    let entry = bin.as_str().or_else(|| {
        bin.get(match kind {
            GuideProviderKind::Codex => "codex",
            GuideProviderKind::ClaudeCode => "claude",
            GuideProviderKind::GeminiCli => "gemini",
            GuideProviderKind::GrokBuild => "grok",
        })
        .and_then(serde_json::Value::as_str)
    })?;
    let entry = fs::canonicalize(root.join(entry)).ok()?;
    (entry.starts_with(fs::canonicalize(root).ok()?) && entry.is_file()).then_some(entry)
}

pub(super) fn command(executable: &Path) -> Result<Command, String> {
    let extension = executable
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut command = if matches!(extension.as_str(), "js" | "mjs" | "cjs") {
        let mut dirs = Vec::new();
        // A version-manager installation must use its own adjacent Node before
        // an unrelated system runtime. Do not alter the parent process PATH.
        for ancestor in executable.ancestors().skip(1).take(10) {
            if ancestor
                .file_name()
                .is_some_and(|name| name == "node_modules")
            {
                if let Some(prefix) = ancestor.parent() {
                    dirs.push(prefix.to_path_buf());
                    dirs.push(prefix.join("bin"));
                    if prefix.file_name().is_some_and(|name| name == "lib")
                        && let Some(root) = prefix.parent()
                    {
                        dirs.push(root.join("bin"));
                    }
                }
                break;
            }
        }
        dirs.extend(search_dirs());
        let node = dirs
            .iter()
            .flat_map(|dir| names_in(dir, "node"))
            .find(|path| {
                path.is_file()
                    && (!cfg!(windows)
                        || path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("exe")))
            })
            .ok_or_else(|| {
                "The installed CLI needs Node.js, but its runtime could not be found.".to_string()
            })?;
        let mut command = Command::new(&node);
        scrub_guide_environment(&mut command);
        command.arg(executable);
        if let Some(parent) = node.parent() {
            prepend_path(&mut command, parent);
        }
        command
    } else if cfg!(windows) && extension == "ps1" {
        let system = env_path("SYSTEMROOT")
            .ok_or_else(|| "Windows could not locate PowerShell.".to_string())?;
        let mut command =
            Command::new(system.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        scrub_guide_environment(&mut command);
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(executable);
        command
    } else {
        let mut command = Command::new(executable);
        scrub_guide_environment(&mut command);
        if let Some(directory) = executable.parent() {
            prepend_path(&mut command, directory);
        }
        command
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW: probes must not flash consoles.
    }
    command.env("NO_COLOR", "1");
    Ok(command)
}

fn prepend_path(command: &mut Command, directory: &Path) {
    let mut dirs = vec![directory.to_path_buf()];
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }
    if let Ok(path) = env::join_paths(dirs) {
        command.env("PATH", path);
    }
}

#[cfg(windows)]
fn registry_string(
    hive: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    name: &str,
) -> Option<std::ffi::OsString> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let key = key.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut buffer = vec![0u16; 32_768];
    let mut size = (buffer.len() * 2) as u32;
    // Fixed-size read only; the API expands REG_EXPAND_SZ using this user's environment.
    let result = unsafe {
        RegGetValueW(
            hive,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result != 0 {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0)?;
    Some(std::ffi::OsString::from_wide(&buffer[..end]))
}
