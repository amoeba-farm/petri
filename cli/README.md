# Petri

<p align="center">
  <img src="assets/petri-icon.png" alt="Petri ghost logo" width="128" height="128">
</p>

Petri is Amoeba's terminal for **capped futures** on the RAMX and NANDX
memory-chip price indexes, plus the index oracle and its research workspace.
A position is a range of the index: a long pays a fixed amount per lot (the
market's lot value, `petri market` shows it) for every point the index settles
above the range's low end, up to its high end, and a short pays the rest. You pay up front, that is the most you can
lose, and nothing is ever liquidated. Use CLI commands for markets, the book, quotes, trades,
positions, and the oracle's months, sources and evidence. `petri tui` opens
the terminal interface. [Command guide](docs/amoeba-cli-command-guide.md) ·
[Oracle research guide](docs/oracle-research.md).

The executable is `petri`; Amoeba remains the company and product name.

> [!IMPORTANT]
> A release build of this source talks to Amoeba's production deployment on
> Solana mainnet-beta (`https://api.amoeba.farm`); the dev build and
> `--profile dev` talk to the dev localnet (see [Configuration](#configuration)).
> Releases from v0.1.10 run this code. The earlier "Devnet preview" downloads
> (v0.1.9 and before) talk to a retired deployment: replace them.

## Download Petri

Download and extract the package for your computer:

| Platform | Download | Checksum |
| --- | --- | --- |
| Windows 10/11 x64 | [Petri for Windows](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-windows-x64.zip) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-windows-x64.zip.sha256) |
| macOS 14+ Apple silicon | [Petri for Mac](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-arm64.zip) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-arm64.zip.sha256) |
| Linux x86_64 | [Petri for Linux](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-linux-x86_64.tar.gz) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-linux-x86_64.tar.gz.sha256) |

Open `Petri.cmd` on Windows or `Petri.app` on macOS. Petri opens in a terminal.
From an existing terminal, use `petri tui` for the interface or `petri --help` for commands.
Rust is not required to launch; the oracle research workspace needs Node.js 24 on `PATH`.

Windows requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe).
Install it first if it is not already installed, especially if Windows reports
`VCRUNTIME140.dll` missing. This prerequisite is supplied by Microsoft, not bundled in the ZIP.

Release packages from v0.1.10 are signed: the macOS app with a Developer ID
and notarized by Apple, the Windows build with Authenticode. Verify the SHA-256
anyway. On Windows, compare `Get-FileHash .\Petri-windows-x64.zip -Algorithm SHA256`
with the downloaded checksum; on macOS run `shasum -a 256 -c Petri-macos-arm64.zip.sha256`.
The earlier preview packages, and a binary you build from source, are not
publisher-signed or Apple-notarized.

## Install the CLI

