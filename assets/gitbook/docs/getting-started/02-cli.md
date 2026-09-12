# CLI

Petri is the Amoeba Farm command-line interface.

Use it when you want a repeatable way to inspect markets, check wallet state, read oracle status, plan trades, or work with JSON output.

## Install from source

Petri is a pre-production Devnet preview. Windows and macOS downloads and CLI
installers are at [amoeba-farm/petri](https://github.com/amoeba-farm/petri#readme).
Preview packages are not publisher-signed or Apple-notarized; verify their
checksums before opening. The public SDK and native dependencies need no private
repository access. To build a complete source checkout, install Rust 1.93.1 and
Node.js 22–24, then run:

```bash
node scripts/build-sdk-runtime.mjs
cargo build --release --locked --bin petri
./target/release/petri --version
```

Use a dedicated testing wallet and do not use production funds.

## Common Commands

```bash
petri
petri markets
petri markets show ramx
petri contracts --market ramx
petri oracle recipe ramx
petri config show
```

Use `petri help` for the current command list.

## What the CLI Is Good For

The CLI is useful for:

- listing markets such as RAMX and NANDX;
- inspecting a market before trading;
- checking wallet and ledger state;
- reading source recipes and oracle status;
- preparing the same bounded option routes used by the web product;
- producing JSON output for scripts and agent workflows.

V3 package capability does not grant runtime permission. Frozen, uninitialized
or paused state prevents supported wallet operations. MCP only inspects or
drafts; it cannot approve, sign or submit transactions.

## Wallet Safety

CLI examples should use placeholder keys and public keys only.

Do not paste private key material, seed phrases, API keys, or keypair JSON contents into documentation, chat, or terminal history.

For the complete command reference that matches your installed version, run
`petri help <command>` or `petri <command> --help`.
