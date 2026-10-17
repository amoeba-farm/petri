# `ameba-sdk/futures`

Client for Ameba futures (the `ameba-futures` program), the
product users trade, and the oracle `Period` accounts its markets settle
against. Golden vectors from the Rust client crate
(`futures/crates/futures-client`) pin every encoder and decoder. Design:
`futures/docs/futures.md`; on-chain contract:
`futures/docs/interfaces.md`; HTTP contract: `api/API.md`, "Futures".

```ts
import {
  FuturesClient, FuturesMarket, FuturesBook, decodeProposal, reviewPrepared, signPrepared,
} from "ameba-sdk/futures";
```

It depends only on `@solana/web3.js` (v1) and `buffer`, and runs in Node and
browsers.

## Conventions

* 64-bit and wider integers (`u64`, `i64`, `u128`, `i128`) are `bigint`:
  lots, atoms, premiums, sequence numbers. Grid positions, slot numbers,
  claim indexes and other `u8`/`u16`/`u32` values are `number`.
* Inputs that take an address accept a `PublicKey` or a base58 string.
* Builders validate every integer against its wire width and throw a
  `RangeError` instead of truncating.
* Sides are `0` = long and `1` = short (`FuturesSide`).

## Futures markets

A market is one account per index and expiry (`RAMX Oct 26`). Prices
sit on a grid of positions `0..=grid` (dev: 200 steps of one index point). A
**long** claim on `[lo, hi)` pays `clamp(S - lo, 0, hi - lo)` steps of `k`
atoms per lot at settlement `S`, the **short** the rest; a long plus a short
on one interval is a complete set, backed by `(hi - lo) * k` atoms per lot.
Leverage is the shape: a long's `lo` (a short's `hi`) caps its loss. Orders of
any shape trade against each other, and maker ladders, through balanced
packages the program verifies against the whole book (best total price, then
the most lots, then FIFO). Nothing is liquidated; there are no fees or funding.

| Name | Meaning |
|---|---|
| position | grid index `0..=grid`; index value `pos * stepValue` (`positionValue`) |
| `k` | atoms one lot of a one-step claim pays in full (dev: 10,000 = $0.01) |
| premium | atoms per lot the long pays; the short receives it and funds `set - premium` |
| fine units | `1/k` of a step: entries and settlements; `entry = lo * k + premium` |
| side | `0` long, `1` short (`FuturesSide`); requests also take `"long"`/`"short"` |
| mode | `0` ioc, `1` gtc (rest the remainder), `2` post-only (`FuturesMode`) |
| `(slot, seq)` | names an order or a claim; a claim's `seq` changes on split, merge, transfer or partial use |

### Trading through the futures API

The futures API solves every package on its cached book (the Rust client's
exact solver) and sponsors the fee, so a client only quotes, signs and sends.
`FuturesClient` covers the market, account and quote routes and the
sponsored transaction flow:

```ts
import { FuturesClient, preparedTransaction, reviewPrepared, signPrepared } from "ameba-sdk/futures";

const api = new FuturesClient({ baseUrl: "https://api.amoeba.farm" }); // a dev API: http://127.0.0.1:4500
const program = (await api.config()).programs.futures!;
const owner = keypair.publicKey;

const market = await api.market("RAMX");                     // also markets(), book(), trades(), candles()
const depth = await api.depth("RAMX", { lo: 86, hi: 104 });  // executable asks and bids of one shape
const quote = await api.quote({ market: "RAMX", owner, side: "long", lo: 86, hi: 104, lots: 3 });

// Open: an immediate-or-cancel long on [86, 104), paying at most 180,000 atoms a lot.
const prepared = await api.prepare(owner, "submit", {
  market: "RAMX", side: "long", lo: 86, hi: 104, lots: 3, limit: 180_000n,
});
console.log(reviewPrepared(preparedTransaction(prepared), program)); // what you sign
const { signature } = await api.submit(prepared, signPrepared(prepared, keypair)); // or wallet.signTransaction
const result = await api.wait(signature, { commitment: "confirmed" }); // events: take, fill, …

// Close a claim, cancel an order, withdraw: prepare, sign and submit in one call.
const sign = (tx) => { tx.partialSign(keypair); return tx; };
const { markets: [held] } = await api.account(owner);
const [claim] = held.claims, [order] = held.orders;
await api.send(owner, "close", { market: held.market, slot: claim.slot, seq: claim.seq }, sign);
await api.send(owner, "cancel", { market: held.market, slot: order.slot, seq: order.seq }, sign);
await api.send(owner, "withdraw", { market: held.market, amount: "all" }, sign);
```

