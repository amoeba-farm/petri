# Petri CLI Command Guide

## Capped futures

Petri trades Amoeba's capped futures on the RAMX and NANDX memory-chip indexes,
the same markets the website trades. There is one market per index and expiry
(`RAMX Oct 26`, `NANDX Nov 26`, …).

- **What you trade.** A position is a claim on a range `[lo, hi)` of the index.
  A **long** pays $0.01 per lot (the dev lot size) for every index point the
  index settles above `lo`, up to `hi`. A **short** pays the rest: every point
  below `hi`, down to `lo`.
- **How much you can lose.** A long pays its premium up front. A short puts in
  the rest of the range's value up front. Either way, that up-front amount is
  the most the position can lose. There are no margin calls and no
  liquidations.
- **What you can make.** At most the range's width times $0.01 per lot, less
  what you paid: a long gets it all if the index settles at `hi` or above, a
  short if it settles at `lo` or below.
- **Choosing the range.** Leverage sets the end on the losing side: entry minus
  entry/leverage for a long, entry plus entry/leverage for a short (default
  10x). `--to` sets the end on the winning side; by default it is the
  market's suggested range, where the index can reach by expiry (2σ).
  `--range LO-HI` names both ends instead. Range ends sit on the market's
  grid, so the leverage you get at the fill price can differ a little from
  the one you asked for; the quote shows it.
- **One book.** Every range trades on one composed book per market: different
  ranges trade against each other, and a treasury market maker fills the
  pieces in between. At equal prices, older orders fill first (FIFO).
- **No trading fee.** The markets charge no fee. Your wallet or the
  API's sponsor pays the Solana network fee (see [Sending](#sending)).
- **When it settles.** At expiry the market settles on the oracle's index for
  that product and expiry. The keeper then pays every position into its
  owner's free cash in that market; `petri withdraw` moves it to your wallet.
