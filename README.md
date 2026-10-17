# Petri futures CLI source

This repository contains Petri's capped-futures CLI/TUI source and the
`ameba-sdk` TypeScript and Rust SDK (on npm as `ameba-sdk`), plus the Rust
source dependencies they build against. Start with [Petri's guide](cli/README.md)
or [the SDK](sdk/README.md).

The source is mirrored from Amoeba monorepo commit
`c49d5594b0240a4279a3726267abd1d649a289a2` (release `2026-10-17-cli-sdk`); its
CLI version is `0.1.10` and its SDK version `0.4.0`. Signed v0.1.10 packages for
Linux, macOS and Windows are built from this commit and published on this
repository's Releases page. The v0.1.9 and earlier previews contain older code
for a retired deployment.

Build from `cli/` with Rust 1.93.1: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies; `api/API.md` is the futures API's HTTP contract. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
