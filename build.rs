use sha2::{Digest, Sha256};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=PETRI_UPDATE_CHANNEL");
    let update_channel = std::env::var("PETRI_UPDATE_CHANNEL").unwrap_or_else(|_| "source".into());
    assert!(
        matches!(update_channel.as_str(), "source" | "preview"),
        "unsupported Petri update channel"
    );
    println!("cargo:rustc-env=PETRI_UPDATE_CHANNEL={update_channel}");
    let runtime = std::path::Path::new("target/petri-sdk-runtime.bin");
    println!("cargo:rerun-if-changed=scripts/sdk-worker.mjs");
    println!("cargo:rerun-if-changed=scripts/sdk-oracle.mjs");
    println!("cargo:rerun-if-changed=scripts/sdk-carry.mjs");
    println!("cargo:rerun-if-changed=target/petri-sdk-runtime.bin");
    println!("cargo:rerun-if-env-changed=PETRI_REQUIRE_SDK_RUNTIME");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let mut digests = String::from("const EXPECTED_SDK_SCRIPTS: &[(&str, &str)] = &[\n");
    for (name, source) in [
        ("worker.mjs", "scripts/sdk-worker.mjs"),
        ("sdk-oracle.mjs", "scripts/sdk-oracle.mjs"),
        ("sdk-carry.mjs", "scripts/sdk-carry.mjs"),
    ] {
        let bytes = std::fs::read(source).expect("read checked-in SDK script");
        let digest = format!("{:x}", Sha256::digest(bytes));
        digests.push_str(&format!("    ({name:?}, {digest:?}),\n"));
    }
    digests.push_str("];\n");
    std::fs::write(output.join("sdk-script-digests.rs"), digests)
        .expect("write independent source digests");
    let embedded = output.join("petri-sdk-runtime.bin");
    if runtime.is_file() {
        std::fs::copy(runtime, embedded).expect("copy qualified SDK runtime into build");
    } else {
        assert!(
            std::env::var("PETRI_REQUIRE_SDK_RUNTIME").ok().as_deref() != Some("1"),
            "Release packaging requires the pinned SDK runtime"
        );
        // Existing native trade/writer flows remain independent of this worker.
        // Platform release packaging requires building the SDK runtime first.
        std::fs::write(embedded, []).expect("write absent SDK runtime marker");
    }
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
