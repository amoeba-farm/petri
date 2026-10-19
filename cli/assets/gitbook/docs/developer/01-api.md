# API reference

The futures API (`ameba-futures-api`) serves futures market, account, Earn, oracle and leaderboard data for Amoeba's capped futures on Solana mainnet. It also quotes orders, prepares transactions that its sponsor pays for, and submits them once you have signed. Reads need no account and no API key.

The API is at `https://api.amoeba.farm`: every route below is a path on it, such as `https://api.amoeba.farm/v1/markets`. The web app reaches it through its own origin at `/api/futures/path`. Browsers' JSON-RPC goes to `https://api.amoeba.farm/rpc` (see [Browser RPC gateway](#browser-rpc-gateway)).

> **Note:**
> The API never holds your keys. A prepared transaction has the sponsor as fee payer and an empty owner signature. Your wallet signs it, and the API adds the sponsor's signature when you submit it.

## Conventions

- **Amounts** are strings of integer atoms. USDC has 6 decimals, so `"10000000"` is 10 USDC. Signed amounts may start with `-`.
- **`*Value` fields** are index values as decimal strings with no trailing zeros, such as `"100"` or `"97.37"`.
- **Numbers.** Grid positions, lots, counts, slots and timestamps are JSON numbers. Cumulative atom totals and sequence numbers (`seq`) are strings.
- **Times.** `expiryTs`, `openedTs`, `placedTs`, `lastTradeTs` and `chainTime` are unix seconds of the chain's clock, the one the program uses for tradability and expiry; it can differ from wall time. Trades, history, candles and index points carry wall time. `/health` reports both clocks.
- **Sides.** Requests accept `"long"`, `"short"`, `"buy"`, `"sell"`, `"bid"`, `"ask"`, `0` or `1`. Positions and orders use `"long"` or `"short"`. In trades, `side` is the taker's: `"buy"` (long) or `"sell"` (short).
- **`:market`** is a market's address, a symbol such as `RAMX` (the open market of that symbol with the nearest expiry), a label such as `RAMX Oct 26` (any case) or a series key such as `RAMX-1793491200`. A retired market (see [Retired markets](#retired-markets)) resolves by address, label or series key, never by symbol.
- **Errors** return a non-2xx status with `{ "error": string, "logs"?: string[] }`:

  | Status | Meaning |
  |---|---|
  | 400 | Bad input, or an action the program would refuse (`"name: reason"`, for example `"NoLiquidity: …"`) |
  | 403 | Refused to sign: the owner is a reserved account |
  | 404 | Unknown market or route |
  | 410 | The prepared id is unknown, expired or already used |
  | 422 | The node rejected a submitted transaction outright; `logs` is included |
  | 429 | Rate limited; a `Retry-After` header is set |
  | 500 | Internal error, for example the sponsor failed to sign |
  | 502 | RPC failure |
  | 503 | The database or market state is not ready yet |

  Program errors are named: the futures program's (`0x46520000 + n`: `InsufficientFunds`, `MinFill`, `WouldCross`, `NotBestPrice`, `ClaimReserved`, …), the governance gate's (`GovernanceGateFrozen`, `GovernanceGateEpochMismatch`, …) and the token program's (`TokenInsufficientFunds`, …).

## Units

| Quantity | Meaning |
|---|---|
| `lo`, `hi`, `pos` | Grid positions, `0..=grid`. A position's index value is `pos × stepValue`; on Amoeba's markets one step is one index point. |
| `k` | Atoms one lot of a one-step claim pays in full: 10,000 on Amoeba's markets, so a lot is $0.01 a point (`lotUsdPerPoint`). Read it from each market. |
| Claim payouts | At settlement `S`, a long on `[lo, hi)` pays `clamp(S − lo, 0, hi − lo)` steps of `k` atoms per lot; a short pays the rest of the set, `set = (hi − lo) × k` atoms per lot. |
| Price per lot | Atoms per lot that the long side pays, in fields named `premium` or `price`. A long's cost is `premium × lots`; a short's is `(set − premium) × lots`. |
| Entries | Fine units of `1/k` step: `lo × k + premium`. `*Value` entries are index values rounded to 4 decimals. |
| Orders and claims | Named by `(slot, seq)`. A claim's `seq` changes whenever it is split, merged, transferred or partly used. |

## Configuration and health

