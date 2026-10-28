# Petri futures CLI source

This repository contains Petri's capped-futures CLI/TUI source and the
`ameba-sdk` TypeScript and Rust SDK (on npm as `ameba-sdk`), plus the Rust
source dependencies they build against. Start with [Petri's guide](cli/README.md)
or [the SDK](sdk/README.md).

The source is mirrored from Amoeba monorepo commit
`28b77480d5ec6c3cdc714cd94878fa6eb3b020c0` (release `2026-10-28-oracle-stages`);
its CLI version is `0.1.19` and its SDK version `0.4.0`. Unsigned v0.1.19 preview
packages for Windows and macOS are built from this commit by the Preview binaries
workflow and published on this repository's Releases page. The v0.1.9 and earlier previews contain older code
for a retired deployment.

Build from `cli/` with Rust 1.95.0: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies; `api/API.md` is the futures API's HTTP contract. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
