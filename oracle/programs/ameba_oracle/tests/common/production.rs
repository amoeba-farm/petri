//! The production (mainnet) artifact and the builds beside it: `tests/production_artifact.rs` and
//! `tests/production_failed_month.rs` load the formation build (`SBF_OUT_DIR`) as an upgradeable
//! program and upgrade it to the `v1-months` build (the production profile with `v1-months`) and,
//! to compare its tables, the production artifact. `tests/production_fresh_boot.rs` deploys the
//! production artifact alone, as mainnet takes it (`Harness`, `start_deployed`).
use super::*;

pub fn sbf_elf(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The formation build (`SBF_OUT_DIR`, as every SBF test loads it).
pub fn formation_elf() -> Vec<u8> {
    let dir = std::env::var_os("SBF_OUT_DIR")
        .or_else(|| std::env::var_os("BPF_OUT_DIR"))
        .expect("SBF_OUT_DIR names the test-sbf build");
    sbf_elf(&std::path::Path::new(&dir).join("ameba_oracle.so"))
}

/// An artifact named by `variable`, which must exist, or else `oracle/target/<dir>/ameba_oracle.so`
/// when it was built (`None`: these tests skip without it).
fn artifact(variable: &str, dir: &str) -> Option<Vec<u8>> {
    if let Some(path) = std::env::var_os(variable) {
        return Some(sbf_elf(std::path::Path::new(&path)));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(dir)
        .join("ameba_oracle.so");
    if !path.exists() {
        eprintln!(
            "skipped: no artifact at {} (build it, or set {variable})",
            path.display()
        );
        return None;
    }
    Some(sbf_elf(&path))
}

/// The production artifact: `AMEBA_ORACLE_PRODUCTION_SO` or `oracle/target/deploy`.
pub fn production_elf() -> Option<Vec<u8>> {
    artifact("AMEBA_ORACLE_PRODUCTION_SO", "deploy")
}

/// The v1 months without their formation: the production profile with `v1-months`
/// (`AMEBA_ORACLE_V1_MONTHS_SO` or `oracle/target/deploy-v1-months`).
pub fn v1_months_elf() -> Option<Vec<u8>> {
    artifact("AMEBA_ORACLE_V1_MONTHS_SO", "deploy-v1-months")
}

/// The formation build, upgradeable to the `v1-months` build (`v1_months_elf`) and then to the
/// production artifact (`None`: skip).
pub async fn start_formation_v1_months() -> Option<(Harness, Vec<u8>, Vec<u8>)> {
    let production = production_elf()?;
    let v1_months = v1_months_elf()?;
    let formation = formation_elf();
    assert_ne!(formation, v1_months, "the two builds are the same file");
    assert_ne!(v1_months, production, "the two builds are the same file");
    let capacity = formation.len().max(v1_months.len()).max(production.len());
    Some((
        start_upgradeable(&formation, capacity).await,
        v1_months,
        production,
    ))
}
