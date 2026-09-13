use super::{
    HELPER_ARG, Platform, Release, Result, archive, file_hash, platform, valid_hash, version,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LOCK: &str = ".petri-update.lock";
const LAST: &str = ".petri-update.last";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    protocol: u32,
    platform: Platform,
    from_version: String,
    to_version: String,
    app_name: Option<String>,
    parent_pid: u32,
    restart: bool,
    created_at: u64,
    files: BTreeMap<String, String>,
    originals: BTreeMap<String, Option<String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    protocol: u32,
    status: String,
    helper_pid: u32,
    message: String,
}

fn mac_storage_name(app_name: &str) -> Result<&'static str> {
    match app_name {
        "Petri.app" => Ok(".petri-app-updates"),
        "Petri Preview.app" => Ok(".petri-preview-app-updates"),
        _ => {
            Err("Install Petri using its original app name before using standalone updates.".into())
        }
    }
}

fn storage_root(root: &Path, platform: Platform) -> Result<PathBuf> {
    if platform == Platform::WindowsX64 {
        return Ok(root.to_path_buf());
    }
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid Petri app name.")?;
    Ok(root
        .parent()
        .ok_or("Invalid Petri app directory.")?
        .join(mac_storage_name(name)?))
}

fn root_from_stage(stage: &Path, plan: &Plan) -> Result<PathBuf> {
    let storage = stage.parent().ok_or("Invalid update directory.")?;
    if plan.platform == Platform::WindowsX64 {
        if plan.app_name.is_some() {
            return Err("Invalid Windows update target.".into());
        }
        return Ok(storage.to_path_buf());
    }
    let app_name = plan
        .app_name
        .as_deref()
        .ok_or("Missing Mac update target.")?;
    if storage
        .file_name()
        .is_none_or(|s| s != mac_storage_name(app_name).unwrap_or(""))
    {
        return Err("The update staging directory does not belong to this Mac app.".into());
    }
    platform::owned_directory(storage)?;
    Ok(storage
        .parent()
        .ok_or("Invalid Mac update directory.")?
        .join(app_name))
}

