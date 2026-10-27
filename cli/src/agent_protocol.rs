//! How AI agents connect to Amoeba (`petri mcp manifest`): the futures API's
//! hosted MCP server (api/API.md, "MCP"), which `petri mcp enable` registers
//! by URL in Claude Code and Codex. Its tools are its own `tools/list`; none
//! signs. Research agents use the local research workspace instead
//! (`petri research mcp`).

use serde_json::{Value, json};

pub(crate) fn mcp_manifest(url: &str) -> Value {
    json!({
        "ok": true,
        "action": "mcp_manifest",
        "protocol": {
            "name": "petri-agent-protocol",
            "version": "v1",
            "mcpSpecVersion": "2025-06-18",
            "transport": "streamable-http",
        },
        "server": {
            "name": "petri",
            "url": url,
            "tools": "the server's tools/list: market, account, Earn, stake and oracle reads; quote; prepare_order and prepare_transaction; submit_transaction and send_transaction",
        },
        "clientConfig": {
            "claudeCode": {"configPath": "~/.claude.json", "scope": "user", "serverName": "petri",
                "entry": {"type": "http", "url": url}, "setup": "petri mcp enable"},
            "codex": {"configPath": "~/.codex/config.toml", "scope": "user", "serverName": "petri",
                "entry": {"url": url, "default_tools_approval_mode": "writes"}, "setup": "petri mcp enable"},
        },
        "humanWorkflows": {
            "futuresTrade": {
                "workflow": "Read markets_list, market_show and market_book, then quote. prepare_order builds the order for the owner's wallet: an agent holding that wallet signs it and calls submit_transaction (sponsored) or send_transaction (owner-paid); a person opens its reviewLink and places it from their own wallet. petri long, short and close do the same in a terminal with the local wallet."
            },
            "oracleWrite": {
                "workflow": "Read oracle_months and oracle_sources. Oracle writes (petri oracle collateral, evidence, sources, prints, updates, stakes) are built from the oracle program's own instruction builders, reviewed, and signed by the user's local wallet in a terminal; --simulate runs them without signing."
            },
            "sourceResearch": {
                "workflow": "Research agents connect through the Petri research workspace (petri research mcp) and submit source candidates to the futures API's candidate store; candidates carry no oracle weight."
            },
        },
        "safety": {
            "signing": "The hosted server never signs: it prepares transactions for the owner's wallet and relays the ones that wallet signed (a sponsored one gets the sponsor's signature only after the owner's).",
            "secretPolicy": "No key, keypair file, .env value or wallet credential goes to the server or comes from it.",
            "readOnlyPetri": "PETRI_MCP_READ_ONLY=1 makes petri itself read-only for an agent running its commands: no key read, no signing, no update.",
        },
    })
}

pub(crate) fn render_mcp_manifest(payload: &Value) -> String {
    [
        "Petri MCP".to_string(),
        format!(
            "server   {} (streamable HTTP)",
            payload["server"]["url"].as_str().unwrap_or_default()
        ),
        "connect  petri mcp enable (Claude Code and Codex)".to_string(),
        "signing  the owner's own wallet; the server prepares and relays, never signs".to_string(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_names_the_hosted_server_for_both_agents() {
        let url = "https://api.amoeba.farm/mcp";
        let manifest = mcp_manifest(url);
        assert_eq!(manifest, mcp_manifest(url));
        assert_eq!(manifest["server"]["url"], url);
        assert_eq!(
            manifest["clientConfig"]["claudeCode"]["entry"],
            json!({"type": "http", "url": url})
        );
        assert_eq!(manifest["clientConfig"]["codex"]["entry"]["url"], url);
        assert!(!manifest.to_string().contains("node"));
        assert!(render_mcp_manifest(&manifest).contains(url));
    }
}
