use super::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let name: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("petri-updater-check-{name}"));
        platform::create_private_directory(&root).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("petri-updater-check-")
        );
        let _ = fs::remove_dir_all(&self.0); // Only this test's fresh, random fixture.
    }
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn release_json(platform: Platform) -> Value {
    let base = format!(
        "https://github.com/{REPOSITORY}/releases/download/v0.1.4/{}.zip",
        platform.stem()
    );
    json!({"id": 42, "tag_name":"v0.1.4", "draft":false, "prerelease":false,
    "html_url":format!("https://github.com/{REPOSITORY}/releases/tag/v0.1.4"),
    "assets":[
        {"id":1,"name":format!("{}.zip",platform.stem()),"size":100,"state":"uploaded","browser_download_url":base,"digest":format!("sha256:{}","a".repeat(64))},
        {"id":2,"name":format!("{}.zip.sha256",platform.stem()),"size":88,"state":"uploaded","browser_download_url":format!("{base}.sha256"),"digest":format!("sha256:{}","b".repeat(64))}
    ]})
}
fn release() -> Release {
    parse_release(
        &serde_json::to_vec(&release_json(Platform::WindowsX64)).unwrap(),
        Platform::WindowsX64,
    )
    .unwrap()
}
fn package_files() -> BTreeMap<String, Vec<u8>> {
    let mut files: BTreeMap<String, Vec<u8>> = [
        "petri.exe",
        "Petri.cmd",
        "Petri.ico",
        "LICENSE",
        "THIRD_PARTY_NOTICES.md",
        "THIRD_PARTY_LICENSES.md",
        "README.txt",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), b"synthetic fixture only".to_vec()))
    .collect();
    files.insert(
        "petri-update.json".into(),
        serde_json::to_vec(
            &json!({"protocol":1,"channel":"preview","version":"0.1.4","platform":"windows-x64"}),
        )
        .unwrap(),
    );
    files
}
fn make_archive(root: &Path, files: &BTreeMap<String, Vec<u8>>, bad_manifest: bool) -> PathBuf {
    let path = root.join("download.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut sums = String::new();
    for (name, bytes) in files {
        zip.start_file(format!("Petri-windows-x64/{name}"), options)
            .unwrap();
        zip.write_all(bytes).unwrap();
        sums.push_str(&format!(
            "{}  {name}\n",
            if bad_manifest {
                "0".repeat(64)
            } else {
                sha(bytes)
            }
        ));
    }
    zip.start_file("Petri-windows-x64/SHA256SUMS", options)
        .unwrap();
    zip.write_all(sums.as_bytes()).unwrap();
    zip.finish().unwrap();
    path
}

#[test]
fn release_versions_are_canonical_and_ordered_without_downgrades() {
    assert!(version("0.1.10").unwrap() > version("0.1.9").unwrap());
    for invalid in [
        "01.2.3",
        "v1.2.3",
        "1.2",
        "1.2.3-beta",
        "1.2.3/../../x",
        "1.2.-3",
        "1.2.3000000000",
    ] {
        assert!(version(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn metadata_binds_exact_repo_platform_assets_and_digests() {
    for (platform, metadata_name) in [
        (Platform::WindowsX64, "windows-x64"),
        (Platform::MacosArm64, "macos-arm64"),
        (Platform::MacosX86_64, "macos-x86_64"),
    ] {
        assert_eq!(
            serde_json::to_value(platform).unwrap(),
            json!(metadata_name)
        );
        assert_eq!(
            serde_json::from_value::<Platform>(json!(metadata_name)).unwrap(),
            platform
        );
        assert_eq!(
            parse_release(
                &serde_json::to_vec(&release_json(platform)).unwrap(),
                platform
            )
            .unwrap()
            .platform,
            platform
        );
    }
    for change in 0..7 {
        let mut value = release_json(Platform::WindowsX64);
        match change {
            0 => value["draft"] = true.into(),
            1 => value["assets"][0]["digest"] = Value::Null,
            2 => {
                value["assets"][0]["browser_download_url"] = "https://evil.invalid/petri.zip".into()
            }
            3 => value["html_url"] = "https://github.com/other/petri/releases/tag/v0.1.4".into(),
            4 => value["assets"][0]["size"] = (MAX_ARCHIVE + 1).into(),
            5 => {
                let duplicate = value["assets"][0].clone();
                value["assets"].as_array_mut().unwrap().push(duplicate);
            }
            _ => value["assets"][0]["state"] = "new".into(),
        }
        assert!(
            parse_release(&serde_json::to_vec(&value).unwrap(), Platform::WindowsX64).is_err(),
            "case {change}"
        );
    }
}

#[test]
fn redirects_remain_https_and_cannot_send_requests_to_other_hosts() {
    for allowed in [
        "https://github.com/amoeba-farm/petri/releases/download/v0.1.4/a",
        "https://release-assets.githubusercontent.com/path?signature=value",
    ] {
        assert!(safe_redirect(&Url::parse(allowed).unwrap()));
    }
    for rejected in [
        "http://github.com/a",
        "https://github.com.evil.invalid/a",
        "https://user:pass@github.com/a",
        "https://localhost/a",
        "https://release-assets.githubusercontent.com:8443/a",
        "https://github.com/a#fragment",
    ] {
        assert!(!safe_redirect(&Url::parse(rejected).unwrap()), "{rejected}");
    }
}

#[test]
fn response_sizes_are_bounded_even_without_content_length() {
    assert_eq!(read_bounded(&b"abcd"[..], 4).unwrap(), b"abcd");
    assert!(read_bounded(&b"abcde"[..], 4).is_err());
}

#[test]
fn package_extracts_only_owned_files_and_checks_every_digest() {
    let fixture = Fixture::new();
    let files = package_files();
    let path = make_archive(&fixture.0, &files, false);
    let hashes = archive::extract(&path, &fixture.0.join("package"), &release()).unwrap();
    assert_eq!(hashes.len(), files.len());
    for (name, bytes) in files {
        assert_eq!(hashes[&name], sha(&bytes));
    }
}

#[test]
fn bad_checksums_and_wrong_channel_metadata_fail_before_installation() {
    for mode in 0..3 {
        let fixture = Fixture::new();
        let mut files = package_files();
        if mode == 1 {
            files.insert(
                "petri-update.json".into(),
                br#"{"protocol":1,"channel":"source","version":"0.1.4","platform":"windows-x64"}"#
                    .to_vec(),
            );
        }
        if mode == 2 {
            files.remove("Petri.cmd");
        }
        let path = make_archive(&fixture.0, &files, mode == 0);
        assert!(archive::extract(&path, &fixture.0.join("package"), &release()).is_err());
    }
}

#[test]
fn archive_rejects_traversal_unowned_files_and_truncated_directories() {
    for name in [
        "../escaped",
        "C:/escaped",
        "petri.exe:stream",
        "wallet.json",
        "Contents/../../escaped",
        "petri.exe ",
    ] {
        let fixture = Fixture::new();
        let mut files = package_files();
        files.insert(name.into(), b"not allowed".to_vec());
        let path = make_archive(&fixture.0, &files, false);
        assert!(
            archive::extract(&path, &fixture.0.join("package"), &release()).is_err(),
            "{name}"
        );
        assert!(!fixture.0.join("escaped").exists());
    }
    let fixture = Fixture::new();
    let path = make_archive(&fixture.0, &package_files(), false);
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(32)
        .unwrap();
    assert!(archive::extract(&path, &fixture.0.join("package"), &release()).is_err());
}

fn transaction_fixture(
    fixture: &Fixture,
) -> (
    PathBuf,
    PathBuf,
    BTreeMap<String, String>,
    BTreeMap<String, Option<String>>,
) {
    let root = fixture.0.join("app");
    fs::create_dir(&root).unwrap();
    let stage = root.join(".petri-update-0123456789abcdef0123456789abcdef");
    fs::create_dir_all(stage.join("package")).unwrap();
    fs::create_dir(stage.join("backup")).unwrap();
    let mut files = BTreeMap::new();
    let mut originals = BTreeMap::new();
    for name in ["petri.exe", "README.txt", "Petri.cmd"] {
        fs::write(stage.join("package").join(name), format!("new-{name}")).unwrap();
        files.insert(name.to_owned(), sha(format!("new-{name}").as_bytes()));
        if name != "Petri.cmd" {
            fs::write(root.join(name), format!("old-{name}")).unwrap();
            fs::write(stage.join("backup").join(name), format!("old-{name}")).unwrap();
            originals.insert(name.to_owned(), Some(sha(format!("old-{name}").as_bytes())));
        } else {
            originals.insert(name.to_owned(), None);
        }
    }
    fs::write(
        root.join("user-config.json"),
        b"user-owned configuration sentinel",
    )
    .unwrap();
    (root, stage, files, originals)
}

#[test]
fn installation_replaces_app_files_without_touching_user_configuration() {
    let fixture = Fixture::new();
    let (root, stage, files, originals) = transaction_fixture(&fixture);
    install::test_transaction(&root, &stage, files, originals, false).unwrap();
    assert_eq!(fs::read(root.join("petri.exe")).unwrap(), b"new-petri.exe");
    assert_eq!(
        fs::read(root.join("user-config.json")).unwrap(),
        b"user-owned configuration sentinel"
    );
    assert_eq!(
        fs::read(stage.join("backup/petri.exe")).unwrap(),
        b"old-petri.exe"
    );
}

#[test]
fn failed_startup_restores_old_files_and_removes_only_new_owned_files() {
    let fixture = Fixture::new();
    let (root, stage, files, originals) = transaction_fixture(&fixture);
    assert!(
        install::test_transaction(&root, &stage, files, originals, true)
            .unwrap_err()
            .contains("restored")
    );
    assert_eq!(fs::read(root.join("petri.exe")).unwrap(), b"old-petri.exe");
    assert_eq!(
        fs::read(root.join("README.txt")).unwrap(),
        b"old-README.txt"
    );
    assert!(!root.join("Petri.cmd").exists());
    assert_eq!(
        fs::read(root.join("user-config.json")).unwrap(),
        b"user-owned configuration sentinel"
    );
}

#[test]
fn concurrent_changes_and_nonordinary_target_paths_are_not_overwritten() {
    let fixture = Fixture::new();
    let (root, stage, files, originals) = transaction_fixture(&fixture);
    fs::write(root.join("README.txt"), b"user edit after approval").unwrap();
    assert!(install::test_transaction(&root, &stage, files, originals, false).is_err());
    assert_eq!(
        fs::read(root.join("README.txt")).unwrap(),
        b"user edit after approval"
    );
    assert_eq!(fs::read(root.join("petri.exe")).unwrap(), b"old-petri.exe");
    assert!(platform::plain(&root, false).is_err());
    assert!(platform::child_path(&root, "../outside", true).is_err());
    let mac_app = fixture.0.join("Petri Preview.app");
    fs::create_dir(&mac_app).unwrap();
    let mac_storage = install::test_mac_storage(&mac_app).unwrap();
    assert!(
        !mac_storage.starts_with(&mac_app),
        "Staging must not alter the signed app bundle."
    );
    assert_eq!(mac_storage.parent(), mac_app.parent());
    #[cfg(unix)]
    {
        let linked = root.join("linked");
        std::os::unix::fs::symlink(root.join("README.txt"), &linked).unwrap();
        assert!(platform::plain(&linked, false).is_err());
    }
}

fn retired_files(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".petri-retired-")
        })
        .collect()
}

#[test]
fn a_file_moved_aside_keeps_its_contents_until_tidied() {
    let fixture = Fixture::new();
    let root = &fixture.0;
    fs::write(root.join("petri.exe"), b"old-petri.exe").unwrap();
    fs::write(root.join("new"), b"new-petri.exe").unwrap();
    fs::write(root.join(".petri-retired-not-ours"), b"kept").unwrap();
    fs::write(root.join("user-config.json"), b"kept").unwrap();
    install::test_move_aside(&root.join("new"), &root.join("petri.exe")).unwrap();
    assert_eq!(fs::read(root.join("petri.exe")).unwrap(), b"new-petri.exe");
    let retired: Vec<_> = retired_files(root)
        .into_iter()
        .filter(|path| !path.ends_with(".petri-retired-not-ours"))
        .collect();
    assert_eq!(retired.len(), 1);
    assert_eq!(fs::read(&retired[0]).unwrap(), b"old-petri.exe");
    install::test_tidy_retired(root);
    assert!(!retired[0].exists());
    assert!(root.join(".petri-retired-not-ours").exists());
    assert!(root.join("user-config.json").exists());
    // A failed swap puts the installed file back.
    assert!(install::test_move_aside(&root.join("missing"), &root.join("petri.exe")).is_err());
    assert_eq!(fs::read(root.join("petri.exe")).unwrap(), b"new-petri.exe");
}

/// The case `replace` exists for: an update while another petri runs (a
/// second window, an agent's MCP server). Windows refuses to replace or
/// delete a running executable but lets it be renamed: the swap goes
/// through, and the running process keeps going.
#[cfg(windows)]
#[test]
fn an_executable_that_is_running_is_replaced() {
    let fixture = Fixture::new();
    let root = &fixture.0;
    let system = std::env::var_os("SystemRoot").unwrap();
    let ping = Path::new(&system).join("System32").join("PING.EXE");
    let target = root.join("petri.exe");
    fs::copy(&ping, &target).unwrap();
    let mut running = std::process::Command::new(&target)
        .args(["-n", "120", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    // The move aside itself, on the running file.
    fs::write(root.join("new"), b"new-petri.exe").unwrap();
    install::test_move_aside(&root.join("new"), &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new-petri.exe");
    assert!(
        running.try_wait().unwrap().is_none(),
        "the running petri was stopped"
    );
    let retired = retired_files(root);
    assert_eq!(retired.len(), 1);
    install::test_tidy_retired(root);
    assert!(
        retired[0].exists(),
        "a running executable cannot be deleted"
    );
    // And `replace`, as an update calls it, over a petri running from it.
    fs::copy(&ping, root.join("second.exe")).unwrap();
    fs::rename(root.join("second.exe"), &target).unwrap();
    let mut second = std::process::Command::new(&target)
        .args(["-n", "120", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    fs::write(root.join("newer"), b"newer-petri.exe").unwrap();
    install::test_replace(&root.join("newer"), &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"newer-petri.exe");
    assert!(
        second.try_wait().unwrap().is_none(),
        "the running petri was stopped"
    );
    for child in [&mut running, &mut second] {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    // Windows lets go of an exited program's file a moment later.
    for _ in 0..50 {
        install::test_tidy_retired(root);
        if retired_files(root).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(retired_files(root).is_empty());
}