fn nonce() -> Result<String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| "Could not create a secure update identifier.")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn stage_name(value: &str) -> bool {
    value.strip_prefix(".petri-update-").is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn stage_at(root: &Path, name: &str) -> Result<PathBuf> {
    if !stage_name(name) {
        return Err("Invalid update recovery reference.".into());
    }
    let stage = root.join(name);
    platform::owned_directory(&stage)?;
    Ok(stage)
}
fn read_reference(root: &Path, name: &str) -> Result<String> {
    let path = platform::child_path(root, name, false)?;
    let value = String::from_utf8(super::read_bounded(
        File::open(path).map_err(|_| "Could not read update recovery information.")?,
        128,
    )?)
    .map_err(|_| "Invalid update recovery information.")?;
    if !stage_name(&value) {
        return Err("Invalid update recovery reference.".into());
    }
    Ok(value)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Could not create update recovery information.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save update recovery information.".into())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if path
        .try_exists()
        .map_err(|_| "Could not inspect update state.")?
    {
        platform::plain(path, false)?;
    }
    let parent = path.parent().ok_or("Invalid update state path.")?;
    let temporary = parent.join(format!(".petri-state-{}", nonce()?));
    write_new(&temporary, bytes)?;
    fs::rename(&temporary, path).map_err(|_| "Could not commit update recovery information.".into())
}
fn save_plan(stage: &Path, plan: &Plan) -> Result<()> {
    atomic_write(
        &stage.join("plan.json"),
        &serde_json::to_vec(plan).map_err(|_| "Could not encode update recovery information.")?,
    )
}
fn receipt(stage: &Path, status: &str, message: &str) -> Result<()> {
    let value = Receipt {
        protocol: 1,
        status: status.into(),
        helper_pid: std::process::id(),
        message: message.into(),
    };
    atomic_write(
        &stage.join("status.json"),
        &serde_json::to_vec(&value).map_err(|_| "Could not encode update status.")?,
    )
}
fn read_json<T: serde::de::DeserializeOwned>(stage: &Path, name: &str) -> Result<T> {
    let path = platform::child_path(stage, name, false)?;
    serde_json::from_slice(&super::read_bounded(
        File::open(path).map_err(|_| "Missing update recovery information.")?,
        32_768,
    )?)
    .map_err(|_| "Invalid update recovery information.".into())
}
fn package_root(stage: &Path, platform: Platform) -> PathBuf {
    if platform == Platform::WindowsX64 {
        stage.join("package")
    } else {
        stage.join("package/Petri.app")
    }
}

fn verify_plan(stage: &Path, plan: &Plan) -> Result<PathBuf> {
    let root = root_from_stage(stage, plan)?;
    platform::validate_root(&root)?;
    platform::owned_directory(stage)?;
    if !stage
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(stage_name)
        || plan.protocol != 1
        || plan.files.is_empty()
        || plan.files.len() > 32
        || plan.files.len() != plan.originals.len()
        || version(&plan.to_version)? <= version(&plan.from_version)?
    {
        return Err("The update recovery plan is invalid.".into());
    }
    for (name, hash) in &plan.files {
        if !archive::managed(plan.platform, name)
            || !valid_hash(hash)
            || !plan.originals.contains_key(name)
        {
            return Err("The update recovery plan contains an unowned file.".into());
        }
        platform::child_path(&root, name, true)?;
        if let Some(old) = &plan.originals[name] {
            if !valid_hash(old)
                || file_hash(&platform::child_path(&stage.join("backup"), name, false)?)? != *old
            {
                return Err("The previous installation's recovery copy is invalid.".into());
            }
        }
    }
    let old_binary = plan
        .originals
        .get(plan.platform.executable())
        .and_then(Option::as_ref)
        .ok_or("The update has no original executable for recovery.")?;
    let helper = platform::child_path(stage, plan.platform.helper(), false)?;
    if file_hash(&helper)? != *old_binary {
        return Err("The saved update helper differs from the original executable.".into());
    }
    Ok(root)
}

fn copy_durable(source: &Path, destination: &Path) -> Result<()> {
    platform::plain(source, false)?;
    let parent = destination
        .parent()
        .ok_or("Invalid update file destination.")?;
    fs::create_dir_all(parent).map_err(|_| "Could not create an update backup directory.")?;
    if destination
        .try_exists()
        .map_err(|_| "Could not inspect the update backup.")?
    {
        return Err("The update backup already exists.".into());
    }
    let mut input = File::open(source).map_err(|_| "Could not read an application file.")?;
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not create the recovery copy.")?;
    std::io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|_| "Could not save the recovery copy. Check free disk space.")?;
    fs::set_permissions(
        destination,
        input
            .metadata()
            .map_err(|_| "Could not inspect application permissions.")?
            .permissions(),
    )
    .map_err(|_| "Could not preserve application file permissions.".into())
}

pub(super) fn pending() -> Result<Option<String>> {
    let root = platform::install_root(Platform::current()?)?;
    let storage = storage_root(&root, Platform::current()?)?;
    if !storage
        .try_exists()
        .map_err(|_| "Could not inspect update recovery information.")?
    {
        return Ok(None);
    }
    platform::owned_directory(&storage)?;
    match fs::symlink_metadata(storage.join(LOCK)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect a pending update.".into()),
        Ok(_) => (),
    }
    let name = read_reference(&storage, LOCK)?;
    let stage = stage_at(&storage, &name)?;
    let message = match read_json::<Receipt>(&stage, "status.json") {
        Ok(record) if record.protocol == 1 && platform::process_alive(record.helper_pid) => {
            "A Petri update is already running. Close other Petri windows and let it finish."
        }
        _ => {
            "A Petri update was interrupted. Run petri update recover to restore the previous files."
        }
    };
    Ok(Some(message.into()))
}

