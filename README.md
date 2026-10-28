# Petri futures CLI source

This repository contains Petri's capped-futures CLI/TUI source and the
`ameba-sdk` TypeScript and Rust SDK (on npm as `ameba-sdk`), plus the Rust
source dependencies they build against. Start with [Petri's guide](cli/README.md)
or [the SDK](sdk/README.md).

The source is mirrored from Amoeba monorepo commit
`b24a4ed2e058d2feb6122d38c445f4abe379d8d2`.
Petri `0.1.20` accepts the optional operator-assessment record used by the
upcoming Futures upgrade and retains support for existing market accounts.
The Preview binaries workflow builds unsigned Windows and macOS packages.
The SDK `0.4.1` package is prepared; this source publication does not claim npm publication.
The contract's selected backend policy is not included in this repository.

Build from `cli/` with Rust 1.95.0: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies; `api/API.md` is the futures API's HTTP contract. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
