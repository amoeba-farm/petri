# Ameba futures (dev build)

The implementation of the founder's range-composition handoff (Revision 2,
14 October 2026): one range-aware market per index and expiry, packages
composed from actual orders, an optional funded maker that trades missing and
surplus pieces, fully funded accounting, no protocol fees and no funding.
Dev chain only; not audited.

| Piece | Where |
|---|---|
| Math: claims, packages, the exact solver, the on-chain verifier, depth | `crates/futures-math` (`ameba-futures-math`) |
| Program: market account, custody, matching, claims, settlement | `programs/futures` (`ameba-futures`) |
| Client: builders, account decoder, proposer | `crates/futures-client/src/lib.rs` |
| Acceptance and regression tests (LiteSVM, real SBF) | `integration/tests/market.rs`; brute force in `crates/futures-math/tests/brute.rs` |

## Product

A **range claim** on `[lo, hi)` (grid positions, dev: one index point per
step) pays the long `clamp(S - lo, 0, hi - lo)` points per lot at settlement
`S`; the short holds the complement, `(hi - lo) - long`. A long and a short on
the same interval form a **complete set**, backed by exactly `(hi - lo) * k`
atoms per lot (dev `k` = 10,000 atoms, so a lot is $0.01 a point).

A position is bought at a **premium** per lot. The premium places the entry on
the market's single price line: `entry = lo + premium / k`. So the handoff's
four parameters map directly. `(F, q, P_min, P_max)` is the claim `q R[lo,hi)`
bought at entry `F`, with `lo = F + P_min/q` and `hi = F + P_max/q`. A long's
leverage is `F / (F - lo)`, a short's `F / (hi - F)`, so the profit-side end
of either is free to choose. The 50x long and 5x short of handoff section 5
are one exact pair on 98–120 at entry 100 (test `a1`).

## Packages and priority

An order is one claim shape with a limit premium per lot. Maker **ladders**
offer every one-step claim `[x, x+1)` separately (a bid and an ask per step),
so they can fill any missing or surplus piece; they are ordinary offers from an
ordinary account.

An execution is a **package**: at every grid step, the filled long lots
covering it equal the filled short lots covering it. Read as a graph over grid
positions, a long fill is flow `lo -> hi` and a short fill `hi -> lo`, so a
package is a circulation. With arc costs `-price` (long) and `+price` (short),
resting orders trade at their own limits and the incoming order (the taker)
takes the rest. Adjacent pieces, overlaps, a buyer absorbing an overlap and a
maker filling a gap are all just circulations. Partial fills keep every
order's whole shape (lots scale; intervals never split). Only a ladder step,
which is its own shape, fills piecewise.

**Comparator (published).** Of all packages over the complete book, the
taker is owed the one that is:

1. **best total price**: minimum circulation cost with the taker's arc at its
   limit. This is the existing book's price priority, generalized to
   composites;
2. **most lots**: among those, the taker fills as many lots as it can at its
   limit (equal-price liquidity trades, as in any book);
3. **older first (FIFO)**: among those, the vector of resting fills ordered by
   arrival rank (oldest first) is lexicographically largest. An older order
   fills as much as any equal-price package allows before a newer one is used.

A rank is `(arrival sequence, sub-index)`. Orders, ladder quotes and claims
draw their sequence from one counter per market. A ladder's steps rank by
position, bid before ask. The taker is newest. The rule names no order type,
owner or route, so a maker, a direct order and a composite route compete on the
same terms (tests `r1`, `r2`, `r6`). An old order that cannot be part of any
balanced, limit-compliant package is in no optimal package, so it blocks
nothing (`r3`). Partial quantities are just the fill vector. Splines (ladders)
are their per-step arcs. Equal-price ladders tie-break by quote age, then step.

**Engineering choice: propose off-chain, verify on-chain.** Anyone may compute
a proposal with the exact solver (`solve`):

1. cancel negative cycles in the book;
2. successive shortest paths for the taker;
3. potentials from shortest distances;
4. a per-arc max-flow pass in rank order inside the optimal face.

The program then rebuilds the **complete** arc list from its own account (every
live order and ladder step; nothing comes from the proposal) and checks:

- **shape and balance:** fills within lots; coverage equal at every node;
- **price:** complementary slackness against node potentials supplied with the
  proposal. By LP duality no cheaper package exists, and since the potentials
  must price *every* arc consistently, an omitted cheaper or older order makes
  the check fail;
