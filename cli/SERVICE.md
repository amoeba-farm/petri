# Petri CLI Service

## Role

Rust petri CLI/TUI for capped futures (markets, book, trades, charts, quotes, long/short, positions, close, cancel, cash) and the index oracle (months, sources, candidates, bonds and evidence).

## Runtime

| Field | Value |
| --- | --- |
| Path | `ameba_cli` |
| Runtime | Rust CLI/TUI |
| Port | n/a |
| Health | `petri --help`, `petri markets`, or launcher smoke |
| Readiness | Rust binary or Windows launcher command smoke |
| Metrics | structured command output/log metrics target |

## Ownership Boundaries

- src/cli.rs owns command grammar
- src/main.rs owns dispatch/output
- src/research/ owns TUI state/layout/keyboard behavior (src/research/markets.rs: the TUI's Markets view)
- src/futures/ owns futures reads, quotes, orders, sending and the sponsored-transaction check
- src/futures/ owns the shared context, settings, RPC and API clients, send path, wallet creation, status and faucet
- src/chart.rs owns terminal chart rendering
- src/endpoints.rs owns URL construction

## Cross-Repo Contracts

- Read root `CONTRACT_REGISTRY.md` before changing shared API, catalog, Redis, Postgres, CLI, oracle, or settlement behavior.
- Runtime source of truth stays in this repo and its sibling runtime repos; `amoeba-context-library` is documentation/context only.
- Behavior changes should update this file, `TESTING.md`, `METRICS.md`, and public/context docs when contracts move.

## Baseline Checks

```bash
node scripts/check-petri-launcher.mjs
scripts/devloop.sh
cargo fmt -- --check
cargo check --locked --bin petri
cargo test --locked
```
