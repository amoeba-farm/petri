# CLI

Petri is the Amoeba Farm command-line interface.

Use it when you want a repeatable way to inspect markets, quote and send futures trades, check positions and wallet state, read oracle status, or work with JSON output.

Petri trades Amoeba's capped futures on Solana mainnet, in real USDC. Out of the box it uses the futures API at `https://api.amoeba.farm` and Solana's public mainnet RPC at `https://api.mainnet-beta.solana.com`, and trusts only Amoeba's production programs (`petri config show`). Solana's public RPC is rate-limited: if you trade or read a lot, use your own full mainnet RPC with `--rpc` or `PETRI_RPC_URL`, or save it with `petri config set rpc`. See [Configuration and wallet safety](https://docs.amoeba.farm/petri-cli/02-configuration-and-wallets/#use-your-own-rpc).

Petri's packaged mainnet release is coming; the packages published today (v0.1.9) run older code with different commands. See [Petri CLI](https://docs.amoeba.farm/getting-started/02-cli/#availability).

## Install from source

Build Petri with Rust 1.93.1; the research workspace also needs Node.js 24 on
`PATH`. Build from the `cli/` folder of the Amoeba monorepo: the futures SDK
and the oracle program crate are path dependencies in the same tree.

```bash
cargo build --release --locked --bin petri
./target/release/petri --version
```

Preview packages are not publisher-signed or Apple-notarized; verify their
checksums before opening.

## Common Commands

```bash
petri
petri markets
petri market ramx
petri book ramx
petri quote long ramx 100
petri long ramx 100
petri positions
petri close ramx 78
petri withdraw ramx-oct26 all
petri oracle
petri oracle sources ramx
petri config show
```

Use `petri help` for the current command list.

## What the CLI Is Good For

The CLI is useful for:

- listing markets such as RAMX and NANDX;
- inspecting a market before trading;
- checking positions, cash in each market and wallet state;
- reading oracle months, sources and source candidates;
- quoting and sending futures trades (`petri quote`, `long`, `short`, `close`, `cancel`, `deposit`, `withdraw`);
- bonding oracle work: evidence, source proposals and supports, opening prints and updates;
- producing JSON output for scripts and agent workflows.

Every trade and oracle write is reviewed and confirmed (or `--yes`) and signed
by your local wallet. Oracle writes accept `--simulate` to run the transaction
without signing. MCP only reads; it cannot approve, sign or submit transactions.

## Wallet Safety

CLI examples should use placeholder keys and public keys only.

Do not paste private key material, seed phrases, API keys, or keypair JSON contents into documentation, chat, or terminal history.

For the complete command reference that matches your installed version, run
`petri help <command>` or `petri <command> --help`.