- **quantity:** if the taker is short of its quantity, no zero-reduced-cost
  residual path closes another lot;
- **FIFO:** for every arc with lots left and zero reduced cost, no
  zero-reduced-cost residual path through strictly newer arcs closes a cycle
  through it. Every optimal package differs from this one by zero-cost cycles
  (conformal decomposition), so this is exactly "no cost-neutral exchange fills
  an older order more". Two cheap filters skip the search. A
  strongly-connected-components pass skips every arc not on a zero-cost
  cycle. A cut pass skips every arc for which some grid cut between its ends
  has no newer zero-cost edge crossing back: the way back from head to tail
  must cross every such cut. The cut pass settles a ladder that a package
  fills partly, step after step (each step's only way back is its own
  reverse edge). Without it, a wide range across a long ladder searched once
  per step, and the work grew with the square of the range's width. What
  remains to search is exact price ties.

Rules 2 and 3 hold for any valid dual: every optimal primal satisfies
complementary slackness with every optimal dual. The brute-force tests
enumerate every feasible package of random small books (about 50,000 books,
plus 2,000 all-ties books). The solver returns exactly the owed package, and
the verifier accepts it and rejects every other package, under both the
solver's potentials and the package's own best potentials.

The resting book never crosses: `Submit` matches before it rests a remainder
(GTC), post-only fails if anything would fill, and `Quote` is post-only
against the whole book. Every order's entry is therefore checked against the
whole book, so the book never holds a zero-or-negative-cost cycle, and a
package never contains trades unrelated to its taker. Self-matching is allowed;
it nets out (below).

**Freshness and double use.** A proposal is checked against the book as it is
when the transaction executes. A stale proposal either still proves optimality
or fails, and the client recomputes (`c3`). All state is one account, so two
packages are serialized and the second sees the first's consumption.

## Capacity (finite dev capacity, measured)

Full search covers the whole admitted market; the admitted market is bounded:

| Limit | Dev value | Why |
|---|---|---|
| grid | 0..200 points | index around 100 |
| maker ladders | 4 per market, ≤ 256 steps wide (the whole dev grid); 1,024 ladder arcs (steps × tiers × sides with lots) per book | verification heap and compute |
| resting orders | 64 per market | same |
| traders | 128 per market (idle slots are reused) | account size |
| claims | 1,024 per market | account size |
| FIFO search work | 30,000 edges per package | only exact price ties search; beyond it `SearchLimit` |

**Measured worst supported cases.** Two worst cases were measured:

- `r14`: four ladders (bids and asks, 1,024 arcs) and all 64 order slots
  resting, then a wide taker filling 150 lots across all of them. `Submit`
  costs **1,052,934 CU**. Its proposal is 307 bytes and the whole transaction
  735 bytes (limit 1,232).
- `wide`: the dev treasury's whole-grid ladder (912 arcs), a second
  maker in the rest of the 1,024 ladder arcs and all 64 order slots, then a
  taker over the whole grid filling 6,000 lots through every ladder. As the
  API sends it (compute limit, heap frame, deposit, submit, withdraw, gate
  bound), it costs **1,114,869 CU** in a 1,220-byte transaction.

Against the treasury ladder alone:

- a 1x position costs about 0.81M CU to open and 0.76M to close (1,104
  bytes);
- a whole-grid range costs 0.94M;
- the treasury's `Quote` costs 0.72M.

Exceeding a capacity fails with an explicit error (`OrderCapacity`,
`ClaimCapacity`, `MakerCapacity`, `TraderCapacity`, `SearchLimit`,
`InvalidPrice` for a too-wide ladder or a core outside it) and changes nothing
(`r5`). `Submit` and `Quote` need a 256 KiB heap
frame (`RequestHeapFrame`). The program's allocator grows upward, so every
other instruction runs in the default heap.

## Cash, claims and exits

One market account holds every trader's `free` (withdrawable) and `reserved`
(escrow) cash, every claim, and the market's `backing`. After every
instruction the program recomputes:

- custody (deposits − withdrawals) = free + reserved + backing;
- reserves = exactly the escrow of resting orders and ladders;
- while open, backing = `k` × long coverage = `k` × short coverage: one
  complete set behind every promised payoff;
- once final, backing = what the remaining claims pay.

