# Changelog

## 0.4.0 - 2026-10-16

The first release published to npm. It is the client for the futures program
and API that run on Solana mainnet-beta (`https://api.amoeba.farm`); the 0.3.x
entries below describe earlier development builds that were never published.

- Compressed USDC (the Light tokens wallets from Amoeba's earlier product
  still hold): `FuturesClient.walletCompressedUsdc(owner)`
  (`GET /v1/wallets/:owner/compressed-usdc`, `WalletCompressedUsdc`) and
  `walletUsdc(owner)` (`WalletUsdc`). `PreparedTransaction.before` lists the
  transactions that move compressed USDC into a USDC account ahead of an
  order or cash-out; `FuturesClient.send` signs every one with the main
  transaction, submits them in order, waits for each, and returns their
  signatures as `before`.

- One Earn vault replaces the per-market Earn pools (2026-10-16, the
  futures program's `CreateVault` … `PoolMigrate`, tags 19–24):
  - `FuturesEarnVault` (the Rust client's `EarnVault`): `derive`,
    `fromAccount`, `address`, `position`, `create`, `deposit`, `withdraw`
    (idle cash, then the free cash of the markets passed as draws), `fund`
    and `defund`; `FuturesMarket.poolMigrate` moves a legacy pool into the
    vault (dev builds). `operated` acts for the vault as for a pool.
  - `findVaultAddress` (`["ameba-range-v1", "earn", operator]`),
    `findVaultPositionAddress` (`[…, "earn-lp", vault, owner]`),
    `decodeVault` / `FuturesVault` (`AFRVALT1`, 608 bytes, at most
    `VAULT_MARKETS` = 13 markets), `decodeVaultPosition` /
    `FuturesVaultPosition` (`AFRVLPS1`), the `VAULT_*` constants and the error
    `VaultCapacity` (`0x46520000 + 31`).
  - The codec reads and writes `createVault`, `vaultDeposit`,
    `vaultWithdraw`, `vaultFund`, `vaultDefund` and `poolMigrate`.
  - Removed `FuturesMarket.createPool` and `poolDeposit`: the program
    refuses `CreatePool` and `PoolDeposit` (`InvalidInstruction`). Their tags
    (15, 16) still decode, with `retired: true`, so an old transaction
    reviews instead of throwing. `poolWithdraw`, `pool`, `lp`, `decodePool`
    and `decodeLpPosition` stay for the legacy pools' exit on dev.
  - `FuturesClient.earn()` (`GET /v1/earn`), `earnAccount(owner)`
    (`GET /v1/accounts/:owner/earn`) and `earnActivity(owner, { limit,
    before })`, with the `EarnView`, `EarnVault`, `EarnVaultMarket`,
    `EarnAccountView`, `EarnPosition` and `EarnActivity` types; actions
    `earnDeposit { amount }` and `earnWithdraw { amount | "all" }`. The
    `poolDeposit` action is gone; `poolWithdraw` stays as the legacy exit, as
    do `MarketView.pool` and the account's `lp`.
  - The golden vectors' Earn vault entries (PDAs, instructions, accounts,
    `VaultCapacity`) were written by hand from the program's layouts;
    `tools/futures-vectors` is updated to emit them. Regenerate with
    `npm run vectors` to confirm them against the Rust client.
- Renamed for the futures product (2026-10-15): nothing is named "range"
  any more except a claim's price interval. Types and classes `Range*` are
  `Futures*` (`FuturesClient`, `FuturesMarket`, `FuturesBook`,
  `FuturesApiError`, `FuturesProposalError`, `FuturesSide`, …), and the API
  views take `api/API.md`'s names (`MarketView`, `BookView`, `OrderView`,
  `LadderView`, `SourceView`, `DepthView`, `TradeView`, `ClaimView`,
  `AccountView`). Functions and constants lose the `range` prefix
  (`findRegistryAddress`, `decodeProposal`, `longPerLot`, `heapFrame`,
  `operated`, `ERROR_BASE`, `HEADER_LEN`, …), or take `futures` where the
  bare name would be ambiguous (`futuresErrorName`, `futuresSide`,
  `futuresMode`, `encodeFuturesInstruction`, `decodeFuturesInstruction`,
  `FUTURES_NAMESPACE`, `FUTURES_MAGIC`, `FUTURES_VERSION`);
  `reviewPreparedRange` is `reviewPrepared`, and a reviewed instruction's
  `program` is `"futures"`. The modules are `program.ts`, `accounts.ts`,
  `instructions.ts`, `proposal.ts` and `api.ts`; the golden vectors are
  `futures.json`. On-chain values are unchanged: the program id, the seeds
  (`"ameba-range-v1"`, …), the account magics (`AFRANGE1`, …), the event
  tags and the error codes.
