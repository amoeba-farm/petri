# Ameba futures: interfaces (dev)

This is the contract that the futures API service, the TypeScript SDK, the
frontend and the CLI build against. The design record is
[futures.md](futures.md). Rust sources of truth:
`programs/futures/src/{lib.rs,state.rs,processor.rs}` (instructions,
layouts, events) and the root of `crates/futures-client` (`src/lib.rs`:
addresses, builders, the market decoder and the package proposer). The HTTP contract is
[`api/API.md`](../../api/API.md).

## 1. Product

One market per index and expiry (RAMX, NANDX; monthly expiries) on a grid of
positions `0..=grid` (dev: 200, one index point per step). A **range claim** on
`[lo, hi)` pays the long `clamp(S - lo, 0, hi - lo)` steps per lot at
settlement `S`; the short holds the rest of the complete set,
`(hi - lo) - long`. A complete set is backed by exactly `(hi - lo) * k` atoms
per lot (dev `k` = 10,000 atoms: a lot is $0.01 a point).

A position is bought at a **premium** per lot, which puts its entry on the
market's single price line: `entry = lo + premium / k`. Every leverage and
range trades against every other through balanced packages, verified on chain
against the whole book: best total price, then the most lots, then older-first
FIFO. There are no liquidations, no protocol fees and no funding.

### Units