`GET /v1/config` returns the deployment: `cluster`, `rpcUrl` and `wsUrl` (public endpoints, or `null`), `genesisHash`, `programs` (`futures` and `oracle`), `treasury`, `usdcMint`, `usdcDecimals`, `sponsor`, `computeUnitLimit` (1,400,000), `preparedTtlSecs` (90), `governanceController` (`{ program, targets }`) and `oracleReleaseTag`. See [Deployment](https://docs.amoeba.farm/technical/05-deployment/).

`GET /health` and `GET /v1/health` return `{ ok, markets, retiredMarkets, slot, database, chainTime, wallTime, logNotes, logNotesMatched, logs }`. `markets` counts the markets listed now and `retiredMarkets` the retired ones; `slot` is the newest slot of any served market's account (`null` before the first); `chainTime` and `wallTime` are the chain's clock and the server's; `logNotes` and `logNotesMatched` (also `logs.futures`) count the futures program's log notifications and those that matched a transaction the API follows.

## Markets

### `GET /v1/markets?all=1` and `GET /v1/markets/:market`

The first returns `{ markets: MarketView[] }`: the markets listed now, and with `all=1` the retired ones after them. The second returns one view:

```text
MarketView = {
  market, symbol, productId, expiryTs, label ("RAMX Oct 26"), status ("open"|"settled"|"failed"), retired,
  tradable, period, periodState: null | { status ("forming"|"live"|"settled"|"failed"), plannedListingTs,
                                          effectiveListingTs, expiryTs, valueDecimals, settlementValue },
  vault, grid, k, stepValue, valueDecimals, lotUsdPerPoint, referenceValue,
  index: null | { value, ts },
  lastEntryValue, lastTradeTs, settlementValue, change24hValue, volume24hLots,
  trades, volumeLots, volumePremium, backing, custody, claimsOpen,
  suggestion: { sigmaValue, lowValue, highValue, lo, hi, centerValue, vol, source },
  maker: null | { owner, slot, free, reserved, maker, lo, hi, escrow, bidLots, askLots, inventoryLots, payoutMin, payoutMax },
  pool: null,
  slot, chainTime
}
```

- `tradable` is whether the market takes orders now: it is open and its oracle period is tradable at the chain clock, the program's own rule. A market can be listed on a Forming oracle period ahead of its month. It takes no trade until the period is Live (`periodState.status` `"live"`), no earlier than `plannedListingTs`; `effectiveListingTs` is 0 until then.
- `status` is `"settled"` once the market settled on its oracle period, and `"failed"` once the governance council set its settlement value instead. `settlementValue` is that value.
- `index` is the product's oracle reference index, sampled about every 30 seconds. It is indicative and never settles anything. `referenceValue` is the index value the market was listed at; it never settles a market either.
- `lastEntryValue` is the last trade on the price line: a trade whose range contains the index. Far-away ranges trade near their own ends and say nothing about the market's level.
- `suggestion` is the ticket's default range: the index ± two standard deviations (`sigmaValue`) of the settlement, in whole grid positions, kept within the span the maker quotes. With a live maker ladder `source` is `"book"`: `sigmaValue` is half the distance between the settlements the book prices at an 84.1% and a 15.9% chance of being exceeded, read from the one-step claims' mid prices. Without one it is `"oracle-index"`, from the index's realized volatility over the last 30 days, or a fixed 40% a year (`"dev-fixture"`) until there are enough index samples; `vol` is the annualized volatility in every case.
- `maker` is the treasury maker's trader in this market: the Earn vault's, where it has one. The Earn vault is [`GET /v1/earn`](#get-v1earn). See [Earn](https://docs.amoeba.farm/earn/02-earn/).
- `pool` (and `lp` in accounts) is always `null` on mainnet: it belongs to the legacy Earn pools of Amoeba's development chain. See [Testing on a local chain](https://docs.amoeba.farm/technical/05-deployment/#testing-on-a-local-chain).

### `GET /v1/markets/:market/depth?lo=&hi=&max=`

```text
{ lo, hi, loValue, hiValue, asks: Level[], bids: Level[] }
Level = { premium (signed atoms per lot), entryValue, lots, cumulativeLots }
```

The executable depth of one range: `asks` are the successive prices of buying `[lo, hi)`, `bids` of selling it, each level one more cheapest package increment across every order and maker step, composite routes included, up to `max` lots. Without `lo` and `hi` the market's suggested range is used. `lo` must be less than `hi`, both from 0 to `grid`, else 400.

### `GET /v1/markets/:market/book`

```text
{ market, slot, orders: OrderView[], ladders: LadderView[], sources: SourceView[] }
```

Every resting order (with its range, lots, price, `entryValue`, escrow and whether it closes a claim), every maker ladder (its range, depth tiers and per-step bids, asks, lots and inventory), and `sources`, the complete list of offers the package solver uses. Different ranges are different claims: combine quantities only for the same side, the same ends and the same price. For what a given range costs, use `/depth` or the quote.

### `GET /v1/markets/:market/trades?limit=100&before=signature`

```text
{ trades: [{ signature, eventIndex, slot, ts, side, lo, hi, loValue, hiValue, lots, premium (total),
             entryValue (average), sources ("orders"|"ladder"|"mixed"), taker }] }
```

One entry per order that filled, newest first. `before` pages to older trades; `limit` is 1 to 1000.

### `GET /v1/markets/:market/candles?resolution=300&from&to`

`{ resolution, candles: [{ t, o, h, l, c, v }] }` over the price line's average entries and filled lots. `resolution` is 60, 300, 900, 3600, 14400 or 86400. By default `to` is now and `from` is 500 resolutions earlier. Buckets without trades are omitted; the market has no candles before its first price-line trade.

### `GET /v1/markets/:market/index?from&to&limit`

```text
{ market, symbol, expiryTs, month | null, points: [{ t, value }] }
```

The reference index of the market's symbol over time, oldest first, sampled about every 30 seconds; `value` is a number in index points. `from` and `to` default to the last 24 hours; `limit` (default 5,000, at most 20,000) keeps the newest points. `month` is the address of the oracle month the market settles on.

### Retired markets

A market that has left the list (for example after its expiry) is **retired**. It stays readable: by address, label or series key on every market route above, in `GET /v1/markets?all=1`, in accounts, history, the leaderboard and the stream, with `retired: true`. Its only transactions are `withdraw`, `cancel`, `finalize` and `settle`; any other action, and `quote-prepare`, is refused with 400 `RetiredMarket`. `POST /v1/quote` still answers. History is never cleared.

## Accounts

### `GET /v1/accounts/:owner`

```text
{ owner, usdc, usdcAta, usdcAtaExists,
  markets: [{ market, symbol, label, expiryTs, status, retired, tradable, markValue,
              trader: null | { slot, free, reserved, paidOut },
              orders: OrderView[], claims: ClaimView[], ladder: LadderView | null, lp: null }] }

ClaimView = { slot, seq, side, lo, hi, loValue, hiValue, lots, cost, reserved, entryValue, openedTs,
              maxPayout, payoutAtIndex, pnlAtIndex,
              exit: null | { lots, premium (total), net, entryValue } }
```

- Only markets where the owner has a trader account are listed, retired ones after the others. `usdc` is the owner's USDC token account balance.
- `payoutAtIndex` values the claim at `markValue`: the reference index while the market is open, the settlement once it is final.
- `exit` is the sale of the claim the book would take now: `lots` fill for `premium` in total, and `net` is what the owner's free cash gains. It is `null` when nothing fills or the market is not tradable.
- The owner's Earn position is [`GET /v1/accounts/:owner/earn`](#get-v1accountsownerearn).

### `GET /v1/wallets/:owner/usdc`

`{ owner, ata, usdcAtoms, decimals, slot }`: the owner's USDC token account, `"0"` when it does not exist.

### `GET /v1/accounts/:owner/history?limit=100&before=signature&types=take,settle`

The owner's indexed events, newest first, across all markets, retired ones included: `{ owner, items: [{ signature, eventIndex, slot, ts, instrument, symbol, label, type, role, data }] }`. `instrument` is the market (the vault for Earn events), and events with an interval carry `loValue` and `hiValue`. An event is listed when the owner is its party or its counterparty. `types` filters by a comma-separated list; the types are listed under [Events](#events). `limit` is 1 to 1000.

## Earn

### `GET /v1/earn`

```text
{ vault: null | EarnVault, chainTime }

EarnVault = { address, operator, cashAccount, shares, cash, deposited, withdrawn, valueLow, valueHigh,
              markets: [{ market, symbol, label, expiryTs, status, tradable, free, reserved, valueLow, valueHigh }] }
```

- The Earn vault, one for the whole product; `null` while the deployment has none. `cash` is its idle cash, outside any market. `markets` are the markets its cash is in (at most 13), in the vault's order, each with the vault trader's `free` and `reserved` cash there and its lowest and highest value there.
- `valueLow` and `valueHigh` are the vault's value over every settlement outcome: its idle cash plus its value in each market. Deposits buy shares at `valueHigh`; withdrawals pay at `valueLow`.

### `GET /v1/accounts/:owner/earn`

```text
{ owner, usdc, vault: null | EarnVault,
  position: null | { address, exists, shares, valueLow, valueHigh, deposited, withdrawn,
                     earningsLow, earningsHigh, share, withdrawable },
  chainTime }
```

- `position` is the owner's vault shares (`null` while there is no vault). `address` is the position account, which `exists` from the owner's first deposit. `valueLow` is what withdrawing every share pays at the vault's lowest value and `valueHigh` their value at its highest; `earningsLow` and `earningsHigh` are those less deposits plus withdrawals, and can be negative. `share` is the owner's fraction of all shares as a decimal string, such as `"0.25"`.
- `withdrawable` is the most an `earnWithdraw` can pay now. `usdc` is the owner's USDC token account balance.

### `GET /v1/accounts/:owner/earn/activity?limit=100&before=signature`

`{ owner, activity: [{ signature, slot, ts, kind, amount, shares }] }`: the owner's Earn deposits (`kind` `"deposit"`) and withdrawals (`"withdraw"`), newest first, with the USDC atoms and the shares of each. `before` pages to older entries.

## Quotes

### `POST /v1/quote`

```json
{ "market": "RAMX", "owner": "…", "side": "long", "lo": 98, "hi": 120, "lots": 3, "limit": "220000",
  "mode": "ioc", "close": { "slot": 4, "seq": "17" } }
```

```text
{ market, side, lo, hi, loValue, hiValue, lots, limit, mode,
  filled, premium (signed total), averageEntryValue,
  pay (what the taker puts in: long = premium, short = set × filled − premium; negative = receives),
  maxPayout, legs: { orders, ladderSteps },
  free?, usdc?, redeemed?, netPay?, escrow?, deposit?      (with owner)
  slot, error? }
```

- `owner`, `limit`, `mode` (`"ioc"` by default, `"gtc"` or `"post"`), `minFill` and `close` are optional. The default `limit` is the most permissive: `set` for a long, 0 for a short. A `gtc` or `post` order needs one.
- With `owner`: `redeemed` is the cash the fill releases by netting against the owner's exact opposite claims, `netPay = pay − redeemed`, `escrow` is what a resting remainder holds, and `deposit` is what the transaction would move from the owner's USDC account.
- `error`: `NotTradable`, `NoLiquidity` (an `ioc` order fills nothing), `WouldCross` (a `post` order would fill), `MinFill`, `InsufficientFunds` (`deposit` exceeds the USDC account), and for `close` `ClaimNotFound`, `NotComplement` or `ClaimReserved`.

Quotes are solved on the API's cached copy of the market with the exact package solver. Nothing simulates.

## Preparing and sending a transaction

1. **Prepare.** While the user edits, call `POST /v1/tx/quote-prepare` with the quote body plus `owner` (and optionally `minFill` and `withdraw`). It returns `{ quote, prepared }`, where `prepared` is the transaction for that quote, or `null` when the quote has an `error`. For other actions, call `POST /v1/tx/prepare`.
2. **Check.** Decode `prepared.transaction` and check it before the wallet signs. See [Prepared transactions](https://docs.amoeba.farm/developer/06-operation-plans/).
3. **Sign.** The owner signs the message bytes unchanged.
4. **Submit.** `POST /v1/tx/submit` with `{ "id": prepared.id, "transaction": "base64, owner-signed" }`. The response is `{ signature, lastValidBlockHeight }` at once.
5. **Wait.** `GET /v1/tx/:signature/wait?commitment=processed` resolves when the transaction lands, fails or expires. `…&commitment=confirmed` adds its decoded events.

### `POST /v1/tx/prepare`

```json
{ "owner": "…", "action": "submit", "params": { "market": "RAMX", "side": "long", "lo": 87, "hi": 135, "lots": 100, "limit": "110000" } }
```

```text
{ id, transaction (base64, fee payer = sponsor, owner signature empty), blockhash, lastValidBlockHeight, summary }
```

| action | params | instructions |
|---|---|---|
| `submit` | `market, side, lo, hi, lots, limit?, minFill?, mode?, close?: { slot, seq }, withdraw?` | heapFrame, [deposit], submit, [createAta, withdraw all] |
| `close` | `market, slot, seq, lots?, limit?, minFill?, withdraw? (default true)` | heapFrame, [deposit], submit naming the claim, [createAta, withdraw all] |
| `cancel` | `market, slot, seq, withdraw?` | cancel, [createAta, withdraw all] |
| `deposit` | `market, amount` | deposit from the owner's USDC account |
| `withdraw` | `market, amount` (atoms, or `"all"`) | [createAta], withdraw |
| `finalize` | `market` | finalize |
| `settle` | `market, withdraw? (default true)` | heapFrame, [finalize], [settle], [settleMaker], [prune], [createAta, withdraw all] |
| `earnDeposit` | `amount` | vaultDeposit from the owner's USDC account into the Earn vault |
| `earnWithdraw` | `amount` (atoms, or `"all"`) | [createAta], vaultWithdraw |
| `cashOut` | `destination, amount` | [createDestinationAta], a USDC transfer from the owner's wallet to another wallet |

- Steps in brackets are added only when needed. Every transaction starts with `SetComputeUnitLimit(1_400_000)`; `submit` steps also request a 256 KiB heap frame.
- **`submit`** is priced like `/v1/quote`; a quote `error` is refused with 400. The package is solved now and travels in the instruction. When the owner's free cash in the market is short, a deposit of exactly the shortfall comes first. `minFill` defaults to 1 for `ioc` and 0 otherwise.
- **`close`** sells (or buys back) the claim `(slot, seq)`: the opposite side on its range, `ioc`, all its unreserved lots at the most permissive limit by default. By default it withdraws all free cash in the same transaction.
- **`finalize`** settles an expired market on its oracle period once the oracle has settled it (`status` `"settled"`). Anyone may send it; Amoeba's keeper normally does. It is refused with 400 `NotExpired`, `AlreadyFinal` or `NotSettled`: a period the oracle has not settled (or failed) waits for the governance council, which can set the settlement value any time after expiry (`status` `"failed"`); the reason says from when. Summary: `status`, `settlementValue`, `periodStatus`.
- **`settle`** pays the owner out of an expired market without waiting for the keeper: it finalizes first when needed (with the same refusals), pays the owner's claims (at most 40 per transaction; `remainingClaims` says how many are left, so prepare again for them), settles their maker ladder, releases their resting orders' escrow, and by default withdraws all free cash to the owner's USDC account (`withdraw: false` leaves it in the market). 400 `NoAccount` without a trader in the market, `NothingToSettle` when there is nothing to do. Summary: `status`, `settlementValue`, `periodStatus`, `finalize`, `claims`, `remainingClaims`, `maker`, `orders`, `payout`, `released`, `free`, `withdraw` and `amount`.
- **`earnDeposit`** mints vault shares at the vault's highest value. **`earnWithdraw`** pays `amount` at its lowest value, from its idle cash and then its cash in its markets, retiring the vault's ladder in a market whose free cash is short (the maker quotes again on its next pass); `"all"` sends every share. More than `withdrawable` is refused with 400 `InsufficientFunds: at most X USDC atoms can be withdrawn now…`, and the vault's last shares while it holds open positions with 400 `NotSettled: …`. Neither takes a `market`.
- **`cashOut`** sends USDC from the owner's wallet to another wallet's USDC account, creating it if needed; the sponsor pays the fee and that rent. The destination must be a plain wallet address (not a program address, the owner, the sponsor or the USDC mint), and `amount` at most the owner's balance.
- `summary` names the action, market (none for `cashOut`, `earnDeposit` and `earnWithdraw`, which name the `vault` or `destination`), owner, fee payer and steps; for orders it adds `expectedFill`, `premium`, `averageEntryValue`, `pay`, `netPay`, `legs`, `deposit` and `withdraw`.
- A prepared transaction stays valid for about 60 seconds, and its id for 90. A transaction over 1,232 bytes is refused.

### `POST /v1/tx/quote-prepare`

The `/v1/quote` body plus `owner` (required), and optionally `mode`, `minFill` and `withdraw`. It returns `{ quote, prepared }`: the quote, and the `submit` for it (`null` when the quote has an `error`). Call it while the user edits; every prepared message stays usable until it expires. A retired market is refused with 400 `RetiredMarket`.

### `POST /v1/tx/submit`

The API co-signs only when the id was prepared less than 90 seconds ago and not used, the message bytes are identical to the prepared ones, the fee payer is the sponsor, and every other required signature, including the owner's, verifies. It then sends the transaction without preflight and resends it every 300 ms until it is processed or its blockhash expires. Failures come back through the wait, `GET /v1/tx/:signature` and the owner's stream, with the decoded error name. Each id can be submitted once.

### `GET /v1/tx/:signature/wait?commitment=processed|confirmed&timeoutMs=30000`

```text
{ signature, status ("pending"|"processed"|"confirmed"|"failed"|"expired"),
  slot, processedMs, confirmedMs, err?, error? (decoded name), failedInstruction?, events?, logs?,
  timedOut, waitedMs }
```

It resolves as soon as the signature reaches the commitment, fails or expires, or after `timeoutMs` (at most 120,000). At `confirmed` it returns the decoded events. A failure names the error and the step that failed (`failedInstruction`, such as `"submit"`). A package that went stale fails with an error such as `NotBestPrice`, `FifoViolation` or `BadProposal`; prepare it again.

### `GET /v1/tx/:signature`

```text
{ signature, status ("pending"|"confirmed"|"finalized"|"failed"), slot?, error?, blockTime?, events?, logs? }
```

Any transaction's status, with its decoded events and logs once it has landed. For a transaction submitted through the API it also carries `commitment`, `processedMs`, `confirmedMs` and, on failure, `err` and `failedInstruction`; an expired one is `failed` with `error` `"BlockhashExpired"`.

## Stream

### `GET /v1/stream?market=market&owner=pubkey`

Server-sent events. Give `market` (address, symbol, label or series key; `instrument` is another name for it), `owner`, or both.

- For the market: `event: book` with the market's live offers (`{ market, slot, sources: [{ source, side, lo, hi, loValue, hiValue, lots, entryValue }] }`), once on connect and then on every change; and `event: trade`, one `/trades` entry plus `market`, for each new trade.
- For the owner: `event: tx` on every change of the owner's submitted transactions (the wait body); `event: wallet`, `{ owner, usdcAtoms, slot }`, on connect and whenever the owner's USDC balance changes; and `event: account`, the owner's entry of `/v1/accounts/:owner` for one market plus `owner` and `slot`, whenever their own state there changes. Nothing is sent for `account` on connect: open the stream, then read `/v1/accounts/:owner` once.

Keep-alive comments are sent every 15 seconds. A client that accepts gzip gets the stream gzip encoded, each event flushed as it is ready. Polling `/book` and `/trades` every one to two seconds is equivalent.

## Events

Each market's transactions are indexed into an append-only history that is never cleared. Event types, with their log tags:

| type (tag) | data |
|---|---|
| `deposit` (`ARDEPO01`), `withdraw` (`ARWDRW01`) | `owner`, `amount` |
| `take` (`ARTAKE01`) | `owner`, `side`, `mode`, `lo`, `hi`, `lots`, `filled`, `premium`, `paid`, `seq`, `rested` |
| `fill` (`ARFILL01`) | `owner` (the resting order's), `taker`, `side`, `close`, `lo`, `hi`, `lots`, `price`, `slot`, `seq` |
| `ladderFill` (`ARLADR01`) | `maker`, `taker`, `side` (the maker's), `lo`, `hi`, `lotSteps`, `premium` |
| `place` (`ARPLACE1`) | `owner`, `slot`, `seq`, `side`, `close`, `lo`, `hi`, `lots`, `limit`, `escrow` |
| `cancel` (`ARCANCL1`) | `owner`, `target` (`order` or `ladder`), `slot`, `seq`, `released` |
| `quote` (`ARQUOTE1`) | `owner`, `maker`, `seq`, `lo`, `hi`, `escrow`, `bidLots`, `askLots` |
| `redeem` (`ARREDM01`) | `owner`, `lo`, `hi`, `lots`, `cash` |
| `transfer` (`ARXFER01`) | `from`, `to`, `side`, `lo`, `hi`, `lots` |
| `finalize` (`ARFINL01`) | `market`, `status`, `settlementFine`, `settlementValueAtoms` |
| `settle` (`ARSETL01`) | `owner`, `claim`, `side`, `lo`, `hi`, `lots`, `payout` |
| `settleMaker` (`ARMKSE01`) | `owner`, `maker`, `payout` |
| `market` (`ARMRKT01`) | `market`, `symbol`, `productId`, `expiryTs`, `registry` |
| `earnCreate` (`ARVCRT01`) | `vault`, `operator`, `cashAccount` |
| `earnDeposit` (`ARVDEP01`) | `owner`, `vault`, `amount`, `shares` |
| `earnWithdraw` (`ARVWDR01`) | `owner`, `vault`, `shares`, `amount` |
| `earnFund` (`ARVFND01`), `earnDefund` (`ARVDFD01`) | `vault`, `market`, `amount` |

`earnFund` and `earnDefund` are the operator moving the vault's cash into and out of a market. The vault trades as its own trader in each market, so its fills, quotes and cancels are ordinary events whose `owner` is the vault's address.

## Oracle

Two documents from the oracle program, refreshed every 30 seconds, in a `{ ok: true, data }` envelope. Before the first read they answer 503 `ORACLE_SNAPSHOT_PREPARING`.

- `GET /v1/oracle/public-snapshot`: one fact per oracle period (its month's phase, settlement status and source count), and each month's verified source inventory with buckets, weights and canonical source links. `freshness` says how old the read is. With no oracle month yet, `status` is `"empty"`: a valid answer, not an outage.
- `GET /v1/oracle/catalog`: each product's periods with their settlement times.

Research agents use `/oracle/agents` and `/oracle/source-candidates` at the API root (not under `/v1`) to register and submit source candidates without a signature. A candidate has no oracle weight. `POST /oracle/source-candidates/:contentHash/prepare` (and `…/refresh`) sponsors a candidate's listing bond: it takes `{ sponsor }`, the wallet that pays, and returns the sponsorship's transactions unsigned, for that wallet to sign and send through `/rpc/sendTransaction`. It refuses in plain words: 409 `ORACLE_SPONSOR_CLOSED`, `ORACLE_SPONSOR_EXISTS` or `ORACLE_SPONSOR_COLLATERAL`, 422 `ORACLE_SPONSOR_REJECTED`, 503 `ORACLE_SPONSOR_UNAVAILABLE`. `GET /oracle/sponsorships` (or `?funder=wallet`) lists the agreements.

## Leaderboard

`GET /v1/leaderboard?days=7|30|90` (absent: all time) returns `{ generatedAt, since, clockOffsetSecs, traders: [{ owner, pnlAtoms, paidAtoms, volumeAtoms, trades, lastTradeAt, unpriced }], index }`. P&L covers the lots a trader opened in the period: sale proceeds less opening cost for closed lots, plus open lots at what the book would pay for them now. Lots the book cannot price count at their payout at the reference index and set `unpriced`; a settled market pays its settlement. Retired markets count like the others. The deployment's own keys, including the treasury maker and the Earn vault, are excluded.

## News

`GET /v1/news` returns the home page's hardware news links: `{ ok: true, data: { fetchedAt, stories: [{ url, title, category, pageUrl, writtenUtc, imageUrl, imageSrc, imageAlt }], … } }`, or 502 when the news source is unavailable. News is for display and never enters a price or a settlement.

## Browser RPC gateway

`POST https://api.amoeba.farm/rpc` (or `POST /rpc/method`) forwards one JSON-RPC request to the chain and returns the answer unchanged. It allows 18 methods, `sendTransaction` among them, each with an exact parameter grammar, from Amoeba's web origins or from clients without an `Origin` header, with per-client rate limits. It never signs: a `sendTransaction` whose fee payer is the sponsor is refused with 409 `GAS_SPONSORSHIP_UNAVAILABLE`, because sponsored transactions go through `/v1/tx/submit`. A relayed transaction is resent until it lands, and `/v1/tx/:signature/wait` follows it. Refusals are `{ ok: false, message, code }`, for example 422 `RPC_METHOD_NOT_ADMITTED` or 429 `RPC_GATEWAY_RATE_LIMITED`.

`GET /v1/debug/cache/:address` returns what the API's account cache holds for an address: `{ known, exists, slot, owner, lamports, len, head }`, public chain data only.

## Limits

- Quotes, prepares and submits have no per-owner limit. Prepares and quote-prepares share one limit across all owners, 12,000 a minute.
- Prepared transactions expire after 90 seconds.
- The sponsor never signs for itself, the futures or oracle program, the USDC mint, the treasury maker, the Earn vault or any other key of the deployment's own. Requests for these owners get 403.