- `FuturesClient` follows the API's rename: routes `/v1/markets…`,
  `/v1/accounts/:owner` and `/v1/quote`; actions `deposit`, `withdraw`,
  `submit`, `close`, `cancel`, `poolDeposit`, `poolWithdraw`, `finalize`
  and `settle`, plus `cashOut` (USDC to another wallet); event types
  `take`, `fill`, `ladderFill`, …; `config()` `programs.futures` and
  `treasury`.
- Removed the legacy capped-futures clearing and matcher surface: the
  clearing instruction builders (`FuturesDeployment`, `FuturesMarket`), the
  instrument, wallet, LP, positions and config decoders, the matcher book and
  take simulation, `parseFuturesEvents`, the clearing math,
  `bindClearingGovernance`, `futuresErrorName` / `futuresErrorSource`, and
  the Rust `FuturesClient`. The futures market is the only product
  (`transactionErrorName` names a futures transaction's errors); the
  clearing names `FuturesMarket` and `futuresErrorName` now name its builder
  and error names.
- `CouncilSettle` (tag 18): `FuturesMarket.councilSettle`, encoded and
  decoded. A failed oracle period no longer settles at the reference value;
  the governance council sets the value.
- Authenticate canonical governance gates and bind their epochs before signing.
- The futures market (`ameba-futures`), the product users trade:
  - `FuturesClient`: the futures API's market, account and quote routes
    (markets, book, depth, trades, candles, account, quote, quote-prepare)
    and the sponsored prepare → sign → submit → wait flow, with
    `FuturesApiError`, `preparedTransaction`, `signPrepared` and
    `reviewPrepared`;
  - `FuturesMarket`: every futures instruction with the Rust client's exact
    account metas, `encodeFuturesInstruction` / `decodeFuturesInstruction`
    (governed data included), `closeArgs`, `heapFrame`, `computeLimit`;
  - `FuturesBook`: the market account (header, traders, orders, maker
    ladders and steps, claims), its canonical arc list and trader values;
    `decodeRegistry`, `marketFilters`, depth-tier helpers;
  - the proposal codec (`encodeProposal`, `decodeProposal`, `proposalDense`,
    `takerPremium`);
  - PDAs, statuses, sides, modes, error names and exact payoff and unit math;
  - Earn pools: `findPoolAddress`, `findLpAddress`, `decodePool`,
    `decodeLpPosition`, `FuturesMarket.createPool`, `poolDeposit`,
    `poolWithdraw`, `operated`, and the API's `poolDeposit` /
    `poolWithdraw` actions and `pool` / `lp` views.
- `decodeGovernanceTail` splits a bound instruction's data.
- `FuturesClient` follows `api/API.md`: the `finalize` and `settle`
  actions; `suggestion.source` `"book"`; `retired` on market and account
  views; nullable `owner`, `claim.seq`, `failedInstruction` and config
  fields; `err`, `failedInstruction` and `commitment` on `TxStatusResult`.
  `config().rpcUrl` is the deployment's public RPC URL or `null`.
- Rust: `ameba_sdk::futures` (the client crate, everything at its root) is
  documented and tested against the same vectors (`rust/tests/vectors.rs`).
  The client crate is linked by path from the monorepo, not pinned by
  revision.
- Golden vectors: `futures.json` from `tools/futures-vectors`.

All notable changes to the SDK are documented here. This project follows
[Semantic Versioning](https://semver.org/).

## 0.3.1 - Unreleased

### Changed

- Follow futures `a6603ec`. The matcher book is storage v4 (320-byte makers,
  28-byte regions). Instruments carry `minLeverage`/`maxLeverage` instead of a
  leverage mask, and `CreateInstrument` takes them. Bands are clipped to the
  settlement range: `halfBandIn` = `min(ceil(E/L), U - E)`, and
  `maxHalfBandUpto` gives a quote's escrow band. `orderEscrow` and
  `collateralPerLot` now need the universe, and `quoteEscrowPerLot` is new.
- Rust: `ameba-futures-client` is pinned to `a6603ec`.

### Removed

- `leverageMask`, `leverages(mask)`, `instrumentLeverages`, `minLeverage(i)`
  and `maxEntryTick`. `leverages(instrument)` takes the leverage range, and
  `maxTick` is `U - 1`.

## 0.3.0 - Unreleased

### Added

- `ameba-sdk/futures` (also the root entry) is the client for single-line
  capped futures. It covers:
  - constants and error names;
  - PDAs;
  - all 23 clearing instruction builders plus matcher prune and grow, with
    exact account metas;
  - instruction encoding and decoding;
  - account decoders for the instrument, wallet, LP position, positions arena
    and config;
  - matcher book depth and take simulation;
  - a typed event parser;
  - bigint math.
- Rust `ameba_sdk::futures` re-exports `ameba-futures-client` and adds
  `FuturesClient`.
- Golden vectors generated from the Rust crates by `tools/futures-vectors`.