- **Why trust the settlement.** The index comes from the standalone oracle
  program, built from public sources with bonded evidence. You can read every
  month, source and recipe with `petri oracle` (see [Oracle](#oracle)).

A worked example, from `petri quote long ramx 100` on the dev chain:

```text
Long 100 RAMX Oct 26 · range 87–135 · 10x
  Entry       97.37 average
  Per point   $1.00 from 87 to 135
  You pay     $10.37, the most you can lose
  Pays up to  $48.00 if RAMX settles at 135 or above
  Deposit     $10.37 from your wallet into the market first
Send it: petri long ramx-oct26 100
```

If RAMX settles at 110, the position pays (110 − 87) × $1.00 = $23.00, a
$12.63 gain. At 87 or below it pays nothing and the $10.37 is lost. At 135 or
above it pays $48.00.

## Naming a market

Every market command takes `MARKET` as:

- a symbol: `ramx` or `nandx`, the open market of that index expiring first;
- a name: `ramx-nov26`, or the label `"RAMX Nov 26"`;
- the market's address.

`--expiry` picks the expiry for a bare symbol: `nov26`, `2026-11` or unix
seconds (`petri book ramx --expiry nov26`). Expiries follow the cluster clock.

## Command overview

```text
petri                                   front door: markets, trade, positions
petri help                              every command

markets [MARKET]                        every market, or one (alias: market)
book <MARKET> [--range LO-HI] [--depth N]
trades <MARKET> [--limit N]
chart <MARKET> [1h|24h|7d|30d|all] [--height N]

quote long|short <MARKET> [LOTS] [order options]
long <MARKET> [LOTS] [order options]    alias: buy
short <MARKET> [LOTS] [order options]   alias: sell
positions [OWNER]                       aliases: account, pos
close <MARKET> <#|all> [-n LOTS] [--slippage PCT] [--keep-cash]
cancel <MARKET> <#|all>
deposit <MARKET> <USD>
withdraw <MARKET> <USD|all>
history [OWNER] [--limit N]

wallet [OWNER]
wallet address
wallet new [--out PATH] [--force]
faucet [OWNER] [--sol N]                dev builds only (--features dev-faucet)
status

oracle …                                see "Oracle"
research …                              see "Oracle Research Workspace"
tui [--no-update-check]                 the terminal interface

config [show]
config set rpc|api|keypair|deployment <VALUE>
config reset rpc|api|keypair|deployment
mcp status|enable|repair|disable|manifest
update [check|info|recover] [--restart]
```

Order options: `--usd USD`, `-x/--leverage X`, `--to PRICE`, `--range LO-HI`,
`--limit PRICE`, `--post`, `--slippage PCT`, `--expiry E`. Every command
explains itself with `--help`, and every read accepts `--json`.

## Reading the market

These commands need no wallet.

### `markets [MARKET]`

Without `MARKET`: every market with its last price, 24h change, index,
24h volume, settlement date and status.

```text
$ petri markets
Capped futures
MARKET        NAME          PRICE    24H  INDEX    VOL 24H  SETTLES             STATUS
NANDX Oct 26  nandx-oct26   93.96  -6.44  91.89  2,662,769  Nov 1 (in 16d 23h)  open
RAMX Oct 26   ramx-oct26    96.70  -2.00  95.00  2,767,922  Nov 1 (in 16d 23h)  open
NANDX Nov 26  nandx-nov26  108.24  +8.64  91.89  2,521,134  Dec 1 (in 46d 23h)  open
RAMX Nov 26   ramx-nov26    93.16  -7.04  95.00  2,354,753  Dec 1 (in 46d 23h)  open
```

With `MARKET` (or `petri market MARKET`): its price, index, suggested range,
best prices on that range, activity and how it settles. A settled market also
shows its settlement value.

```text
$ petri market ramx
RAMX Oct 26 (ramx-oct26) · open · settles 2026-11-01 00:00 UTC (in 16d 23h)
  Price       96.70 last trade · -2.00 in 24h
  Index       95.00 · 24s ago
  Suggested   55–135, where the index can reach by expiry (2σ)
  Book        buy at 96.97 · sell at 95.37 on 55–135
  Activity    2,767,922 lots in 24h · 1,171 trades · 156 open positions
  Settlement  the RAMX index at expiry, from the oracle (period live)
```

### `book <MARKET>`

What one range costs, level by level: every price you could buy it at (sell
rows) and sell it at (buy rows), with the lots there. Different ranges trade
against each other, so each range has its own depth. Without `--range` this is
the market's suggested range; `--depth` sets the levels per side (default 10).

```text
$ petri book ramx --depth 4
RAMX Oct 26 · range 55–135 (suggested)
        PRICE   LOTS   TOTAL
  sell  97.63  7,670  23,901
  sell  97.41  6,421  16,231
  sell  97.19  5,351   9,810
  sell  96.97  4,459   4,459
  ── 96.17 mid, 1.60 wide · index 95.00 ──
  buy   95.37  4,459   4,459
  buy   95.05  5,351   9,810
  buy   94.73  6,421  16,231
  buy   94.43    311  16,542
```

`petri book ramx --range 90-120` shows another range.

### `trades <MARKET>`

Recent trades: time, the taker's side, lots, entry price and range. `--limit`
sets how many (default 20).

```bash
petri trades ramx --limit 10
```

### `chart <MARKET> [WINDOW]`

A price chart of the market's trades, with volume, in the terminal. `WINDOW`
is `1h`, `24h` (default), `7d`, `30d` or `all`; `--height` sets the rows
(default 14).

```bash
petri chart ramx 7d
```

## Trading

Trades sign with your wallet (see [Connection and wallet](#connection-and-wallet)).

### `quote long|short <MARKET> [LOTS]`

Prices an order exactly as `petri long` or `petri short` would send it,
without signing anything: the range, the leverage, the average entry, the
cost per point, what you pay (the most you can lose), the most it pays, and
any deposit it needs. It takes the same options as `long` and `short`.

```bash
petri quote long ramx 100
petri quote short nandx --usd 50 -x 5
```

### `long <MARKET> [LOTS]` and `short <MARKET> [LOTS]`

`long` buys a range; `short` sells one. `buy` and `sell` are aliases. Petri
quotes the order, shows the review, and asks before sending unless `--yes`.

| Option | Meaning |
| --- | --- |
| `LOTS` | Lots to trade; a lot pays $0.01 a point inside its range. |
| `--usd USD` | Spend about this much USDC instead of naming lots. Petri walks the book and buys whole lots that cost no more than that. |
| `-x, --leverage X` | Sets the range's end on the losing side: entry ∓ entry/X. Default 10. |
| `--to PRICE` | Sets the range's end on the winning side. Default: the market's suggested range. |
| `--range LO-HI` | Names the whole range instead of `-x` and `--to` (also `LO:HI`). |
| `--limit PRICE` | A limit order at this entry: what fills now fills, the rest rests on the book. |
| `--post` | With `--limit`: only rest, never fill now. |
| `--slippage PCT` | Market orders: how much worse than the quote they may fill, in percent. Default 2. |
| `--expiry E` | Pick the expiry for a bare symbol. |

```bash
petri long ramx 100                   # 100 lots at 10x on the suggested range
petri long ramx 100 -x 5 --to 130     # 5x, the range ending at 130
petri long ramx --usd 50              # spend about $50
petri long ramx 100 --range 90-120    # name the range
petri long ramx 100 --limit 99        # rest a bid at 99.00
petri short nandx 50 -x 20 --to 80
petri short ramx 100 --limit 104 --post
```

**Market orders** (no `--limit`) fill now, against what the book holds. Petri
sends at most the lots the quote filled, with a limit `--slippage` percent
worse than the quote, and refuses if nothing fills inside it. If the book
moves before the order lands, Petri solves it again once; past that, quote
again.

**Limit orders** (`--limit`) enter at that price. What can fill now fills;
the rest rests on the book as an order until it fills or you cancel it, and
its cost is set aside from your cash in the market. `--post` refuses an order
that would fill now.

**Funding.** An order pays from your free cash in that market. If that is not
enough, the same transaction first deposits the shortfall from your wallet's
USDC; the review shows it as `Deposit`. If you hold the opposite position on
exactly the same range, the order nets against it: those lots close and their
cash is freed (`Netting` in the review).

To get out of a position, use `close`: it trades on the position's own range
and can return the cash to your wallet in the same transaction.

### `positions [OWNER]`

Your positions, resting orders and cash in each market, with P&L. `account`
and `pos` are aliases. Pass an address to read another wallet.

```text
$ petri positions
Positions EDSr…zNEV · wallet $0.00 USDC

RAMX Oct 26 (ramx-oct26) · index 95.00 · cash $4,623.72 free
  #    SIDE   RANGE     LOTS   ENTRY     PAID        MAX       P&L  CLOSE NOW
  78   long   110–134  4,504  115.79  $260.70  $1,080.96  -$260.70     $20.17
  189  short  84–106   3,898  101.11  $190.62    $857.56  +$238.16    $399.91
P&L values each position at the index (the settlement once settled); CLOSE NOW is what selling it on the book returns.
```

`PAID` is what the position cost (the most it can lose), `MAX` the most it
pays at settlement. Resting orders are listed with the cash each holds.

### `close <MARKET> <#|all>`

Closes a position at the book, on the position's own range: a long is sold, a
short bought back. `#` is the position number from `petri positions`; `all`
closes every position in that market, one transaction each. Petri quotes each
close (what you get, what you paid, the P&L) and asks first.

By default the same transaction then moves your free cash in that market back
to your wallet; `--keep-cash` leaves it in the market. `-n, --lots N` closes
part of one position. `--slippage PCT` (default 2) is how much worse than the
quote the close may fill.

```bash
petri close ramx 78
petri close ramx-nov26 all
petri close ramx 78 --lots 10 --keep-cash
```

Lots held by a resting close order cannot be closed again; cancel that order
first.

### `cancel <MARKET> <#|all>`

Cancels a resting order (its number from `petri positions`), or every order in
that market. The cash it held goes back to your free cash in the market.

```bash
petri cancel ramx 12
```

### `deposit <MARKET> <USD>` and `withdraw <MARKET> <USD|all>`

`deposit` moves USDC from your wallet into a market. You rarely need it:
orders deposit their own shortfall. `withdraw` moves free cash (cash not held
by orders) back to your wallet; `all` takes all of it. Use it after
settlement, after a `close --keep-cash`, or after cancelling orders.

```bash
petri deposit ramx 100
petri withdraw ramx all
```

### `history [OWNER]`

Your trades, orders, fills of your resting orders, cancels, deposits,
withdrawals, nettings and settlements, newest first. `--limit` sets how many
(default 50).

### Settlement

At expiry each market settles on the oracle period it was created against:
the same index, expiry and oracle program. The keeper settles every position
into its owner's free cash in that market, and `petri history` shows each
payout as `settled`. Then `petri withdraw <MARKET> all` moves it to your
wallet. `petri market <MARKET>` shows the settlement value once it is set.

## Wallet

### `wallet [OWNER]`

Your wallet's address, SOL, USDC, free cash (and cash in orders) in each
market, and your oracle bond collateral if you have any. Pass an address to
read another wallet.

### `wallet address`

Prints the signing wallet's public key.

### `wallet new`

Creates a new signing wallet and prints only its address. The key file is
written next to Petri's config (`futures-id.json`, private to you) and saved
as your `keypair` setting. `--out PATH` writes it there instead and leaves your
signing wallet unchanged; `--force` replaces an existing file (its key is
lost).

### `faucet [OWNER]`

Dev builds only (`--features dev-faucet`; release builds have no such
command): mints test USDC to a wallet through the dev futures API. `--sol N`
also airdrops SOL from a local validator.

```bash
petri faucet --sol 1
```

### `status`

Checks the cluster (genesis, version, clock lag against the wall clock), the
futures API (health, and whether it serves the same programs as your trusted
deployment), the deployment's programs and USDC mint, the open markets and
your wallet.

## Sending

`--route` chooses who pays the network fee:

- **`local`**: your wallet pays. Petri reads the market account from chain,
  checks that the trusted futures program owns it, that it settles in the
  deployment's USDC and reads the deployment's oracle, and solves your order
  against it. It refuses before signing if nothing would fill inside your
  limit, if a `--post` order would fill, or if your wallet cannot fund the
  deposit.
- **`sponsored`**: the futures API prepares the transaction and its sponsor
  pays the fee. Petri refuses the route if the API serves other programs than
  your trusted deployment, or names a different sponsor. It checks the
  prepared transaction before signing: the sponsor is the fee payer; it calls
  only the compute budget, the creation of your own USDC account and the
  trusted futures program; every futures step is in this market, acts for you and
  uses your own token account; it does exactly the order you asked for; and
  any deposit stays within what the order can need at its limit.
- **`auto`** (default): local when the wallet holds at least 0.01 SOL,
  sponsored otherwise.

Transactions Petri builds itself set a 1.4M compute-unit limit. Program
errors are reported by name, with a hint for the common ones: `MinFill`
(nothing filled inside the limit), `WouldCross` (a post-only order would
fill), `NotTradable` (the market is not trading), `ClaimReserved` (a resting
close order holds those lots), token errors as `Token…`.

Petri asks before every order, close, cancel, deposit and withdrawal. `--yes`
skips only the prompt, never the checks. Without a terminal to ask on, a
command that needs confirmation refuses unless you pass `--yes`.

## Oracle

The index oracle is the standalone oracle program. It forms each product's
index over a fixed expiry and publishes it as a `Period`; each futures market
settles against its period. Each oracle month belongs to one period (one
product and expiry), runs through source submission, opening and Game phases,
and settles at expiry.

**Reads** come from the futures API (`--api`): `/v1/oracle/public-snapshot`
for every month's phase, source count and settlement status and each month's
verified source inventory, `/v1/oracle/catalog` for periods and settlement
times, and `/oracle/source-candidates` for research candidates. They need no
wallet.

**Writes** are built with the oracle program's own instruction builders (the
`ameba_oracle` crate, from the monorepo's `oracle/`, the source the deployment runs)
against the deployment's oracle program id, reviewed, confirmed (or `--yes`)
and signed by the attached wallet as fee payer. `--simulate` runs the same
transactions through `simulateTransaction` without loading a signer. A
transaction that needs an earlier one on chain first (a sponsorship or
proposal needs its evidence uploaded) is not simulated: it reads "checked
when sent, once the evidence above is on chain", and the send checks it
after the evidence lands. Before
anything is signed, Petri checks that the RPC's genesis hash is the
deployment's, that the oracle and futures programs are deployed there, and
that the oracle's config bonds in the deployment's USDC mint. Months and
sources are read from the oracle program itself and checked
(owner, length, layout, canonical address) before use. Program errors are
named (`InvalidOraclePhase: … (oracle error 6103)`).

`MARKET` is a product symbol (`ramx`), a month (`RAMX-202610`) or a period
address; `--expiry` takes `oct26`, `2026-10` or unix seconds. A bare symbol
picks the live month with the nearest expiry.

```text
oracle [months] [MARKET]                  months: phase, sources, settlement
oracle latest [MARKET]                    the most recently updated month
oracle history [MARKET]                   months by last update
oracle catalog                            periods and settlement times
oracle sources [MARKET] [--expiry E] [--query Q] [--limit N]
oracle recipe [MARKET] [--expiry E] [--query Q]
oracle candidates [--market M] [--expiry E] [--agent ID] [--limit N]
oracle candidates show <HASH>
oracle collateral [--owner PUBKEY]        USDC bond collateral
oracle collateral deposit <USDC>
oracle collateral withdraw <USDC|all>
oracle evidence write --kind locator|definition|archive (--text T | --file F)
oracle evidence show [OBJECT] [--kind K --text T] [--payer PUBKEY]
oracle evidence close <OBJECT>
oracle sources propose MARKET --bucket SKU --url URL --source-type T
                       (--definition T | --definition-file F) --bond USDC [--source-id HEX]
oracle sources support MARKET --source ID --stake USDC
oracle sources close MARKET
oracle sources challenge MARKET --source ID --reason N --bond USDC --evidence-hash HEX
                         [--comparison ID] [--challenge-id HEX]
oracle prints opening MARKET --source ID --value V --archive-url URL --stake USDC
oracle prints challenge MARKET --source ID --value V --archive-url URL --bond USDC
oracle prints finalize|expire MARKET --source ID
oracle prints finalize-phase MARKET
oracle updates commit MARKET --source ID --value V --archive-url URL --stake USDC
oracle updates reveal <CLAIM>
oracle updates challenge MARKET --claim CLAIM --value V --archive-url URL --bond USDC
oracle updates expire MARKET --claim CLAIM
oracle stakes settle MARKET --kind KIND --source ID [--owner PUBKEY] [--claim CLAIM] [--challenge ADDRESS]
oracle rewards [MARKET] [--expiry E] [--owner PUBKEY]   your rewards, month by month
oracle rewards claim MARKET [--kind proposer|supporter|opening|update]
oracle sponsor <CANDIDATE>                your collateral posts a research candidate's listing bond
oracle sponsorships [settle|claim <SOURCE>]
oracle reserve withdraw --agent ID <USDC> a researcher's held-back bounty share, to collateral
oracle emergency propose|apply|recover MARKET --kind K --target ADDRESS --target-id HEX [--account A …]
oracle emergency vote MARKET --kind K --target ADDRESS --target-id HEX --choice N [--account A …]
```

Every write accepts `--simulate`, and `--yes` skips the confirmation prompt.

### Collateral and bonds

Bonds, stakes and challenge pots are debited from your `UserCollateral` ledger
inside the oracle's bond vault and credited back (or to the winning side) when
they settle. `petri oracle collateral deposit 25` opens the ledger on first use
and moves USDC from your token account; `withdraw` moves available collateral
back. `petri wallet` shows the ledger next to SOL and USDC.

### Evidence

Evidence objects are immutable public preimages: a source URL (`locator`,
committed as `SHA-256("locator" || url)`), a source definition (`definition`,
stored as `"ameba-oracle-source-definition-v1" || 0 || u32 length || text`, the
commitment the futures API's candidate store uses) or a Wayback archive URL
(`archive`). `evidence write` creates and seals the object in chunks of 192
bytes (any payer may write), resumes a partial draft, and reports an
already-sealed object without sending. `evidence show` reads one back;
`evidence close` refunds an unsealed draft.

### Sources

`sources propose` writes the locator and definition evidence it needs, then
proposes the source with a listing bond and the SKU Merkle proof of its bucket.
A bucket is one SKU: its id is the SKU label's ASCII bytes, zero-padded to 32
bytes. Petri proves membership against the governed SKU set of the product
(the monorepo oracle's `governance/*_product_sku_manifest.v1.json`, compiled in),
or against the bucket set the month's sources carry when that is what the
month's coverage manifest commits to (dev bootstrap months); it refuses when
neither reproduces the committed root. `sources support` stakes on a source;
`sources challenge` bonds a challenge (reason codes are the program's).
`sources close` closes a month's source window (it finalizes the SKU
coverage, which anyone may send) once every product has a supported source;
it refuses before that, saying how many are covered. Scramble's challenge
days, its freeze, Opening and Game then follow on the calendar.

### Opening prints

`prints opening` submits a bonded opening value for a frozen source with its
Wayback archive URL; the source time is the archive's capture time, and the
archive object is written first. Before sending anything it checks the
oracle's rules and names what fails: the month is in Opening on the chain's
clock (from the end of Scramble's seven days to listing), the capture time is
from the Opening's start up to the chain's clock, the archive is of the
source's own URL, and `--stake` is the product's opening bond exactly.
`prints challenge` bonds an alternative. `prints finalize` accepts an
unchallenged print 300 slots after it landed (before listing), `prints
expire` marks an unclaimed frozen source inactive (from listing), and `prints
finalize-phase` moves the month into Game and the period to Live, once every
frozen source is resolved and the oracle has weighted them.

### Game updates

`updates commit` seals a source update with a stake: only a commitment hash goes
on chain. The value, archive URL and secret salt are saved in Petri's private
`oracle-update-claims.json` (mode 0600, next to the config) and never printed;
`updates reveal <CLAIM>` writes the archive evidence and reveals the claim from
that file. `updates challenge` bonds an alternative to a revealed update, and
`updates expire` refunds a commitment nobody revealed after its deadline. A
month's update lane needs its finalized active-weight manifest; months formed
by the dev bootstrap do not have one, so commits there fail with
`InvalidOracleActiveWeightManifest`.

### Escrow settlement

`stakes settle` returns a finished bond or stake to collateral
(permissionless): `listing-bond`, `support-stake` and `source-challenge`
(before listing), `opening-claim`, `opening-challenge`, `update-claim` and
`update-challenge`. Claimants and challengers are read from chain.

### Rewards

Once a month is Settled, its operator registers every reward and freezes
them; each recipient then claims its own (tag 174). `rewards [MARKET]` lists
the wallet's rewards by month and kind, read from chain: `proposer`,
`supporter` (paid by the source a supported one merged into), `opening` and
`update`. Each shows its amount once frozen, `claimed` once paid, `after
settlement` or `not frozen yet` before. Sponsored sources the wallet funded
or found show as `sponsored`, with its share: `sponsorships claim <SOURCE>`
claims them. Without MARKET it lists every month where the wallet has a
reward, a bare symbol that product's months; months without rewards are left
out.

`rewards claim MONTH [--kind K]` claims each frozen, unclaimed reward of the
month into the wallet's oracle collateral, a few to a transaction
(`collateral withdraw` takes it out). It refuses before the month settles,
before its rewards are frozen, and when nothing is left.

### Sponsorship

`sponsor <CANDIDATE>` (a content hash from `oracle candidates`) puts a research
agent's find on the oracle with your collateral posting the product's exact
listing bond, while the month takes sources; the bond comes back when the
source settles (`sponsorships settle`), and its reward splits with the
researcher (`sponsorships claim`). `reserve withdraw --agent ID <USDC>` moves
what sponsored bounties held back for one of your agents (its 64-hex id) into
your collateral.

### Council disputes

`petri oracle emergency` runs current-roster council cases:

```text
petri oracle emergency propose RAMX-202610 --kind bucket --target <bucket-address> --target-id <bucket-id-hex> --simulate
petri oracle emergency vote RAMX-202610 --kind bucket --target <bucket-address> --target-id <bucket-id-hex> --choice 0
petri oracle emergency recover RAMX-202610 --kind bucket --target <bucket-address> --target-id <bucket-id-hex>
```

`propose` opens a case; `vote` uses the attached wallet's current council seat;
`apply` executes the existing decision after its onchain decision deadline.
`recover` applies that same decision for a thin bucket. Three of five current
seats must agree; a round without three matching votes uses the case's defined
fallback. Roster rotation invalidates prior-round votes. The client reads the
current exact controller config, case evidence commitment and round before
signing.

`--kind source|opening|update` supports the dispute target kinds using repeated
`--account <address>` entries in the target-specific oracle builder order;
use `ro:<address>` for a readonly account. The program validates the complete
canonical account set. Source cases need source and source guard (plus a
comparison source and guard where applicable); opening cases need claim and
source; update cases need claim, source, observations, active manifest,
challenge guard and bucket. Applying a source/opening/update decision also
requires the coverage/merge/checkpoint accounts prescribed by the builder.
Bucket cases require no extra accounts.

The legacy oracle profile is permitted only in Petri's dev profile, when the
deployment explicitly reports `oracleReleaseTag: "localnet-dev"` on the
trusted localnet genesis.
Other oracle profiles require a registered oracle governance target.

Operator instructions (signer registry, economics templates, recipe and active
weights, challenge resolution, update finalization, month close) are run by the
deployment's operators (`futures-ops` on the dev localnet), not by Petri.

## Install from source

Build with Rust 1.93.1 from the Amoeba monorepo: the futures SDK (`../sdk`) and
the oracle program crate (`../oracle`) are path dependencies in `Cargo.toml`. The research workspace runs
on Node.js 24 (`node` on `PATH`, or `PETRI_RESEARCH_NODE`). Preview packages are
not publisher-signed or Apple-notarized; verify their checksums before opening.

```bash
cargo build --release --locked --bin petri
./target/release/petri --version
```

## Connection and wallet

Petri talks to one futures API (`--api`, `PETRI_API_URL`) and one Solana RPC
(`--rpc`, `PETRI_RPC_URL`). The profile (`--profile`, `PETRI_PROFILE`, the
saved `profile`) sets their defaults and what Petri trusts:

| Profile | Futures API | Solana RPC | Trusts |
| --- | --- | --- | --- |
| `production` (alias `mainnet`; a release build's default) | `https://api.amoeba.farm` | `https://api.mainnet-beta.solana.com` (Solana's public RPC) | mainnet-beta, the oracle `45YCerssGjg2M29RFc9D4fAJh7dfFpMJ3ZcZtaTnRXRS`, the futures program `Bvp9kiRtJ7dT8unxP4Da39gv2eXDLEHLpcDfzcEuRKMu` and the controller `8fhNi6QHU5TYNhoPDM4vs89ZBztnpxp3LnBXRgkBVKtx` only |
| `dev` (alias `localnet`; the default of a `--features dev-chain` build, such as the dev VM's petri) | `http://127.0.0.1:4500` | `http://127.0.0.1:8899` | the program ids the API's `/v1/config` names; on the dev chain's genesis also its own controller and the oracle's `localnet-dev` release |

In the production profile an API that names another cluster, other programs or
another controller is refused, so a wrong API can never point Petri at other
programs. Program ids come from the API's `/v1/config`, or from a deployment
manifest you trust (`--deployment`, `PETRI_DEPLOYMENT`; the production ids are
not imposed on a manifest).

### RPC methods

`--rpc` must be a full Solana RPC node: Solana's public mainnet-beta RPC (the
production default), a provider's, or the dev chain's validator. The futures
API's `/rpc` browser gateway serves only the web app's methods, so some of
these fail there; Petri names the method and says so when an RPC refuses one.
Every method Petri calls (`src/futures/rpc.rs`, `METHODS`; a test keeps this
list complete):

| Method | Used for | API `/rpc` gateway |
| --- | --- | --- |
| `getAccountInfo` | accounts: markets, the oracle, gates, the Clock sysvar | yes |
| `getMultipleAccounts` | accounts, up to 100 at a time | yes |
| `getProgramAccounts` | the oracle's periods, sources, sponsorships and a wallet's reward claims | no |
| `getBalance` | a wallet's SOL | yes |
| `getSlot` | `petri status` | yes |
| `getGenesisHash` | the deployment identity gate | yes |
| `getVersion` | `petri status` | no |
| `getBlockHeight` | whether a sent transaction expired | yes |
| `getLatestBlockhash` | building transactions | yes |
| `simulateTransaction` | oracle writes with `--simulate` | no |
| `sendTransaction` | sending signed transactions | yes |
| `getSignatureStatuses` | confirming them | yes |
| `getTransaction` | a landed transaction's logs | yes |
| `requestAirdrop` | `petri faucet --sol` (dev builds; a local validator only) | no |

The oracle commands, and the TUI's Fund and Rewards views, read with
`getProgramAccounts`, which Solana's public RPC rate-limits per caller
(`rewards` sends several at once; the views read every few seconds). Pass
`--rpc <provider URL>` or set `PETRI_RPC_URL` to an RPC provider's node. When
an RPC turns a call away for its rate, Petri says so and names `--rpc`; it
doesn't retry.

The signing wallet is `--keypair` (a keypair file or hardware-wallet URL), else the saved
`keypair` setting, else the Solana CLI config's keypair. `petri wallet new`
creates a dedicated wallet, saves it as the `keypair` setting and prints only
its address. Petri never prints keypair contents.

`chain_identity` verifies the deployment before anything is signed: the RPC's
genesis hash must be the deployment's, and the oracle and futures programs must
be deployed programs there; oracle writes also check the
oracle config's bond mint. Governed deployments also have their controller gate
accounts and epochs checked, for local transactions and sponsor-prepared ones;
a stale epoch or a substituted gate is refused. `petri status` shows the
result.

## Safety Notes

- Do not print, paste, or commit keypair JSON contents, `.env` contents, signer material, update-claim salts or operator credentials.
- It is safe to document keypair path placeholders such as `/path/to/id.json`; the file contents are secret-bearing.
- It is safe to document hardware wallet URL placeholders such as `usb://ledger?key=0`; the private key remains on the device.
- Every trade and oracle write is reviewed first; `--yes` skips only the prompt, never the checks.
- A position never loses more than it cost; check the `You pay` line before you confirm.

## Global Options

| Option | Env | Default | Use |
| --- | --- | --- | --- |
| `--profile <PROFILE>` | `PETRI_PROFILE` | saved `profile`, else the build's (`production`; `dev` in a `dev-chain` build) | Deployment: `production` or `dev` (see "Connection and wallet"). |
| `--rpc <URL>` | `PETRI_RPC_URL` | saved `rpc`, else the profile's: `https://api.mainnet-beta.solana.com`; dev `http://127.0.0.1:8899` | Solana RPC: a full RPC node (see "RPC methods"). |
| `--api <URL>` | `PETRI_API_URL` | saved `api`, else the profile's: `https://api.amoeba.farm`; dev `http://127.0.0.1:4500` | Futures API: market and oracle reads, quotes, sponsored sends (and the dev build's faucet). |
| `--keypair <PATH>` | `SOLANA_KEYPAIR` | saved `keypair`, then the Solana CLI config | The wallet that signs: a keypair file or `usb://` hardware wallet. |
| `--route <ROUTE>` | none | `auto` | Who pays the fee: `local`, `sponsored` or `auto`. |
| `--json` | none | false | Print JSON. |
| `--yes` | none | false | Send without asking for confirmation. |
| `--no-color` | none | false | No color (alias `--plain`). |

These are accepted but not listed in `--help`:

| Option | Env | Default | Use |
| --- | --- | --- | --- |
| `--deployment <PATH>` | `PETRI_DEPLOYMENT` | saved `deployment`, else the API's `/v1/config` | Deployment manifest with the program ids to trust. |
| `--solana-config <PATH>` | `SOLANA_CONFIG` | `$HOME/.config/solana/cli/config.yml` | Solana CLI config to find the keypair in. |
| `--allow-insecure-keypair` | `AMEBA_ALLOW_INSECURE_KEYPAIR` | false | Allow signing with group- or world-readable keypair files on Unix. |
| `--output <plain|json>` | `AMEBA_OUTPUT` | `plain` | Output format; `--json` is the shortcut. |

## Oracle Research Workspace

`petri` prints the CLI front door. `petri tui` opens the terminal interface,
including the oracle research and data submission workspace: Connect,
Research, Evidence, and Rewards.

```bash
petri tui
petri tui --no-update-check
petri research --help
petri research agent login --name cedar --harness Codex
petri research mcp --agent cedar --harness Codex
```

Approve agent pairings and connect the wallet rewards go to in Connect. Research
lists the products that need price pages (pick one to put it first for your
agents, add hints) and the research goals; Evidence shows submitted sources;
Rewards shows the upstream validation and reward result. No wallet key or model
API key is needed by this workspace. See [Oracle research](oracle-research.md)
for commands, local state, service configuration, and keyboard controls.

Installed preview apps update automatically after a safe idle period. Source
builds retain explicit `petri update`; the TUI never changes a source checkout.

## Config Commands

`petri config` (or `petri config show`) prints the effective profile and
cluster, RPC, API, keypair (path, public key and file permissions), deployment
and route, and where each came from (flag/env, Petri config, Solana CLI config
or default). `config set <key> <value>` saves `profile`, `rpc`, `api`,
`keypair` or `deployment` in the Petri config file; `config reset <key>`
removes it. The older names `rpc-url` and `futures-api-url` are still
accepted.

```bash
petri config set profile dev          # the dev chain on loopback
petri config set deployment /path/to/deployment.json
petri config
petri config reset profile            # back to this build's default
```

## Update Commands

The standalone updater below is included in v0.1.4 and later. Older v0.1.3
apps need a one-time manual installation of v0.1.4 or later.

### Downloaded Windows/Mac preview apps

`petri update info` reports the compiled update channel and version without a
network request. Preview-channel `petri update check` reads the official Petri
release and reports the newest supported version for this computer; it never
downloads an executable or modifies the app. Background TUI checks use the same
read-only path before the automatic idle-time install. `--no-update-check` and
`PETRI_UPDATE_CHECK=0` disable the automatic update path.

`petri update` displays the exact release and requests confirmation before download.
Use `petri update --yes` for explicit non-interactive approval and `--restart` to
reopen the terminal interface. Installed preview TUIs check at startup and every
six hours, then install and reopen after 30 idle seconds with no open form or
in-flight request. Update progress uses the existing status line.

The updater uses HTTPS to the fixed `amoeba-farm/petri` release repository, verifies
GitHub asset digests plus archive/file checksums, and rejects older versions,
wrong-platform metadata, changed releases, unsafe archive paths, links and
oversized files. These are still unsigned preview packages, not publisher-signed
or notarized releases. No downloaded installer is run, and signed installation
requirements are not bypassed.

After staging and startup validation, a private helper waits for Petri to exit,
replaces only managed app files, validates startup and restores backup files if
installation fails. Wallets, config files and agent registrations are never targets.
`petri update recover` explicitly restores the saved previous app files after an
interruption. Close other Petri windows first. Unknown concurrent file changes
stop recovery rather than being overwritten. Mac staging lives beside, never
inside, `Petri.app` / `Petri Preview.app`, preserving bundle signature checks.

`--no-fetch` and `--skip-shim` apply only to the source workflow below. Preview
mode is compiled by release maintainers using `PETRI_UPDATE_CHANNEL=preview`;
changing an environment variable at runtime cannot switch trust channels.

### Source checkouts

### `update check`

Checks the local `ameba_cli` workspace against `origin/HEAD` and the built Petri
binary. It fetches refs by default; use `--no-fetch` when you only want to
inspect local refs. A clean local branch that is ahead of `origin/HEAD` is not
reported as a remote update. If both sides contain unique commits, Petri reports
that the branch histories need a manual merge or rebase without claiming that
the worktree has uncommitted changes.

```bash
petri update check
petri --json update --no-fetch check
```

### `update`

Runs the local workspace updater. It refuses dirty worktrees, fast-forwards only
when `HEAD` can move to `origin/HEAD` safely, requires the canonical repository
origin, and merges only an exact full commit explicitly supplied through
`PETRI_TRUSTED_UPDATE_COMMIT`. Verify that commit through the trusted release
channel before setting it. The updater then rebuilds
`target/release/petri.exe`, and reinstalls the Windows release app unless
`--skip-shim` is set. If a running Petri process locks the release executable,
close it and run the update again. The updater never substitutes the debug
build; `target/debug/petri.exe` belongs exclusively to `Petri Dev` and
`petri-dev`.

```bash
PETRI_TRUSTED_UPDATE_COMMIT=<full_origin_head_commit> petri update
PETRI_TRUSTED_UPDATE_COMMIT=<full_origin_head_commit> petri update --skip-shim
```

Source checkouts update only through an explicit `petri update` command.


## MCP Commands

### Setup for users

MCP is the local connection that lets supported AI agents use Petri's public
market and oracle reads. Petri can work with Claude Code, Codex, Gemini, and
other supported agents. One-click setup currently manages the compatible local
Codex and Claude Code registrations without requiring users to edit an agent
configuration file:

```bash
petri mcp status
petri mcp enable
petri mcp repair
petri mcp disable
```

Enable registers Petri in both assistants' user-level settings:

- Codex: `${CODEX_HOME:-$HOME/.codex}/config.toml`
- Claude Code: `$HOME/.claude.json`

The registration survives a reboot and is available across projects. It does
not install a login item or an operating-system daemon. Petri disables only its
owned entries (marked `PETRI_MCP_MANAGED_BY=petri-tui-v1`) and refuses to
overwrite an unmarked MCP server already named `petri`. `petri mcp repair`
diagnoses and rebuilds the owned connection in place.

The MCP surface is read-only. Assistants start with `market.list` and
`market.show`, then `market.chart`, `futures.book`, `futures.trades` and
`futures.quote` (a price; nothing is signed). Oracle reads are
`oracle.read_months`, `oracle.read_latest`, `oracle.read_history`,
`oracle.read_catalog`, `oracle.read_sources`, `oracle.read_recipe` and
`oracle.read_candidates`. Owner-scoped reads (`futures.positions`,
`wallet.balance`, `oracle.read_collateral`, `history.read`) require an explicit
`ownerPubkey`. `wallet.address` reads the connected wallet's public key. Trades
and oracle writes sign with the user's local wallet and are run by the user in
a terminal; no MCP tool signs or sends. Research agents submit source
candidates through the research workspace (`petri research mcp`).

MCP schemas reject undeclared arguments and flag-like values before Petri is
launched, and the child process runs with `PETRI_MCP_READ_ONLY=1`.

### `mcp manifest`

Prints the agent/MCP manifest: the same tool names the stdio server lists, the
CLI form each read wraps, and the signing boundary.

```bash
petri --json mcp manifest
```

### Stdio MCP wrapper

```bash
node scripts/petri-mcp-server.mjs
node scripts/check-petri-mcp-server.mjs
```

The wrapper targets MCP protocol `2025-06-18`, exposes tool titles,
annotations and output schemas, and returns `structuredContent` plus text. The
CLI setup flow embeds it and materializes a stable per-user copy.