* Reads: `markets()`, `market(id)`, `book(id)` (orders, ladders and the
  solver's complete source list), `depth(id, { lo, hi, max })`,
  `trades(id, { limit, before })`, `candles(id, { resolution, from, to })`,
  `account(owner)` (cash, orders, claims with their executable `exit`,
  ladder); for Earn, `earn()` (the vault: shares, idle cash, value low and
  high, its markets), `earnAccount(owner)` (the owner's shares, their value,
  earnings, share and what can be withdrawn now) and
  `earnActivity(owner, { limit, before })`. `id` is a market address, a symbol (its open market with the
  nearest expiry), a label (`RAMX Oct 26`) or a series key
  (`RAMX-1793491200`); a retired market resolves by address, label or series
  key, never by symbol. `config()` returns `GET /v1/config`
  (`programs.futures`, `treasury`).
* `quote(request)` prices an order without signing; `quotePrepare` returns
  the quote and its `submit` together.
* `prepare(owner, action, params)` for `deposit`, `withdraw`, `submit`,
  `close`, `cancel`, after expiry `finalize` and `settle` (settle the owner's
  claims, orders and ladder, then withdraw unless `withdraw: false`; a
  retired market takes only withdrawals, cancels, finalize and settle),
  `cashOut` (USDC from the owner's wallet to another wallet), the Earn
  vault's `earnDeposit { amount }` and `earnWithdraw { amount }` (atoms or
  `"all"`; these three take no `market`), and on dev the legacy pools' exit
  `poolWithdraw { market, shares }`; `submit(prepared, signed)`;
  `wait(signature, { commitment, timeoutMs })` (confirmed: the decoded events,
  `take`, `fill`, `ladderFill`, …); `transaction(signature)`;
  `send(owner, action, params, sign)` does all three.
* Views (`MarketView`, `BookView`, `DepthView`, `TradeView`, `AccountView`,
  `FuturesQuote`, …) follow the API's units: amounts are strings of atoms,
  `*Value` fields decimal strings, positions, lots and timestamps numbers.
  Requests take `bigint` for any integer.
* A non-2xx answer throws `FuturesApiError` with `status`, `error`, `logs`
  and `retryAfter`.
* Not wrapped yet (call them with `fetch`): `markets?all=1`,
  `markets/:market/index`, `/v1/stream` (events `book`, `trade`, `account`,
  `tx`, `wallet`), `/v1/accounts/:owner/history` and
  `/v1/wallets/:owner/usdc`.
* `preparedTransaction(prepared)` keeps the prepared message bytes exactly
  (the API refuses any other); `signPrepared(prepared, keypair)` adds the
  owner's signature; `reviewPrepared(tx, program)` decodes every
  futures instruction (and its governance epoch) so a client can check what
  it signs.

### Instructions

`FuturesMarket` mirrors `Market` of the Rust client:

```ts
const m = FuturesMarket.derive(program, marketAddress, { vault, period, mint }, "RAMX", expiryTs);
// or from the account: FuturesMarket.fromHeader(program, marketAddress, FuturesBook.decode(data).header)
m.deposit(owner, usdcAccount, amount);
m.withdraw(owner, usdcAccount);                 // all free cash (u64::MAX)
m.submit(owner, { side: "long", lo: 86, hi: 104, lots: 3n, limit: 180_000n, minFill: 1n, mode: "ioc" }, proposal);
m.close(owner, claim, k, proposal);             // the opposite side on the claim's interval, naming it
m.cancel(owner, order.slot, order.seq);
m.quote(owner, ladder, knots, proposal); m.cancelQuote(owner);
m.split(owner, claim, at); m.merge(owner, keep, absorb); m.redeem(owner, long, short, lots);
m.transfer(owner, claim, lots, to);
m.finalize(); m.settle(claimSlots); m.settleMaker(maker); m.prune(orderSlots); // permissionless
m.create(creator, oracleProgram, params);        // admin

// The Earn vault: one for the whole product, run by an operator who can never withdraw its cash.
const earn = FuturesEarnVault.fromAccount(program, decodeVault(vaultData)); // or .derive(program, operator, { cash, mint })
const { markets } = decodeVault(vaultData);      // every market of the vault, in its order
earn.deposit(owner, usdcAccount, amount, payer, markets);          // shares at the vault's highest value
earn.withdraw(owner, usdcAccount, U64_MAX, markets, [m]);          // all shares at its lowest, from idle cash, then m's free cash
earn.create(payer); earn.fund(m, amount); earn.defund(m);          // the operator's
operated(m.cancel(earn.vault, slot, seq), earn.operator);          // the operator trading for the vault in m

// Legacy per-market pools (dev builds only): their exit, and the move into the vault.
m.poolWithdraw(m.pool(operator), owner, usdcAccount);
m.poolMigrate(m.pool(operator), holder, earn, payer, markets);
```

* `Submit` and `Quote` carry a solved **proposal**. The SDK does not solve
  packages: take it from a transaction the API prepared, or from the Rust
  client (`Book::propose`). Send `Submit` with `heapFrame()` (256 KiB)
  and `computeLimit()` (1.4M).
* On a governed deployment (dev and mainnet) every futures instruction
  carries the governance envelope: authenticate the futures program's gate
  with `GovernanceGateBinding.fromAccount(controller, program, gate, owner,
  data)` and `bind(ix)` before signing. The API's transactions are bound
  already.
* `encodeFuturesInstruction` / `decodeFuturesInstruction(data, { governed })`
  read and write the Borsh data; `closeArgs(claim, k, options)` gives a
  close's arguments (defaults as the API's `close`). The retired tags 15
  (`CreatePool`) and 16 (`PoolDeposit`), which the program always refuses,
  still decode, with `retired: true`, so an old transaction reviews; nothing
  builds them.