Windows PowerShell (downloads and verifies the release, adds `petri` to your user
PATH, and creates a Start-menu shortcut):

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/amoeba-farm/petri/main/cli/scripts/install-preview.ps1 -OutFile "$env:TEMP\install-petri-preview.ps1"
powershell -ExecutionPolicy Bypass -File "$env:TEMP\install-petri-preview.ps1"
```

macOS Terminal (downloads the matching architecture, verifies checksums, and
installs the app plus `~/.local/bin/petri`):

```bash
curl -fsSL https://raw.githubusercontent.com/amoeba-farm/petri/main/cli/scripts/install-preview.sh -o /tmp/install-petri-preview.sh
bash /tmp/install-petri-preview.sh
export PATH="$HOME/.local/bin:$PATH"
petri
```

You can inspect the installer before running it. Alternatively, run the included
`install-preview.ps1` or `install-preview.sh` from the extracted ZIP. Restart your
Windows terminal after installation. Add the macOS PATH line to your shell profile
to keep the command available. Update this preview by rerunning these instructions;
the signed automatic updater does not accept unsigned preview packages.

### Standalone updates

Petri v0.1.4 adds release updates to Windows and Mac preview apps.
**Older v0.1.3 downloads do not contain this feature.** Install v0.1.4 or later
once using the instructions above to receive future updates through Petri.

```bash
petri update info
petri update check
petri update --restart
```

The oracle TUI checks for updates at startup and every six hours. Installed preview
apps automatically verify, install, and reopen after 30 seconds without input,
when no form, save, or request is active. The existing status line shows progress.
`petri tui --no-update-check` or `PETRI_UPDATE_CHECK=0` disables automatic updates.
The CLI's explicit `petri update` command still asks for confirmation; `--yes`
approves it non-interactively. Source builds use the manual Git/Rust updater.

Preview updates trust only `amoeba-farm/petri` releases (the release repository
a build is made for, `PETRI_RELEASE_REPOSITORY` at build time) over HTTPS and require
matching GitHub asset SHA-256 digests, package checksums, platform, version and
update-channel metadata. They remain **unsigned/non-notarized previews**, not
publisher-authenticated releases. Signed installers and source-checkout trust
requirements are unchanged. No downloaded installer script is executed.

Updates replace only owned application files, keep recovery copies, and leave
wallets, configuration and agent registrations alone. If an update is interrupted,
run `petri update recover` to restore the saved previous app files; use `--restart`
to reopen the TUI afterward. This requires a writable, local, user-owned installation.
Mac apps must retain the name `Petri.app` or `Petri Preview.app`; staging and recovery
files are kept outside the app bundle. The most recent recovery copy is retained;
an older verified copy is removed only after the next successful update.

Source builds continue using the existing Git/Rust updater. Preview maintainers
build with `PETRI_UPDATE_CHANNEL=preview`; this is a build-time setting, not a
runtime trust override. `petri update info` reports the compiled channel.

## SDK

Futures commands use the futures-only Amoeba SDK of this monorepo
([`../sdk`](../sdk)), a path dependency in `Cargo.toml` named
`ameba_sdk_futures`. Its `futures` module re-exports the futures client crate,
including the market's instruction builders, its account decoder and the
order solver Petri runs on the local route.

Oracle commands use the oracle program's own crate, `ameba_oracle`, from this
monorepo ([`../oracle`](../oracle), the source the dev chain runs), by path:
its instruction builders, PDAs and account layouts. Petri builds it
without the localnet dev profile and passes the deployment's oracle program id
to every builder.

## Futures quickstart

A release build trades on production: the futures API `https://api.amoeba.farm`
and Solana's public RPC `https://api.mainnet-beta.solana.com`, with the
production programs only. The dev build, `cargo build --release --bin petri --features
dev-chain,dev-faucet` (`scripts/install-dev.sh` builds and installs the dev VM's
`/opt/ameba-dev/bin/petri`), defaults to the dev localnet instead (a local
validator at `http://127.0.0.1:8899` and the futures API at
`http://127.0.0.1:4500`) and adds `petri faucet`; any build reaches the dev
localnet with `--profile dev`. Program ids come from the API's `/v1/config`, or
from a deployment manifest you trust with `--deployment`.

```bash
petri wallet new                    # prints only the address
petri faucet --sol 1                # dev only: USDC and SOL (--features dev-faucet)
petri markets
petri market ramx                   # price, index, suggested range
petri book ramx
petri quote long ramx 100
petri long ramx 100                 # 100 lots at 10x on the suggested range
petri short nandx --usd 50 -x 5     # spend about $50 going short at 5x
petri long ramx 100 --limit 99      # rest a bid at 99.00
petri positions
petri cancel ramx <#>
petri close ramx <#>                # sells it and returns the cash to your wallet
```

