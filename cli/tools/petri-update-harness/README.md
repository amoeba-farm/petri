# Local Petri updater checks

This developer-only harness compiles the exact `src/release_update/` modules with
the same locked dependency versions as Petri. It has no market, SDK, wallet or
production-service code, and is excluded from public source exports and packages.

```powershell
cargo test --manifest-path tools/petri-update-harness/Cargo.toml --locked --offline -- --test-threads=1
cargo build --manifest-path tools/petri-update-harness/Cargo.toml --locked --offline
rustc --edition=2024 tools/petri-update-harness/fixtures/next-version.rs -o tools/petri-update-harness/target/next-version.exe
node tools/petri-update-harness/check-windows-handoff.mjs
```

The ten bounded cases cover release identity, version ordering, redirect policy,
response limits, archive ownership/digests/channel, traversal/truncation, atomic
app-file replacement, startup-failure rollback and concurrent file preservation.
The Windows process check uses disposable executables and a private fixture under
this harness's ignored `target/`: it verifies that the old app remains unchanged
while running, then that the helper installs the new executable after exit.
No installed Petri app, user wallet or actual configuration is modified.

On a Mac, compile `fixtures/next-version.rs` to `target/next-version`, then run
`node tools/petri-update-harness/check-macos-handoff.mjs` from the repository root.
The private **Standalone updater qualification** workflow runs these bounded
checks on Windows, Mac Apple silicon and Mac Intel. It does not publish artifacts
or deploy. Interactive Terminal reopening remains a manual user-flow check.
To refresh the harness dependency resolution after deliberate
root dependency changes, seed its lockfile from the root Cargo.lock and run
`cargo check --manifest-path tools/petri-update-harness/Cargo.toml --offline`.
