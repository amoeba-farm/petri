# Ameba futures (dev build)

The implementation of the founder's range-composition handoff (Revision 2,
14 October 2026): one range-aware market per index and expiry, packages
composed from actual orders, an optional funded maker that trades missing and
surplus pieces, fully funded accounting, no protocol fees and no funding.
Since 2026-10-17 the Earn vault's maker may also borrow what staked AMBA
backs (below, "Staked AMBA backs the maker"): the one exception to full
funding, the user's call. Dev chain only; not audited.

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
| claims | 1,024 per market (a resting opening order holds one; "Claim slots") | account size |
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

- custody (deposits − withdrawals) + debt = free + reserved + backing,
  where `debt` is what the market's credit owner (the Earn vault, when its
  stake pool lent it credit) borrowed; 0 everywhere else;
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
  lot, except the lots its inventory backs (below, "The maker").
- **Overlap redemption (2026-10-17):** a fresh claim nets against its owner's
  opposite claims wherever their intervals overlap (`state::plan_net`):
  exact complements first, then the others in slot order. Both claims give
  up the matched lots; the parts outside the overlap stay as claims of those
  lots (`Split`: `R[a,c) = R[a,b) + R[b,c)`, the cost basis split by
  width), and the overlap's sets are redeemed into free cash at once. A long
  90-110 and a short 100-120 become a long 90-100 and a short 110-120 plus
  the 100-110 sets in cash; the owner's payoff is the same at every
  settlement (`c6`, `c9`). It happens at fill time (taker, resting order,
  transfer). Reserved lots (a resting close order's) stay out. It is best
  effort: a match whose parts find no free claim slot, and every match after
  it, is left as it was, and the fill trades all the same (`c10`).
  `Redeem` still redeems an exact pair explicitly. A ladder's inventory is
  per step, so its one-step claims net per step.
- **Claim slots (2026-10-18):** a fresh claim takes a slot last. It first
  redeems against the claim its close names (or its close order reserves)
  and its owner's other exact complements, without a slot of its own. What
  is left joins its owner's claim of the same side and range (as `Merge`
  joins: lots and cost basis add up, the earliest open time stays), or else
  takes a free slot, and then nets the overlaps as above. A resting opening
  order holds one slot from the moment it rests: its owner's claim of its
  side and range, or an empty placeholder. Every fill joins it, so a fill
  never waits for a free slot, and the slot is free again when the order
  stops resting. `Split` and an adjacent `Merge` of a held claim move the
  claim out to free slots; the order keeps its slot. An order placed before
  this change holds nothing until its first fill takes a slot and holds it
  (`k5`). So on a full table every exit trades: a close against ladders,
  resting orders and close orders, a close order's fills, buying the
  complement, and `Redeem`, `Merge`, `Cancel`, `Withdraw` and settlement
  (`k1`, `k2`). What waits for a free slot: a new position that joins no
  claim its owner holds, a new resting order (`ClaimCapacity`), `Split`, a
  partial `Transfer` to a wallet without that claim, and overlap parts
  (best effort). Repeat fills of one range stay one claim (`k3`, `k4`).
  A full table still keeps new positions out until claims close or the
  market settles, and filling one is cheap: on the mainnet shape, 1,024
  one-lot, one-step claims at the treasury's far quotes cost about $0.20
  plus about 0.005 SOL of fees (measured on dev, 2026-10-18). A cost per
  slot or per-trader position accounts would close that; neither exists
  (the user's call).
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

**Inventory backs its quotes (2026-10-17).** A step's first ask lots to fill
(best tier first), as many as it holds long, sell that inventory: a fill
nets it, the redeemed set pays the ask's `k - ask` share, and the maker keeps
the premium, as a close order pays from the claim it reserves. So those lots
need no escrow; likewise its first bid lots, as many as it holds short
(`Maker::step_escrow`). `Quote` reserves that, summed over its steps. A fill
pays its escrowed lots' share from the escrow and its inventory-backed lots'
from the sets they redeem; what else it frees (inventory that now backs the
step's other side) stays reserved until the next quote, so the escrow never
falls short of what the steps need, and a fill costs no more compute than
before (the grid-wide sweep of `wide` still fits). Admission stays
balance-based: every lot is either escrowed or backed by inventory the maker
owns. More inventory, more depth for
the same cash (`b4`, `b5`). `futures-maker` sizes its ladder by this exact
escrow (`Book::ladder_escrow`), not a full step per lot.

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
what was earned; it is paid all the same. Depositors claim whenever they
like: the AMBA streams in by the second and nothing waits for the end of a
month (the user, 2026-10-20: "I don't want it to pay at the end of every
month. I want it to like stream the tokens in, like show their earnings so
they can cash out early"). Unclaimed AMBA stays theirs across streams: a new
stream only adds to `acc`. Until 2026-10-20 the keeper paid every position
once a stream had ended; it no longer does. `futures-ops vault-rewards-pay`
pays everyone on demand (`rewards::pay_all`: an idempotent
associated-account create and a claim per depositor, the payer paying the
fees and rent). `VaultRewardsWithdraw` (tag 27) lets the operator take back only the
surplus: the reward account's balance less what depositors are owed
(`distributed − paid`) and what the stream still releases.

**The next month.** On 2026-10-19 the user chose one stream per calendar
month, 100,000 AMBA each (October from then to Nov 1, then Nov 1 to Dec 1,
Dec 1 to Jan 1), funded by the user ahead of time. The program holds one
stream at a time and needs no change for this: the maker, which holds the
operator's key, rolls the stream over (`futures-maker`,
`MAKER_EARN_ROLLOVER=on`, `MAKER_EARN_ROLLOVER_AMBA`; off by default). Once
the stream has ended, and only while the reward account holds what
depositors are still owed (everything they haven't claimed, which the
account already holds from the months before) plus the month's amount, it
sends `VaultRewards` with the same mint, account and house, from
that moment by the chain's clock to the next month's 1st. Starting at the
reset rather than the 1st matters: a reset accrues the old stream and
releases nothing of the new one from before itself, so a stream back-dated
to the 1st would leave the part between the 1st and the reset to nobody. A
month funded late still runs to its 1st at the full amount. Nothing starts
unfunded: until the AMBA is there the maker logs once what it waits for and
how much to send (the transfer fee added). The seconds between the
maker's clock read and its transaction landing release nothing to anyone;
that sliver (about 0.04 AMBA a second at 100,000 a month) stays in the
reward account and counts toward the next month's funding.

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

## Staked AMBA backs the maker (2026-10-17)

The user's notes "AMBA as Backing for the Market Maker" (2026-10-16/17) and
their decisions of 2026-10-17, built on dev first. With Earn's USDC fully
deployed, the maker can quote no more. Stakers' pooled AMBA now backs the
part of the maker's obligations that its USDC doesn't, for as long as it
takes to offset them: the maker borrows inside one market's custody (the
traders' own collateral) and its stakers carry what that could cost. Once the
maker offsets a piece, its netting repays the debt. AMBA is never sold; it
backs credit, and in a tail loss it is paid to that market's winners. An
independent review of 2026-10-18 found nine holes, and its re-review two
more; the rules below include their fixes (tests `f1` to `f7`).

**Accounts.** One stake pool per Earn vault, a PDA `["ameba-range-v1",
"stake", vault]` (`AFRSTAK1`, 800 bytes, `state::StakePool`). The vault's
operator creates it (`CreateStakePool`, tag 28) over two token accounts the
pool's address owns: its AMBA (Token-2022) and its USDC. The program's
upgrade authority signs it as the payer; once the program is governed,
three council seats sign instead, as for `CreateMarket`. An immutable
program has no pools. A pool lends credit in any market, so nobody can make
themselves a credit owner against a token they minted (`f1`). The pool
counts both balances itself, so gifts are not counted. Each staker has a
position `["ameba-range-v1", "stake-pos", pool, owner]` (`AFRSTKP1`, 208
bytes). The market header's spare bytes (376..512 and 28..32) carry the
market's credit: `credit_owner` (the vault), `credit_limit`, `debt`,
`credit_until`, the vault's funding in and out, the time-weighted credit and
funding, and after a shortfall `shortfall`, `recovery_amba`, `winners`. A
market from before reads all zero: no credit.

**Price.** The operator posts the AMBA price (`StakePrice`, tag 30: USDC
atoms per whole AMBA times `PRICE_SCALE` = 10^6). The program uses a
time-weighted average of the posts. After a post the average moves linearly
from where it stood toward the posted price and reaches it `twap_window`
seconds later (dev: an hour). Source: the user, 2026-10-17, "operator posts,
TWAP". On dev the treasury maker posts DexScreener's price of the Meteora
AMBA/SOL pair every five minutes (`ops::price_feed`; the user, 2026-10-18).
The price is trusted on dev.

A price counts only while it is fresh (the user, 2026-10-18, after Codex's
review of `PoolPrice`). One posted more than `MAX_PRICE_AGE` = 3,600 s ago
(an hour, the averaging window) gives the pool no capacity: no new
`CreditFund`, and no `Unstake` while credit is lent, since what stays must
keep the capacity at or above the credit lent. Credit already lent stays,
as when the price falls. A shortfall's `CreditSettle` refuses
(`NotSettled`) until a fresh price is posted, so it never sets AMBA aside
at an old average. Dev's operator post every five minutes keeps the price
fresh; on mainnet anyone may crank `PoolPrice` (below). In the production
build (no `operator-price`) a new pool also lends nothing until its prices
have run `MAX_PRICE_AGE` from its first post (`first_price_ts`; zero in an
older pool, read as long ago), so no first sample lends alone.

On mainnet there is no trusted poster (the user, 2026-10-18): the
production build refuses `StakePrice` (`Unauthorized`; only dev builds,
feature `operator-price` in `localnet-governance`, keep it), and anyone may
crank `PoolPrice` (tag 40, `programs/futures/src/price_source.rs`), which
reads two mainnet accounts and posts their product into the same average:

- **AMBA/SOL**: the Meteora pair `DxUB19khkvd3FehY8sovQ1FTTcEktPL4CM6yc49coWQH`
  is a DAMM v2 (CP-AMM) pool, program
  `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`, not DLMM: token A is AMBA
  (`BxKJN4PKAT2CFhVbsyKzVyCznu5a2n2oVh8Pb2wdDjam`, Token-2022, 9 decimals),
  token B wrapped SOL; its `sqrt_price` is Q64.64 of lamports per AMBA atom.
  DAMM v2 keeps no oracle or cumulative price, and this pool's dynamic fee
  (whose `sqrt_price_reference` would be a pre-trade price) is off, so there
  is no average in the pool to read: the program takes its spot price as the
  sample. On 2026-10-18 (slot 458,288,450) it held about 97.9M AMBA and 174
  SOL at a 1.25% fee, and priced AMBA at 1.774e-6 SOL ($0.000184).
- **SOL/USD**: Pyth's sponsored SOL/USD feed account
  `7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE` (a `PriceUpdateV2` owned by
  the Pyth Solana Receiver `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`,
  feed id `ef0d8b6f…80b56d`, exponent -8). The program takes its
  `ema_price`, Pyth's own exponentially weighted average (about an hour),
  only from a `Full`y verified update (every Wormhole guardian signature
  checked by the receiver), at most `PYTH_MAX_AGE` = 300 s old (a safety
  check: the sponsored feed updated every 50 to 210 s when sampled).

Every address, owner, discriminator, length and field offset is a
compile-time constant; the pool's mints must be mainnet AMBA (also the stake
pool's `amba_mint`) and wrapped SOL, and a disabled, not yet active or empty
pool is refused. The price in the pool's units is `sqrt_price^2 * ema_price *
10^(amba_decimals + 3 + exponent) / 2^128` (USDC 6 decimals, `PRICE_SCALE`
10^6), rounded down (`docs/interfaces.md` has the layouts and sources).

Manipulation. The Meteora pool keeps no average, so the program averages
the samples itself, and each sample is the pool's spot price. What keeps it
honest is the program: `PoolPrice` must be the only instruction in its
transaction besides compute-budget ones (instructions sysvar), so a
swap-crank-swap back in one transaction is refused; a sample moves the
average only linearly over `twap_window`, so a sample that a later honest
one replaces after `t` seconds moves it by `t / twap_window` of its error
(with the five-minute crank, a twelfth); and the freshness and first-hour
rules above keep an old or lone sample from lending. The risk that remains:
a bundle of separate transactions (swap, `PoolPrice`, swap back, in one
slot) after each honest crank can still skew the average, at a cost. On the
pool above, pushing the price to half costs about 1.6 SOL of fees per
crank. Each such crank moves the average a twelfth of the way there, which
would give a shortfall's winners about 4% more AMBA from `CreditSettle`;
repeated after every honest crank, at that cost each time, it keeps
pulling. Pushing the other way (doubling the price, about 72 SOL in and 1.8
SOL of fees per crank) raises the capacity the maker lends against. The
user chose on 2026-10-18 to ship with these safety checks and start
mainnet credit small. A deeper pool, or a pool that keeps its own
time-weighted price (Meteora DLMM's oracle), would raise the bar; this pair
keeps none. On mainnet the treasury maker cranks it every five minutes
(`futures-maker`; `futures-ops stake-pool-price` by hand).

**Capacity.** The pool's credit capacity is its staked AMBA at the average
price, times `haircut_bps` (the user, 2026-10-17: a 50% haircut; no cap
relative to Earn's USDC), leaving out the AMBA of shares that are unstaking
(`StakePool::unbonding`, below: they back no new credit). The operator lends it to markets: `CreditFund`
(tag 36) adds to a market's `credit_limit` while the sum lent (`allocated`)
stays within the capacity. `CreditDefund` (37) takes it back down to what
the vault's position there could still cost (all of its debt when the vault
has no trader slot there), and only while the market is open: once it is
final the credit stays for `CreditSettle` (`NotTradable`). A falling price
lends nothing new and lets no stake leave that backs lent credit. Credit
already lent stays: covering a price drop is what the haircut is for.

**The creator's rate** (the user, 2026-10-21: "I want it to stay 50% for
everyone, besides me as creator, I want a special path", the creator's AMBA
at 90%, which they lower if the price moves). The program keeps one
`haircut_bps` per pool, so the maker sets it (`StakeConfig`) to the blend of
the creator's active shares at `MAKER_STAKE_CREATOR_BPS` and everyone else's
at `MAKER_STAKE_BASE_BPS` (default 5000), rounded down, whenever the pool's
shares or its rate move (`ops::stake_creator`). The capacity is then the
creator's AMBA at its rate plus everyone else's at theirs. A shortfall still
takes all staked AMBA in proportion, so the other stakers' AMBA also covers
part of the creator's extra credit. The user chose that for now ("blend
now, upgrade later"); a creator slot in the program, with its own rate and
covering its extra credit first, waits for the next mainnet upgrade. The
creator's wallet is the `creator` role (Cloud KMS on mainnet, the user's
ask). The maker stakes the AMBA sent to it every pass, except what the
creator's own `Unstake` paid back (counted in the maker's state), so that
can leave. The Earn page shows the pool's rate, which is the blend.

**Borrowing.** In a market with credit, the vault's trader may escrow more
than its free cash: `Quote` and the vault's own `Submit` borrow what its
free cash lacks, raising `debt` (virtual cash in its free balance that custody
does not hold). Hence the invariant above. Every instruction ends by
repaying the debt from the vault's free cash there (`repay`, in `store`).
While it owes, the vault therefore has no free cash in that market to move
out, and what it earns or nets pays the debt first. Two rules bound the debt
(`state::credit_risk`):

- **Exposure**, what borrowing exposes the stakers to: the debt less the
  vault's free cash and the least it holds at any settlement from 80 to 120
  (below): what its claims and inventory pay there and its reserved cash,
  less what fills could take there (an escrowed bid at or above the
  settlement, an escrowed ask below it, each at its step's best price; an
  order's contribution beyond what its claim pays). At most
  `credit_limit`.

  **Band funding (the user, 2026-10-27: "80-120").** The settlements counted
  are the reference ± `CREDIT_BAND_BPS` = 20% of it (`state::credit_band`;
  80 to 120 on the 100 reference). Until then the escrow counted as spent
  at every settlement at once, so a two-sided whole-grid ladder was funded
  for the index ending at 0 and at 200 together; on production's 2026-10-27
  books that rule took $1,167 a market where the worst settlement in 0-200
  needed $587 and the worst in 80-120 $231. Now the vault's cash and credit
  cover the band, and the maker quotes as much as that admits: about 5x on
  those books (the measurement is in the design note of 2026-10-27).
  Beyond the band nothing reserves the loss. A settlement there that costs
  more than the vault's cash and credit is a shortfall like any other
  (below): the buffer, then the staked AMBA at the average price (at most
  all of it, whichever markets it lends to), then the winners' payouts cut
  pro rata (`b1`, `b2`).
- **Carry**, what pays the debt back: the vault's own free and reserved cash
  there plus the backing its own claims and inventory hold, `k` times the
  larger of its long and short lots at each step. Long and short coverage
  match at every step, so that is never more than the market's backing, and
  only the vault's own instructions and the fills of its resting liquidity
  move it. The debt stays within it. A carry past a `u64` counts as
  `u64::MAX`, which covers any debt, so an admitted fill can't overflow it
  (Codex's review, 2026-10-18). So everyone else's free and reserved
  cash stays in custody, and no one else's `Redeem`, `Transfer` or netting
  ever answers to the vault's rules (`f6`). (`repay` still checks the debt
  against the open sets' backing plus the vault's cash, the backing alone
  once the credit has settled; the carry implies it.) A loss beyond the
  vault's cash can only come out of settlement payouts, where the buffer and
  the stakers' AMBA make the winners whole.

Both rules are checked when the vault acts, at the worst any later fill of
its resting orders, close orders and ladder could do (the re-review of
2026-10-18): after its own `Quote` or `Submit` while it owes anything
(`CreditLimit` otherwise); `CreditDefund` never takes the limit below that
worst exposure; its withdrawals take no more than both rules leave. A fill
of its liquidity is, before netting, a new claim (or inventory) of the
order's side paid by its contribution: from escrow (an opening order, a
ladder lot its inventory doesn't back) or from the sets its netting then
redeems (a close order, a lot its inventory backs). Netting and `repay`
leave both rules as they are. For the exposure only the pieces paid from
free cash count: at settlement `s` a lot costs at most `(c - pays(s))^+`
more, so every close-order lot and every inventory-backed ladder lot (the
first to fill, best tier first) is counted as sold for only its price,
summed at each grid position. For the carry, at each step the costliest
lots that offset its net position hold nothing new and cost their price, any
other lot holds `k` for at most `k`, and a pair of opposite lots holds `k`
for both prices; the program sums each step's exact least. Both bounds hold
for every partial fill in any order, so the checks that follow a fill of the
vault's liquidity can't fail (they stay, as asserts) and a taker's package
through it, which price priority may require, always trades (`f4`, `f7`).
Its compute: the program evaluates both rules in one pass over the grid in
64-bit arithmetic (`state::credit_risk_fast`: each ladder step read once, a
ladder alone counted tier by tier only as far as the step's inventory and
net position reach), with the same values as the reviewed two-pass 128-bit
form (checked over random books), which decides wherever a lot count,
price or `k` reaches `2^31`. On the SBF target 128-bit arithmetic is
branches and spills and a checked multiply a call: in that form an indebted
vault's whole-grid `Quote` at the dev markets' shape ran out of the 1.4M cap
on dev (2026-10-18); now it takes about 875k, the check about 140k (`c1`).
The price of that: before expiry the vault can't post asks, bids or a close
that could realize a loss beyond its own cash there (`f2`); losses the
stakers carry arise only at settlement. While the credit is unsettled, the
vault's trader slot holds its debt: no other trader takes it, even when idle
(`f3`). Rule for the oracle
front-running concern (the user, 2026-10-17: "no credit near expiry"): from
`credit_until` (expiry less the pool's `expiry_window`, dev a day) the vault
borrows nothing new. Trading itself stays open, and its quotes may still be
re-priced within what it already owes.

**Earn.** `VaultDeposit` and `VaultWithdraw` value the vault's trader in a
credit market at its equity (`state::credit_equity`):

1. the raw value less its debt, never below zero, since the stakers carry
   the rest;
2. less the stakers' share of its profit (below).

Earn depositors can lose at most the vault's own cash in a market. The
stakers carry the rest, up to its credit limit. A withdrawal may still take
cash a retired ladder releases while the vault owes; that leaves the
exposure as it was. A withdrawal from a credit market (`VaultWithdraw`,
`VaultDefund`) leaves the most the stakers' share of what the vault has
earned there can become in the market for `CreditSettle`: `profit_bps` of
its best-case profit, at full credit weight, since a withdrawal makes credit
a larger part of its capacity from then on. Settlement pays the stakers
their actual share and the rest is the vault's again. Only the credit
owner's own `VaultFund` and withdrawals move the market's funding in and
out, the baseline of that share; another vault's cash there doesn't (`f5`).

**Settlement (the tail rule, automatic and published).** After `Finalize`,
the vault's own claims, inventory and orders settle first (`Settle`,
`SettleMaker`, `Prune`). Their payouts repay its debt. Everyone else's
settlement waits (`NotSettled`), and so does the vault's cash there. Then
anyone sends `CreditSettle` (38):

- **No debt left:** the stakers get their share of the vault's profit there,
  `profit_bps` (the user, 2026-10-17: 50%) of its equity above its net
  funding, times the credit weight. The weight is the credit limit's share
  of limit plus net funding, time-weighted from the first `CreditFund` to
  expiry. The first `CreditFund` sets the funding baseline to what the
  vault's position there was worth at best, so stakers share only what it
  earns after credit arrives. The share moves to the pool's USDC account. It
  fills the pool's buffer up to `buffer_bps` of the credit lent (dev 10%),
  and the rest goes to the shares.
- **A shortfall:** the buffer pays first, into the market's custody. Then
  the rest is set aside in AMBA at the average price (rounded up), taken
  from the staked AMBA (at most all of it, so a very low price takes every
  atom rather than overflowing; the shares lose it pro rata). The market
  records the uncovered `shortfall`, the AMBA and the `winners` (the
  remaining payouts). Each winner then settles through `SettleRecovery`
  (39), per owner: its payout less its share of the shortfall in USDC (cut
  rounded up, so the whole shortfall is spread), plus its share of the AMBA
  (rounded down) straight into its AMBA account. Plain `Settle` pays only
  the claims that pay nothing (`NeedsRecovery` otherwise). The winners'
  USDC is exactly what custody holds. `repay` kept the debt within the open
  sets' backing and the vault's cash, which the vault's own settlement has
  paid into the debt by now, so the winners' payouts carry what the buffer
  didn't (`Insolvent` otherwise, checked after the buffer pays).

Either way the credit goes back to the pool, and a buffer above its new
target goes to the shares.

From `Finalize` until the credit settles, and for good if it settles short,
claims leave the market only through settlement (`Settle`,
`SettleRecovery`). `Transfer` and `Redeem` refuse everyone but the vault
(`NeedsRecovery`): a redemption, or a transfer's netting, would pay a set in
full past the shortfall's cut (`f3`). The vault's own redemptions repay its
debt, so they stay open to it.

**Stakers.** `Stake` (31) takes AMBA by a checked transfer; the shares are
for what arrives (the 0.25% fee comes out). Unstaking takes a timer (the
user, 2026-10-18: 7 days, as staking normally works; before, `Unstake` paid
at once whatever wasn't backing lent credit, and since the maker lends the
whole capacity and re-lends freed credit every pass, stakers were in effect
locked with no date). `StartUnstake` (41) marks `shares` (`u64::MAX`: all
the position's shares not unstaking yet) as unstaking: the position's
`unbonding` and the pool's grow by them and the position's `unbond_ts` is
now; adding more restarts the timer for all of them. Unstaking shares stay
shares: they earn the USDC profit share and the AMBA stream and bear
shortfalls exactly as the rest, but count for no capacity, so the maker
lends nothing against them and takes back the credit they backed (down to
each market's exposure, as for any capacity drop). `CancelUnstake` (42)
returns them to active. `Unstake` (32) pays unstaking shares (`u64::MAX`:
all of them) once `UNBOND_SECS` = 604,800 s (7 days) have passed since
`unbond_ts` (`NotSettled` before), as long as what stays, unstaking AMBA
included, keeps the capacity at or above `allocated` (`CreditLimit`
otherwise: the rest comes out as markets settle and the credit comes back).
A stale price gives no capacity, so while credit is lent nothing leaves
until a fresh price is posted.

Nor does anything leave while a market the pool lends to is near its end
(Codex's review of the timer, 2026-10-18, as agreed with the user). Once a
market is in its last `expiry_window` (dev a day: the user's rule of no new
credit near expiry), or past expiry with its credit unsettled, its
shortfall may be certain; a staker whose timer is done could leave before
`CreditSettle`, and the stakers who stay would bear the whole loss. So
`Unstake` waits (`NotSettled`) from the earlier of the market's
`credit_until` and its expiry less the pool's window until its
`CreditSettle`. For the program to check that, the pool counts the
markets holding its credit unsettled (`credit_markets`): the first
`CreditFund` in a market counts it and marks its header (`CREDIT_COUNTED`
in `credit_flags`), and its `CreditSettle` uncounts it. `Unstake` takes
exactly that many markets after its fixed accounts, distinct, each owned by
the program, with the pool's vault as `credit_owner`, counted and
unsettled (`InvalidAccount` otherwise), so none can be left out. The ops
tools find them by a scan of the program's markets (`stake::unstake_markets`,
through `ameba_futures_client::unstake_markets`); the API from its cached
markets. A pool from before the count (792 bytes or shorter) may have lent
in markets it never counted: it reads its count as unknown
(`credit_counted` false, also once grown) and refuses every `Unstake` while
it lends anything; once the credit lent is back to zero every market it
didn't count has a zero limit, which no shortfall can reach, and its count
is complete from then on (a market it lends to again is counted). Only a
pool of `STAKE_LEN` counts at `CreditFund`.

Anyone may send `Unstake`: it pays only into the owner's own AMBA account,
and the keeper completes every finished timer each pass
(`stake::complete_unstakes`; a refusal waits for a later pass). Anyone but
the owner pays all of the position's unstaking shares, never a part
(`Unauthorized`), and an `Unstake` that would pay no AMBA is refused
(`InsufficientFunds`), so nobody burns another's shares for dust (Codex's
review of the timer). A pool or position from before the timer (784 or
704 bytes, 192 bytes) reads nothing unstaking; `StartUnstake` grows it to
`STAKE_LEN` = 800 / `STAKE_POS_LEN` = 208 bytes first, the payer paying the
rent (a 792-byte pool too).
`StakeClaim` (33, permissionless, into the owner's own accounts) pays the
position's USDC (`usdc_acc`, at a finer scale than Earn's, since shares are
AMBA atoms) and its AMBA subsidy. The subsidy is a stream like Earn's
(`StakeRewards`, 34, no house), funded by AMBA's transfer fees ("remmeber
it also comes from token transfer etc", the user, 2026-10-20).
`StakeRewardsWithdraw` (35) takes back only its surplus. Stakers claim their
AMBA whenever they like (the user, 2026-10-20, as for Earn: no payout at
the end of a month); the keeper pays stakers only after a credit settlement
paid them USDC, and `StakeClaim` then pays their streamed AMBA with it.

**The stakers' monthly rewards.** `MAKER_STAKE_ROLLOVER=on` creates a
separate reward token account owned by the pool, then starts or rolls a
stream once that account holds tokens beyond what stakers are already
owed. The budget is that available balance (net of any incoming transfer
fee), from now to the next month's 1st. A running stream is left alone;
funding received during it stays for the next month. Unclaimed AMBA remains
reserved. The first run registers the account with a zero-amount stream
that ends at once; the owner funds that same account whenever ready.
`futures-ops stake-rollover` runs this by hand. The maker signs as the
operator and needs no fee key. No program change is required.

**Optional fee collection.** AMBA's 0.25% fee is withheld in each transfer's
destination account (and, once harvested, on the mint). Only the mint's
withheld-withdraw authority can collect it, using
`WithdrawWithheldTokensFromAccounts` or `WithdrawWithheldTokensFromMint`.
The user keeps that authority and can withdraw fees into the reward account
personally. `MAKER_STAKE_SWEEP=off` disables automatic collection entirely.
If enabled, `MAKER_STAKE_SWEEP=on` also enables monthly rewards and attempts
to collect fees first with the treasury key. A different or inaccessible
fee key skips collection; it does not block manually funded rewards.
Finding the fees is one `getProgramAccounts` with a 64-byte slice per token
account each month; batches fit the packet limit. `futures-ops stake-sweep`
runs collection by hand and requires the correct fee authority.

**Epochs.** A shortfall can take every staked atom while shares are still
out, leaving them worth nothing, or leave a few atoms against many shares
(a near-total wipe-out), when a large new stake would need more shares than
a `u64` holds. So `Stake` starts
a new share epoch (`StakePool::epoch`) when every staked atom is gone while
shares are out, or when the shares for what arrives (computed in `u128`)
don't fit a `u64`, alone or added to the pool's (`StakePool::rolls_over`; no
other threshold). First the AMBA stream accrues to the old shares; the pool
keeps the stream's and the USDC's accumulators as of the epoch's end
(`epoch_ends`) and its rate (`epoch_rates`): the `R` atoms still staked
against its `N` shares. The new epoch starts at one share per atom, with
those `R` atoms as `R` shares, the old cohort's. The pool keeps the last
`EPOCH_ENDS` = 4 ends (all its header has room for; the rates follow the
stream). A position of an earlier epoch, at its next `Stake`,
`StartUnstake`, `CancelUnstake`, `Unstake` or `StakeClaim`
(`StakePool::touch`), is paid against its epoch's end what its
shares earned, then its shares convert to `shares * R / N`, rounded down,
and so on through each later end kept, to the current epoch; it earns in
each epoch on the way. Its unstaking shares convert the same way (never
above its shares), and so does the pool's `unbonding` at the rollover. So the old stakers keep the residual's value: their
converted shares are worth the atoms their old ones were. A full wipe-out
has `R = 0`: the old shares convert to none, as before. Rounding leaves less
than one share (one atom) per old position owned by nobody: those atoms stay
staked in the pool, back its credit and take their part of later
shortfalls, and their part of the USDC and the stream stays in the pool's
accounts unclaimed (`usdc_owed` counts it).

A position untouched through more than four rollovers has no kept end for
its epoch, yet its principal still converts (Codex's review of the
conversion, 2026-10-18; before, it lost its shares). The pool keeps a
running share index (`share_index`, times `INDEX_ONE` = 2^126; zero reads
as one): every rollover's rate `R / N` multiplied up since the last full
wipe-out, each product in 256 bits and rounded down. Each position keeps
its epoch's index (`index`). Such a position converts at once to
`shares * share_index / index` (likewise in 256 bits), rounded down, never
more than it had nor than the rates exactly give. A full wipe-out, or a rate
so small the index would round to zero, restarts the index at one and
records the epoch it starts (`wipe_epoch`); a position of an earlier epoch
converts to none, however far back. The scale keeps that rounding to
positions worth under an atom: an index of 2^64 or more rounds to zero only
at a rate under one over it, and a position holds under 2^64 shares; only
once the rates multiplied up fall below 2^-62 can it take an atom (Codex's
review of the index, 2026-10-18: at 10^18, 10^18 shares left one atom, then
a rollover keeping 1%, wiped positions still worth millions of atoms). Only
what its shares earned since it was last touched and never collected is
lost (its pending amounts stay); the keeper pays every staker after each
credit settlement, so that is at most what accrued since.

The rates and the index came after the first pools (the reviews of
2026-10-18): a pool created before is `STAKE_V1_LEN` = 704 bytes, without
them, and reads the rates as zero, which is what its rollovers were (full
wipe-outs), and the index as one. Every instruction takes all four sizes
(704, 784 before the unstake timer, 792 before the count of credit
markets, 800). A rollover of an older pool first grows it to `STAKE_LEN` =
800 bytes, the rent paid by the `Stake`'s payer.

**Trust.** The operator (the treasury key) posts the price, lends the credit
and quotes with it. The program bounds what that can cost stakers, at most
each market's limit, but trusts its quoting, as Earn depositors already do.
The pool's parameters (`StakeConfig`, 29) are the operator's to change;
creating a pool is the upgrade authority's (above).

**Dev parameters** (`ops::stake::DEFAULT_PARAMS`): haircut 50%, profit
share 50% (both the user's), buffer target 10% of the credit lent, average
over 3,600 s, no new credit 86,400 s before expiry (Claude's defaults).

**Maker.** `futures-maker` lends each open, tradable market an equal share
of the pool's capacity until the market's credit window closes, and takes
back what a market holds above its share. The 1% band only spares a
transaction every pass while the average drifts. It quotes with its own cash
plus `Book::credit_room`, as many lots a step as the credit rules admit at
the ladder's worst fill (`Book::quote_admits`): holding inventory bought on
credit, with asks that could sell it at a loss beyond its own cash, it quotes
thinner until they fit, so its quotes keep landing. It lends the whole
capacity, with no unlent reserve (the user, 2026-10-18): unstaking shares
leave the capacity, so it hands their credit back as it can, and the rest
comes back at settlement. It does not yet steer the credit toward close-outs and
buybacks, as the user's notes recommend; the credit simply deepens its whole
ladder.

**Events.** `ARSCRT01` [pool, vault, mint], `ARSCFG01`, `ARSPRC01` (`StakePrice` or `PoolPrice`) [pool,
price, average, ts], `ARSSTK01` [pool, owner, AMBA, shares], `ARSUNS01`
[pool, owner, shares, AMBA], `ARSUBD01` (`StartUnstake`) [pool, owner,
shares added, unstaking, timer's end], `ARSUBC01` (`CancelUnstake`) [pool,
owner, shares],
`ARSCLM01` [pool, owner, AMBA, USDC], `ARSRWD01`, `ARSRWX01`, `ARCFND01` /
`ARCDFD01` [market, pool, amount, limit], `ARCSET01` [market, pool, share,
covered, shortfall, AMBA], `ARRCVR01` [market, owner, payout, USDC, AMBA].

**Tests.** `integration/tests/stake.rs`:

- `s1`: credit lets the maker quote beyond its cash, within the market's
  limit and the pool's capacity. Fills leave the exposure as it was, Earn
  prices the vault at its equity, and a profit pays the stakers their share.
- `s2`: a shortfall drains the buffer, then pays the winner in AMBA.
  Unstaking waits past expiry until the credit settles, its timer long
  done.
- `s3`: unstaking keeps what backs credit, the price's average moves the
  capacity, an offset repays the debt, nothing new is borrowed near expiry,
  and the subsidy stream pays.
- `s5`: the stakers' month: the fees swept (the holders' accounts and the
  mint) and streamed, a claim mid-month, the next month's fees and the
  unclaimed AMBA still owed.
- `s6`: manual funding without a fee key: reusable account, net transfer
  amount streamed, mid-month claims, no active-stream reset, next-month
  rollover preserving owed rewards, and no new stream without new funding.
- `s4`: the keeper's settlement and payments.
- `e1`: a near-total wipe-out, then a stake whose shares used to overflow
  starts a new epoch; the old staker unstakes its residual (on a pool of
  either size: the legacy one grows).
- `u1` to `u6`, the unstake timer: `Unstake` waits the week and the
  unstaking shares back no new credit (`u1`); a shortfall during the timer
  cuts them like every share, and they keep earning the stream (`u2`);
  cancelling, restarting the timer, only the owner starting or cancelling
  (`u3`); a 192-byte position and a 704-, 784- or 792-byte pool grow, the
  ops tools finding both sizes (`u4`); anyone, and the keeper, completes a
  finished timer, only into the owner's account and only all of it (`u5`);
  an unstake that would take AMBA backing lent credit waits, then goes
  through once the credit is back (`u6`); an unstake paying no AMBA is
  refused (`u2`).
- `w1`, `w2`, the markets an unstake waits for: a finished timer can't
  leave in a lending market's last day nor after its expiry before its
  `CreditSettle`, then can, the keeper waiting as well; missing, repeated,
  wrong or uncounted market accounts are refused (`w1`); a pool grown from
  792 bytes refuses while it lends, its count complete once the credit is
  back, a market lent to again counted (`w2`).
- `f1` to `f7`, the reviews' fixes: only the upgrade authority creates a
  pool (`f1`); the vault's asks that would realize a loss beyond its cash
  are refused when posted, the least it may ask fills in full, and another
  trader's deposit stays withdrawable (`f2`); after expiry a transfer waits
  for settlement, the indebted vault's slot stays its own and its credit
  can't be taken back (`f3`); a close order whose fill could take the
  exposure over the limit is refused when it rests, and once one rests the
  limit can't go below its worst fill, which then trades (`f4`); the
  stakers' share stays in the market when the vault withdraws, and another
  vault's funding leaves the baseline alone (`f5`); others' sets redeem
  through fills and transfers while the vault owes (`f6`); takers trade
  through the vault's two-sided ladder at the least asks it may post, in
  pieces and in full, the rules holding after every fill (`f7`).
- `c1`, the compute of the indebted vault's own `Quote` at the dev markets'
  shape (grid 200, 1,024 claim slots), futures-maker's whole-grid 5-tier
  ladder sized as the maker sizes it, inventory on most of the grid: under
  1,000,000 CU, 400k below the cap for what it leaves out (resting vault
  orders, a full claim table).

The integration tests write the price into the pool as `PoolPrice` posts
it, its prices taken as having run `MAX_PRICE_AGE`, and post a fresh one
(at the average so far) before each `CreditSettle`.

`state` unit tests cover the math, the capped AMBA conversion and the
epochs included (full and near-total wipe-outs, chained conversions, a
position past the kept ends converting by the share index, a full wipe-out
restarting it, the legacy size), the unstake timer (unstaking shares out
of the capacity, their conversion at a rollover, every account size),
check that a stale price, and in the
production build a pool's first hour of prices, lend nothing
(`a_stale_or_new_price_lends_nothing`), check a step's least over every
fill (`step_least`, in both forms) against brute force, and the one-pass
credit check against the reviewed form over random books
(`credit_risk_matches_the_reviewed_form`).

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
| Cancel; shifted intervals net their overlap (since 2026-10-17) | | `c6`, `c7`, `c9`, `c10` | |
| Exits on a full claim table, resting orders hold their slot, fills join one claim (since 2026-10-18) | | `k1`–`k5` | |
| Inventory-backed ladder quotes (since 2026-10-17) | | `b4`, `b5` | |
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