| Name | Meaning |
|---|---|
| grid position | integer price step. `index value = position * step_value / 10^value_decimals` |
| `step_value` | oracle value units per step (the period's decimals; dev: one index point) |
| `k` | quote atoms one lot of a one-step claim pays in full (dev: 10,000 = $0.01). USDC has 6 decimals |
| premium | atoms per lot paid by a long (received by a short) for a claim |
| fine | settlement value in `k`-fractions of a step (`settlement_fine`) |

Sides: `0` = long, `1` = short.

## 2. Programs and PDAs

| Program | Role |
|---|---|
| futures (`ameba-futures`) | market accounts (traders, orders, maker ladders, claims), the USDC vault, matching, settlement, Earn pools |
| oracle (`ameba_oracle`; the dev chain runs its `localnet-dev-authority` build) | `Period` accounts (`ameba-oracle-interface` layout): Forming → Live → Settled / Failed, and the `OracleMonth` behind each period that forms its index |
| governance controller (`governance_controller_v3`) | the council and the target gates every futures instruction checks (section 6) |

The LiteSVM integration tests run the same SBF artifacts: the oracle is built
from this monorepo's `oracle/`, in the same dev profile, under a test-only
program id. The oracle fails a period only while it is Forming (tag 79), and
a market trades only while its period is Live (`NotTradable` otherwise), so a
market that traded never ends on a Failed period. A Live period that is never
settled goes to the council: `CouncilSettle` any time after expiry
(`futures.md`, "Trading window" and "Settlement"). The reference value never
settles a market.

| PDA | Seeds | Program |
|---|---|---|
| registry | `["ameba-range-v1", "registry", product_id[32], expiry_ts u64 LE]` | futures |
| vault authority | `["ameba-range-v1", "authority", market]` | futures |
| Earn pool | `["ameba-range-v1", "pool", market, operator]` | futures |
| LP position | `["ameba-range-v1", "lp", pool, owner]` | futures |
| Earn vault | `["ameba-range-v1", "earn", operator]` | futures |
| Earn position | `["ameba-range-v1", "earn-lp", vault, owner]` | futures |
| stake pool | `["ameba-range-v1", "stake", vault]` | futures |
| stake position | `["ameba-range-v1", "stake-pos", pool, owner]` | futures |
| period | `["ameba-oracle-v1", "period", product_id[32], expiry_ts u64 LE]` | oracle |

`product_id` is the ASCII symbol zero-padded to 32 bytes (`"RAMX"`, `"NANDX"`).
The `ameba-range-v1` seed namespace, the account magics (`AFRANGE1`,
`AFRNGREG`, `AFRPOOL1`, `AFRLPOS1`), the `AR*` event tags and the error codes
(`0x4652xxxx`) are the deployed program's on-chain identity: they keep their
values, "range" included.
The market account itself is an ordinary account owned by the program (its
key is recorded in the registry); the vault is a classic SPL token account
owned by the vault authority.

## 3. Account layouts (little-endian)

The market account (`AFRANGE1`, version 1) is a 512-byte header followed by
four fixed arenas, sized at creation (`state::market_len`):

| Region | Record | Size |
|---|---|---|
| header | `Header` | 512 |
| traders | `Trader` × `trader_cap` | 64 each |
| orders | `Order` × `order_cap` | 64 each |
| makers | `Maker` head + `grid` × `Step` | 64 + 32·grid each |
| claims | `Claim` × `claim_cap` | 56 each |

Field offsets are in `programs/futures/src/state.rs`;
`ameba_futures_client::Book::decode` reads the whole account. The other
accounts: registry `AFRNGREG` (48 bytes), Earn pool `AFRPOOL1` (128) and LP
position `AFRLPOS1` (128), Earn vault `AFRVALT1` (608, or 800 with its
reward stream) and position `AFRVLPS1` (128), stake pool `AFRSTAK1` (800;
792 when created before its count of credit markets, 784 before the unstake
timer, 704 before its epoch rates and share index: read all four) and stake
position `AFRSTKP1` (208, or 192
before the unstake timer: read both, and query both sizes when filtering by
`dataSize`). Since 2026-10-17 the market header's bytes
376..512 and 28..32 hold the market's stake-backed credit (all zero without
any; `Header::credit_owner` and after, `futures.md`, "Staked AMBA backs the
maker").

Since 2026-10-18 the stake pool and its positions carry a share epoch and a
share index, and the pool when its first price was posted (`futures.md`,
"Staked AMBA backs the maker", Epochs and Price). Accounts from before read
all of them as 0.

| Account | Field | Offset |
|---|---|---|
| stake pool | `epoch` u32 | 368 |
| stake pool | `wipe_epoch` u32: the epoch the last full wipe-out (or an index that rounded to zero) started; shares of earlier epochs are worth nothing | 372 |
| stake pool | `first_price_ts` u64: when the first price was posted (0 in a pool from before: long ago) | 376 |
| stake pool | `epoch_ends[i]`, i = 0..4: the AMBA stream's `acc` u128 then `usdc_acc` u128 at the end of epoch `epoch - 1 - i` | 384 + 32i, 400 + 32i |
| stake pool | the AMBA subsidy stream (`Rewards`, all zero without one) | 512..704 |
| stake pool | `epoch_rates[i]`, i = 0..4: `staked` u64 then `shares` u64 at the end of epoch `epoch - 1 - i` (784-byte pools only; zero in a 704-byte one) | 704 + 16i, 712 + 16i |
| stake pool | `share_index` u128: every rollover's rate (`staked` / `shares`) multiplied up since the last full wipe-out, times `INDEX_ONE` = 2^126, each product rounded down; 0 reads as `INDEX_ONE` (784-byte pools only) | 768 |
| stake pool | `unbonding` u64: the shares unstaking, summed over the positions (792- and 800-byte pools only) | 784 |
| stake pool | `credit_markets` u32: the markets holding its credit unsettled that it counts (header `credit_flags` & `CREDIT_COUNTED` = 4); `Unstake` takes every one (800-byte pools only; 0 before) | 792 |
| stake pool | `credit_counted` u8: 1 when `credit_markets` covers every market holding its credit; 0 in a pool grown from a shorter size while it lends anything (`StakePool::counted`: also complete with nothing lent); then three zero bytes (800-byte pools only) | 796 |
| market header | `credit_flags` (byte 28): `CREDIT_SETTLED` = 1, `CREDIT_SHORTFALL` = 2, `CREDIT_COUNTED` = 4 (in its pool's `credit_markets`: set by the `CreditFund` that counts it, cleared by its `CreditSettle`) | 28 |
| stake position | `epoch` u32 (the pool epoch its shares are of) | 168 |
| stake position | `index` u128: the pool's `share_index` in that epoch (0: `INDEX_ONE`) | 176 |
| stake position | `unbonding` u64: of its `shares`, those unstaking (208-byte positions only) | 192 |
| stake position | `unbond_ts` u64: when its last `StartUnstake` started the timer (208-byte positions only) | 200 |

A position whose `epoch` isn't the pool's has shares of an old epoch: don't
read them from the account. Up to four epochs behind, it is owed what they
earned up to its epoch's end in `epoch_ends`, and they convert at that
end's rate (its shares times the rate's `staked` over its `shares`, rounded
down), then at each later end's, into shares of the current epoch (none
after a full wipe-out). Further behind, its shares convert at once to its
shares times the pool's `share_index` over its `index` (each 0 read as
`INDEX_ONE`; the product needs 256 bits), rounded down and never more than
it had; none if its `epoch` is before the pool's `wipe_epoch`. What they
earned since it was last touched is lost; it keeps its pending amounts
(Codex's review of the conversion, 2026-10-18; before, it held no shares at
all).
Its `unbonding` converts the same way, never above its shares.
`ameba_futures_client::stake_position_at` returns the
position as the program would settle it (its shares, unstaking shares,
pending AMBA and USDC); `stake_at` just the pending amounts.
`StakePool::unbond_ends` gives when its unstaking shares may leave
(`unbond_ts + UNBOND_SECS`, none with nothing unstaking).

