//! The machine-readable MCP/agent manifest (`petri mcp manifest`). It names
//! the same tools the stdio server (`scripts/petri-mcp-server.mjs`) lists:
//! reads that wrap `petri --json` commands, plus the resident
//! `wallet.address` action. No tool signs or sends.

use serde_json::{Value, json};

/// `(name, description, CLI form)` of every read the stdio server wraps.
const READ_TOOLS: &[(&str, &str, &[&str])] = &[
    (
        "petri.mcp_manifest",
        "Read Petri's machine-readable MCP/agent manifest.",
        &["mcp", "manifest"],
    ),
    (
        "market.list",
        "List futures markets: price, 24h change, index, 24h volume, settlement date and status.",
        &["markets"],
    ),
    (
        "market.show",
        "Read one futures market: price, index, suggested range, best prices, activity and settlement.",
        &["markets", "<MARKET>"],
    ),
    (
        "market.chart",
        "Read a futures market's price chart.",
        &["chart", "<MARKET>"],
    ),
    (
        "futures.book",
        "Read the executable depth of one range (default: the suggested range).",
        &["book", "<MARKET>"],
    ),
    (
        "futures.trades",
        "Read a futures market's recent trades.",
        &["trades", "<MARKET>"],
    ),
    (
        "futures.positions",
        "Read a wallet's futures positions with P&L, resting orders and cash per market.",
        &["positions", "<OWNER>"],
    ),
    (
        "futures.quote",
        "Price a futures order without signing (read-only).",
        &["quote", "<SIDE>", "<MARKET>", "<N>"],
    ),
    (
        "oracle.read_months",
        "Read the index oracle's months: phase, sources, settlement status.",
        &["oracle", "months"],
    ),
    (
        "oracle.read_latest",
        "Read the most recently updated oracle month.",
        &["oracle", "latest"],
    ),
    (
        "oracle.read_history",
        "Read oracle months by last update.",
        &["oracle", "history"],
    ),
    (
        "oracle.read_catalog",
        "Read every oracle period with its settlement time.",
        &["oracle", "catalog"],
    ),
    (
        "oracle.read_sources",
        "Read a month's verified source inventory.",
        &["oracle", "sources", "<MARKET>"],
    ),
    (
        "oracle.read_recipe",
        "Read a month's buckets with weights and sources.",
        &["oracle", "recipe", "<MARKET>"],
    ),
    (
        "oracle.read_candidates",
        "Read source candidates submitted by research agents.",
        &["oracle", "candidates"],
    ),
    (
        "oracle.read_collateral",
        "Read one wallet's USDC bond collateral in the oracle.",
        &["oracle", "collateral", "--owner", "<OWNER>"],
    ),
    (
        "wallet.balance",
        "Read a wallet's SOL, USDC, cash in each market and oracle bond collateral.",
        &["wallet", "<OWNER>"],
    ),
    (
        "history.read",
        "Read a wallet's futures activity.",
        &["history", "<OWNER>"],
    ),
];

pub(crate) fn mcp_manifest() -> Value {
    let mut tools: Vec<Value> = READ_TOOLS
        .iter()
        .map(|(name, description, cli)| {
            let mut command = vec!["petri", "--json"];
            command.extend_from_slice(cli);
            json!({"name": name, "description": description, "cli": command, "status": "wired_read_only"})
        })
        .collect();
    if let Some(actions) = crate::mcp_actions::manifest()["tools"].as_array() {
        tools.extend(actions.iter().cloned());
    }
    json!({
        "ok": true,
        "action": "mcp_manifest",
        "protocol": {
            "name": "petri-agent-protocol",
            "version": "v0",
            "commandStarter": "petri",
            "mcpSpecVersion": "2025-06-18",
            "transport": "stdio MCP wrapper over petri --json reads, plus the resident wallet.address action",
        },
        "clientConfig": {
            "claudeCode": {"configPath": "~/.claude.json", "scope": "user", "serverName": "petri",
                "setup": "Use Home > Connect your AI agent in the Petri TUI."},
            "codex": {"configPath": "~/.codex/config.toml", "scope": "user", "serverName": "petri",
                "setup": "Use Home > Connect your AI agent in the Petri TUI."},
        },
        "stdioMcpServer": {
            "command": ["node", "scripts/petri-mcp-server.mjs"],
            "status": "wired_v0",
        },
        "humanWorkflows": {
            "futuresTrade": {
                "workflow": "Read market.show and futures.book, then futures.quote. Trades (petri long, short, close, cancel, deposit, withdraw) sign with the user's local wallet and are run by the user in a terminal; they show the quote, what the order costs and the most it pays, and ask before sending."
            },
            "oracleWrite": {
                "workflow": "Read oracle.read_months and oracle.read_sources. Oracle writes (petri oracle collateral, evidence, sources, prints, updates, stakes) are built from the oracle program's own instruction builders, reviewed, and signed by the user's local wallet in a terminal; --simulate runs them without signing."
            },
            "sourceResearch": {
                "workflow": "Research agents connect through the Petri research workspace (petri research mcp) and submit source candidates to the futures API's candidate store; candidates carry no oracle weight."
            },
        },
        "safety": {
            "signing": "No MCP tool signs or sends a transaction. Trades and oracle writes are run by the user in a terminal.",
            "secretPolicy": "Never expose keypair JSON, .env values, signer tokens, update-claim salts or wallet credentials.",
            "signerAccess": "wallet.address resolves the locally connected public identity only.",
            "transactionControls": "No generic signer, raw packet broadcaster, shell or arbitrary-file tool.",
        },
        "tools": tools,
    })
}

pub(crate) fn render_mcp_manifest(payload: &Value) -> String {
    let tool_count = payload
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    [
        "Petri MCP manifest".to_string(),
        "protocol=petri-agent-protocol.v0".to_string(),
        format!("tools={tool_count}"),
        "Reads wrap petri --json commands; wallet.address is the resident action.".to_string(),
        "No MCP tool signs or sends; trades and oracle writes run in the user's terminal."
            .to_string(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn manifest_is_deterministic_and_read_only() {
        let manifest = mcp_manifest();
        assert_eq!(manifest, mcp_manifest());
        let tools = manifest["tools"].as_array().unwrap();
        let names: HashSet<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names.len(), tools.len(), "duplicate tool names");
        assert!(names.contains("wallet.address"));
        assert!(names.contains("oracle.read_sources"));
        for retired in [
            "staking.status",
            "operations.execute",
            "oracle.drafts.list",
            "oracle.stake.settle",
            "settlement.show",
            "futures.account",
            "wallet.collateral",
        ] {
            assert!(!names.contains(retired), "{retired}");
        }
        assert!(render_mcp_manifest(&manifest).contains(&format!("tools={}", tools.len())));
    }

    #[test]
    fn every_read_tool_parses_as_a_cli_command() {
        use clap::Parser;
        for (name, _, cli) in READ_TOOLS {
            let mut argv = vec!["petri", "--json"];
            argv.extend(cli.iter().map(|a| match *a {
                "<MARKET>" => "ramx",
                "<OWNER>" => "11111111111111111111111111111111",
                "<SIDE>" => "long",
                "<N>" | "<X>" => "1",
                other => other,
            }));
            assert!(
                crate::cli::Cli::try_parse_from(&argv).is_ok(),
                "{name}: {argv:?}"
            );
        }
    }
}
