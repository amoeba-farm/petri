use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::backend::CliError;

pub(crate) const LOCAL_DRAFT_STATUS: &str =
    "queued semantic draft; SDK transaction preparation not wired";
const MAX_DRAFT_STORE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct OracleSubmissionField {
    pub(crate) label: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct OracleSubmissionDraft {
    pub(crate) id: String,
    pub(crate) created_at_unix_seconds: u64,
    pub(crate) market_id: String,
    pub(crate) month_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expiry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) oracle_month: Option<String>,
    pub(crate) action: String,
    pub(crate) phase: String,
    pub(crate) breadcrumb: Vec<String>,
    pub(crate) node_label: String,
    pub(crate) node_kind: String,
    pub(crate) row_label: String,
    pub(crate) source_state: String,
    pub(crate) update_state: Option<String>,
    pub(crate) modules: String,
    pub(crate) summary: String,
    pub(crate) fields: Vec<OracleSubmissionField>,
    pub(crate) backend_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
struct OracleSubmissionStore {
    submissions: Vec<OracleSubmissionDraft>,
}

#[allow(dead_code)]
pub(crate) fn default_path() -> Option<PathBuf> {
    env::var_os("AMEBA_ORACLE_SUBMISSIONS_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("APPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    path.join("Amoeba")
                        .join("Petri")
                        .join("oracle_submissions.json")
                })
        })
        .or_else(|| {
            env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    path.join("Amoeba")
                        .join("Petri")
                        .join("oracle_submissions.json")
                })
        })
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    path.join("amoeba")
                        .join("petri")
                        .join("oracle_submissions.json")
                })
        })
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| {
                    path.join(".config")
                        .join("amoeba")
                        .join("petri")
                        .join("oracle_submissions.json")
                })
        })
}

pub(crate) fn load_recent_at_path(
    path: &Path,
    limit: usize,
) -> Result<Vec<OracleSubmissionDraft>, CliError> {
    let mut submissions = load_all_at_path(path)?;
    submissions.sort_by_key(|submission| submission.created_at_unix_seconds);
    let keep_from = submissions.len().saturating_sub(limit);
    Ok(submissions.split_off(keep_from))
}

pub(crate) fn load_all_at_path(path: &Path) -> Result<Vec<OracleSubmissionDraft>, CliError> {
    Ok(read_store(path)?.submissions)
}

pub(crate) fn append_at_path(
    path: &Path,
    draft: OracleSubmissionDraft,
) -> Result<OracleSubmissionDraft, CliError> {
    append_validated_at_path(path, draft, |_| Ok(())).map(|(saved, ())| saved)
}

pub(crate) fn append_validated_at_path<T>(
    path: &Path,
    mut draft: OracleSubmissionDraft,
    validate: impl FnOnce(&OracleSubmissionDraft) -> Result<T, CliError>,
) -> Result<(OracleSubmissionDraft, T), CliError> {
    let mut store = read_store(path)?;
    let now = now_unix_seconds();
    draft.created_at_unix_seconds = now;
    draft.id = format!(
        "oracle-{}-{}-{}",
        draft.market_id.to_ascii_lowercase(),
        now,
        store.submissions.len() + 1
    );
    draft.backend_status = LOCAL_DRAFT_STATUS.to_string();
    let validated = validate(&draft)?;
    store.submissions.push(draft.clone());
    write_store(path, &store)?;
    Ok((draft, validated))
}

fn read_store(path: &Path) -> Result<OracleSubmissionStore, CliError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(CliError::new(
                "oracle submission draft store must be a regular file, not a link",
            ));
        }
        Ok(metadata) if metadata.len() > MAX_DRAFT_STORE_BYTES => {
            return Err(CliError::new(format!(
                "oracle submission draft store exceeds Petri's {MAX_DRAFT_STORE_BYTES}-byte limit"
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::new(format!(
                "failed to inspect oracle submission drafts: {error}"
            )));
        }
    }
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| {
            CliError::new(format!("failed to parse oracle submission drafts: {error}"))
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(OracleSubmissionStore::default())
        }
        Err(error) => Err(CliError::new(format!(
            "failed to read oracle submission drafts: {error}"
        ))),
    }
}

fn write_store(path: &Path, store: &OracleSubmissionStore) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::new(format!(
                "failed to create oracle submission draft directory: {error}"
            ))
        })?;
    }
    let contents = serde_json::to_string_pretty(store).map_err(|error| {
        CliError::new(format!(
            "failed to serialize oracle submission drafts: {error}"
        ))
    })?;
    if contents.len() as u64 > MAX_DRAFT_STORE_BYTES {
        return Err(CliError::new(format!(
            "oracle submission draft store exceeds Petri's {MAX_DRAFT_STORE_BYTES}-byte limit"
        )));
    }
    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(CliError::new(
            "oracle submission draft store must be a regular file, not a link",
        ));
    }

    let parent = path
        .parent()
        .ok_or_else(|| CliError::new("oracle submission draft path has no parent directory"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temporary = None;
    for attempt in 0..16 {
        let candidate = parent.join(format!(
            ".oracle_submissions.{}.{nonce}.{attempt}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(CliError::new(format!(
                    "failed to create temporary oracle submission draft store: {error}"
                )));
            }
        }
    }
    let (temporary_path, mut file) = temporary.ok_or_else(|| {
        CliError::new("failed to allocate a unique temporary oracle submission draft store")
    })?;
    let result = (|| -> Result<(), CliError> {
        file.write_all(contents.as_bytes()).map_err(|error| {
            CliError::new(format!("failed to write oracle submission drafts: {error}"))
        })?;
        file.sync_all().map_err(|error| {
            CliError::new(format!("failed to flush oracle submission drafts: {error}"))
        })?;
        drop(file);
        install_store_file(&temporary_path, path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(unix)]
fn install_store_file(temporary_path: &Path, path: &Path) -> Result<(), CliError> {
    fs::rename(temporary_path, path).map_err(|error| {
        CliError::new(format!(
            "failed to install oracle submission draft store: {error}"
        ))
    })
}

#[cfg(windows)]
fn install_store_file(temporary_path: &Path, path: &Path) -> Result<(), CliError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(CliError::new(format!(
            "failed to install oracle submission draft store: {}",
            io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
