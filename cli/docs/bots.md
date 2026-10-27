# Petri bots: build in chat, share, rank

Status: contract for the bot hub (2026-10-26, dev). The user's decisions:
bots are made mostly in the chat (an agent writes them), people publish
bots and install other people's, bots carry sandboxed strategy scripts,
each running bot trades from a wallet of its own so a leaderboard can rank
real on-chain results, and nothing in the plugin's bots pages is typed
into: no weight editors, no forms (the TUI keeps its weights editor). This file is the contract between petri (`cli/`), the
futures API (`api/`) and the DeepSeek Harness plugin (`plugins/petri/`).

## The bundle

One TOML file a bot in `bots/` beside petri's config, as before. `[bot]`
gains four optional settings; every existing bundle still parses.

```toml
[bot]
id = "dip-buyer"              # letters, digits, - and _ (as before)
market = "ramx-nov26"         # as before
name = "Dip buyer"            # shown name; default: the id
about = "Buys RAMX above the index after a 3% dip, sells back at the index."  # one line
budget = 100                  # USDC its wallet gets on start; default 100
from = "dip-buyer"            # the registry id it was installed or published as

[[row]]
id = "strategy"
kind = "strategy/script"
source = '''
fn tick(m) {
  if m.holdings.len() == 0 && m.change(3600) < -0.03 {
    long(#{ usd: 10, leverage: 5 });
  }
}
'''
```

* `budget` in `[bot]` wins over a `weights` row's `budget`; a bundle with
  only the row's keeps working (the row's value is the default).
* Rows are as before (`weights`, `signal/normal-fair`,
  `signal/inventory-skew`, `strategy/ladder`) plus `strategy/script`. A bot
  may mix them: a ladder and a script in one bot is fine.
* The bundle's **hash** is the SHA-256 (lowercase hex) of its text with
  the runner's own lines left out: every line inside `[bot]` that sets
  `id`, `market`, `budget` or `from` (matching `^\s*(id|market|budget|from)\s*=`
  between `[bot]` and the next table header), lines kept with their `\n`.
  So the hash is the strategy: renaming an installed copy, moving it to
  another month or changing its budget keeps it counted for the bot it came
  from; changing a row or the script makes it another bot. Petri and the
  API hash the same bytes the same way. Petri publishes the whole file text
  (minus `from`); an install writes it back with `from` added. A table
  header is any line whose first non-blank character is `[`; `[bot]` starts
  only at a line that is exactly `[bot]` once trimmed (`[bot] # note` does
  not), and the next header ends it.

  Test vector (both sides test it): the text

  ```text
  [bot]
  id = "dip-buyer"
  market = "ramx-nov26"
  name = "Dip buyer"
  budget = 100
  from = "dip-buyer"

  [[row]]
  id = "strategy"
  kind = "strategy/script"
  source = '''
  fn tick(m) {
    let id = 1;
  }
  '''
  ```

  (each line ending in `\n`, no indentation) hashes the bytes
  `[bot]\nname = "Dip buyer"\n\n[[row]]\nid = "strategy"\nkind = "strategy/script"\nsource = '''\nfn tick(m) {\n  let id = 1;\n}\n'''\n`
  to `03f3aea639cdd13e7bb4580da1df62e8a6b659d546a3204cf0d9a4697741e90a`.

## `strategy/script`

A Rhai script, `source` inline. Petri compiles it when the bot is written
or started (a script that does not compile is the bot's `problem`, like a
row that does not build) and calls `tick(m)` on every look (every 2 s).

Sandbox: Rhai's raw engine plus its pure packages (arithmetic, logic,
strings, arrays, maps, math, iterators, function pointers: what `.len()`,
`abs`, `floor`, `"a" + x` and `for i in 0..n` need) and the functions
below; not its time package (`timestamp`) nor its language core (`sleep`,
`exit`), and no module resolver (`import` finds nothing). `print` and `debug` go nowhere. No files, no network, no clock
but `m.now`, no keys. Its limits are protections: a call to `tick` gets at
most 5,000,000 operations, so a downloaded script that loops forever can't
stop the worker every other bot runs on, and a string, array or map holds
at most 1 MiB or 100,000 items, so one can't run petri out of memory with
every ladder still up (cite these in the code). A tick that hits it, or throws, is the
bot's `waiting` note for that look, and nothing it queued that look goes
out; the bot keeps running.