Every order is quoted first and asks for confirmation (`--yes` skips the
prompt). Leverage (`-x`, default 10) sets the range's end on the losing side;
`--to` sets the winning side (default: the market's suggested range), or
`--range 90-120` names both. Market orders fill within `--slippage` percent
(default 2) of the quote; `--limit` rests what does not fill, and `--post`
only rests. Orders deposit any shortfall from your wallet. The markets
charge no trading fee.

With at least 0.01 SOL your wallet pays its own network fee and Petri solves
the order against the market read from chain (`--route local`). Otherwise the
futures API prepares the transaction and its sponsor pays (`--route
sponsored`); Petri checks the fee payer, the programs, the market, your token
account and the exact order before signing.

## Oracle quickstart

```bash
petri oracle                              # months: phase, sources, settlement
petri oracle sources ramx --expiry oct26  # one month's verified sources
petri oracle collateral deposit 10        # USDC bond collateral for oracle work
petri oracle evidence write --kind locator --text https://example.com/source
petri oracle sources support ramx --source <ID> --stake 1 --simulate
```

Oracle writes are reviewed, confirmed (or `--yes`) and signed by your wallet;
`--simulate` runs them without signing. See
[localnet-dev.md](docs/localnet-dev.md) and the
[command guide](docs/amoeba-cli-command-guide.md#oracle).

## Install from source

Released public source lives at `amoeba-farm/petri`, with the folders petri builds
from by path: the SDK (`sdk/`), the futures and oracle crates (`futures/`, `oracle/`)
and the governance interface. Install Rust 1.93.1, Node.js 24 (for the research
workspace), and native compiler prerequisites, then:

```bash
git clone https://github.com/amoeba-farm/petri.git
cd petri/cli
cargo build --release --locked --bin petri
./target/release/petri --version
petri --version
```

On Windows the output is `target\release\petri.exe`. Linux source builds also
require `libssl-dev`, `libudev-dev`, and `pkg-config` (Debian/Ubuntu package names).
On macOS, run `export CARGO_PROFILE_RELEASE_LTO=false` before building; this avoids
an LLVM bitcode incompatibility with Apple's system linker.

## Start here

```bash
petri
petri markets
petri market ramx
petri chart ramx 7d
petri positions
petri oracle
petri oracle sources ramx
petri tui
```

Use `petri help <command>` or `petri <command> --help` for the command reference
shipped by your build. Add `--json` when integrating Petri with scripts.
Charts print a terminal snapshot. Oracle writes are reviewed, confirmed and
signed like trades, and `--simulate` runs them without signing. Source updates
remain explicit: verify the full `origin/HEAD` commit and set
`PETRI_TRUSTED_UPDATE_COMMIT` before installing fetched source.

| Question | Command |
| --- | --- |
| What markets are available? | `petri markets` |
| What range should I trade? | `petri market ramx` (the suggested range) |
| What does the book look like? | `petri book ramx` or `petri book ramx --range 90-120` |
| What would a trade cost, and what can it pay? | `petri quote long ramx 100` |
| What do I hold, and what is it worth? | `petri positions` |
| How do I get out? | `petri close ramx <#>` |
| What is the oracle doing this month? | `petri oracle` |
| How did a market settle? | `petri market ramx-oct26`, then `petri history` |
| What evidence supports the oracle? | `petri oracle sources ramx` |
| What bond collateral do I hold? | `petri oracle collateral` |
| Is my setup healthy? | `petri status` |

## Safety model

Petri treats every API and RPC response as untrusted input. Before it signs
anything, it checks that the RPC's genesis hash is the trusted deployment's and
that the deployment's oracle and futures programs are deployed there; oracle
writes also check that the oracle config bonds in the deployment's USDC mint.
A market read from chain must belong to the trusted futures program,
settle in the deployment's USDC and read the deployment's oracle.
Oracle accounts (periods, months, sources, claims) are read from the oracle
program and checked for owner, length, layout and canonical address before
their fields are used.

Every order shows its range, leverage, average entry, cost per point, what you
pay (the most you can lose), the most it pays, and any deposit before you
confirm. Sponsored transactions are checked before signing: the sponsor must be
the fee payer; only the compute budget, the creation of your own USDC account
and the trusted futures program may be called; every step must be in this market
with your own token account; the order must be exactly the one you asked for;
and any deposit must stay within what the order can need.

Oracle writes show the month, source, amounts and every transaction they will
send before you confirm. A sealed update's value, archive URL and salt stay in
a private file until you reveal it.

MCP supplies read-only inspection. It cannot sign or submit transactions.

## Configuration

Petri talks to one Solana RPC and one futures API; program ids come from the
API's `/v1/config` or from a deployment manifest you trust. The profile sets
the defaults and what Petri trusts:

| Profile | Futures API | Solana RPC | Programs |
| --- | --- | --- | --- |
| `production` (a release build's default) | `https://api.amoeba.farm` | `https://api.mainnet-beta.solana.com` | mainnet-beta and the production oracle, futures program and governance controller only; an API naming others is refused |
| `dev` (the dev build's default) | `http://127.0.0.1:4500` | `http://127.0.0.1:8899` | what the API names, with the dev chain's own controller |

```bash
petri --profile dev markets                      # or PETRI_PROFILE=dev
petri config set profile dev                     # keep it
export PETRI_RPC_URL=https://rpc.example         # or --rpc; a full Solana RPC node
export PETRI_API_URL=https://api.amoeba.farm     # or --api
export PETRI_DEPLOYMENT=/path/to/deployment.json # optional; or --deployment
petri config set keypair ~/.config/amoeba/petri/futures-id.json
```

Run `petri config` to see the effective settings and where each came from;
`petri config set profile|rpc|api|keypair|deployment <value>` saves one. Petri
also follows the standard Solana CLI configuration for the keypair path when no
keypair is set. It never prints keypair contents.

## MCP and connected agents

Open **Connect your AI agent** in Petri, or run `petri mcp enable`, to connect
Petri with supported AI agents. Petri can work with tools such as Claude Code,
Codex, Gemini, and other supported agents; one-click setup currently manages
the compatible local Codex and Claude Code registrations. The MCP tools are
read-only: markets, charts, the book, trades, quotes, positions, balances
and history, and the oracle's months, sources and candidates. `wallet.address`
reads the locally connected public identity; owner-scoped reads take an
explicit owner. Trades and oracle writes are run by you in a terminal; keys and
private salts are never sent to an agent.

If Petri detects a broken managed connection, select **Repair Petri MCP** or run
`petri mcp repair`. Repair diagnoses the connection and rebuilds it in place
without requiring a disconnect first.

MCP setup requires Node.js on `PATH`. Petri writes only its owned user-level
registration and does not install a startup daemon.

## Build the release binary

```bash
cargo build --release --locked --bin petri
./target/release/petri --version
```

The signed release workflow (`.github/workflows/cli-release.yml` in the
monorepo, run by hand on a `vX.Y.Z` tag that matches `Cargo.toml`) builds the
production binary for Linux x86_64 (tar.gz and SHA-256), Apple silicon macOS
(Developer ID signed, notarized and stapled) and Windows x64 (Authenticode
signed), checks that it defaults to production without the dev faucet, and
opens a draft GitHub Release in the release repository (`amoeba-farm/petri` by
default). Release packages also carry the project license, third-party
inventory, and complete third-party license corpus.

## Package for macOS

This is a release-maintainer path, not a currently published download. It
requires an Apple Developer ID signing identity and a configured notarization
profile:

```bash
export PETRI_MACOS_SIGNING_IDENTITY="Developer ID Application: ..."
export PETRI_MACOS_NOTARY_PROFILE="petri-notary"
./scripts/package-petri-macos.sh
```

The packager embeds the installer inside `Petri.app` before signing, submits the
whole bundle for notarization, staples the result, verifies it, and emits the
archive and SHA-256 file. It fails closed without the required credentials.

## Package for Windows

This is also a release-maintainer path. Build the exact release files, sign the
executable and installer helpers with a publicly trusted Authenticode identity,
then package without rebuilding:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-petri-windows.ps1 -BuildOnly
# Sign target\release\petri.exe, scripts\install-petri.ps1, and
# scripts\finish-petri-windows-update.ps1.
powershell -ExecutionPolicy Bypass -File scripts/package-petri-windows.ps1 -SkipBuild
```

The Windows packager and installer reject unsigned or altered release files and
require the executable, installer, and updater helper to share one publisher
certificate.

## Documentation and security

- Start with the bundled [Amoeba documentation](assets/gitbook/README.md).
- Read [SECURITY.md](SECURITY.md) before reporting a vulnerability.
- Third-party components are listed in
  [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), with full license text in
  [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

Please do not open a public issue containing a vulnerability, wallet material,
seed phrase, API key, keypair JSON, or private endpoint credential.

## License

Petri is licensed under the [Apache License, Version 2.0](LICENSE). Amoeba and
Petri names and artwork are covered by [TRADEMARKS.md](TRADEMARKS.md).

