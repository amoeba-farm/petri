# Connect an AI agent

Petri includes a Model Context Protocol (MCP) server. Once connected, a local AI assistant such as Claude Code or Codex can read Amoeba's futures markets, the book, quotes, positions and the index oracle through Petri, without you copying terminal output into the chat. The connection only reads: it never signs or sends a transaction, and your keys never leave your computer.

> **Note:**
> The agent sees what your installed Petri build can see. Petri's release for Solana mainnet is coming; the packages published today (v0.1.9) run older code. See [Availability](https://docs.amoeba.farm/getting-started/02-cli/#availability).

## Turn it on

You need Petri installed and Node.js on your `PATH`. Then run:

```bash
petri mcp enable
```

This registers a server named `petri` in your user-level settings for both supported assistants:

| Assistant | Settings file |
|---|---|
| Claude Code | `~/.claude.json` |
| Codex | `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`) |

Restart or reload an assistant that was already open. From then on the assistant starts Petri when it opens and stops it when it closes. Nothing runs in the background, and the connection works in all your projects.

Petri changes only its own entry. It will not replace a different server that is already named `petri`. Other MCP clients are not set up automatically.

You can also use **Connect your AI agent** on the terminal interface's home screen.

## Check, repair or turn it off

```bash
petri mcp status    # is the connection healthy?
petri mcp repair    # rebuild Petri's entry in place
petri mcp disable   # turn the connection off
```

If you move Petri or Node.js, `petri mcp status` reports that a repair is available. `petri mcp disable` turns off Petri's Codex entry and removes its Claude Code entry; it leaves your other settings alone.

If Node.js is installed somewhere other than your `PATH`, set `PETRI_MCP_NODE_BIN` to its full path before running `petri mcp enable`. On Windows with WSL, running Petri inside WSL sets up the WSL assistants only, not the Windows ones.

## What you can ask

Start broad and let the assistant pick the tool:

- "Use Petri to list the Amoeba markets."
- "Quote a 100-lot RAMX long at 5x and tell me what it costs and the most it pays."
- "Show the book for NANDX on the range 90–120."
- "Explain the NANDX oracle recipe and which month the next market settles on."
- "Show the positions, P&L and recent activity of this public address."

For wallet reads, give the assistant the public address you want it to read. It can also ask Petri for the public address of your configured signer.

## What the assistant can and cannot do

Through Petri, an assistant can:

- read markets, charts, the book and recent trades;
- quote an order without sending it;
- read positions, cash, balances and history for a public address;
- read oracle months, sources, recipes, research candidates and bond collateral;
- ask Petri for the public address of your configured wallet.

Each tool runs one of Petri's read commands with `--json`; `petri mcp manifest` lists them.

It cannot:

- sign or send anything: trades, cash moves and oracle writes;
- see or export your private key, seed phrase or commit-reveal secrets;
- choose which wallet signs, or change your network or service settings;
- run shell commands, send arbitrary transactions or write files.

To act on what the assistant found, run the command yourself in a terminal, such as `petri long ramx 100`. Petri shows the quote and asks before it signs. See [Trading and wallet actions](https://docs.amoeba.farm/petri-cli/04-trading-and-strategy-commands/).

## Keep secrets out of chats

Never paste a seed phrase, private key, wallet file, commit-reveal recovery file or provider credential into a prompt or an MCP setting. Petri does not need them in the conversation, and the assistant cannot use them safely.

## Oracle research agents

Petri's terminal interface (`petri tui`) holds the oracle research workspace. It connects research agents through a separate login and its own MCP setup snippet under **Connect**. That connection is for gathering and submitting source evidence, not for wallet actions.
