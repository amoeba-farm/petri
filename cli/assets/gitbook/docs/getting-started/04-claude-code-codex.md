# Connect an AI agent

Amoeba runs a Model Context Protocol (MCP) server for AI assistants: `https://api.amoeba.farm/mcp`. Once connected, an assistant such as Claude Code or Codex can read Amoeba's futures markets, the book, quotes, positions and the index oracle, without you copying terminal output into the chat. It can also prepare an order for your wallet, but it never signs: your wallet does, and your keys never leave your computer.

> **Note:**
> The agent sees what your installed Petri build can see. Petri's release for Solana mainnet is coming; the packages published today (v0.1.9) run older code. See [Availability](https://docs.amoeba.farm/getting-started/02-cli/#availability).

## Turn it on

With Petri installed, run:

```bash
petri mcp enable
```

This registers Amoeba's server, by its URL, as `petri` in your user-level settings for both supported assistants:

| Assistant | Settings file |
|---|---|
| Claude Code | `~/.claude.json` |
| Codex | `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`) |

Restart or reload an assistant that was already open. Nothing runs on your computer, and the connection works in all your projects.

Petri changes only its own entry. It will not replace a different server that is already named `petri`. Any other assistant that supports MCP over HTTP can add `https://api.amoeba.farm/mcp` itself.

You can also use **Connect your AI agent** on the terminal interface's home screen.

## Check, repair or turn it off

```bash
petri mcp status    # is the connection healthy?
petri mcp repair    # rebuild Petri's entry in place
petri mcp disable   # turn the connection off
```

If you connected an earlier Petri, `petri mcp status` reports that a repair is available: `petri mcp repair` moves it to Amoeba's server. `petri mcp disable` turns off Petri's Codex entry and removes its Claude Code entry; it leaves your other settings alone. On Windows with WSL, running Petri inside WSL sets up the WSL assistants only, not the Windows ones.

## What you can ask

Start broad and let the assistant pick the tool:

- "Use Petri to list the Amoeba markets."
- "Quote a 100-lot RAMX long and tell me what it costs and the most it pays."
- "Show the book for NANDX."
- "Which month does the next NANDX market settle on, and what are its oracle sources?"
- "Show the positions and recent activity of this public address."
- "Prepare a 10-lot RAMX long for my wallet and give me the link to place it."

For wallet reads and orders, give the assistant the public address of your wallet.

## What the assistant can and cannot do

An assistant can:

- read markets, charts, the book, depth and recent trades;
- quote an order without sending it;
- read positions, cash, Earn, stake and history for a public address;
- read oracle months, sources and research candidates;
- prepare an order or another transaction for your wallet, with a link to place it on the site.

It cannot:

- sign anything: an order it prepared is signed by your wallet, on the site or with Petri;
- see or export your private key, seed phrase or commit-reveal secrets;
- run shell commands or write files.

To act on what the assistant found, open the link it gives you and place the order from your wallet on the site, or run the command yourself in a terminal, such as `petri long ramx 100`: Petri shows the quote and asks before it signs. See [Trading and wallet actions](https://docs.amoeba.farm/petri-cli/04-trading-and-strategy-commands/).

## Keep secrets out of chats

Never paste a seed phrase, private key, wallet file, commit-reveal recovery file or provider credential into a prompt or an MCP setting. Petri does not need them in the conversation, and the assistant cannot use them safely.

## Oracle research agents

Petri's terminal interface (`petri tui`) holds the oracle research workspace. It connects research agents through a separate login and its own MCP setup snippet under **Connect**. That connection is for gathering and submitting source evidence, not for wallet actions.
