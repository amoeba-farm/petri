//! `petri wallet new` as the first command on a machine, before Petri's
//! config folder exists (the README's quickstart starts with it).
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn fresh_home() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("petri-wallet-new-{}-{nonce}", std::process::id()))
}

#[test]
fn wallet_new_works_before_the_config_folder_exists() {
    let home = fresh_home();
    fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_petri"))
        .args(["wallet", "new"])
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("PETRI_CONFIG_PATH")
        .env_remove("SOLANA_KEYPAIR")
        .output()
        .expect("run petri");
    assert!(
        output.status.success(),
        "petri wallet new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let petri = home.join(".config").join("amoeba").join("petri");
    let mode = |path: PathBuf| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(petri.clone()) & 0o077,
        0,
        "the config folder is private"
    );
    assert_eq!(mode(petri.join("futures-id.json")), 0o600);
    let config = fs::read_to_string(petri.join("config.json")).unwrap();
    assert!(
        config.contains("futures-id.json"),
        "saved as the wallet: {config}"
    );
    fs::remove_dir_all(&home).unwrap();
}
