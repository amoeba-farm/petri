# CLI

Petri is the Amoeba Farm command-line interface.

Use it when you want a repeatable way to inspect markets, check wallet state, read oracle status, plan trades, or work with JSON output.

## Install from source

Petri currently ships as a pre-production DevNet source preview. Official
macOS and Windows downloads are not published yet; they will be offered only
after their platform signatures are configured and independently verified.

The generated public source destination is `SPACE999978/amoeba-cli`. The exact
SDK and Spread Git dependencies require authorized private-repository access;
anonymous source installation is not available. With those dependencies supplied,
install [Rust with rustup](https://rustup.rs/) and run inside that checkout:

```bash
cargo install --locked --path . --bin petri
petri --version
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