pub(super) fn stage_and_launch(release: &Release, restart: bool) -> Result<()> {
    let root = platform::install_root(release.platform)?;
    let storage = storage_root(&root, release.platform)?;
    if !storage
        .try_exists()
        .map_err(|_| "Could not inspect the update directory.")?
    {
        platform::create_private_directory(&storage)?;
    }
    platform::owned_directory(&storage)?;
    let name = format!(".petri-update-{}", nonce()?);
    let stage = storage.join(&name);
    // Exclusive update ownership; never remove somebody else's pending lock.
    write_new(&storage.join(LOCK), name.as_bytes()).map_err(|_| "Another update is pending. Run petri update check, or petri update recover after an interruption.")?;
    let prepared = (|| {
        platform::create_private_directory(&stage)?;
        receipt(
            &stage,
            "downloading",
            "Downloading the approved release; application files are unchanged.",
        )?;
        let archive_path = stage.join("download.zip");
        super::download(release, &archive_path)?;
        let files = archive::extract(&archive_path, &stage.join("package"), release)?;
        let package = package_root(&stage, release.platform);
        platform::verify_app(&package, release.platform)?;
        platform::smoke(
            &package.join(release.platform.executable()),
            &release.version,
        )?;
        let mut originals = BTreeMap::new();
        platform::create_private_directory(&stage.join("backup"))?;
        for name in files.keys() {
            let target = platform::child_path(&root, name, true)?;
            let old = match fs::symlink_metadata(&target) {
                Ok(_) => {
                    platform::plain(&target, false)?;
                    let hash = file_hash(&target)?;
                    let backup = stage.join("backup").join(name);
                    copy_durable(&target, &backup)?;
                    if file_hash(&backup)? != hash {
                        return Err(
                            "An application file changed during backup. Retry the update.".into(),
                        );
                    }
                    Some(hash)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err("Could not inspect an installed application file.".into()),
            };
            originals.insert(name.clone(), old);
        }
        let plan = Plan {
            protocol: 1,
            platform: release.platform,
            from_version: env!("CARGO_PKG_VERSION").into(),
            app_name: (release.platform != Platform::WindowsX64).then(|| {
                root.file_name()
                    .expect("validated app name")
                    .to_string_lossy()
                    .into_owned()
            }),
            to_version: release.version.clone(),
            parent_pid: std::process::id(),
            restart,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "The system clock is invalid.")?
                .as_secs(),
            files,
            originals,
        };
        copy_durable(
            &root.join(release.platform.executable()),
            &stage.join(release.platform.helper()),
        )?;
        platform::set_executable(&stage.join(release.platform.helper()), true)?;
        save_plan(&stage, &plan)?;
        verify_plan(&stage, &plan)?;
        launch(&stage, &plan, false)
    })();
    if prepared.is_err() {
        let _ = receipt(
            &stage,
            "preparation-failed",
            "The download or preparation failed. Application files were not changed.",
        );
        release_lock(&storage, &name)?;
        // This random private stage was created by this call, and no app file
        // was replaced. Do not accumulate incomplete multi-megabyte downloads.
        if stage_at(&storage, &name).is_ok() {
            let _ = fs::remove_dir_all(&stage);
        }
    }
    prepared
}