* PDAs: `findRegistryAddress(program, product, expiryTs)`,
  `findAuthorityAddress(program, market)` (the market vault's owner),
  `findVaultAddress(program, operator)` (the Earn vault),
  `findVaultPositionAddress(program, vault, owner)` and, for the legacy
  pools, `findPoolAddress(program, market, operator)` and
  `findLpAddress(program, pool, owner)`. Their seeds start with
  `FUTURES_NAMESPACE`, `"ameba-range-v1"`: the deployed program's on-chain
  identity, which keeps its name (as do the account magics, `FUTURES_MAGIC`
  `"AFRANGE1"` and the others).

### The market account

`FuturesBook.decode(data)` (or `decodeMarket`) checks the magic, version
and exact length and reads records on demand, mirroring the Rust `Book`:

* `header` (`FuturesHeader`: grid, `k`, `stepValue`, caps, status,
  settlement, totals, volume, last entry);
* `trader(slot)`, `order(slot)`, `claim(slot)`, `maker(slot)`,
  `step(maker, pos)`, and the live lists `traders()`, `traderOf(owner)`,
  `orders()`, `claims()`, `claimsOf(trader)`, `makers()` (each ladder with
  its quoting or inventory steps);
* `arcs()`: the complete arc list the program verifies proposals against
  (orders by slot, then ladder steps by maker, step, bid before ask, tier);
* `traderValue(slot)` / `valueOf(owner)`: what a trader holds is worth,
  `[lowest, highest]` atoms over every settlement outcome (the program's
  own valuation).

`tierSizes`, `tierRemaining` and `tierPrice` split a ladder step's lots and
prices into depth tiers. `decodeRegistry` and `marketFilters()` (for
`getProgramAccounts`) find markets. `decodeVault` and `decodeVaultPosition`
read the Earn vault and a depositor's position (`decodePool` and
`decodeLpPosition` the legacy pools').

### The Earn vault

One Earn vault serves the whole product. A deposit buys shares of
everything it holds: its idle cash (`FuturesVault.cash`) and, in each of its
markets (`FuturesVault.markets`, at most `VAULT_MARKETS` = 13), the trader
slot owned by the vault's address, which the treasury maker trades.

* Value: idle cash plus `book.valueOf(vaultAddress)` in every one of its
  markets, `[lowest, highest]` over every settlement outcome.
* `VaultDeposit` mints `amount * shares / valueHigh` (1:1 into an empty
  vault), so a deposit never dilutes the others. It names every market of
  the vault, in its order.
* `VaultWithdraw` pays `shares * valueLow / totalShares` from idle cash,
  then from the vault's cash in the markets passed as draws (where a
  drawn market's free cash is short, it first retires the vault's ladder
  there to release the escrow; the maker quotes again). The
  vault's last shares wait while its highest value is above what they would
  get (`NotSettled`). The gap between low and high is the range of the open
  positions; it closes as markets settle.