The tests also recheck, after every transaction, coverage equality at every
step, the vault balance, and that every settlement value pays out exactly the
backing.

- **Opening:** a long pays its premium. A short funds the rest of the set,
  `(hi - lo) * k - premium`. A resting order escrows exactly what it would put
  in at its own price. A ladder escrows `bid` per bid lot and `k - ask` per ask
  lot.
- **Exact-complement redemption:** whenever an owner holds a long and a short
  claim on the *same absolute interval*, the matched lots are redeemed for
  their backing into free cash. This happens at fill time for the owner's own
  exact complements, or explicitly with `Redeem`. Shifted or different-width
  intervals are not complements and release nothing (`c6`). `Split` (exact:
  `R[a,c) = R[a,b) + R[b,c)`) followed by `Redeem` releases precisely an exact
  overlap. A ladder's inventory is per step, so its one-step claims net per
  step.
- **Sell an existing claim:** submit the opposite side on the same interval
  naming the claim. If it rests, it reserves the claim, not cash. When filled,
  the new opposite claim and the old one redeem, and the seller's free cash
  rises by exactly the sale premium, withdrawable at once. The buyer's claim is
  backed by the same set as before (`r7`).
- **Buy the complement:** the same thing without naming the claim: any exact
  complement nets (`r8`). The original premium is never refunded a second time;
  the only cash released is the redeemed set.
- **Cancel:** releases exactly the remaining escrow, or the claim reservation
  (`c7`).
- **Partial exit or fill:** shape-preserving lots. The rest keeps its payoff
  and backing (`r11`).
- **Rounding:** settlement is measured in fine units of `1/k` step, so every
  payout per lot is an integer and a long plus its short pays exactly the set.
  A split divides the cost basis (informational) exactly (`c8`).
- **Fees and funding:** none (`r12`: every dollar deposited is withdrawn).

## The maker (treasury) account

