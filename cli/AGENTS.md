# ameba_cli Agent Instructions

This repo owns the Rust CLI/TUI entrypoint for Amoeba. The executable command is
`petri`; Amoeba remains the company and product name.

## Working Model

- Keep the market and wallet CLI. The TUI hosts the futures Markets dashboard
  next to oracle research and data submission.
- The CLI trades **futures** (capped futures), the product the site trades
  (`futures/docs/futures.md`): commands read the futures API's `/v1/markets`,
  `/v1/accounts` and `/v1/quote` (`--api`) and the chain (`--rpc`) and build
  transactions with the futures client crate (`ameba_sdk_futures::futures`).
  The TUI's Markets view trades the same markets through the same paths.
  Futures and the oracle are the only product: there is no clearing market or
  matcher.
- Oracle commands read the futures API's oracle documents (`/v1/oracle/*`, the
  candidate store at `/oracle/*`) and build writes with the oracle program's own
  crate (`ameba_oracle`, `../oracle` by path). Instruction families the oracle
  program has not ported have no command.
- Preserve `petri` as the command starter. Do not revive `ameba`/`amoeba` as the
  primary initializer.
- Keep market discovery, charts, quotes, trading, signing, and supported wallet actions in CLI commands. The TUI's Markets view reuses futures functions; it owns no trading logic of its own. Research views own agent connection, research work, evidence, and rewards.
- Do not expose internal route IDs, scheduler knobs, proof-token internals,
  keypair material, or backend implementation details in default user flows.

## Read Order

1. `README.md`
2. `docs/petri-public-cli-user-theory-of-mind.md`
3. `docs/html/index.html`
4. `docs/html/source-map.html`
5. `docs/amoeba-cli-command-guide.md`
6. `src/cli.rs` for the public command surface
7. `src/main.rs` for dispatch and output behavior
8. `src/futures/` for the markets: reads, quotes, orders and sending, with
   the shared clients, identity and units
9. `src/research/mod.rs` for the interactive TUI (`src/research/markets.rs` for the Markets view,
   `src/research/fund.rs` for the Fund view, `src/research/rewards.rs` for the Rewards view's
   earnings)
10. `src/chart.rs` for terminal chart rendering

When a change touches browser parity, inspect the sibling repos before changing
contracts:

- `../web` for the web surface
- `../api` for the futures API (`API.md` is the contract)
- `../futures` for the futures program, the deployment manifest and dev ops
- `../sdk` for the futures SDK (`ameba_sdk_futures`, a path dependency)
- `../oracle` for the standalone oracle program (read-only from CLI tasks; a path
  dependency, so the CLI always builds against the monorepo's oracle)

## Boundaries

- `src/cli.rs` owns CLI shape, help text, flags, value enums, and user-facing
  command grammar.
- `src/main.rs` owns command dispatch and plain/json output wiring.
- `src/research/` owns the oracle TUI, terminal lifecycle, rendering, input, and automatic installed-app updates. `research/` contains the bundled Node 24 service and research CLI. Private workspace data stays outside the application bundle.
- `src/futures/` owns the futures commands: `mod.rs` (the trusted futures
  program, market resolution, the market read and checked from chain, order
  evaluation and plans), `shape.rs` (the range, leverage and limit math of the
  web ticket), `reads.rs` (markets, market, book, trades, chart, positions,
  history, wallet), `trade.rs` (quote, long, short, close, cancel, deposit,
  withdraw; local and sponsored sends) and `check.rs` (the
  sponsored-transaction check), and what they share: `args.rs` (grammar),
  `units.rs` (decimals, USDC atoms), `api.rs` (the futures API client and its
  views: `/v1/config`, `/v1/markets`, `/v1/accounts`, `/v1/quote`, prepare and
  submit) and `rpc.rs` (the chain client),
  `settings.rs` (flag, env, the Petri config, then the profile's defaults:
  `production`, a release build's default, or `dev`, the default of a build
  with the `dev-chain` feature), `market.rs` (trusted deployment, held to the
  production ids in the production profile; market name resolution),
  `governance.rs`
  (gate epochs for governed targets), `txbuild.rs` (named plans, the compute
  budget and the owner's USDC account), `exec.rs` (local and sponsored
  sending), `error.rs` (error naming), `ops.rs` (`wallet new`, `wallet
  address`, `status`) and `funds.rs` (the dev faucet: `petri faucet`, only
  in a build with the `dev-faucet` feature, marked `DEV-ONLY: REMOVE FOR
  PROD`; `scripts/install-dev.sh` builds the dev VM's petri with it, and
  release builds leave it off).
- `src/futures/trade.rs` splits each order into a prepare step (priced,
  reviewed) and a send step (`prepare_open`/`send_open`, `prepare_close`/
  `send_close`, `prepare_cancel`/`send_cancel`); the commands and the TUI's
  Markets view (`src/research/markets.rs`) both call them.
- `src/chart.rs` owns terminal chart rendering.
- `scripts/petri-launcher.mjs` owns only repo-local binary delegation and
  installation/update work. It delegates to the explicit release binary (or
  debug only when `PETRI_DEV=1`) and must not implement market, config, MCP, or
  backend-response behavior. Installed apps invoke the Rust binary directly.
- `src/endpoints.rs` owns the futures API's oracle routes (futures routes live in
  `src/futures/api.rs`).