`m` (read only; numbers are floats unless noted):

| field | what |
|---|---|
| `m.market`, `m.symbol` | its label (`ramx-nov26`) and product (`RAMX`) |
| `m.index` | the reference index now, in index points |
| `m.step` | index points per grid step |
| `m.low`, `m.high` | the grid's ends, in index points |
| `m.expiry_in` | seconds to expiry (int) |
| `m.now` | chain time, unix seconds (int) |
| `m.tradable` | whether the market trades now |
| `m.cash` | USDC the bot can still use: its wallet's USDC plus its free cash in the market |
| `m.budget` | its budget, USDC |
| `m.holdings` | array of `#{ side: "long"\|"short", low, high, lots (int), cost, value }`: its open positions, `value` at the index now (USDC) |
| `m.orders` | int: its resting orders |
| `m.ladder` | bool: its ladder is up |
| `m.history` | array of `#{ t (int), v }`: index samples, one a minute for the last 24 h (from the API at start, then its own reads), oldest first; `t` is wall time, unix seconds, as the API's index points |
| `m.change(secs)` | fractional change of the index over the last `secs` of wall time: against the newest sample at least that old (0.0 without one) |
| `m.prob_above(x)` | chance the index settles above `x`, normal around the index with `m.vol` a year |
| `m.vol` | the `weights` row's `vol` if the bot has one, else 0.40 |
| `m.memory` | a map the script may write; it persists between ticks of one run |

