# Petri v0.1.19 — preview

## New in v0.1.19

- **Every oracle stage you take part in.** Beside proposing and supporting
  sources, Petri now covers the rest of a month:
  - `petri oracle challenges` lists the sources open to challenge and how
    yours stand; `petri oracle sources challenge` challenges one.
  - `petri oracle prints` shows the opening prints; `prints opening
    --accept` (and `observe --accept`) sends one and accepts it once its
    challenge window has passed, in one step.
  - `petri oracle updates` shows the Game updates; `updates commit
    --reveal` (and `observe --reveal`) commits an update and reveals it at
    once, inside its short reveal window.
  - `petri oracle stakes` shows every bond and stake of yours: locked,
    ready to settle, or settled and where it went. `stakes settle --all`
    settles everything that pays you.
- **Exact amounts, refused before sending.** Every bond and stake defaults
  to the exact amount the program takes, and Petri refuses, before
  anything is sent, what the program would refuse: a print or update
  outside its window, an unchanged value, evidence from the wrong page or
  time.
- **In DeepSeek Harness.** The Petri plugin (0.6.0) gives its agent all of
  these, and its Oracle tab shows each month's stage and your part in it.

## New in v0.1.18

- **Trading bots.** The TUI's **Bots** tab (`o`) and `petri bots` run bots
  that trade a thesis for you, each from a wallet of its own: the house
  maker's ladder with your weights, a strategy script, or both. A bot on a
  product (`nandx`) trades its front month and rolls to the next. Publish
  yours to the marketplace or install someone else's, from the marketplace
  or straight from GitHub; the leaderboard ranks them on their wallets'
  real P&L. A bot you started runs again whenever Petri does, until you
  stop it. Petri's own updates no longer wait for running bots: their
  ladders come down and go back up.
- **Research needs no Node.** The research workspace, an agent's commands
  and everything they talk to are part of `petri` itself. Your workspace,
  agents and their history carry over as they are.
- **Agents use Amoeba's hosted MCP server.** `petri mcp enable` connects
  Claude Code and Codex to `https://api.amoeba.farm/mcp` by its address;
  nothing runs on your computer for it. Agents you connected with an
  earlier Petri move to it the first time the new Petri opens.
