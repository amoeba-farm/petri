# Petri v0.1.11 — preview

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

Download the Windows x64, macOS Apple silicon, or macOS Intel ZIP below and its `.sha256` checksum. Each package includes the Petri executable, app launcher or executable, installer, and licenses. Rust is not required to launch Petri; the oracle research workspace needs Node.js 24 on `PATH`.

These preview packages are **not publisher-signed or Apple-notarized**. macOS bundles have a local ad-hoc signature only. Verify the SHA-256 before opening; macOS may require Privacy & Security → Open Anyway for this particular app. Do not disable system security globally.

Windows: extract the ZIP and open `Petri.cmd`, or run `powershell -ExecutionPolicy Bypass -File .\install-preview.ps1` in the extracted folder. The Start-menu shortcut opens the TUI. From an existing terminal, run `petri tui` or `petri --help`.

Windows also requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe). Install it first if absent or if Windows reports `VCRUNTIME140.dll` missing. This Microsoft prerequisite is not bundled in the ZIP.

macOS: extract the ZIP and open `Petri.app` to launch Terminal, or run `bash ./install-preview.sh` in the extracted folder. The installer adds `~/.local/bin/petri` and `~/Applications/Petri Preview.app`.

After installing this release, use `petri update check` to check for newer versions, then `petri update` to review and confirm installation. The TUI's `U` action also offers an update when available. No Git, Rust, or source checkout is required. Close other Petri windows first. App files are replaced after Petri exits; wallets and settings are left alone. `petri update recover --restart` restores the saved previous app files and reopens Petri.

Preview updates trust the official GitHub repository over HTTPS and require matching SHA-256 digests and package identity. They are not publisher-authenticated; signed installers remain separate. Existing v0.1.3 installations need one manual download/install to obtain the new updater.

These are preview packages; check `petri status` for the deployment identity before use. Unfinished features fail closed. Before release, Petri's checks ran in full and its trades, including ones spending compressed USDC, were exercised against Amoeba's dev deployment; these packages are built and checked for startup on GitHub's runners.

See the [README](https://github.com/amoeba-farm/petri#readme) for CLI installation and usage.