Actions (each queues one intent; a tick may queue several; they go out in
order after the tick, through petri's own order paths and checks):

| call | what (keys mirror `petri long`/`short`/`close`/`cancel`) |
|---|---|
| `long(#{...})`, `short(#{...})` | open: `usd` or `lots`, and optional `leverage`, `to`, `range` ("90-120"), `limit`, `post` (bool), `slippage` (percent) |
| `close(#{...})` | close positions: `#{}` closes all; `range` or `side` narrows it |
| `cancel()` | cancel its resting orders |
| `say(text)` | the bot's `waiting` note until the next `say` (`say("")` clears it) |

## Bot wallets

Each bot trades from a wallet of its own: `bots/.wallets/<id>.json` (a
Solana keypair file, folder 0700, file 0600, made on first start, never
printed, never deleted by petri, kept when the bundle is removed so no
funds are stranded).

* **Start** (budget B): if the bot wallet's USDC plus its free market cash
  is below B, petri moves the difference from the main wallet (creating the
  bot's USDC account if needed), and tops the bot wallet's SOL up to
  0.02 SOL from the main wallet for its fees. Then it registers the run
  with the API (below) and starts the bot.
* **Every send** of the bot is signed by its wallet; feeds that are the
  bot's own (`position:<market>`: its cash, wallet USDC, inventory,
  holdings) are read for its wallet. The market's index and book stay
  shared.
* **Stop**: its ladder and resting orders come down, its free market cash
  is withdrawn to its wallet, and all its wallet USDC goes back to the main
  wallet. Its open positions stay in its wallet (the card shows them; a
  script closes its own, or the user asks the agent to). The SOL float
  stays for the next run.
* The per-bot lock is unchanged.

## Publishing, installing, ranking: the registry (futures API)

Signatures are ed25519 over the UTF-8 message, base58 in the body; the
API verifies them against the named public keys.

* `POST /v1/bots` `{ text, author, signature }`; message
  `"amoeba bot publish v1\n" + hash`. The API parses `[bot]` `id`,
  `name`, `about`, `market` (resolved to the market address and product
  symbol; an unknown market is a 400) and which row kinds it has. The
  registry id is the bundle's `id`: the first author to publish an id owns
  it; the same author publishing again adds a version (same hash: no new
  version, 200 with the existing one); another author gets 409.
  → `{ id, version, hash, publishedAt }`.
* `GET /v1/bots?market=&sort=pnl|new|runs` → `{ bots: [BotSummary] }`;
  `market` filters by address, label or product symbol.
  `BotSummary = { id, name, about, author, market, label, symbol, kinds:
  ["script","ladder",...], version, hash, publishedAt, runs, runners,
  pnlAtoms, volumeAtoms }` (`runs`: registered wallets on any published
  version; `runners`: distinct owners; P&L and volume summed over them).
* `GET /v1/bots/:id` → `BotSummary & { text, versions: [{ version, hash,
  publishedAt }] }` (`text`: the latest version's).
* `POST /v1/bots/runs` `{ wallet, owner, bot, hash, name, market,
  walletSignature, ownerSignature }`; message `"amoeba bot run v1\n" +
  wallet + "\n" + owner + "\n" + (bot or "") + "\n" + hash`, signed by the
  bot wallet and by the owner (main wallet). Idempotent per wallet: the
  latest registration replaces the earlier one. `bot` may be null (an
  unpublished bot still gets its P&L).
  → `{ wallet, bot, counted }` (`counted`: its hash is a published version
  of `bot`, so it counts for that bot's ranking).
* `GET /v1/bots/runs/:wallet` → `{ wallet, owner, bot, hash, name, market,
  counted, pnlAtoms, equityAtoms, depositedAtoms, withdrawnAtoms,
  volumeAtoms, points: [{ t, pnlAtoms }] }`.
* `GET /v1/bots/leaderboard?days=7|30|90` (absent: all time) →
  `{ generatedAt, bots: [{ rank, id, name, author, label, symbol, kinds,
  runs, runners, pnlAtoms, returnBps, volumeAtoms }] }`, best P&L first.
  Only counted runs count. `returnBps` = P&L over what their wallets
  deposited into markets, in basis points (0 when nothing was deposited).
  A window's P&L is P&L now less P&L at the newest sample at or before its
  start (0 before the first sample).

**A wallet's P&L** (all markets, manifest and retired):
`withdrawn − deposited + equity`, where `deposited`/`withdrawn` sum the
wallet's futures `deposit`/`withdraw` events and `equity` is, in each
market where it has a trader slot, its free and reserved cash plus each
claim's executable `exit.net` (else `payoutAtIndex`, else 0) plus its
ladder inventory valued like a claim. Volume: its fills' volume as the
trader leaderboard counts it, plus its ladder fills' premium. The API
samples every registered wallet's P&L every 5 minutes into an append-only
table (never cleared) for `points` and the windows.

Atoms are decimal strings, as everywhere in the API.

## `petri bots` (headless) and `petri bots serve`

Commands (JSON on stdout as now):

* `petri bots` — the list, as before, with the new bot fields.
* `petri bots write <file|->` — validate a bundle (parse, build every row,
  compile scripts) and save it as `bots/<id>.toml` (replacing that bot's
  file; a running bot picks it up). → `{ written: ID, bot }`.
* `petri bots read <id>` — the bot's file as it is now, so an agent can
  change it and `write` it back. → `{ bot, text }`.
* `petri bots remove <id>` — stop it and delete its file (its wallet file
  stays). → `{ removed: ID }`.
* `petri bots publish <id>` → `{ published: { id, version, hash } }`; sets
  `from` in the file.
* `petri bots discover [--market M] [--sort pnl|new|runs]` → the API's
  list.
* `petri bots show <registry-id>` → the API's bot, with `text`.
* `petri bots install <registry-id> [--market M]` — writes the latest
  version as a new bundle (`from` set; `market` replaced when given; id
  made unique with `-2`, `-3`… if taken). → `{ installed: ID, bot }`.
* `petri bots leaderboard [--days N]` → the API's board.
* `petri bots new`, `petri bots set` — as before.

`serve` takes, besides the current commands: `{"start":ID,"budget":N}`
(budget optional; saved to `[bot] budget` first), `{"write":TEXT}`,
`{"remove":ID}`, `{"publish":ID}`, `{"install":{"id":REGID,"market":M?}}`.
Replies add `{"written":ID}`, `{"removed":ID}`, `{"published":{...}}`,
`{"installed":ID}`; failures are `{"note":TEXT}` as now.

A bot's JSON gains: `name`, `about`, `from`, `wallet` (its public key),
`budgetAtoms` (from `[bot]` or the row), `kinds` (row kinds), `holdings`
(as `m.holdings`, atoms as strings: `costAtoms`, `valueAtoms`), `held`
(as `m.held`, the same with `market`), `orders`,
`pnlAtoms` and `points` (from `GET /v1/bots/runs/:wallet`, read at most
every 30 s while the list is served; null until known), `counted`.
`weights` stays in the JSON for the TUI; the plugin does not show it.

## Marketplace (round 2, 2026-10-26)

The user's direction: the marketplace is the heart of bots. It is a list
you scroll: each bot stands for a thesis someone holds ("long NAND, hold,
roll each month"), some carry a verified check, and installing one has it
trade that thesis for you, persistently. Authors put their bots on GitHub.
Bots are not arbitrage-only; most will be plain thesis trades. Nobody but
their users writes bots: petri ships no example bots.

* **Thesis.** `about` is the bot's thesis, one line, shown with its name in
  the list. No new field.
* **`author`** (optional, `[bot]`): the publishing wallet's public key.
  Part of the hash (not a runner line). `petri bots publish` adds it (the
  main wallet) when the file has none, before hashing, so copies on GitHub
  carry it.
* **Product bots roll.** `market` may be a product symbol (`nandx`): the
  bot trades that product's front open month and moves to the next one when
  the front month stops trading (`m.market` changes, `m.symbol` stays). What
  it holds in the old month stays in its wallet until that month settles.
  Every look petri sweeps the bot's free cash from every market where its
  wallet has a trader slot back into its wallet (settled payouts included;
  if a final market still has the bot's unsettled claims and the program
  lets their owner settle them, petri settles them first). The market it
  trades now is left alone while it is open: its free cash already counts
  in `m.cash`, and its ladder and orders draw on it (sweeping it every look
  would only send it back and forth). The program lets anyone settle a
  final market's claims (`Settle`, `SettleMaker`), so petri does. So a hold-and-roll
  script buys again once `m.holdings` is empty in the new month.
  `m.holdings` lists the current month only; `m.held` lists every month it
  holds, the same maps plus `market`. A month (`ramx-nov26`) never moves.
* **Persistent.** Petri keeps the ids of running bots in `bots/.running`,
  one a line. A worker (the TUI's, or `serve`) starts them again when it
  starts, each under its lock (one running in another petri is left to it).
  A stop or remove takes the id out; quitting keeps it, so an installed
  thesis bot runs again whenever petri does.
* **GitHub.** A bot on GitHub is a public repository holding the bot file
  (`petri-bot.toml` at its root unless a path says otherwise).
  * `petri bots install github:OWNER/REPO[/PATH][@REF]` fetches
    `https://raw.githubusercontent.com/OWNER/REPO/REF/PATH` (REF `HEAD` and
    PATH `petri-bot.toml` by default), checks it like `write`, and saves it
    with `from = "github:OWNER/REPO/PATH@REF"`. It needs no listing. Its
    runs register with `bot` null (`from` names no registry id).
  * `petri bots publish <id> --github OWNER/REPO[/PATH][@REF]` publishes
    with `github` in the body.
  * `POST /v1/bots` takes an optional `github: "OWNER/REPO/PATH@REF"`. The
    API fetches that raw file. If its hash equals the posted text's and its
    `[bot] author` is the signing author, the listing is **verified**:
    whoever controls that GitHub repository published it. A mismatch is a
    400 saying which. Without `github` a listing is unverified.
  * `BotSummary` gains `verified` (bool) and `github` (`{ owner, repo,
    path, ref, url }` with `url` the file's github.com page, or null).
* **Top lists.** `GET /v1/bots` takes `days=7|30|90` with `sort=pnl`: the
  P&L over that window, as the leaderboard computes it, with `rank` and
  `returnBps` on each bot.
* **Kinds** are the row kinds as written (`strategy/script`,
  `strategy/ladder`, `weights`, …) in both petri's JSON and the API's.
* **Later, not built now:** hosted bots, for authors who keep their code
  private: the bot runs on a host (a VM) and followers' bot wallets let it
  trade for them, with the author paid for each follower. Nothing in this
  round depends on it.