- **Petri in DeepSeek Harness.** Oracle research and bots inside DeepSeek
  chats: install the plugin from
  [amoeba-farm/petri-deepseek](https://github.com/amoeba-farm/petri-deepseek).
  It keeps itself and Petri current.
- **Fund.** Proof reviews read right and their buttons stay on screen;
  **Send** works from Fund.
- **Windows updates.** An update replaces `petri.exe` even while another
  Petri runs from it.

## New in v0.1.17

- **NANDX is packaged chips.** From the months after November 1, 2026, NANDX
  prices new, single-die packaged 3D NAND flash chips on a contract basis,
  as RAMX prices DRAM chips; its four buckets and weights stay the same
  (1Tb TLC, 1Tb QLC, 512Gb TLC, 256Gb TLC). `petri oracle series template
  --bucket tlc_1tb` writes a packaged-chip series (die count, condition,
  grade, interface, package), and `petri oracle series check` routes NANDX
  as it routes RAMX: a direct contract price of one exact chip counts, and a
  wafer assessment, an SSD or a multi-die package is the wrong product. The
  bundled guide says the same.

## New in v0.1.16

- **Every proof in reach.** The TUI's Evidence view scrolls through all your
  proofs: click ▾ or ▴ (or Tab to it and press Enter), or turn the mouse
  wheel over the list. Each one opens and submits as before.
- **Submit as a source during Scramble.** The Evidence view's **Submit as a
  source** lists Scramble's months too, not just the months taking sources,
  as sponsoring does.
- **Finds with a series.** `petri evidence submit --candidate` sends a
  find's comparison series with it; the futures API fills in the details
  the series gives. A find without one works as before.
- **Shorter proof names.** Name a proof by any start of its digest, a
  single character too, as long as it names just one; when it names
  several, Petri lists them.
- **Long entries.** The Evidence view's fields no longer stop at 1,024
  characters, typed or pasted; the futures API's own rules answer.
- **Sponsorship split from the API.** A sponsorship gives the researcher
  the share the futures API names, so Mainnet's switch to 80/20 with its
  oracle update needs no Petri update. Without one, Petri keeps today's
  rule: half on Mainnet, 20% elsewhere.

## New in v0.1.15

- **Web proofs.** `petri evidence` proves what an HTTPS page showed you,
  including a page behind your own sign-in. `petri evidence browser <url>`
  opens your own Chrome, Edge, Brave or Chromium with Petri's own profile,
  and you sign in there yourself: Petri never asks for a password. Then
  `petri evidence prove <url> --manifest <file> --browser` makes the proof
  through Amoeba's notary (TLSNotary, MPC mode), and `show`, `verify`,
  `import`, `list` and `submit` follow. A proof discloses the request line,
  the host and the whole response; your cookies stay hidden. Anyone can
  check it later, trusting the notary's key, the web's root certificates and
  the notary's clock.
- **Proofs with your finds.** `petri evidence submit <proof> --candidate
  <find>` attaches a proof to a source find. The futures API checks it
  itself and keeps it for anyone to re-check. The TUI's Evidence view does
  the same by hand, and the messages you paste into your agent tell it to
  use browser use and let you sign in.
- **Email quotes, privately.** `petri evidence email --zk` proves the
  labelled lines of a DKIM-signed email with a zero-knowledge proof made on
  your computer: it discloses those lines, the signing domain and the
  signature time, nothing else (about half a minute, after a first run that
  also compiles the circuit for several minutes; needs `nargo`
  1.0.0-beta.18 and `bb` 3.0.3). A quote stays a quotation, not a source.
- **Not on Mainnet yet:** citing a proof in an on-chain opening, update,
  challenge or observation (`--evidence`) needs an oracle update Mainnet
  hasn't had. Until then the oracle refuses those transactions; Wayback
  archives work as before.
- **Chip buckets.** Oracle months after November 1, 2026 price RAMX on four
  DRAM chip buckets (DDR5 16Gb, DDR5 24Gb, DDR4 16Gb, DDR4 8Gb) and NANDX on
  four NAND wafer buckets. `petri oracle series template|check|propose|show`
  writes and checks a comparison series' definition, and `petri oracle
  observe` records an observation (with an archive or a web proof), signed
  by your wallet.
- **Sponsoring** a find in a month with several products sends its
  activation through the month's lookup table, so it fits Solana's
  transaction size.

## New in v0.1.14

- **A new start screen.** `petri tui` opens on an animated introduction
  while it reads the markets, so the first view opens with its data. Click
  anywhere, or press Enter or Space, to start; q or Esc quits.
- **Help pasting into your agent.** After **Copy agent setup** (and **Copy
  start message**), a pop-up says what was copied and how to hand it over in
  three steps: open a chat with your agent, paste it and send it, then
  approve the same pairing code in Connect. **Copy again** copies it once
  more.

## New in v0.1.13

- **Research in the TUI.** Pick a product, then a month, then a category,
  and see what needs a price. Each connected agent has its own page that
  follows it live: what it searches, the pages it reads, the values it
  finds and the Petri commands it sends. An agent shows as researching only
  while it really is. Give an agent a directive from any gap.
- **Oracle months by phase.** Each month shows where it is: taking sources,
  Scramble, Opening, Game, Settled. Agents get the work that phase takes:
  sources while a month takes them, opening values in Opening, updates in
  Game, claims once it settles.
- **New oracle commands:** `petri oracle sources close` ends a month's
  source window once every product has a supported source;
  `petri oracle sponsor` posts the bond for a find, and `petri oracle
  reserve withdraw` takes out what your agent's reserve holds;
  `petri oracle rewards` lists a wallet's rewards and `rewards claim` pays
  them once a month's rewards are frozen.
- **Fund and Rewards tabs:** your finds and sponsorships, your oracle
  balance, and a chart of what you have earned.
- **Faster views:** every tab opens with its data, read in the background.
- The oracle commands and the Fund and Rewards views read the chain with
  `getProgramAccounts`, which Solana's public RPC limits. Pass
  `--rpc <provider URL>` (or set `PETRI_RPC_URL`) to use your own provider's
  node.


Petri now trades Amoeba's capped futures on Solana mainnet-beta
(`https://api.amoeba.farm`) and reads the standalone index oracle and its
research workspace; `--profile dev` reaches the dev localnet. It replaces the
v0.1.x "Devnet preview" builds, which talk to a retired deployment: run
`petri update` in an installed preview, or download again below.

## Capped futures

- Trade the capped futures the website trades, on the RAMX and NANDX memory-chip indexes: one market per index and expiry. A position is a range of the index; a long pays a fixed amount per lot (the market's lot value) for every point the index settles above the range's low end, up to its high end, and a short pays the rest. What you pay up front is the most you can lose, and nothing is liquidated. No trading fee.
- `petri markets`, `market`, `book`, `trades` and `chart` read the market; `petri quote` prices an order without sending anything.
- `petri long` and `petri short` take lots or `--usd`. Leverage (`-x`, default 10) sets the range's end on the losing side, `--to` the winning side (default: the market's suggested range), or `--range LO-HI` names both. Market orders fill within `--slippage` (default 2%) of the quote; `--limit` rests what does not fill, `--post` only rests. Every order is quoted first and asks for confirmation unless `--yes`.
- `petri positions` shows positions with P&L, resting orders and cash per market. `close` sells a position at the book and returns the cash to your wallet; `cancel`, `deposit`, `withdraw` and `history` manage orders and cash. Markets settle on the oracle's index at expiry, into your cash in the market.
- Your wallet pays its own fee when it holds SOL and the order's USDC (`--route local`); otherwise the futures API's sponsor pays and Petri checks the prepared transaction before signing (`--route sponsored`).
- USDC that Amoeba's earlier product left compressed (Light tokens) counts: `petri wallet` and `petri positions` show it, and an order that needs more than your USDC token account holds spends it. The futures API moves it into your USDC account first, in transactions Petri checks (only your own compressed USDC, only into your own account) and signs with the order. `petri deposit` still moves the token account's USDC only.
- `petri wallet` shows balances, `petri wallet new` creates a signing wallet and `petri status` checks the setup. `petri tui` opens the terminal interface.

## Index oracle

- `petri oracle` reads the oracle's months, sources, catalog and research candidates from the futures API.
- Oracle writes (collateral, evidence, source proposals and supports, opening prints, Game updates, escrow settlement, council disputes) are built with the oracle program's own crate, reviewed and signed by your wallet; `--simulate` runs them without signing.

## Also retained

- Windows updates requested with `--restart`, including the TUI's `U` flow, reopen Petri in a fresh interactive console (the v0.1.9 fix).

## Download and update

Download the Windows x64, macOS Apple silicon, or macOS Intel ZIP below and its `.sha256` checksum. Each package includes the Petri executable, app launcher or executable, installer, and licenses. Neither Rust nor Node.js is required to launch Petri or for oracle research.

These preview packages are **not publisher-signed or Apple-notarized**. macOS bundles have a local ad-hoc signature only. Verify the SHA-256 before opening; macOS may require Privacy & Security → Open Anyway for this particular app. Do not disable system security globally.

Windows: extract the ZIP and open `Petri.cmd`, or run `powershell -ExecutionPolicy Bypass -File .\install-preview.ps1` in the extracted folder. The Start-menu shortcut opens the TUI. From an existing terminal, run `petri tui` or `petri --help`.

Windows also requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe). Install it first if absent or if Windows reports `VCRUNTIME140.dll` missing. This Microsoft prerequisite is not bundled in the ZIP.

macOS: extract the ZIP and open `Petri.app` to launch Terminal, or run `bash ./install-preview.sh` in the extracted folder. The installer adds `~/.local/bin/petri` and `~/Applications/Petri Preview.app`.

After installing this release, use `petri update check` to check for newer versions, then `petri update` to review and confirm installation. The TUI's `U` action also offers an update when available. No Git, Rust, or source checkout is required. Close other Petri windows first. App files are replaced after Petri exits; wallets and settings are left alone. `petri update recover --restart` restores the saved previous app files and reopens Petri.

Preview updates trust the official GitHub repository over HTTPS and require matching SHA-256 digests and package identity. They are not publisher-authenticated; signed installers remain separate. Existing v0.1.3 installations need one manual download/install to obtain the new updater.

These are preview packages; check `petri status` for the deployment identity before use. Unfinished features fail closed. Before release, Petri's checks ran in full and its trades, including ones spending compressed USDC, were exercised against Amoeba's dev deployment; these packages are built and checked for startup on GitHub's runners.

See the [README](https://github.com/amoeba-farm/petri#readme) for CLI installation and usage.
