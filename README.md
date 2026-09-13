# Petri

<p align="center">
  <img src="assets/petri-icon.png" alt="Petri ghost logo" width="128" height="128">
</p>

Petri is Amoeba Farm's open-source CLI/TUI. It allows users discover markets, inspect the option
chain, inspect collective writer sleeves and settlements, follow oracle evidence, stake AMBA,
and connect supported workflows to agents.

The executable is `petri`; Amoeba remains the company and product name.

> [!IMPORTANT]
> Petri v0.1 is a Devnet source preview. The final business audit
> records completed replay with approved oracle differences. It does not claim
> exact economic or future-payoff equivalence. Package capability does not grant runtime permission.

## Download Petri

Download and extract the package for your computer:

| Platform | Download | Checksum |
| --- | --- | --- |
| Windows 10/11 x64 | [Petri for Windows](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-windows-x64.zip) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-windows-x64.zip.sha256) |
| macOS 14+ Apple silicon | [Petri for Mac](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-arm64.zip) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-arm64.zip.sha256) |
| macOS 14+ Intel | [Petri for Intel Mac](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-x86_64.zip) | [SHA-256](https://github.com/amoeba-farm/petri/releases/latest/download/Petri-macos-x86_64.zip.sha256) |

Open `Petri.cmd` on Windows or `Petri.app` on macOS. Petri opens in a terminal.
From an existing terminal, use `petri tui` for the interface or `petri --help` for commands.
The downloads include the SDK runtime; Rust and Node.js are not required to launch.

Windows requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe).
Install it first if it is not already installed, especially if Windows reports
`VCRUNTIME140.dll` missing. This prerequisite is supplied by Microsoft, not bundled in the ZIP.

This first Devnet preview is **not publisher-signed or Apple-notarized**. Verify
the ZIP's SHA-256 before opening it. macOS may require **System Settings → Privacy
& Security → Open Anyway** for this specific app. Do not disable system security
globally. On Windows, compare `Get-FileHash .\Petri-windows-x64.zip -Algorithm SHA256`
with the downloaded checksum; on macOS run `shasum -a 256 -c Petri-macos-arm64.zip.sha256`
(substitute `x86_64` for Intel).

## Install the CLI

Windows PowerShell (downloads and verifies the release, adds `petri` to your user
PATH, and creates a Start-menu shortcut):

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/amoeba-farm/petri/main/scripts/install-preview.ps1 -OutFile "$env:TEMP\install-petri-preview.ps1"
powershell -ExecutionPolicy Bypass -File "$env:TEMP\install-petri-preview.ps1"
```

macOS Terminal (downloads the matching architecture, verifies checksums, and
installs the app plus `~/.local/bin/petri`):

```bash
curl -fsSL https://raw.githubusercontent.com/amoeba-farm/petri/main/scripts/install-preview.sh -o /tmp/install-petri-preview.sh
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

The TUI checks in the background and shows `U` when an update is available.
Pressing `U` closes the TUI and asks before installing and reopening it. The CLI
also asks before installing; `--yes` explicitly approves a non-interactive update.
Close other Petri windows first. `petri tui --no-update-check` or
`PETRI_UPDATE_CHECK=0` disables the background check.

Preview updates trust only `amoeba-farm/petri` releases over HTTPS and require
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

