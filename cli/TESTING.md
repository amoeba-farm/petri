# Petri CLI Testing

## Baseline Commands

```bash
scripts/devloop.sh
cargo fmt -- --check
cargo check --locked --bin petri
cargo test --locked
node scripts/check-petri-launcher.mjs
node scripts/check-petri-packaging.mjs
node scripts/check-petri-mcp-server.mjs
```

## Native Windows ConPTY TUI Harness

The development-only crate at `tools/petri-conpty-harness` launches the real
Petri binary through native ConPTY. It uses current-screen VT parsing, isolated
loopback data sources, provider-off mode, native keyboard/mouse/resize events,
hard deadlines, and one watchdog process per scenario.

```powershell
cargo test --manifest-path tools/petri-conpty-harness/Cargo.toml --locked -j1 -- --test-threads=1
cargo run --manifest-path tools/petri-conpty-harness/Cargo.toml --locked -- smoke --binary target/release/petri.exe
```

This is a Windows release-validation check, not production runtime code and not
a replacement for backend, wallet, transaction, or on-chain tests. It never
uses live providers, wallets, or production services. See
`tools/petri-conpty-harness/README.md` for the isolation contract and focused
scenario commands.

## TDD Priorities

- command parsing
- backend URL handling
- JSON/plain output modes
- market discovery
- futures market resolution (symbol, name, address, `--expiry`)
- range shapes, leverage, limits and `--usd` budgets match the web ticket
  (`src/futures/shape.rs`)
- the futures API's `/v1/markets`, `/v1/accounts` and `/v1/quote` views parse
  (`src/futures/api.rs`)
- sponsored futures transactions are refused unless they match the request
  exactly (`src/futures/check.rs`)
- money formatting and history rows (`src/futures/{trade,reads}.rs`)
- bounded MCP market projections and honest empty-live-set results
- settlement command safety
- mock backend behavior
- installation/update-only launcher behavior, transparent Rust delegation, and proof that no product/backend implementation or catalog fallback exists
- release/debug launcher separation and Windows/macOS app packaging contracts
- native ConPTY keyboard, mouse, resize, screen, and clean-exit behavior

## Policy

- Behavior changes should start with a failing or updated test.
- Prefer deterministic unit tests for pure rules, contract tests for repo boundaries, and small integration smoke tests for critical flows.
- Do not weaken assertions to make CI pass.
- If a test is not applicable, document the reason in the handoff.

## Futures end-to-end (dev localnet)

`RUNBOOK.md` lists the faucet → quote → long/short → limit order → positions →
cancel → close → withdraw walk-through. Run it only against a local validator
(`127.0.0.1:8899`) and the futures API (`127.0.0.1:4500`) with a throwaway
keypair under `.localnet/` (git-ignored). Run it once with `--route local` and
once with `--route sponsored`. The TUI can be rendered headless for review:
`petri tui --render 120x40 --view markets`.

## Skipped Or Heavy Checks

- Network, DevNet, Solana validator, browser visual, and live backend checks must be called out explicitly when skipped.
- Secret-dependent checks must never run in baseline CI.
