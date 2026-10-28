# Petri futures CLI source

This repository contains Petri's capped-futures CLI/TUI source and the
`ameba-sdk` TypeScript and Rust SDK (on npm as `ameba-sdk`), plus the Rust
source dependencies they build against. Start with [Petri's guide](cli/README.md)
or [the SDK](sdk/README.md).

The source is mirrored from Amoeba monorepo commit
`efc87f6a279caec6b4bfef8084fd1745d7a808ea`.
Petri `0.1.21` uses the released 0.1.19 client implementation with the shared
market-account decoder updated for operator-approved credit. Concurrent client
API-cache changes are excluded. Existing market accounts remain supported.
The Preview binaries workflow builds unsigned Windows and macOS packages.
The SDK `0.4.1` package is prepared; npm publication is not claimed.
The selected backend policy is not included in this repository.

Build from `cli/` with Rust 1.95.0: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies; `api/API.md` is the futures API's HTTP contract. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