The [Petri SDK](https://github.com/amoeba-farm/petri-sdk) is also open source:

```bash
npm install https://github.com/amoeba-farm/petri-sdk/releases/download/v0.2.0/ameba-sdk-0.2.0.tgz
```

Its package/import name remains `ameba-sdk`. See its README for TypeScript and Rust usage.

## Current capabilities

The selected deployment is `spread-devnet-v3-writer-terminal-lifecycle-20260906`. Supported direct CLI/TUI
operations use the pinned Rust SDK's governed bytes and finalized revalidation.
Frozen, missing, paused, mismatched, or stale state prevents signing and submission.
Unsupported operator actions remain `not_wired`. MCP public-action parity is
source-only pending integration and tests: agents can prepare a review, then
execute one exact operation only after explicit user authorization.

## Install from source

Public source lives at `amoeba-farm/petri`. Its exact SDK and Spread dependencies
are public snapshots; private-repository access is not required. For a complete
build, install Rust 1.93.1, Node.js 22–24, and native compiler prerequisites, then:

```bash
git clone https://github.com/amoeba-farm/petri.git
cd petri
node scripts/build-sdk-runtime.mjs
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
petri markets show ramx
petri contracts --market ramx
petri writers list
petri staking status
petri oracle latest ramx
petri tui
```

Use `petri help <command>` or `petri <command> --help` for the command reference
shipped by your build. Add `--json` when integrating Petri with scripts.
The TUI keeps related work in existing screens: Market Detail includes a
settlement-evidence tab, while Wallet Ledger includes Account, manager
Liquidity, collective Writers, and History tabs. Writer and staking reads use
the same byte-qualified identity boundary. Mutation grammar remains visible for
parity. Supported expert swaps, writer deposits/bids, staged close continuation,
Flat claims and transfers require a fresh active gate and initialized business
state before wallet access or preparation. Close cancellation and long claims
remain restricted by the action mask.
Manager-liquidity execution,
semantic trade routing without an authoritative route selector, and Oracle
transaction preparation remain visibly unavailable instead of being inferred.
When the TUI reports that an update is available, press `U` to close the TUI
and run the same user-safe `petri update` command. Source-checkout updates are
fail-closed: verify the full `origin/HEAD` commit through the trusted release
channel and set `PETRI_TRUSTED_UPDATE_COMMIT` to that exact 40-character commit
before installing fetched source.

| Question | Command |
| --- | --- |
| What markets are available? | `petri markets` |
| What can I trade? | `petri contracts --market ramx` |
| Are wallet changes available? | `petri config show` reports V3 package metadata; runtime permission requires fresh finalized verification |
| What collective writer sleeves are available? | `petri writers list` |
| Which writer-close custody modes are live? | Open `petri`, then choose Wallet ledger → Writers; agents can call read-only MCP tool `writers.capabilities` |
| Which writer actions does the backend describe for this wallet and sleeve? | Agents can inspect read-only MCP tool `writers.available_actions`; finalized runtime permission overrides every enabled value |
| How do I inspect a writer close? | `petri writers close-status --close-request <REQUEST>`; advancing requires current runtime permission |
| What can I redeem from staking? | `petri staking status` |
| How did a market settle? | `petri settlements show ramx <CURRENT_EXPIRY_ID>` |
| What evidence supports the oracle? | `petri oracle recipe ramx` |

## Safety model

Petri treats every backend and RPC response as untrusted input. Historical RC44
reader semantics remain separate from the exact V3 deployment identity recorded
in `release/current-governance-status.json`. Program/ProgramData bytes, the V3
controller and gate, business state, owner, action, transaction bytes and epoch
must agree with the pinned SDK. Petri never appends its own governance tail or
broadcasts arbitrary transactions. It retains the SDK message and revalidates
finalized deployment and business accounts after user approval, before typed relay.

Petri does not calculate reserves, auction results, close liabilities, settlement
values or security metrics. Missing runtime evidence fails closed. The package's
last recorded gate observation is not a permanent runtime constant.

MCP supplies inspection, semantic drafts and explicitly unsigned previews. It
cannot approve, sign or submit transactions, resolve the configured signer, or
select arbitrary local draft paths. Hosted deployment and activation are separate
publication dependencies.

Review the displayed market, expiry, premium, maximum loss, maximum payout,
settlement source, network, and wallet. Petri does not
substitute another network or fabricate live data when a required source is
unavailable.

## Configuration

Petri connects to one Amoeba service, defaulting to
`https://api.amoeba.farm`. Chain reads are derived from that base as `/rpc`;
Petri does not accept a Solana, Helius, or Photon endpoint and never receives a
private provider credential.

```bash
export AMEBA_BACKEND_URL=https://api.amoeba.farm
export SOLANA_KEYPAIR=/path/to/id.json
export AMEBA_OUTPUT=plain
```

Use `petri config show` to inspect effective non-secret configuration.
`petri config set backend-url <ORIGIN>` can save only the hosted Amoeba API
origin or an explicit loopback development origin; `petri config reset
backend-url` restores the hosted default. This is an Amoeba service setting,
not a chain-provider selector. Petri also follows the standard Solana CLI
configuration for keypair path and commitment, while deliberately ignoring its
RPC URL. It never prints keypair contents.

## MCP and connected agents

Open **Connect your AI agent** in Petri, or run `petri mcp enable`, to connect
Petri with supported AI agents. Petri can work with tools such as Claude Code,
Codex, Gemini, and other supported agents; one-click setup currently manages
the compatible local Codex and Claude Code registrations. The source MCP action
bridge shares public preparation and execution with CLI/TUI. `wallet.address`
reads the locally connected public identity; wallet-scoped actions bind an explicit
owner. `operations.execute` requires explicit approval of the exact operation ID
and plan digest. Signing remains local; keys and private salts are never sent to
an agent. Installation, client integration and tests for this change are deferred.

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

The preview release workflow builds the bundled SDK runtime and Rust binary on
Windows, Apple silicon, and Intel macOS, checks startup/configuration, and packages
checksummed downloads. Public CI performs a small source preflight. This launch
does not rerun the comprehensive trading/test suites. Release packages also carry the
project license, third-party inventory, and complete third-party license
corpus.

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

September 7 release note: the final business audit accepts explicitly documented oracle differences. It does not claim exact oracle economic equality or future payoff equality. The raw identity capture and business audit are dated evidence; every mutation still requires fresh runtime admission. Final integration checks were skipped at user request.
