Petri's first downloadable Devnet preview, including the combined CLI/TUI updates.

Download the Windows x64, macOS Apple silicon, or macOS Intel ZIP below and its `.sha256` checksum. Each package includes the Petri executable, its bundled SDK/Node/native-verifier runtime, app launcher or executable, installer, and licenses. Rust and Node.js are not required to launch Petri.

These preview packages are **not publisher-signed or Apple-notarized**. macOS bundles have a local ad-hoc signature only. Verify the SHA-256 before opening; macOS may require Privacy & Security → Open Anyway for this particular app. Do not disable system security globally.

Windows: extract the ZIP and open `Petri.cmd`, or run `powershell -ExecutionPolicy Bypass -File .\install-preview.ps1` in the extracted folder. The Start-menu shortcut opens the TUI. From an existing terminal, run `petri tui` or `petri --help`.

Windows also requires Microsoft's [Visual C++ v14 x64 runtime](https://aka.ms/vc14/vc_redist.x64.exe). Install it first if absent or if Windows reports `VCRUNTIME140.dll` missing. This Microsoft prerequisite is not bundled in the ZIP.

macOS: extract the ZIP and open `Petri.app` to launch Terminal, or run `bash ./install-preview.sh` in the extracted folder. The installer adds `~/.local/bin/petri` and `~/Applications/Petri Preview.app`.

To update this unsigned preview, download again or rerun the preview installer. The signed automatic updater deliberately does not accept these unsigned packages.

This is pre-production Devnet software. Runtime readiness, live data availability, transaction permissions, and unfinished features remain governed by the existing fail-closed checks. This release includes build/startup/package checks, not a new comprehensive trading qualification.

See the [README](https://github.com/amoeba-farm/petri#readme) for CLI installation and usage, and [Petri SDK](https://github.com/amoeba-farm/petri-sdk) for TypeScript and Rust integration.
