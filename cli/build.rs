use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PETRI_UPDATE_CHANNEL");
    let update_channel = std::env::var("PETRI_UPDATE_CHANNEL").unwrap_or_else(|_| "source".into());
    assert!(
        matches!(update_channel.as_str(), "source" | "preview"),
        "unsupported Petri update channel"
    );
    println!("cargo:rustc-env=PETRI_UPDATE_CHANNEL={update_channel}");
    // The GitHub repository the updater trusts for releases (the release
    // workflow sets it to its release repository; README links name it too).
    println!("cargo:rerun-if-env-changed=PETRI_RELEASE_REPOSITORY");
    let release_repository = std::env::var("PETRI_RELEASE_REPOSITORY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_RELEASE_REPOSITORY.into());
    assert!(
        valid_repository(&release_repository),
        "PETRI_RELEASE_REPOSITORY must be a GitHub owner/name"
    );
    println!("cargo:rustc-env=PETRI_RELEASE_REPOSITORY={release_repository}");
    for git_path in ["HEAD", "index", "packed-refs"] {
        emit_git_rerun_path(git_path);
    }
    if let Some(symbolic_head) = git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        emit_git_rerun_path(&symbolic_head);
    }
    println!("cargo:rerun-if-changed=assets/Petri.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/Petri.ico");
        resource
            .compile()
            .expect("failed to embed the Petri Windows icon");
    }

    let commit = git_stdout(&["rev-parse", "HEAD"])
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=PETRI_BUILD_COMMIT={commit}");
}

const DEFAULT_RELEASE_REPOSITORY: &str = "amoeba-farm/petri";

/// `owner/name` with GitHub's characters only.
fn valid_repository(value: &str) -> bool {
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    let owner_ok = !owner.is_empty()
        && owner.len() <= 39
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let name_ok = !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    owner_ok && name_ok
}

fn emit_git_rerun_path(path: &str) {
    if let Some(path) = git_stdout(&["rev-parse", "--path-format=absolute", "--git-path", path]) {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git_stdout(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
