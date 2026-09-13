Petri Devnet preview, including standalone release updates for Windows and macOS.

## Updated in v0.1.6

- Smaller bundled SDK runtime: include the files used by the three workers, required native libraries, resources, and licenses; omit unused dependencies and installation archives.
- Shared internal TUI cache and modularized feature state, preserving the existing user-facing content and commands.
- Retains the v0.1.5 chart-volume, Oracle reward selection, and writer-action visibility fixes below.

Focused offline worker/native-runtime checks and four critical Rust regressions passed for these source changes. Platform builds also run startup, configuration, runtime, and package checks. This is not a new full-suite or live-trading qualification. Existing v0.1.4 and v0.1.5 users can install this release with `petri update`.

## Retained fixes from v0.1.5

- Missing chart volume displays as `n/a`, not zero.
- Oracle reward forms retain the selected reward's source, claim ID, and reward type. Fields render on separate readable lines; incomplete or unsupported reward identities are rejected.
- Writer actions remain visible in small terminals, with scrolling and mouse targets matching the displayed rows.

## Download and update

Download the Windows x64, macOS Apple silicon, or macOS Intel ZIP below and its `.sha256` checksum. Each package includes the Petri executable, its bundled SDK/Node/native-verifier runtime, app launcher or executable, installer, and licenses. Rust and Node.js are not required to launch Petri.

These preview packages are **not publisher-signed or Apple-notarized**. macOS bundles have a local ad-hoc signature only. Verify the SHA-256 before opening; macOS may require Privacy & Security → Open Anyway for this particular app. Do not disable system security globally.

Windows: extract the ZIP and open `Petri.cmd`, or run `powershell -ExecutionPolicy Bypass -File .\install-preview.ps1` in the extracted folder. The Start-menu shortcut opens the TUI. From an existing terminal, run `petri tui` or `petri --help`.

Windows also requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe). Install it first if absent or if Windows reports `VCRUNTIME140.dll` missing. This Microsoft prerequisite is not bundled in the ZIP.

macOS: extract the ZIP and open `Petri.app` to launch Terminal, or run `bash ./install-preview.sh` in the extracted folder. The installer adds `~/.local/bin/petri` and `~/Applications/Petri Preview.app`.

After installing this release, use `petri update check` to check for newer versions, then `petri update` to review and confirm installation. The TUI's `U` action also offers an update when available. No Git, Rust, or source checkout is required. Close other Petri windows first. App files are replaced after Petri exits; wallets and settings are left alone. `petri update recover --restart` restores the saved previous app files and reopens Petri.

Preview updates trust the official GitHub repository over HTTPS and require matching SHA-256 digests and package identity. They are not publisher-authenticated; signed installers remain separate. Existing v0.1.3 installations need one manual download/install to obtain the new updater.

This is pre-production Devnet software. Runtime readiness, live data availability, transaction permissions, and unfinished features remain governed by the existing fail-closed checks. This release includes build/startup/package checks, not a new comprehensive trading qualification.

See the [README](https://github.com/amoeba-farm/petri#readme) for CLI installation and usage, and [Petri SDK](https://github.com/amoeba-farm/petri-sdk) for TypeScript and Rust integration.
