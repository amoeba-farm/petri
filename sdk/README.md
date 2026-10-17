# ameba-sdk

TypeScript and Rust SDK for Ameba futures (the `ameba-futures` program) on
the oracle's `Period` accounts.

```sh
npm install ameba-sdk
```

- **Capped futures**, what users trade. One
  market per index and expiry (RAMX, NANDX; monthly expiries). A claim on a
  price range `[lo, hi)` pays the index's move through the range, so the
  range is the leverage: losses are capped at the premium (or the short's
  funding), nothing is ever liquidated, and any two shapes trade against each
  other through packages the program verifies against the whole book. Design:
  [`futures/docs/futures.md`](https://github.com/amoeba-farm/petri/blob/main/futures/docs/futures.md); HTTP
  contract: [`api/API.md`](https://github.com/amoeba-farm/petri/blob/main/api/API.md), "Futures"; on-chain
  contract: [`futures/docs/interfaces.md`](https://github.com/amoeba-farm/petri/blob/main/futures/docs/interfaces.md).

## Package surfaces

- `ameba-sdk/futures` (also the root `ameba-sdk` entry) contains:
  - futures: `FuturesClient`, a typed client for the futures API's market,
    account and quote routes and its sponsored prepare → sign → submit flow;
    `FuturesMarket`, builders for every futures instruction with exact account
    metas; `FuturesBook`, the market account decoder (traders, orders, maker
    ladders, claims, the arc list the program verifies, trader values);
    `FuturesEarnVault`, the Earn vault's builders, with its account and
    position decoders; the proposal codec; PDAs, payoffs and error names;
  - the oracle `Period` PDA (`findPeriodAddress`) and unit helpers;
  - governance gate binding.

  It depends only on `@solana/web3.js` v1 and `buffer`, and runs in Node and
  browsers.
- The Rust crate `ameba_sdk` has a `futures` module. It re-exports the futures
  client crate `ameba-futures-client`, linked by path from this monorepo's
  `futures/crates/futures-client` (so it always builds against the same
  tree): `ameba_sdk::futures::{Market, Book, …}`, everything at the crate
  root.

## Futures (`ameba-sdk/futures`)

The futures API solves the package each order is owed and pays the fee; the
owner reviews and signs:

```js
import { FuturesClient, preparedTransaction, reviewPrepared, signPrepared } from "ameba-sdk/futures";

const api = new FuturesClient({ baseUrl: "https://api.amoeba.farm" });
const quote = await api.quote({ market: "RAMX", owner, side: "long", lo: 86, hi: 104, lots: 3 });

const prepared = await api.prepare(owner, "submit", {
  market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: 180_000n,
});
reviewPrepared(preparedTransaction(prepared), program); // decoded instructions (program: config().programs.futures)
const { signature } = await api.submit(prepared, signPrepared(prepared, keypair));
await api.wait(signature, { commitment: "confirmed" });

// Positions and exits: claims carry their executable exit.
const account = await api.account(owner);
await api.send(owner, "close", { market: "RAMX", slot, seq }, (tx) => { tx.partialSign(keypair); return tx; });
```

On chain, `FuturesBook.decode(accountData)` reads a market account and
`FuturesMarket` builds instructions (`submit`, `close`, `cancel`, `deposit`,
`withdraw`, `quote`, `split`, `merge`, `redeem`, `transfer`, settlement, and
the legacy Earn pools' `poolWithdraw` and `poolMigrate` on dev).
`FuturesEarnVault` builds the Earn vault's (`create`, `deposit`, `withdraw`,
`fund`, `defund`): one vault for the whole product, whose shares are a part
of all of Earn.
`Submit` carries a solved proposal: take it from a transaction the API
prepared or from the Rust client's `Book::propose`. Every futures
instruction must be bound to the governance gate before signing
(`GovernanceGateBinding.bind`). See
[docs/FUTURES.md](./docs/FUTURES.md#futures-markets).

64-bit and wider integers are `bigint`, and narrower ones are `number`.
Builders reject out-of-range values instead of truncating them.

## Rust

```rust
use ameba_sdk::futures::{Book, Market, Side, SubmitArgs, MODE_IOC};

let book = Book::decode(&market_account_data)?;
let h = book.header;
let m = Market::derive(program, market, h.vault, h.period, h.quote_mint, &h.product_id, h.expiry_ts);
let order = SubmitArgs { side: Side::Long, lo: 86, hi: 104, lots: 3, limit: 180_000, min_fill: 1, mode: MODE_IOC, close: None };
let ix = m.submit(&owner, &order, book.propose(&order.taker()).proposal);
```

## Verification

```sh
npm ci
npm test            # build plus the TypeScript tests against the golden vectors
npm run check:rust  # cargo fmt and the Rust tests against the same vectors
```

The vectors in `test/fixtures/futures` come from `tools/futures-vectors`,
which links the Rust client and the futures program's state from the
monorepo's `../futures` and the oracle interface from `../oracle`. Regenerate them
with `npm run vectors`. `test/fixtures/api` holds trimmed responses of
the dev futures API for the `FuturesClient` tests.

## License

Apache-2.0. See [LICENSE](./LICENSE) and [NOTICE](./NOTICE).