- `src/backend.rs` owns the CLI error type and terminal-safe output helpers.
- `src/oracle/` owns the oracle product: `args.rs` (grammar), `reads.rs`
  (futures API documents), `chain.rs` (periods, months, sources and ledgers read
  and checked from chain), `skus.rs` (governed SKU sets, part numbers and
  chip buckets, and Merkle proofs), `series.rs` (comparison series of the chip
  months and the commodity spec's routing), `observe.rs` (observations of a
  series and their API records), `send.rs` (review, simulate, sign and
  send), `writes.rs` (one handler per
  instruction), `rewards.rs` (a wallet's rewards read from chain and their
  claims, tag 174), `claims.rs` (private update-claim reveal material) and
  `errors.rs` (oracle error names).
- `src/chain_identity.rs` owns the deployment identity gate: the RPC's genesis
  must be the trusted deployment's and the oracle and futures programs must be
  deployed there (oracle writes also check the oracle config's bond
  mint). Every signing path passes through it; do not add label-only trust.
- Profiles (`src/futures/settings.rs`): a release build defaults to
  `production` (the API `https://api.amoeba.farm`, Solana's public RPC
  `https://api.mainnet-beta.solana.com`, never the API's `/rpc` gateway, which
  lacks methods petri calls: `rpc::METHODS`; only mainnet-beta, the production
  oracle and futures programs and controller `8fhNi6…`, `Deployment::pin_production`).
  `dev` (the loopback API and validator) is the default of the `dev-chain`
  build (`scripts/install-dev.sh`) and `--profile dev` elsewhere. The dev
  chain's exceptions (its own controller, the oracle's `localnet-dev`
  release, the council's development profile) apply only in the dev profile
  (`Deployment::dev_chain`).
- Releases: `.github/workflows/cli-release.yml` (repository root), by hand on
  a `vX.Y.Z` tag. The updater's release repository is `PETRI_RELEASE_REPOSITORY`
  at build time, else `build.rs`'s `DEFAULT_RELEASE_REPOSITORY`; the README's
  links name the same one (`scripts/check-petri-packaging.mjs`).
- `src/onchain.rs`, `src/wallet_signer.rs` and `src/solana_config.rs` own the
  signer configuration and keypair loading.

Prefer small, reviewable changes inside those boundaries. If a boundary starts
to blur, document the intended split in `docs/html/source-map.html` before
adding more surface area.

## Futures Guardrails