Since 2026-10-18 (the user) unstaking takes a timer, `state::UNBOND_SECS`
= 604,800 s (7 days). `StartUnstake` (41) marks shares as unstaking,
`CancelUnstake` (42) returns them, `Unstake` (32) pays them once the timer
is done. Unstaking shares stay in the position's and the pool's `shares`:
they earn and bear shortfalls as every share. The pool's capacity
(`StakePool::capacity`) leaves out the AMBA of its `unbonding` shares; the
check that what stays after an `Unstake` backs the credit lent
(`capacity_of`) counts all the AMBA left, unstaking AMBA included. Accounts:
`StartUnstake` 0 pool (w); 1 position (w); 2 owner (s); 3 payer (s, w: an
older pool's or position's growth to 800 / 208 bytes); 4 system program.
`CancelUnstake` 0 pool (w); 1 position (w); 2 owner (s). `Unstake` 0 pool
(w); 1 position (w); 2 any signer (`StakeAddrs::complete_unstake`; the
owner's own `StakeAddrs::unstake`); 3 the owner's AMBA account (w): the
AMBA goes only into an AMBA account the position's owner owns (`Custody`
otherwise); 4 the pool's AMBA account (w); 5 AMBA mint; 6 its token
program; then, read-only, every market holding the pool's credit
unsettled: exactly `credit_markets` of them, distinct, each owned by the
program with the pool's vault as `credit_owner`, `CREDIT_COUNTED` set and
not `CREDIT_SETTLED` (`Header::counts_credit_of`; `InvalidAccount`
otherwise). `ameba_futures_client::unstake_markets` picks them from any
list of market headers (the builders take them as `markets`); the ops
tools scan the program's accounts for `credit_owner` (bytes 376..408 of a
market).

Since 2026-10-18 (Codex's review of the timer) `Unstake` also waits
(`NotSettled`) while any of those markets is in its last `expiry_window`
(from the earlier of its `credit_until` and its expiry less the pool's
`expiry_window`) or past expiry with its credit unsettled, and while a pool
whose count is incomplete (`credit_counted` 0) lends anything. Anyone but
the owner must pay all of the position's unstaking shares (`Unauthorized`
for a part), and an `Unstake` paying 0 AMBA is refused
(`InsufficientFunds`).

## 4. Instructions (Borsh)

`FuturesInstruction`, tag = variant index. Account lists are in the doc
comments of `programs/futures/src/lib.rs`; `ameba_futures_client::Market`
builds every one of them.

