# Petri MCP Agent Protocol

Petri exposes the same market and oracle reads to humans and agents without
driving the TUI screen:

```text
Agent / MCP host
  -> scripts/petri-mcp-server.mjs (stdio)
  -> petri --json <read command>          (reads)
  -> petri mcp serve (resident)           (wallet.address)
  -> futures API / Solana RPC
```

```text
Human
  -> petri CLI / TUI
  -> futures API / Solana RPC             (reads)
  -> local wallet signs                   (trades, oracle writes)
```

The MCP surface is read-only. Trades and oracle writes sign with the user's
local wallet and run in the user's terminal, where Petri shows the review and
asks for confirmation.

## User Setup

Users can manage the connection directly from the CLI:

```bash
petri mcp status
petri mcp enable
petri mcp repair
petri mcp disable
```

The same controls are available inside the TUI:

1. Open **Home > Connect your AI agent**.
2. Select **Enable Petri MCP**.
3. Restart or reload an assistant that was already open.

The connection exposes read-only tools. Nothing it offers signs or sends a
transaction, and wallet files are never copied into assistant configuration.
Every owner-scoped read takes an explicit owner public key.

The user does not run the wrapper, edit JSON or TOML, or install an operating-
system startup service. Both `petri mcp enable` and the TUI button call the
same setup core. Enable performs one atomic setup across the user-level Codex
and Claude Code configurations:

- Codex: `${CODEX_HOME:-$HOME/.codex}/config.toml`
- Claude Code: `$HOME/.claude.json`

Petri materializes its embedded stdio wrapper at a stable per-user path:

- macOS/Linux: `${XDG_CONFIG_HOME:-$HOME/.config}/amoeba/petri/mcp/petri-mcp-server.mjs`
- Windows: `%APPDATA%\Amoeba\Petri\mcp\petri-mcp-server.mjs`, falling back to
  `%LOCALAPPDATA%`

The registrations use absolute paths for Node, the wrapper, and the current
Petri binary. They include the ownership marker
`PETRI_MCP_MANAGED_BY=petri-tui-v1`. Setup refuses to replace an existing
unmarked server named `petri`, validates both files before writing, preserves
unrelated settings, and rolls back the first write if the second write fails.
Re-enabling an owned entry is idempotent.

The clients own the stdio process lifecycle. After a reboot, Claude Code or
Codex starts the server when that client opens and stops it when the client
closes. Petri does not install `launchd`, `systemd`, a Windows scheduled task,
or any other boot daemon. A Petri process running inside WSL configures that
WSL environment; it does not change the separate Windows-host configuration.

The same TUI button becomes **Disable Petri MCP** after setup, and
`petri mcp disable` performs the same action. Disable sets the owned Codex
entry to `enabled = false` and removes only the owned Claude Code
`mcpServers.petri` entry. Missing or moved Node, wrapper, or Petri binary paths
show **Repair available**. The dedicated **Repair Petri MCP** action and
`petri mcp repair` diagnose the managed connection, rebuild owned entries in
place. A
disconnect-and-connect fallback is shown only after repair cannot complete
safely.

This repository does not include project-scoped `.codex/config.toml` or
`.mcp.json` files. User-level registration is intentional: it works across the
user's projects and avoids a repository-specific trust prompt.

## Tools

| Tool | CLI it wraps | Notes |
| --- | --- | --- |
| `petri.mcp_manifest` | `mcp manifest` | The tool list and signing boundary. |
| `market.list` | `markets` | Every market. |
| `market.show` | `markets <MARKET>` | One market: price, index, suggested range, best prices, settlement. |
| `market.chart` | `chart <MARKET> [WINDOW]` | Price chart; `window` is 1h, 24h, 7d, 30d or all. |
| `futures.book` | `book <MARKET> [--range LO-HI]` | One range's depth; default the suggested range. |
| `futures.trades` | `trades <MARKET>` | Recent trades. |
| `futures.quote` | `quote long|short <MARKET> <LOTS>` | A price with optional `leverage`, `to`, `range`, `limit`; nothing is signed. |
| `futures.positions` | `positions <OWNER>` | Requires `ownerPubkey`. |
| `oracle.read_months`, `oracle.read_latest`, `oracle.read_history` | `oracle months|latest|history` | `/v1/oracle/public-snapshot`. |
| `oracle.read_catalog` | `oracle catalog` | `/v1/oracle/catalog`. |
| `oracle.read_sources`, `oracle.read_recipe` | `oracle sources|recipe` | A month's verified source inventory. |
| `oracle.read_candidates` | `oracle candidates` | Research candidates; no oracle weight. |
| `oracle.read_collateral` | `oracle collateral --owner` | Requires `ownerPubkey`. |
| `wallet.balance` | `wallet <OWNER>` | SOL, USDC, cash per market and oracle collateral; requires `ownerPubkey`. |
| `history.read` | `history <OWNER>` | Requires `ownerPubkey`. |
| `wallet.address` | `wallet address` (resident) | The connected wallet's public key only. |

`src/mcp_actions.rs` defines the resident `wallet.address` action, which the
wrapper loads from `petri --json mcp actions` and calls over the resident
bridge (`petri mcp serve`). `src/agent_protocol.rs` builds the manifest from the
same names; `scripts/check-petri-mcp-server.mjs` checks that the manifest and
`tools/list` agree.

## Rules

- Every schema rejects undeclared arguments; identifiers that start with `-`
  are refused so no value can become a CLI flag.
- The child runs with `PETRI_MCP_READ_ONLY=1`; inside it, signing is blocked and
  only `wallet.address` may resolve the wallet's public key.
- Provider and signer locators (`SOLANA_KEYPAIR`, `SOLANA_CONFIG`, provider RPC
  URLs and keys) are stripped from the child environment, except that
  `wallet.address` keeps the configured wallet.
- Output is terminal-safe JSON; secrets (salts, keypairs, reveal-material
  paths) are filtered from results.
- No tool prepares, signs, sends, opens, closes or cancels anything. Research
  agents submit source candidates through the research workspace's own MCP
  (`petri research mcp`), which stores them in the futures API's candidate store.

## Native MCP Shape

A native server would call the same Rust read functions directly instead of
spawning `petri --json`. The tool names, schemas and the read-only boundary stay
the same.