- Read program ids from the trusted deployment (`--deployment` manifest, else the
  futures API's `/v1/config`; the futures program is the manifest's
  `futures.program` or the API's `programs.futures`) and every market parameter
  (grid, `k`, step, mint, period, oracle program) from the market account.
  Refuse a market the trusted futures program does not own, or one with another
  USDC mint or oracle program. Never hard-code markets, `k`, leverages or
  program ids. (Leverage is the trader's: `-x` takes any; the TUI's ticket
  steps through the site's own `FUTURES_LEVERAGES`, `shape::LEVERAGES`.)
- Use the cluster's Clock sysvar for anything time-related; a dev validator's
  clock can lag wall time.
- Build instructions only with the futures client crate
  (`ameba_sdk_futures::futures`). Solve orders with its own solver
  (`Book::propose`) on the account read from chain.
- Quote first and confirm unless `--yes`. A market order sends at most the
  quoted fill with `min_fill >= 1` and a limit no worse than the quote plus
  `--slippage`; a close names its position and carries the same kind of limit.
  The site's ticket builds the same orders (`web/src/lib/futures/market.ts`);
  keep the two in step.
- Before signing a sponsor-prepared futures transaction, run `futures::check::check`:
  sponsor fee payer, only the compute budget, the owner's own USDC account
  creation and the trusted futures program, this market only, the owner as signer
  and their own token account, exactly the requested order, and a deposit no
  larger than the order can need.
- Set a 1.4M compute-unit limit on every transaction (and the 256 KiB heap
  frame on `Submit`). Name program errors (futures `0x46520000 + n`, governance
  `3100 + n`, token by name).
- Bind every instruction of a governed target to its gate
  (`governance::bind_instructions`, `GateBinding::bind`); the oracle is its own
  target (`oracle/send.rs`).

## User-Facing Standard

Read `docs/petri-public-cli-user-theory-of-mind.md` before changing command
grammar, help text, default output, TUI copy, JSON schemas, MCP manifests, or
public examples.

For normal CLI/TUI output, answer these questions before internal detail:

1. What can I trade?
2. How much can I lose?
3. What can I make or receive?
4. When does it settle?
5. Why can I trust the source/oracle?

Use direct market language. Avoid founder/operator language unless the command
is explicitly internal or diagnostic.

## Devloop

Use the repo devloop before handoff:

```bash
scripts/devloop.sh
node scripts/check-petri-launcher.mjs
node scripts/check-petri-packaging.mjs
node --test research/test/*.test.mjs
```

That script runs the research and MCP bridge checks, then:

```bash
cargo fmt -- --check
cargo check --locked --bin petri
cargo test --locked -- --test-threads=1
cargo clippy --locked --all-targets
```

and finishes with the research binary smoke and the MCP server check.

For UI/TUI work, also do a small PTY smoke test when practical:

```bash
script -q /tmp/petri-smoke.log petri
```

Exit with `q`, then inspect the log for obvious layout regressions.

## Tests And Metrics

- Add or update tests when changing parsing, command dispatch, URL construction,
  data extraction, risk math, chart scaling, or signing behavior.
- Identity changes require mismatch tests for genesis, programs and the oracle's
  bond mint, and must fail before any `sendTransaction` request.
- Prefer deterministic unit tests around pure functions and JSON parsing.
- If a backend endpoint is unavailable, fail honestly with a scoped issue rather
  than fabricating terminal data.
- When adding repeated operational checks, make them scriptable first. Human TUI
  polish should sit on top of a command or helper that agents can validate.

## Docs

The HTML docs under `docs/html/` are the agent-readable navigation layer. Update
them when you add commands, modules, scripts, backend route dependencies, or
validation expectations.

Keep the markdown command guide as the canonical user reference:

- `docs/amoeba-cli-command-guide.md`

The HTML files should link to source and docs rather than duplicating every
command example.

## Oracle Guardrails

- Read the oracle program id from the trusted deployment, never from the crate's
  compiled-in id; build `ameba_oracle` without `localnet-dev-authority`.
- Build oracle instructions only with `ameba_oracle::instruction` builders, and
  load every account through `oracle::chain::decode` (owner, exact length,
  layout, zero padding) plus its canonical address and parent binding.
- Prove SKU membership only against a set that reproduces the month's committed
  coverage root; refuse otherwise.
- Never print or log an update claim's salt; reveal material lives only in the
  private claims file.
- Council 30/18 uses the real Spread controller roster through `oracle emergency`
  commands. Sponsorship 30/22 is `oracle sponsor`, `oracle sponsorships
  [settle|claim]` and `oracle reserve withdraw` (the TUI's Fund tab sends the
  same writes). The other reward claims (tag 174) are `oracle rewards
  claim`; the operator's registration and freeze (170/171/173) are
  `futures-ops`'. Cleanup (30/24) has no command.
- Every write supports `--simulate`, confirms unless `--yes`, and sets the 1.4M
  compute-unit limit. Name oracle errors (`errors.rs`, codes 6000 and up).

## Worktrees And Subagents

This repo is suitable for small parallel patches in git worktrees. Handoffs
should include:

- branch or worktree path
- files touched
- commands run
- test result or exact blocker
- backend assumptions, especially any route that was not available locally

Do not ask a subagent to edit sibling repos unless the contract change is
explicit and the sibling repo has been inspected.

## Generated And Secret-Bearing Paths

Do not commit or quote:

- `target/`
- local logs and terminal captures
- `.env` files
- Solana keypair JSON
- Cloudflare, Helius, KMS, R2, or wallet credentials
- generated reports unless the user explicitly asks for an artifact
