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
position `AFRLPOS1` (128).

## 4. Instructions (Borsh)

`FuturesInstruction`, tag = variant index. Account lists are in the doc
comments of `programs/futures/src/lib.rs`; `ameba_futures_client::Market`
builds every one of them.

| Tag | Instruction | Who |
|---|---|---|
| 0 | `CreateMarket` | the program's upgrade authority (the council once governed) |
| 1 | `Deposit` | owner |
| 2 | `Withdraw` (free balance; `u64::MAX` = all) | owner |
| 3 | `Submit` (one claim shape with a verified package; GTC/post-only rests the remainder) | owner or a pool's operator |
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
| 15 | `CreatePool` | operator |
| 16 | `PoolDeposit` | depositor |
| 17 | `PoolWithdraw` | depositor |

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
config and three seat signers that `CreateMarket` needs once the program's
upgrade authority has moved to the controller. Programs have independent
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