The residual maker is an ordinary trader with a ladder. Admission is
balance-based: a quote must be fully escrowed from the maker's free cash now,
so it can never borrow customer backing or count on a future offset (`c1`,
`c2`). Realized losses shrink what it can quote next (`r10`). Recycling: a
piece it sold and later buys back nets per step and releases the set (`b1`
+$0.10, `b2` −$0.50: the handoff's section 9 numbers). Unoffset inventory stays
funded through settlement at any outcome (`b3`). When the maker is exhausted
or absent, peer-to-peer packages still trade (`c1`).

**Ladder shape.** `Quote` takes a core `[core_lo, core_hi)` inside the
ladder. Core steps quote every depth tier. The ladder's other steps quote only
the last (widest) tier's lots: tiers trade best first, so a step's remaining
lots always sit in its last tiers, and such a step is one thinner, wider quote
per side. Fills, escrow and verification need nothing new. On dev the
treasury quotes one ladder over the whole grid. Its core is ±32 points around
the index with 5 tiers, and every other step quotes the last tier. Every
piece of every range therefore has a maker buyer and seller: any leverage
down to 1x opens and closes (`wide`). Inventory stays recyclable at
every step, because no step leaves the ladder when the core moves with the
index.

On dev the maker runs as `futures-maker` from a dedicated treasury key,
which operates the Earn vault (below): its capital is the vault's cash,
depositors' and the treasury's own (the dev liquidity manager's USDC, moved
once into the treasury at bootstrap, then into the pools and from them into
the vault), never minted for it. It reports committed cash, backing and
worst-case exposure; see `ops/README.md`.

## The Earn vault

One Earn vault for the whole product (2026-10-16; it replaced the
per-market Earn pools of 2026-10-15, below). Depositors put in USDC and get
shares of the whole vault, a stake of all of Earn; the treasury maker trades
the vault's cash in every market.

**Accounts.** The vault is a PDA `["ameba-range-v1", "earn", operator]`
(`AFRVALT1`, 608 bytes: operator, quote mint, its cash account, idle `cash`,
`shares`, cumulative deposited and withdrawn, and its markets, at most
`VAULT_MARKETS`). Its idle cash sits in `cash_account`, a USDC token account
owned by the vault's address (dev: its associated account); the vault counts
`cash` itself, so tokens sent to that account are a gift nobody counts. In
each market where it trades, an ordinary trader slot whose owner is the
vault's address holds its cash and inventory, so the market's custody,
backing and audit cover it unchanged. Each depositor has a position PDA
`["ameba-range-v1", "earn-lp", vault, owner]` (`AFRVLPS1`: shares,
cumulative deposited and withdrawn), created on the first deposit.
`CreateVault` is permissionless: whoever signs as operator runs that vault.
The product's vault is the one the manifest's `futures.treasury` operates
(`futures-ops vault` creates it).

**Operator.** The operator moves the vault's cash between its idle account
and any market (`VaultFund` adds the market to the vault's list,
`VaultDefund` takes free cash back and drops a market where the vault then
holds nothing), and trades for it: `Submit`, `Cancel`, `Quote`,
`CancelQuote`, `Split`, `Merge` and `Redeem` accept the vault account as
their owner (not a signer) when the operator signs as a later account
(client `operated`) and the market is one of the vault's. It can never take
the cash: `Withdraw` and `Transfer` need the owner's signature, which a
program address never gives, `VaultDefund` pays only into the vault's own
cash account, and the vault's cash leaves only to depositors
(`VaultWithdraw`). Nobody can give the vault a claim: `Transfer` goes only to
wallet keys (on the ed25519 curve). Depositors trust the operator's quoting,
as with any managed maker.

**Value.** The vault's value over every settlement outcome is its idle cash
plus, in each of its markets, its trader's free and reserved cash and what
its claims and ladder inventory pay there (`state::trader_value`, the
client's `Book::value_of`). The markets settle independently, so the
lowest and highest values are the sums of each market's. `VaultDeposit` and
`VaultWithdraw` value the vault in every one of its markets, so they name
them all, in the vault's order, and the program checks the list.

**Shares.** `VaultDeposit` mints `amount × shares / high` (one share per
atom for the first deposit). `VaultWithdraw` pays `shares × low /
shares_total`, at once, from the vault's idle cash and then from its free
cash in the markets the withdrawal names after the list (pairs of the
market's token vault and authority); where a named market's free cash is
short, it first retires the vault's ladder there to release the escrow
(depositors come before the maker's quotes; the maker quotes again on its
next pass). Entering at the highest value and leaving at the lowest means no
deposit or withdrawal ever moves value from the other depositors, without
trusting a mark or the oracle. The cost is the gap, the open pieces' range
of outcomes, which closes at settlement: a round trip with open inventory
loses that gap's share to the remaining depositors. The last shares can't
leave while the vault holds more than they would be paid (`NotSettled`):
nobody would own the rest, and the next deposit would get it for nothing.

**Settlement rolls forward.** `Settle`/`SettleMaker` pay the vault's claims
and inventory into its free cash in that market like anyone's; the maker
then moves that cash back to idle (`VaultDefund`) and on into the open
markets (`VaultFund`), and the drained market leaves the vault's list.

**How many markets.** `VAULT_MARKETS` = 13: a withdrawal over 13 markets,
drawing on two of them, with the owner's token account created and every
instruction bound to the governance gate, fills a legacy transaction's
1,232 bytes (1,228; `v5` measures it and the API's own test checks the
bytes; the API sends legacy transactions). On the dev capacities (1,024
claim slots a market) that deposit costs about 435k CU and that withdrawal
about 432k, inside one transaction's 1.4M. Funding a 14th market is refused
(`VaultCapacity`), because deposits and withdrawals could then not name
every market: a lost-funds protection, not a product limit. The dev chain
lists three months of two products, six open markets plus the settling ones
until the maker drains them. An address lookup table would raise the limit
to what one transaction can lock and compute (the valuation scans each
market's claims; `v5` reports the compute).

**Maker.** `futures-maker` operates the vault (`ops/README.md`): every pass
each open, tradable market gets an equal share of the vault's cash (or
`MAKER_BUDGET_USDC`), funded from idle cash; cash above a market's share
goes back to idle as its ladder frees it; a settled or expired market's cash
rolls back to idle. Deposits arrive as idle cash and are funded out on the
next pass. Its report gains `vault`, with `deposited` = the vault's net
funding of that market.

**Events.** `ARVCRT01` [vault, operator, cash account], `ARVDEP01` [vault,
owner, amount, shares], `ARVWDR01` [vault, owner, shares, amount],
`ARVFND01` / `ARVDFD01` [vault, market, amount], `ARVMIG01` [vault, pool,
holder, value, shares]. `VaultDeposit`'s and `VaultWithdraw`'s account 0 is
the vault (not a market); the API's indexer follows it like a market.

**Tests.** `integration/tests/vault.rs`: `v1` (shares of several markets
together, entry at the high value, exit at the low, settlement), `v2` (the
operator funds, defunds and moves cash between markets; settled cash rolls
forward; the market list), `v3` (the operator can't take the funds; strangers
can't move or quote; no market outside the list), `v4` (last shares, no
claim given to the vault, gifts not counted, the retired pool instructions
refused), `v5` (13 markets fit a transaction; the 14th refused; compute),
`v6` (a legacy pool migrates; a shared one exits through `PoolWithdraw`),
`v7` (a withdrawal releases the ladder's escrow), `v8` (the production
build has no legacy pool path).

### Legacy per-market pools (dev only)

From 2026-10-15 to 2026-10-16 each market had an Earn pool: a PDA
`["ameba-range-v1", "pool", market, operator]` with its own shares and a
trader slot the treasury operated. `CreatePool` and `PoolDeposit` are
retired (always `InvalidInstruction`; their tags stay so later variants keep
theirs). The dev chain's build (`localnet-governance`, which implies the
`legacy-pools` feature) keeps the exit: `PoolWithdraw` at the pool's lowest
value from its free cash (the last shares wait for settlement), the
operator's `Cancel`/`CancelQuote` for a pool, and `PoolMigrate`, which moves
a pool one holder owns entirely into the vault (its trader slot becomes the
vault's; the holder gets vault shares for its lowest value at the vault's
highest; the pool's and vault's operator signs too). The production build
has none of them (`v8`); `scripts/build-sbf.sh` builds the dev variant into
`target/deploy-legacy` for the tests (`v6`), never released. `futures-ops vault-migrate` moved the dev chain's pools
(the treasury's and the dev Earn seed's) into the vault.

## Earn rewards (AMBA)

On 2026-10-16 the user asked for an AMBA liquidity incentive for Earn
depositors: 100,000 AMBA distributed over a month, from the 1st to the end
of the month, following the futures expiry schedule, the user funding it
themselves on mainnet. AMBA is a Token-2022 mint
(`BxKJN4PKAT2CFhVbsyKzVyCznu5a2n2oVh8Pb2wdDjam`, nine decimals) with a 0.25%
transfer fee (uncapped), so every transfer in or out loses 0.25% of what
arrives.

**The stream.** The vault carries at most one stream at a time
(`state::Rewards`, `AFRVRWD1`): `amount` atoms of the reward mint released
linearly from `start_ts` to `end_ts` to depositors by their shares. The
operator sets it with `VaultRewards` (tag 25). The first stream grows the
vault account by `REWARDS_LEN` (192 bytes, from 608 to 800; the operator's
payer funds the rent) and fixes the mint, the reward account (a token
account of the mint, of either token program, owned by the vault's address:
the vault's associated AMBA account) and the **house**, the one owner whose
shares earn nothing: the protocol's own deposit (the `treasury` role by
default), so the whole budget goes to outside depositors. A later
`VaultRewards` accrues the current stream up to now and replaces it: what
the old one released stays accrued, the rest of it and any part of the new
one before now are never released. A month's stream runs from its 1st,
00:00 UTC, to the next month's 1st: the expiry of that month's futures
(`futures-ops vault-rewards --month YYYY-MM`). The program never checks the
reward account's balance against the stream: the operator funds it with
plain transfers, before or after setting it.

**Accrual.** A share's part accumulates in `acc` (atoms per share, times
`REWARD_SCALE` = 10^12), which only grows: on every `VaultDeposit`,
`VaultWithdraw` and `PoolMigrate`, before the shares change, the stream is
accrued up to now (`released_at(now) - released_at(last_ts)`, over the
vault's shares less the house's; nothing goes to anyone while only the house
holds shares), and the depositor's position settles its part:
`pending += shares × (acc − snapshot) / REWARD_SCALE`, `snapshot = acc`. The
position keeps these in its spare bytes (`reward_snapshot` u128 at 104,
`reward_pending` at 120, and `reward_paid`, informational, in the seven
bytes after the bump, stopping at 2^56 − 1), so positions created before
rewards read as zero and nothing else moved. The deposit and withdrawal
instructions take no new account, so the 13-market transaction size of "The
Earn vault" is unchanged. Rounding is down everywhere: the sum of every
position's part never exceeds what the stream released (`distributed`).

**Payment.** `VaultRewardsClaim` (tag 26) is permissionless: it accrues,
settles the position and pays its `pending` from the reward account into the
position owner's own token account of the mint (a checked transfer, signed
by the vault; the transfer fee comes out of what arrives). Leaving Earn keeps
what was earned; it is paid all the same. The keeper pays every position
once the stream has ended (`rewards::pay_all`, once per ended stream: an
idempotent associated-account create and a claim per depositor, the keeper
paying the fees and rent), and `futures-ops vault-rewards-pay` does the same at any
time. `VaultRewardsWithdraw` (tag 27) lets the operator take back only the
surplus: the reward account's balance less what depositors are owed
(`distributed − paid`) and what the stream still releases.

