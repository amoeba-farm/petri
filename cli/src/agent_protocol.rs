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
            "oracleHeads": {
                "workflow": "Oracle v2, heads on pikes: sources chosen by staked AMBA. Read the board (oracle_heads, or petri oracle heads; one head with petri oracle heads show <HEAD> --product P; the user's part with petri oracle heads mine). Every product (RAMX, NANDX, BATX) is a basket of items, each observed in three channels, Spot, Contract and LTA (long-term agreements): a component is one item in one channel, with its own frozen target sentence, sources and weighted median. Contracts are quarterly (RAMX Q4 2026 expires on 2027-01-01, then Q1 2027, and so on); each settles on each item's fixed-mix blend of its three channels, weighted against each other by that quarter's mix frozen at listing (today's mixes are assumed placeholders), then on the basket's weights; a channel that would settle degraded keeps its weight at its last valid value. petri oracle heads shows per item its mix, each channel's change and the blend, and heads terms a contract's quarter mix. A source is an origin (who first published the price, in which report), not a page: copies of one report count once, and each source weighs its time-weighted delegation. The flow: stake AMBA (heads stake); someone posts a head by name and spec, delegating at least the board's minimum (heads post); others delegate to it (heads delegate); it turns active at the activation threshold with enough delegators (the proposer alone never does); anyone fills an active head with a claim and its bond (heads fill, committed and revealed in one call, --origin PUBLISHER/REPORT), and the first fill that survives its challenge window makes it live; anyone keeps a live source current the same way (heads update). Anyone challenges a claim while it is open (heads challenge <HEAD> <SEQ> --reason R --code value|off-channel|duplicate-origin|expired-observation): a claim challenge costs only bonds, the losing side's going to the winners. A source challenge (heads challenge-source: it does not exist, its spec can't be filled, its numbers are made up) is what slashes: if the source falls, its proposer loses their whole delegation to the winning challengers and every other delegator is slashed and the source takes a strike (at the strike limit it retires). The council rules on challenges. Undelegated AMBA cools, part held in escrow until the contracts it weighed settle. heads mine lists the next steps (syncCommand, prizeCommand, the rewards' claimCommand); heads settle finishes what is due once a window ends. Writes are signed by the user's local wallet in a terminal; --simulate runs them without signing."
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
