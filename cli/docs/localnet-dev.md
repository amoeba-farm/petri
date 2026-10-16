# Petri on the dev localnet

The dev localnet (genesis `A5ZLaudxEFvFScV6RzNtNN7fzshRxQgXhC6JB6dishHX`) runs
the futures program Petri trades and the standalone oracle program in
its `localnet-dev-authority` profile. Petri reaches it through its `dev`
profile: the default of the dev build (`--features dev-chain`, which
`scripts/install-dev.sh` builds as the dev VM's `/opt/ameba-dev/bin/petri`),
and in any other build `--profile dev`, `PETRI_PROFILE=dev` or `petri config
set profile dev`. A release build defaults to `production` (mainnet-beta,
`https://api.amoeba.farm`). The dev profile takes every program id from the
deployment and accepts the dev chain's own governance controller and the
oracle's `localnet-dev` release on this genesis.

| Service | Dev profile default | Setting |
| --- | --- | --- |
| Solana RPC | `http://127.0.0.1:8899` | `--rpc`, `PETRI_RPC_URL`, `petri config set rpc` |
| Futures API | `http://127.0.0.1:4500` | `--api`, `PETRI_API_URL`, `petri config set api` |
| Deployment manifest | the API's `/v1/config` | `--deployment`, `PETRI_DEPLOYMENT`, `petri config set deployment` |

The futures repo writes the manifest to `../futures/.localnet/deployment.json`.
It names the futures and oracle programs (its `futures` section and
`programs.oracle`), the USDC mint and, in its `oracle` section, the oracle
program, its release commit and the dev periods and months. Pinning it with
`--deployment` makes Petri trust those ids instead of the API's.

## A throwaway wallet

```bash
export PETRI_CONFIG_PATH=$PWD/.localnet/config.json   # .localnet/ is git-ignored
petri config set profile dev  # not needed with the dev build
petri wallet new            # .localnet/futures-id.json; prints only the address
petri config set deployment ../futures/.localnet/deployment.json
petri faucet --sol 1        # dev USDC from the API, SOL from the validator (a dev-faucet build)
petri status                # cluster, clock, programs, identity, API, wallet
```

`petri wallet new` writes the key next to the config file and saves it as the
`keypair` setting. `petri wallet new --out PATH` writes it elsewhere and leaves
the setting alone.

`petri status` reports whether the deployment identity is verified (genesis,
deployed programs, the oracle config and its bond mint) and how far the
cluster clock lags wall time; expiries and listing times follow the cluster
clock.

## Futures

See `RUNBOOK.md` for the trading walk-through. With at least 0.01 SOL the
wallet pays its own fees (`--route local`); otherwise the futures API's sponsor
pays (`--route sponsored`), and Petri checks the prepared transaction before
signing. The dev markets settle after expiry (the Oct 26 series on
2026-11-01); the futures keeper pays positions into each owner's cash in the
market.

## Oracle

```bash
petri oracle                              # months: phase, sources, settlement
petri oracle sources ramx --expiry oct26  # one month's verified sources
petri oracle candidates                   # research candidates (no oracle weight)
petri oracle collateral deposit 10        # USDC bond collateral
petri oracle evidence write --kind locator --text https://example.com/source
petri oracle updates commit ramx --source <ID> --value 101 \
  --archive-url https://web.archive.org/web/<YYYYMMDDhhmmss>/<source URL> --stake 1 --simulate
```

Writes use the oracle crate of this monorepo (`../oracle`, by path), the
source the dev chain runs. The dev
months come from the oracle's dev bootstrap: they are in Game with base 100,
their buckets are the dev sources' own ids, and they have no active-weight
manifest, so the update lane (166 and after) refuses on them with
`InvalidOracleActiveWeightManifest`. Evidence writes, collateral, and
simulations of every write work. Do not run instructions that change a dev
month's phase or settle it; the futures keeper settles dev months at expiry.

## Research workspace

`petri tui` and `petri research …` start the local research service with the
futures API as both the candidate store (`/oracle/agents`,
`/oracle/source-candidates`) and the oracle read endpoint (`/v1/oracle/*`).
Use a separate workspace for a test run:

```bash
export PETRI_RESEARCH_HOME=$PWD/.localnet/research PETRI_RESEARCH_PORT=4199
petri research owner address <PAYOUT_PUBKEY>
petri research agent login --name check --harness test --profile-dir .localnet/agent
petri research owner snapshot             # pending pairing id
petri research owner approve <PAIRING_ID> # registers the agent with the futures API
petri research agent login --name check --harness test --profile-dir .localnet/agent
```

The futures API has no delete route for agents or candidates; candidates leave
the list after 7 days and are deleted after 90.