* The operator signs `VaultFund` and `VaultDefund`, which move cash between
  idle cash and a market, and trades for the vault with `operated(ix,
  operator)`. It can never withdraw: cash leaves only to depositors. Funding a
  17th market fails with `VaultCapacity`.
* There are no fees and no funding.

### Proposals

`decodeProposal(bytes)` and `encodeProposal(proposal)` read and write
`futures-math`'s compact codec exactly (and reject what it rejects).
`proposalDense(proposal, book.arcs())` lays the fills over the book, throwing
when a fill names an arc the book lacks, and `takerPremium(side, arcs, fills)`
is the premium the package charges.

### Payoffs and errors

`setValue` (a complete set's backing, `(hi - lo) * k`), `longPerLot`,
`shortPerLot`, `settleFine`, `contribution`, `premiumAt`, `entryOf`,
`averageEntry`, `fineToValue` and `positionValue` are exact bigint math, as
the Rust `payoff` module and the client's unit helpers. Futures program
errors are `0x46520000 + n` (`ERROR_BASE`, `ERROR_NAMES`, `ERROR_CODES`,
`futuresErrorName`); `transactionErrorName` also names the shared governance
errors. `customErrorCode(err)` extracts a code.

## Constants, PDAs and units

* Program ids: `SYSTEM_PROGRAM_ID`, `TOKEN_PROGRAM_ID`,
  `BPF_LOADER_UPGRADEABLE_PROGRAM_ID`. The deployment's own program ids come
  from the futures API (`GET /v1/config` → `programs`).
* `findPeriodAddress(oracle, product, expiryTs)` (seeds `PERIOD_NAMESPACE`,
  `PERIOD_SEED`) and `findProgramDataAddress(program)` return
  `[PublicKey, bump]`. A `product` is a symbol (`"RAMX"`) or the 32-byte id:
  `productId(symbol)` zero-pads the ASCII symbol, `productSymbol(id)`
  reverses it and `toProductId` accepts either.
* `formatUnits(units, decimals)` and `parseUnits(value, decimals)` convert
  atoms to and from decimal strings exactly.
* `FUTURES_COMPUTE_UNIT_LIMIT` is 1,400,000. Set it on every trading
  transaction.
* Shared governance verifier errors are `3100 + n`
  (`GOVERNANCE_ERROR_BASE`, `GOVERNANCE_ERROR_NAMES`).

## Golden vectors

The files in `test/fixtures/futures/*.json` come from
`tools/futures-vectors`, a small Rust program with its own cargo workspace.
It links the monorepo's `futures/` crates and the oracle interface by path.
It writes:

* `futures.json`: the futures PDAs and the oracle period of each product and
  expiry, and every `FuturesMarket` instruction; a market
  account written by the futures program's state types, with its records,
  arcs and trader values (open, settled and failed); proposals solved by
  `Book::propose`; codec cases (including rejected ones); tiers; payoffs;
  error names; the Earn vault's PDAs, instructions and accounts, and the
  legacy pools' `PoolWithdraw` and `PoolMigrate`.

`test/fixtures/api/responses.json` holds trimmed responses of the dev
futures API for the `FuturesClient` tests.

To regenerate the vectors:

```sh
FUTURES_COMMIT=$(git log -1 --format=%H -- ../futures) \
CARGO_TARGET_DIR=tools/futures-vectors/target \
  cargo run --offline --manifest-path tools/futures-vectors/Cargo.toml -- test/fixtures/futures
```

The `test/futures-*.test.mjs` files assert equality against these vectors.

## Governance binding before signing

Builders return raw instructions. For gated deployments, read the canonical
controller gate accounts with the market snapshot and authenticate them with
`GovernanceGateBinding.fromAccount(controller, target, address, owner, data)`,
then `gate.bind(raw)` each futures instruction. It appends the readonly gate
account and the exact signed `AGV1` epoch (`decodeGovernanceTail` splits it
off again); frozen or malformed gates are refused.

The trusted localnet manifest supplies its deployed controller identity; `TEST_CONTROLLER_PROGRAM_ID` is only a synthetic fixture. The decoder authenticates the explicitly trusted controller argument.
Only select it on the trusted localnet deployment. Gated binaries reject
unbound instructions. A freeze or epoch change after preparation invalidates a signed transaction:
refresh the gates and rebuild before signing again.