fn launch(stage: &Path, plan: &Plan, recovering: bool) -> Result<()> {
    let ready = stage.join("ready");
    if ready
        .try_exists()
        .map_err(|_| "Could not inspect the update helper.")?
    {
        platform::plain(&ready, false)?;
        fs::remove_file(&ready).map_err(|_| "Could not prepare the update helper.")?;
    }
    let mut command = platform::hidden_command(&stage.join(plan.platform.helper()));
    command.arg(HELPER_ARG).current_dir(stage);
    if recovering {
        command.arg("recover");
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the update helper. Nothing was installed.")?;
    let start = Instant::now();
    loop {
        if ready.is_file() {
            return Ok(());
        }
        if child
            .try_wait()
            .map_err(|_| "Could not observe the update helper.")?
            .is_some()
        {
            return Err(
                "The update helper could not validate this installation. Nothing was installed."
                    .into(),
            );
        }
        if start.elapsed() >= Duration::from_secs(30) {
            let _ = child.kill(); // The helper we just created; no application process is terminated.
            let _ = child.wait();
            return Err("The update helper did not start in time. Nothing was installed.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn release_lock(root: &Path, name: &str) -> Result<()> {
    if read_reference(root, LOCK)? != name {
        return Err("Update ownership changed; recovery information was retained.".into());
    }
    fs::remove_file(root.join(LOCK)).map_err(|_| "Could not release the update lock.".into())
}

fn target_hash(root: &Path, name: &str) -> Result<Option<String>> {
    let path = platform::child_path(root, name, true)?;
    match fs::symlink_metadata(&path) {
        Ok(_) => file_hash(&path).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("Could not inspect the installed application files.".into()),
    }
}

fn rollback(stage: &Path, root: &Path, plan: &Plan) -> Result<()> {
    // Validate all targets first. Never overwrite a user's concurrent file change.
    for (name, new) in &plan.files {
        let current = target_hash(root, name)?;
        if current != plan.originals[name] && current.as_deref() != Some(new) {
            return Err("An application file was changed outside this update. Recovery copies were retained; no unknown file was overwritten.".into());
        }
    }
    let restore = stage.join(format!("restore-{}", nonce()?));
    platform::create_private_directory(&restore)?;
    let mut names: Vec<_> = plan.files.keys().collect();
    names.sort_by_key(|name| *name == plan.platform.executable()); // Executable is activated last.
    for name in names {
        let target = platform::child_path(root, name, true)?;
        if target_hash(root, name)? == plan.originals[name] {
            continue;
        }
        if let Some(expected) = &plan.originals[name] {
            let source = platform::child_path(&stage.join("backup"), name, false)?;
            if file_hash(&source)? != *expected {
                return Err("A recovery file has been altered; it was not installed.".into());
            }
            let replacement = restore.join(name);
            copy_durable(&source, &replacement)?;
            fs::rename(&replacement, &target).map_err(|_| "Could not restore an application file. Close other Petri windows and run petri update recover.")?;
        } else {
            fs::remove_file(&target)
                .map_err(|_| "Could not remove a file added by the interrupted update.")?;
        }
    }
    Ok(())
}

fn install_files(
    stage: &Path,
    root: &Path,
    plan: &Plan,
    verify: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let package = package_root(stage, plan.platform);
    for (name, hash) in &plan.files {
        if file_hash(&platform::child_path(&package, name, false)?)? != *hash
            || target_hash(root, name)? != plan.originals[name]
        {
            return Err(
                "The update or installation changed after approval. Nothing was installed.".into(),
            );
        }
    }
    receipt(
        stage,
        "installing",
        "Installing verified app files; recovery copies are ready.",
    )?;
    let result: Result<()> = (|| {
        let mut names: Vec<_> = plan.files.keys().collect();
        names.sort_by_key(|name| *name == plan.platform.executable());
        for name in names {
            if target_hash(root, name)? != plan.originals[name] {
                return Err("An application file changed while updating.".into());
            }
            let target = platform::child_path(root, name, true)?;
            fs::create_dir_all(target.parent().ok_or("Invalid application path.")?)
                .map_err(|_| "Could not create an application directory.")?;
            // Backup copies already exist. Same-volume rename replaces each file
            // atomically, so a crash never leaves the executable missing.
            fs::rename(package.join(name), &target).map_err(
                |_| "An application file is locked. Close other Petri windows before retrying.",
            )?;
        }
        verify()
    })();
    if let Err(error) = result {
        return match rollback(stage, root, plan) {
            Ok(()) => Err(format!(
                "{error} The previous application files were restored."
            )),
            Err(rollback_error) => Err(format!("{error} {rollback_error}")),
        };
    }
    Ok(())
}

pub(super) fn run_helper(recovering: bool) -> Result<()> {
    let executable = std::env::current_exe().map_err(|_| "Could not locate the update helper.")?;
    let stage = executable
        .parent()
        .ok_or("Invalid update helper location.")?;
    let plan: Plan = read_json(stage, "plan.json")?;
    if plan.platform != Platform::current()?
        || executable
            .file_name()
            .is_none_or(|s| s != plan.platform.helper())
    {
        return Err("The update helper is not in its private staging directory.".into());
    }
    let root = verify_plan(stage, &plan)?;
    let name = stage
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid update reference.")?;
    let storage = stage.parent().ok_or("Invalid update storage directory.")?;
    if read_reference(storage, LOCK)? != name {
        return Err("The update helper does not own this update.".into());
    }
    receipt(stage, "waiting", "Waiting for Petri to close.")?;
    write_new(&stage.join("ready"), b"ready")?;
    let result: Result<()> = (|| {
        platform::wait_parent(plan.parent_pid)?;
        verify_plan(stage, &plan)?;
        if recovering {
            rollback(stage, &root, &plan)?;
            platform::smoke(&root.join(plan.platform.executable()), &plan.from_version)?;
        } else {
            install_files(stage, &root, &plan, || {
                platform::verify_app(&root, plan.platform)?;
                platform::smoke(&root.join(plan.platform.executable()), &plan.to_version)
            })?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            receipt(
                stage,
                if recovering { "restored" } else { "installed" },
                "Application files are ready. Wallets and settings were not touched.",
            )?;
            if !recovering {
                let previous = read_reference(storage, LAST).ok();
                atomic_write(&storage.join(LAST), name.as_bytes())?;
                if let Some(previous) = previous.filter(|previous| previous != name) {
                    // Only a verified prior updater-owned directory, on the
                    // same installation, with no live helper. Keep uncertain
                    // recovery data rather than broadening a deletion target.
                    if let Ok(old_stage) = stage_at(storage, &previous) {
                        let old_plan = read_json::<Plan>(&old_stage, "plan.json");
                        let old_status = read_json::<Receipt>(&old_stage, "status.json");
                        if let (Ok(old_plan), Ok(old_status)) = (old_plan, old_status) {
                            if old_status.protocol == 1
                                && matches!(old_status.status.as_str(), "installed" | "restored")
                                && !platform::process_alive(old_status.helper_pid)
                                && verify_plan(&old_stage, &old_plan).ok().as_ref() == Some(&root)
                            {
                                let _ = fs::remove_dir_all(old_stage);
                            }
                        }
                    }
                }
            }
            release_lock(storage, name)?;
            if plan.restart {
                platform::restart(&root, plan.platform)?;
            }
            Ok(())
        }
        Err(error) => {
            receipt(stage, "failed", &error)?;
            // If rollback completed, reopen only the verified previous app.
            // A partially recovered or concurrently changed installation stays closed.
            if plan.restart
                && plan
                    .originals
                    .iter()
                    .all(|(file, old)| target_hash(&root, file).ok().as_ref() == Some(old))
                && platform::verify_app(&root, plan.platform).is_ok()
                && platform::smoke(&root.join(plan.platform.executable()), &plan.from_version)
                    .is_ok()
            {
                let _ = platform::restart(&root, plan.platform);
            }
            Err(error)
        }
    }
}

pub(super) fn launch_recovery(platform: Platform, restart: bool) -> Result<()> {
    let root = platform::install_root(platform)?;
    let storage = storage_root(&root, platform)?;
    platform::owned_directory(&storage)?;
    let pending = fs::symlink_metadata(storage.join(LOCK)).is_ok();
    let name = read_reference(&storage, if pending { LOCK } else { LAST })?;
    let stage = stage_at(&storage, &name)?;
    let status: Receipt = read_json(&stage, "status.json")?;
    if status.protocol != 1
        || (matches!(
            status.status.as_str(),
            "waiting" | "installing" | "downloading"
        ) && platform::process_alive(status.helper_pid))
    {
        return Err(
            "Another update is still running. Wait for it to finish before recovering.".into(),
        );
    }
    let mut plan: Plan = read_json(&stage, "plan.json")?;
    if plan.platform != platform {
        return Err("The recovery copy is for a different platform.".into());
    }
    if verify_plan(&stage, &plan)? != root {
        return Err("This recovery copy belongs to another Petri installation.".into());
    }
    plan.parent_pid = std::process::id();
    plan.restart = restart;
    if !pending {
        write_new(&storage.join(LOCK), name.as_bytes())?;
    }
    save_plan(&stage, &plan)?;
    launch(&stage, &plan, true)
}