**Events.** `ARVRWD01` [vault, mint, start, end, amount], `ARVRCL01` [vault,
owner, amount paid], `ARVRWX01` [vault, destination, amount].

**API and web.** `GET /v1/earn`'s vault carries `rewards` and an owner's
position `rewards: { pending, paid, earned }`, accrued to the chain's time as
the program would (`client::rewards_at`). The Earn page shows the wallet's
earned AMBA under Your Earnings ("+12.34 AMBA") and the month's stream in How
It Earns.

**Tests.** `integration/tests/vault.rs`: `v9` (a Token-2022 mint with
mainnet's fee; the house left out; shares split the stream by the second; a
claim only into the owner's account; the operator's surplus and nothing
more; leaving Earn keeps what was earned; the grown vault still trades),
`v10` (a second stream after the first; mint, account and house fixed; the
house's later moves).

## Trading window

A market can be created on its oracle period while the period is still
Forming, so the coming months can be listed ahead. It takes trades only once
the oracle lists the period: `Submit` and `Quote`, the only instructions that
open exposure, need an open market whose period is Live, from its effective
listing until expiry (`Period::is_tradable_at`), and refuse anything else
with `NotTradable`. Source: the user, 2026-10-15, preflight Decisions 5
(`ship/releases/2026-10-15-preflight.md`): a month pays out only on a real
price. The oracle fails a period only while it is Forming (tag 79), never
once it is Live, so a market that traded can't end on a Failed period; a Live
month the oracle never finishes goes to the council (below).

Deposits, withdrawals, cancels and Earn pool deposits and withdrawals work in
any phase. Claims come only from fills, so while the period is Forming there
is no position to close and no order or ladder to cancel; once it is Live a
position closes like any other order (a `Submit` naming the claim). Tests:
`l1` (orders in every mode and ladders refused while Forming; the same orders
trade, and a position closes, once Live) and `l2` (a period failed while
Forming: nothing traded, every deposit back) in
`integration/tests/market.rs`.

The clients follow the same rule. The treasury maker and the activity bot
quote and trade only where `ops/src/market.rs` `tradable` holds and pass over
other markets quietly. The API's market view has `tradable` and
`periodState.plannedListingTs`, the earliest the market can open
(`api/API.md`). On dev, `list-next` lists each month's period Live at once
(dev tag 240), so dev markets trade from their listing.

## Settlement

`Finalize` reads the oracle `Period` the market was created against: same
index, expiry, oracle program and value decimals (`CreateMarket` refuses
decimals other than the period's); this is the immutable settlement
identity. A Settled period at or after expiry fixes
`S = floor(value * k / step_value)` fine units. Anyone may finalize.

**When the oracle can't settle.** A Failed period (whose market never
traded: see "Trading window"), one neither settled nor
failed by expiry, or one settled
in decimals other than the market's (only a market listed before
`CreateMarket` checked them), doesn't finalize. The council settles it instead: `CouncilSettle { value }` fixes
`S` from `value` (oracle units at the market's decimals) and sets the status
to Failed. A governed program needs three council seats, as for
`CreateMarket`; an ungoverned one needs its upgrade authority's signature;
an immutable one has no one to sign, so its failed markets stay open. The
council may act any time after expiry, with no further wait (the user,
2026-10-16); it can't act before expiry on a period that hasn't failed
(`NotExpired`), or once the oracle has settled it (`InvalidInstruction`),
and acts once (`AlreadyFinal`). Every value pays out exactly the backing, so any value is
solvent. On dev: `futures-ops council-settle MARKET VALUE` with
`AMEBA_GOVERNANCE_SETUP`. The keeper leaves such a market alone until then.
The reference value (`ref_value`) is only the listing reference; it never
settles a market. (It did until 2026-10-15, as a dev placeholder.)

`Settle`, `SettleMaker` and `Prune` then pay claims and ladders into free
cash and release dead orders. All are permissionless.

## Dev deployment (2026-10-14)

On the loopback dev chain (genesis `A5ZLaudx…`), deployed from main by what
is now the markets step of `futures-ops bootstrap`. Since 2026-10-15 the program is governed (see
"Governance on dev" below): its upgrade authority is the council's target
PDA, and new code goes in through `futures-governance upgrade`. Account
layout version: header `AFRANGE1` (the deployed program's on-chain
identity, which keeps "range"), version 1. A layout change means new
markets, not an in-place migration.

| What | Address |
|---|---|
| Program `ameba-futures` (built as `ameba-futures-range` then) | `4fAonimh7DxpK7MbEnJgDYMikakrCK3FUiTYfEPuo7GS` (governed build of `0a266d35`, sha256 `8bb0c808…`; upgrades below) |
| RAMX Oct 26 | `Fi1t8X27PUPewq5Lo2YnpajNqykTmJQmzkH5Xr1gMvFf` |
| NANDX Oct 26 | `AZFr8mf6o7ZPCwV76n8iFPmFKdaUfuFgKciMtMFVZhsp` |
| RAMX Nov 26 | `EYY3Wat6fQpjQutVJpCSQ5WGBsgJcTq7JyhKZF97CMDY` |
| NANDX Nov 26 | `FJD9vzxjzKEjGWQ2N3gxM8E1R7LN8i2A75Uyy7DXNbNF` |
| Treasury (maker) | `2doHNHthdMce4CToUP93xmbMisdKi5Ji1mEfCbhvougt` |

Each market is bound to its series' oracle `Period`, with reference value
100. Later months are listed by `futures-ops list-next` (`ops/README.md`):
each due month (the first of each of the next three months) gets an oracle
period and month and a futures market on it; the keeper settles every series
alike. The treasury holds the dev liquidity
manager's 900,000 USDC (moved once; transaction `3tnmYbtG…`) and the maker
gave each market an equal $225,000 budget.

Running services:
- `ameba-futures-maker`: the treasury maker;
- `ameba-futures-activity`: demo trading on the futures markets;
- `ameba-futures-keeper`: market settlement after expiry;
- `ameba-futures-api`: the futures endpoints (`/v1/markets…`, `/v1/quote`,
  `/v1/accounts/{owner}`);
- the site's trade page: the Capped Futures ticket, depth book, positions and
  exits.

**Evidence: what ran where.**

| Behaviour | Real dev-chain transactions | LiteSVM (real SBF programs, incl. the oracle) | Unit / brute force |
|---|---|---|---|
| Exact pair at different leverages ($100 vs $1,000) | `futures-e2e` receipt `docs/results/localnet-range-1791976375.json` | `a1` (settled at six outcomes) | |
| Package from three adjacent sellers | same receipt | `a2` | brute force |
| Missing piece and overlap in one package | | `a3` | brute force |
| Treasury ladder fills the gap between two sellers | `localnet-range-1791977137.json` (2 orders + 2 ladder steps) | `b1`–`b3` | |
| Maker recycling at +$0.10 / −$0.50, unoffset settlement | live: the maker's per-step netting in production | `b1`, `b2`, `b3` | |
| Sell a claim, withdraw proceeds at once | receipt (sale $250, withdrew $2,150) | `r7` | |
| Buy the complement and redeem | receipt (released $1,050) | `r8` | |
| Cancel; shifted redemption rejected | receipt (`NotComplement`) | `c6`, `c7` | |
| FIFO across direct and composite routes, maker priority | | `r1`, `r2`, `r6` | brute force (every feasible package) |
| Full search, non-greedy, capacity errors, worst-case CU | | `r4`, `r5`, `r14` | brute force |
| Budget exhaustion, one-sided flow, realized loss | | `c1`, `c2`, `r10` | |
| Settlement on the oracle path | not yet: the first dev expiry is 2026-11-01 | every `settle_all` (dev tag 242) | |
| Low leverage: 1x and 2x longs and shorts open and close | API path on the governed programs, 2026-10-15: eight trades of 20 lots on RAMX Oct 26 (opens and closes, 1,086–1,153 bytes); a 1x round trip on NANDX Oct 26 cost $0.66 (long, $20.45 position, mostly the deep in-the-money pieces' outer spread) and $0.10 (short) | `wide` (also whole-grid ranges and the heaviest admitted book) | |
| UI: ticket → fill → position → Exit → withdraw | browser run with an injected test wallet, 2026-10-14 (open 914 lots in 1.3 s, exit in 0.5 s) | | web node tests |

Demo traffic (the activity bot's takers and rester plus the treasury ladder)
trades continuously on all four markets. The API's trade tape and candles show
it.

**Operating it.** Open the trade page on dev.amoeba.farm and pick RAMX or NANDX
and an expiry. In the ticket choose Long or Short, a leverage (it sets the
loss-side end), the profit-side end ("Up to" / "Down to", defaulting to the
dev-fixture 2σ range) and an amount. The `_/‾` shape shows the range; Entry,
Per point and You pay come from the live quote. The book beside it is the
executable depth for exactly that range. Positions show under Portfolio with
Exit (sell at the best executable price, proceeds withdrawn). Resting orders
show under Orders with Cancel and Withdraw. From a shell: `futures-e2e`
replays the scripted flows; the maker's state is in
`.localnet/maker-status.json`.

## Not done here

- No audit or mainnet readiness. One shared writable account per market
  serializes all trading in that market.
- Settlement of the dev markets happens after their expiry (Oct 26 series:
  2026-11-01); until then it is exercised in LiteSVM only.
- The trade page's market price line is the trades whose range contains the
  index (`API.md`, candles). A book-implied forward (the ladder's mids
  summed) would be a better mark; it isn't built.
- Claim slots are scanned per fill. A book whose 1,024 claim slots are nearly
  full and a taker filling dozens of resting orders at once would cost more
  compute than measured in `r14`. Past the limit it fails atomically, but a
  per-trader claim index would remove the scan.
- Beyond the dev capacities, scaling needs a different layout: per-trader
  accounts and route-local verification.


## Governance on dev (2026-10-15)

The futures program verifies the Spread V3 controller gate (the oracle stays
on its `localnet-dev` profile). Until 2026-10-15 the legacy clearing program
and its matcher were governed targets too; they were removed that day, their
gates frozen through the council and their code deleted from the repository. Each instruction carries the
canonical read-only gate account and a signed gate epoch. A frozen gate or a
stale epoch fails closed. Clients (API, ops, keeper, maker, activity) bind the
current epochs before signing, because the manifest names
`governanceController`.

| What | Value |
|---|---|
| Controller (`governance_controller_v3`, `localnet-governance` build) | `9CNKzWmKXKs9vkE3uXs63QZi3AxN3B2hdZCqkN8xf3om`, sha256 `8b7b440d…` |
| Council | five seats, three approve; review 750 slots, delay 150 |
| Targets | futures `4fAonimh…` (active); the removed clearing `G4HiTXC7…` and matcher `35B2xLFH…` (frozen) |
| Governed build (`c692f787`) | futures `153ae543…` |
| Upgrades since | `e451c8af` `96e72283…` (Earn pools); `8942f2c9` `f53ff868…` (creation takes the period's decimals; a pool's last shares wait for open value); `0a266d35` `8bb0c808…` (`CouncilSettle`, no reference-value payout; no claim given to a pool), all 2026-10-15; the Earn vault (2026-10-16, below) |
| ProgramData capacity | futures 768,000 bytes (extended before registration; only the upgrade authority can extend on this validator) |
| Keys and setup | `.localnet/governance-setup.json`, `.localnet/keys/governance/` (ignored; never in worktrees) |
| Rehearsal target | `DkZ67swD3W7eqsHjtwCdXqG8516ybrVSLd2voVd7Q3zw`: a throwaway copy of the futures program, registered and upgraded through the council before the live programs; not used by anything |

The cutover order, which kept the old code serving until the moment the new
code went live:

1. Deploy and initialize the controller.
2. Extend each target.
3. Register each target. Its gate starts frozen, and the old binaries ignore
   it.
4. `futures-governance upgrade` all three. It writes and verifies the
   buffers, then runs three approvals per proposal. After the delay it
   executes the installs and then the activations back to back.
5. Add `governanceController` to the manifest and restart the services on
   the same commit's binaries.

Later releases use the same `upgrade` command (`ops/README.md`, "Council
governed rollout"). It runs in about half a minute on dev.
