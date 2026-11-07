# Petri futures CLI source

This repository contains Petri's capped-futures CLI/TUI source and the
`ameba-sdk` TypeScript and Rust SDK (on npm as `ameba-sdk`), plus the Rust
source dependencies they build against. Start with [Petri's guide](cli/README.md)
or [the SDK](sdk/README.md).

The source is mirrored from Amoeba monorepo commit
`bc49b6a2d6324b0a030cd5294441eb1d2a4fabd5` (release `2026-11-07-oracle-reboot`);
its CLI version is `0.1.24` and its SDK source version is `0.4.1`.
Petri 0.1.24 recognizes both Mainnet Oracle addresses through the Oracle cutover.
Unsigned v0.1.24 preview packages for Windows and macOS are built by the Preview binaries workflow.
The selected backend credit policy is not included in this repository.

Build from `cli/` with Rust 1.95.0: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies; `api/API.md` is the futures API's HTTP contract. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
