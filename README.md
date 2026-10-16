# Petri futures CLI source

This repository contains Petri's current capped-futures CLI/TUI source, plus the
Rust source dependencies it builds against. Start with [Petri's guide](cli/README.md).

The source is mirrored from Amoeba monorepo commit
`37af722bb47ffa44a5f14dea9cba6bf25212cb49`; its CLI version is `0.1.10`.
The source update does not publish a new binary. Existing downloadable v0.1.9
previews contain older code. Platform packages and signing are a separate release.

Build from `cli/` with Rust 1.93.1: `cargo build --release --locked`.
The `sdk/`, `futures/`, `oracle/`, and `governance/` directories preserve upstream
Cargo path dependencies. No private deployment files, keys or environment files
are included. See [source provenance](PUBLIC_SOURCE_PROVENANCE.json).

The root license, notices, security policy and trademarks are retained from this
public repository. Prior CLI versions remain in Git history and existing releases.
