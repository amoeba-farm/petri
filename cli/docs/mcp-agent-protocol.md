# Petri MCP Agent Protocol

AI agents reach Amoeba through its hosted MCP server, the futures API's
`/mcp` (api/API.md, "MCP"), not through a server petri runs:

```text
Agent / MCP host
  -> https://api.amoeba.farm/mcp           (streamable HTTP, MCP 2025-06-18)
  -> the futures API's own routes          (reads, quotes, prepare, relay)
```

```text
Human
  -> petri CLI / TUI, or the site
  -> futures API / Solana RPC              (reads)
  -> their own wallet signs                (trades, oracle writes)
```

## Setup

`petri mcp enable` registers the server as `petri`, by URL, in the user-level
settings of Claude Code (`~/.claude.json`: `{ "type": "http", "url": … }`) and
Codex (`$CODEX_HOME/config.toml`: `url`, `default_tools_approval_mode =
"writes"`). The URL is `<api>/mcp` for the futures API petri uses
(`https://api.amoeba.farm` by default, `http://127.0.0.1:4500` for the dev
build). `petri mcp status`, `repair` and `disable` look after that entry
only: a URL entry for an Amoeba API's `/mcp`, or the local Node server's
petri set up before 2026-10-26 (marked `PETRI_MCP_MANAGED_BY=petri-tui-v1`),
which `repair` replaces, deleting that server's files. Any other `petri`
entry is left alone, and nothing is written when a settings file does not
parse.

`petri mcp manifest` prints the server's URL, the entry each assistant gets
and the signing boundary.

## Tools and signing

The tools are the server's `tools/list` (api/API.md lists each with its
route). None signs:

- reads: markets, book, depth, trades, candles, the index, quotes; a
  wallet's positions, history, USDC, Earn, stake and portfolio; the oracle's
  months, sources and candidates;
- `prepare_order` and `prepare_transaction` build a transaction for the
  owner's wallet, with `howToSign` and a `reviewLink` (the site's ticket,
  where a person places the order from their own wallet);
- `submit_transaction` (sponsored: the sponsor co-signs only after the
  owner's signature, and only the prepared message) and `send_transaction`
  (owner-paid) relay what the owner's wallet signed.

An agent that holds the owner's key signs what it prepared; otherwise the
person does, on the site or with petri (`petri long`, `short`, `close`, …,
which show the quote and ask before signing).

## Petri under an agent

An agent may also run petri's commands itself. `PETRI_MCP_READ_ONLY=1` makes
petri read-only for it: no key read, no signing, no update.

## Research agents

Research agents connect to the owner's local research workspace instead
(`petri research mcp`, docs/oracle-research.md): pairing, runs and source
candidates, no wallet actions.