| Tag | Instruction | Who |
|---|---|---|
| 0 | `CreateMarket` | the program's upgrade authority (the council once governed) |
| 1 | `Deposit` | owner |
| 2 | `Withdraw` (free balance; `u64::MAX` = all) | owner |
| 3 | `Submit` (one claim shape with a verified package; GTC/post-only rests the remainder, which holds a claim slot for its fills: `ClaimCapacity` on a full table) | owner or a pool's operator |
| 4 | `Cancel` | owner or a pool's operator |
| 5 | `Quote` (the owner's maker ladder, post-only) | owner or a pool's operator |
| 6 | `CancelQuote` | owner or a pool's operator |
| 7 | `Split` | owner |
| 8 | `Merge` | owner |
| 9 | `Redeem` (an exact complement for its backing) | owner |
| 10 | `Transfer` | owner |
| 11 | `Finalize` | anyone, after expiry |
| 12 | `Settle` | anyone, after finalization |
| 13 | `SettleMaker` | anyone, after finalization |
| 14 | `Prune` | anyone, after finalization |
| 15 | `CreatePool` (retired) | |
| 16 | `PoolDeposit` (retired) | |
| 17 | `PoolWithdraw` (dev builds) | depositor |
| 18 | `CouncilSettle` | the council (or the upgrade authority) |
| 19–24 | `CreateVault`, `VaultDeposit`, `VaultWithdraw`, `VaultFund`, `VaultDefund`, `PoolMigrate` | see `futures.md`, "The Earn vault" |
| 25–27 | `VaultRewards`, `VaultRewardsClaim`, `VaultRewardsWithdraw` | see "Earn rewards (AMBA)" |
| 28 | `CreateStakePool` | the vault's operator and the program's upgrade authority (the council once governed) |
| 29 | `StakeConfig` | operator |
| 30 | `StakePrice` (USDC atoms per AMBA × 10^6; dev builds only, `operator-price`: the production build refuses it, `Unauthorized`; a price older than `MAX_PRICE_AGE` = 3,600 s gives no capacity, below) | operator |
| 31 | `Stake` | staker |
| 32 | `Unstake` (of the unstaking shares, `u64::MAX` = all, once `UNBOND_SECS` = 7 days have passed since the last `StartUnstake`: before, `NotSettled`; more than are unstaking, or 0 AMBA, `InsufficientFunds`; what backs lent credit stays, `CreditLimit`; while a market holding the pool's credit is in its last `expiry_window` or past expiry unsettled, `NotSettled`; those markets follow its fixed accounts, below) | anyone, into the owner's own AMBA account; anyone but the owner pays all of it |
| 33 | `StakeClaim` (AMBA stream and USDC, into the owner's accounts) | anyone |
| 34 | `StakeRewards` | operator |
| 35 | `StakeRewardsWithdraw` (surplus only) | operator |
| 36 | `CreditFund` | operator |
| 37 | `CreditDefund` (`u64::MAX` = all it can; open markets only) | operator |
| 38 | `CreditSettle` (a shortfall waits for a price at most `MAX_PRICE_AGE` old: `NotSettled`; the pool uncounts the market from `credit_markets`, and `CreditFund` (36) counts it at its first credit) | anyone, after finalization and the vault's own settlement |
| 39 | `SettleRecovery` (one owner; USDC part to free cash, AMBA part to its AMBA account) | anyone, in a shortfall market |
| 40 | `PoolPrice` (the AMBA price from mainnet's Meteora AMBA/SOL pool and Pyth's SOL/USD EMA; below) | anyone, alone in its transaction |
| 41 | `StartUnstake { shares }` (`u64::MAX` = all not unstaking yet; capped at them; none: `InsufficientFunds`; restarts the timer for all the position's unstaking shares) | staker |
| 42 | `CancelUnstake` (the unstaking shares back to active; none: `InsufficientFunds`) | staker |

`PoolPrice` accounts (`StakeAddrs::pool_price` builds them): 0 stake pool
(w); 1 the Meteora DAMM v2 pool `DxUB19khkvd3FehY8sovQ1FTTcEktPL4CM6yc49coWQH`
(owner `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`); 2 Pyth's sponsored
SOL/USD `PriceUpdateV2` `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE` (owner
the Pyth Solana Receiver `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`); 3 the
instructions sysvar. Every address, owner and layout is a compile-time
constant (`ameba_futures::price_source`). The transaction may hold only this
instruction (top level) and compute-budget instructions, else
`InvalidInstruction`. Wrong addresses, owners, layouts, feed id or mints:
`InvalidAccount`. A Pyth update older than `PYTH_MAX_AGE` (300 s) or not
`Full`ly verified, a pool that is disabled, not active yet or empty, or a
price that rounds to zero or overflows: `InvalidPrice`. It emits `ARSPRC01`,
as `StakePrice` does.

Since 2026-10-18 (the user, after Codex's review of `PoolPrice`) the posted
price lends only while fresh (`StakePool::price_lends`): a last post more
than `MAX_PRICE_AGE` = 3,600 s old gives the pool zero capacity, so
`CreditFund` returns `CreditLimit`, and so does `Unstake` while credit is
lent; `CreditSettle` with a debt left returns `NotSettled` until a fresh
post. In the production build (no `operator-price`) a pool also has zero
capacity until `MAX_PRICE_AGE` after its first post (`first_price_ts`, 0 in
an older pool: long ago). Crank `PoolPrice` (or, on dev, `StakePrice`)
before these. The pool keeps no average of its own, so the program
averages the samples; a bundle of separate transactions after each honest
crank can still skew that average at a cost (`futures.md`, "Staked AMBA
backs the maker", Price).

What it reads (DAMM v2 `Pool`, 1,112 bytes, discriminator `f19a6d0411b16dbc`):
`token_a_mint` @168 (AMBA `BxKJN4PKAT2CFhVbsyKzVyCznu5a2n2oVh8Pb2wdDjam`),
`token_b_mint` @200 (wrapped SOL), `liquidity` u128 @360, `sqrt_price` u128
@456 (Q64.64, square root of lamports per AMBA atom), `activation_point` u64
@472, `activation_type` u8 @480 (0 slot, 1 timestamp), `pool_status` u8 @481
(0 trading). Source: `programs/cp-amm/src/state/pool.rs` and `state/fee.rs`,
github.com/MeteoraAg/damm-v2. Pyth `PriceUpdateV2`, 134 bytes,
discriminator `22f123639d7ef4cd`: `verification_level` @40 (1 = `Full`; the
message offsets that follow assume it), `feed_id` @41
(`ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d`),
`exponent` i32 @89, `publish_time` i64 @93, `ema_price` i64 @109. Source:
`pyth-solana-receiver-sdk` `price_update.rs`, github.com/pyth-network/
pyth-crosschain; feed ids at pyth.network/developers/price-feed-ids. The
price, in the pool's units, is `sqrt_price^2 * ema_price *
10^(amba_decimals + 3 + exponent) / 2^128`, rounded down.

Errors added with them: `CreditLimit` (32), `NeedsRecovery` (33).

`CreateStakePool` accounts (since 2026-10-18; `StakeAddrs::create` builds
0 to 9): 0 pool (w, PDA `["ameba-range-v1", "stake", vault]`); 1 Earn
vault; 2 operator (s); 3 payer (s, w; the program's upgrade authority while
it is ungoverned); 4 AMBA mint; 5 the pool's AMBA account; 6 the pool's
USDC account; 7 system program; 8 this program; 9 its ProgramData; then,
once the program is governed, the council config and three seats (s), as
`CreateMarket` (`with_council_authorization`). An ungoverned program
refuses any other payer, and an immutable one every payer (`Unauthorized`).

Since 2026-10-18 the credit rules (`futures.md`, "Staked AMBA backs the
maker") return errors from more instructions:

- `CreditLimit` from the credit owner's own `Quote` and `Submit` while it
  owes anything, when some later fill of its resting orders, close orders
  or ladder could take its exposure over the market's credit limit or its
  debt beyond its own free and reserved cash plus the backing its claims
  and inventory hold (`state::credit_admits`; since the re-review of
  2026-10-18). Fills of its liquidity, and everyone else's instructions,
  then can't fail on its account. `CreditDefund` returns it below the
  worst-case exposure (`state::credit_exposure`).
- `CreditLimit` from any instruction that would leave the credit owner's
  debt above the open sets' backing plus its own free and reserved cash in
  that market (the backing alone once the credit has settled): a check the
  rule above keeps from failing while the market is open.
- `NeedsRecovery` from `Transfer` and `Redeem` after finalization while the
  market's credit is unsettled or settled short, for every owner but the
  credit owner: claims leave only through `Settle` and `SettleRecovery`.
- `NotTradable` from `CreditDefund` once the market is final.

## 5. Events

`sol_log_data` lines (`Program data: <b64> <b64> …`, one base64 field per
slice, emitted by the futures program). The first field is the 8-byte tag.

| Tag | Fields after the tag |
|---|---|
| `ARMRKT01` | market, product_id, expiry u64, registry |
| `ARDEPO01` / `ARWDRW01` | owner, amount u64 |
| `ARPLACE1` | owner, slot u16, seq u64, [side, flags], lo u16, hi u16, lots left u64, limit u64, escrow u64 |
| `ARTAKE01` | owner, [side, mode], lo u16, hi u16, lots u64, filled u64, premium u64, paid u64, seq u64, rested u64 |
| `ARFILL01` | resting owner, taker, [side, flags], lo u16, hi u16, lots u64, price u64, slot u16, seq u64 |
| `ARLADR01` | maker, taker, [side], lo u16, hi u16, lot-steps u64, premium u64 |
| `ARCANCL1` | owner, slot u16 (`u16::MAX` = the ladder), seq u64, released u64 |
| `ARQUOTE1` | owner, [maker], seq u64, lo u16, hi u16, escrow u64, bid_lots u64, ask_lots u64 |
| `ARREDM01` | owner, lo u16, hi u16, lots u64, cash u64 |
| `ARXFER01` | from, to, [side], lo u16, hi u16, lots u64 |
| `ARFINL01` | market, [status], settlement_fine u64, settlement_value u64 |
| `ARSETL01` | owner, claim u32, [side], lo u16, hi u16, lots u64, payout u64 |
| `ARMKSE01` | owner, [maker], payout u64 |
| `ARPOOL01` | pool, market, operator |
| `ARPDEP01` | pool, owner, amount u64, shares u64 |
| `ARPWDR01` | pool, owner, shares u64, amount u64 |
| `ARSCRT01` | stake pool, vault, AMBA mint |
| `ARSPRC01` | stake pool, price u64, average u64, ts u64 |
| `ARSSTK01` / `ARSUNS01` | stake pool, owner, AMBA u64 / shares u64, shares u64 / AMBA u64 |
| `ARSUBD01` | stake pool, owner, shares added u64, unstaking u64, timer's end ts u64 (`StartUnstake`) |
| `ARSUBC01` | stake pool, owner, shares u64 (`CancelUnstake`) |
| `ARSCLM01` | stake pool, owner, AMBA u64, USDC u64 |
| `ARCFND01` / `ARCDFD01` | market, stake pool, amount u64, limit u64 |
| `ARCSET01` | market, stake pool, stakers' share u64, buffer covered u64, shortfall u64, AMBA set aside u64 |
| `ARRCVR01` | market, owner, payout u64, USDC u64, AMBA u64 |

The authoritative encoding is the `sol_log_data` call in `processor.rs`; the
API's indexer decodes these lines (its events module, in `api/api/src/`).

## 6. Spread V3 governance envelope

Every futures instruction (and every oracle instruction in the oracle's governed
builds) requires its canonical controller-owned gate as the final, read-only,
non-signer account and a 16-byte signed instruction suffix: `AGV1`, version 1,
three zero bytes, then the current gate epoch as little-endian u64. The shared
verifier lives in `governance/crates/governance-interface`. It binds the owner,
PDA, council config, target program, canonical ProgramData, exact layout,
Active status and epoch. A frozen target accepts no state-changing instruction,
including custody exits, until the council explicitly reactivates it.
Reactivation increments the epoch, so transactions signed before the freeze
cannot resume afterward.

`ameba_futures_client::governance::GateBinding::bind` adds the gate account
and suffix to an instruction; `with_council_authorization` adds the council
config and three seat signers that `CreateMarket` and `CreateStakePool` need
once the program's upgrade authority has moved to the controller. Programs have independent
target gates; pausing the oracle alone does not pause custody in the futures
program. Council proposals can freeze the affected targets.

Production builds pin the reviewed Spread controller and preserve its timing.
The explicit `localnet-governance` profile pins the public controller key from
`AMEBA_GOVERNANCE_CONTROLLER_ID` and uses the development timing described in
`governance/README.md`. The existing oracle `localnet-dev-authority` profile is
development plumbing, not the governed production build.

Off-chain deployment manifests opt into envelopes with
`governanceController: {program, targets}`. A configured gate that is missing,
frozen or invalid fails preparation.

Oracle Period version 2 permits a genuine settled value of zero. Readers still
accept exact version 1 accounts under their original nonzero-settlement rule.
The layout and addresses are unchanged, and new writes use version 2.


## 7. Operator credit assessments

`CreditAuthorize` is appended to the Borsh instruction enum (tag 43):
`snapshot [u8;32]`, `exposure u64`, `action Vec<u8>`. The action is an
unwrapped Quote or Submit, or empty to refresh. Its usual accounts start
with the writable market and the Earn vault, followed by its operator
(signer, writable) and the system program. The governance envelope wraps
only the outer instruction. The operator must be the registered vault's
operator, and the vault must be this market's credit owner.

The snapshot is SHA-256 of `ameba-credit-approval-v1`, program address,
market address, and the exact pre-action account bytes. There is no range
parameter. The exposure is a trusted assessment of every possible later fill;
carry, custody, and accounting remain independently enforced.

Existing market offsets stay unchanged. First approval appends a 16-byte
tail: `AFCRISK1` (8 bytes), exposure (little-endian u64). The operator pays
rent. Readers accept the legacy exact length or that length plus this tail.
New Quote/Submit actions replace the assessment; ordinary fills preserve its
upper bound. Cash draws increase it; settled markets use actual settlement
accounting. An account without the tail uses full-grid exposure.
