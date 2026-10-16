# Petri CLI Runbook

## Read First

1. Root `../AGENTS.md`
2. Target Petri topic for the task
3. This repo's `AGENTS.md`
4. This repo's `README.md`
5. `SERVICE.md`, `TESTING.md`, and `METRICS.md`

## Local Run

```bash
cargo run -- --help
scripts/devloop.sh
```

## Futures on the dev localnet

Futures commands talk to a Solana RPC and the futures API. The dev profile
(the default of the dev build, `--features dev-chain`; `--profile dev`
otherwise) uses `http://127.0.0.1:8899` and `http://127.0.0.1:4500`; a release
build defaults to production. Use a dedicated config and a throwaway keypair so
nothing touches your normal wallet:

```bash
export PETRI_CONFIG_PATH=$PWD/.localnet/config.json   # .localnet/ is git-ignored
petri config set profile dev  # not needed with the dev build
petri wallet new              # .localnet/futures-id.json; prints only the address
petri status                  # cluster, clock lag, programs, API, wallet
petri faucet --sol 1          # dev USDC from the API, SOL from the validator (a dev-faucet build)
petri market ramx             # price, index, suggested range
petri quote long ramx 100
petri long ramx 100 --yes
petri short ramx 50 -x 5 --yes
petri long ramx 100 --limit 90 --post --yes   # rests on the book
petri positions
petri cancel ramx <#> --yes
petri close ramx <#> -n 10 --keep-cash --yes
petri withdraw ramx all --yes
petri history
```

Pass `--deployment /path/to/.localnet/deployment.json` to pin program ids to a
manifest instead of the API's `/v1/config`. With at least 0.01 SOL the wallet
pays its own fees (`--route local`); otherwise it sends through the API sponsor
(`--route sponsored`). Force one with `--route` to test both.

Triage:

- `petri status` shows whether the API serves the same programs as the trusted
  deployment, and how far the cluster clock lags wall time. Expiries and listing
  times follow the cluster clock.
- Program errors are named: futures errors as `Name (futures error 0x…)`, token
  errors as `Token…`, each with a hint for the common ones. Program logs are
  printed to stderr.
- An order or close that fails on `MinFill` found nothing inside its limit: the
  book moved; quote again or raise `--slippage`. Petri already re-solves once
  when the book changed under a sent order (`BadProposal`, `NotBestPrice`,
  `FifoViolation`, …).
- `WouldCross`: a `--post` order would fill now. `ClaimReserved`: a resting
  close order holds those lots; cancel it first.

## Local Validation

```bash
scripts/devloop.sh
cargo fmt -- --check
cargo check --locked --bin petri
cargo test --locked
```

## Incident / Debug Triage

1. Check health/readiness target if this repo runs a service.
2. Check the narrow logs or generated report for the current run only.
3. Reproduce with the smallest deterministic command.
4. Add or update a regression test before fixing behavior.
5. Update docs and metrics notes if the failure mode becomes part of the contract.

## Safety

- Do not inspect or print secrets.
- Do not commit generated caches, reports, dependency folders, logs, local browser profiles, wallets, keypairs, or `.env` files.
- Keep patches small and repo-scoped unless the task explicitly requires cross-repo contract work.
