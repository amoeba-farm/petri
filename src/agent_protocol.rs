use serde_json::Value;

const MCP_MANIFEST_JSON: &str = r#"{
  "action": "mcp_manifest",
  "clientConfig": {
    "claudeCode": {
      "configPath": "~/.claude.json",
      "lifecycle": "Claude Code starts the Petri stdio server when Claude Code opens.",
      "scope": "user",
      "serverName": "petri",
      "setup": "Use Home > Connect your AI agent in the Petri TUI."
    },
    "codex": {
      "configPath": "~/.codex/config.toml",
      "lifecycle": "Codex starts the Petri stdio server when Codex opens.",
      "scope": "user",
      "serverName": "petri",
      "setup": "Use Home > Connect your AI agent in the Petri TUI."
    }
  },
  "currentRelease": {
    "packageWriteCapable": true,
    "governanceGeneration": 3,
    "governanceGate": "Cdym9p7FvtxEAjF8XuCqSrishB7LmBDXaZGDMMgWczu",
    "governanceStatus": "Active",
    "governanceEpoch": "9",
    "observationOnly": true,
    "observedAt": "2026-09-06T23:17:03.694Z",
    "runtimePermission": "requires-finalized-verification",
    "historicalReaderSemanticRelease": "v0.1.0-rc.44",
    "programId": "2jVQSPny9eFoaG1ZWoJVAezQ5VgqJtF8rQCQXMktuBVw",
    "sdkCommit": "a21b324a7a64da87046c7650355b80ea20c47540",
    "writeCompatibility": "governance-gate-v1",
    "writeErrorCode": "CURRENT_PROGRAM_WRITE_ABI_UNAVAILABLE"
  },
  "humanWorkflows": {
    "writerClose": {
      "cancelAvailability": "Writer-close cancellation is unavailable in the current action-mask boundary; the preserved CLI grammar is not an executable fallback.",
      "capabilityTool": "writers.capabilities",
      "executionBoundary": "MCP and direct CLI/TUI require finalized V3 permission and initialized business state. MCP execution additionally binds explicit approval to the exact saved operation, plan digest and owner.",
      "availabilityBoundary": "Capability, hot/cold custody, exact owner+sleeve action masks, previews, and status remain read-only evidence. The finalized runtime gate overrides every dynamic enabled value.",
      "actionMaskTool": "writers.available_actions",
      "releaseContract": {
        "governanceGeneration": 3,
        "governanceStatus": "Active",
        "sdkCommit": "a21b324a7a64da87046c7650355b80ea20c47540",
        "writeCompatibility": "governance-gate-v1",
        "writeErrorCode": "CURRENT_PROGRAM_WRITE_ABI_UNAVAILABLE"
      },
      "mutationStatus": "requires_finalized_runtime_permission",
      "previewTool": "writers.close_preview",
      "statusTool": "writers.close_status",
      "workflow": "Inspect exact owner+sleeve availability and close preview. Use writers.close_begin.prepare or writers.close_advance.prepare, review the exact stage, then explicitly authorize operations.execute. Inspect close_status after each stage. Cancellation stays unavailable where the public CLI lacks current native admission."
    }
  },
  "nativeMcpServer": {
    "crateOrPackage": "petri-mcp",
    "status": "planned",
    "v0Implementation": "use scripts/petri-mcp-server.mjs as the stdio wrapper",
    "v1Implementation": "call the same protocol core functions used by CLI/TUI"
  },
  "ok": true,
  "protocol": {
    "commandStarter": "petri",
    "mcpSpecVersion": "2025-06-18",
    "name": "petri-agent-protocol",
    "transport": "CLI JSON wrapper first; shared core extraction before a native MCP server",
    "version": "v0"
  },
  "safety": {
    "defaultAgentMode": "inspect_prepare_review_authorized_execute_recover",
    "managedSigningBoundary": "Managed hosted execution has not been activated. Local V3 capability does not authorize managed signing.",
    "secretPolicy": "Never expose keypair JSON, .env values, Helius keys, signer tokens, or wallet credentials.",
    "signerAccess": "wallet.address resolves the locally connected public identity. Preparation binds ownerPubkey. Only exact approved operation execution may use the local signer; keys and private salts never leave Petri.",
    "signing": "MCP and CLI/TUI share SDK-governed execution after finalized revalidation. operations.execute requires explicit user authorization, ownerPubkey, operationId, preparedPlanDigest and approved=true.",
    "transactionControls": "No generic signer, raw packet broadcaster, shell or arbitrary-file tool. Preparation never signs. Execution is scoped to one reviewed operation. Status recovery never replays.",
    "walletActionAvailability": "Capability and writers.available_actions are read-only evidence. The finalized runtime gate overrides every dynamic enabled value and neither surface can authorize preparation, signing, or submission.",
    "unreleasedWriterActions": "Public action parity is a source-only candidate pending integration and tests; every wallet mutation requires fresh native admission and exact user approval."
  },
  "stdioMcpServer": {
    "command": [
      "node",
      "scripts/petri-mcp-server.mjs"
    ],
    "implementation": "JSON-RPC stdio wrapper that exposes these tools and spawns petri --json commands",
    "status": "wired_v0"
  },
  "tools": [
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "capabilities"
      ],
      "description": "Read historical writer capability and separate hot/cold Light-account evidence. Finalized runtime permission and action admission are required for every wallet change.",
      "name": "writers.capabilities",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "available-actions",
        "--owner",
        "<owner_pubkey>",
        "--sleeve",
        "<sleeve_pubkey>"
      ],
      "description": "Read the exact current owner+sleeve 22-action availability mask as inspection evidence only. Every wallet change needs fresh native and SDK admission.",
      "name": "writers.available_actions",
      "status": "wired_read_only"
    },
    {
      "cli": ["petri", "--json", "writers", "liquidity", "--owner", "<owner_pubkey>", "--sleeve", "<sleeve_pubkey>", "--series-index", "<0..19>"],
      "description": "Read exact writer liquidity, custody, reserve, inventory and frozen budgets for an explicit actor and series. No wallet resolution, preparation or signing.",
      "name": "writers.liquidity",
      "status": "wired_read_only"
    },
    {
      "cli": ["petri", "--json", "writers", "refunds", "--owner", "<owner_pubkey>", "--limit", "<1..32>"],
      "description": "Discover historical auction refund rows for an explicit owner; optional cursor resumes the bounded inventory. This tool never prepares or submits refunds.",
      "name": "writers.refunds",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "mcp",
        "manifest"
      ],
      "description": "Read Petri's machine-readable MCP/agent manifest.",
      "name": "petri.mcp_manifest",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "markets"
      ],
      "description": "List user-facing Petri markets.",
      "name": "market.list",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "markets",
        "show",
        "<market_id>"
      ],
      "description": "Open one market summary.",
      "name": "market.show",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "markets",
        "status",
        "<market_id>"
      ],
      "description": "Read the compact live status for one market.",
      "name": "market.status",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "markets",
        "print",
        "<market_id>"
      ],
      "description": "Read the latest oracle print for one market.",
      "name": "market.print",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "markets",
        "chart",
        "<market_id>",
        "--static"
      ],
      "description": "Read non-interactive market or expiry chart history.",
      "name": "market.chart",
      "status": "wired_static"
    },
    {
      "cli": [
        "petri",
        "--json",
        "contracts",
        "--market",
        "<market_id>"
      ],
      "description": "Read a bounded option chain with prices, depth, availability, risk, freshness, and no-live-contract issues.",
      "name": "contracts.chain",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "contracts",
        "--market",
        "<market_id>",
        "--all"
      ],
      "description": "Find factual call or put candidates by quote, depth, and on-chain availability without opening an order.",
      "name": "contracts.find",
      "status": "wired_mcp_read"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "recipe",
        "ramx",
        "--query",
        "<optional_query>"
      ],
      "description": "Read the RAMX-MOD source tree, row weights, and source pins from the active backend oracle recipe.",
      "name": "oracle.read_recipe",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "state"
      ],
      "description": "Read spread-owned oracle totals and market summaries from the Amoeba API.",
      "name": "oracle.read_state",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "markets",
        "<optional_market_id>"
      ],
      "description": "List spread-owned oracle markets, or inspect one market when a market id is provided.",
      "name": "oracle.list_markets",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "latest",
        "<optional_market_id>"
      ],
      "description": "Read the latest spread-owned oracle month and settlement.",
      "name": "oracle.read_latest",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "history",
        "<optional_market_id>"
      ],
      "description": "Read spread-owned oracle settlement history.",
      "name": "oracle.read_history",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "settlements",
        "show",
        "<market_id>",
        "<expiry_id>"
      ],
      "description": "Read one spread settlement record through the Amoeba API.",
      "name": "settlement.show",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "settlements",
        "check",
        "<market_id>",
        "<expiry_id>"
      ],
      "description": "Read spread settlement oracle preflight for one market expiry.",
      "name": "settlement.check",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "settlements",
        "oracle",
        "<market_id>",
        "<expiry_id>"
      ],
      "description": "Read the spread oracle package/signing state for one settlement.",
      "name": "settlement.read_oracle",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "wallet",
        "balance",
        "<owner_pubkey>"
      ],
      "description": "Read SOL, USDC, and AMBA balances for a wallet.",
      "name": "wallet.balance",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "wallet",
        "collateral",
        "--owner",
        "<owner_pubkey>"
      ],
      "description": "Read current trading-collateral state and the next safe action for a wallet.",
      "name": "wallet.collateral",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "staking",
        "status",
        "--owner",
        "<owner_pubkey>"
      ],
      "description": "Read verified AMBA/sAMBA staking, activation, reward, and unstaking status without changing it.",
      "name": "staking.status",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "list"
      ],
      "description": "Read the current global collective-writer sleeve catalog. This is not a wallet-owned position view.",
      "name": "writers.list",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "show",
        "--sleeve",
        "<sleeve_pubkey>"
      ],
      "description": "Read one current collective-writer sleeve and its staged state.",
      "name": "writers.show",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "policy-audit",
        "--sleeve",
        "<sleeve_pubkey>"
      ],
      "description": "Read the active writer policy and immutable audit hashes for one sleeve.",
      "name": "writers.policy_audit",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "close-preview",
        "--sleeve",
        "<sleeve_pubkey>",
        "--amount",
        "<flat_par_atoms>",
        "--minimum-withdrawal",
        "<minimum_usdc_atoms>"
      ],
      "description": "Inspect a staged-close preview and exact basket/withdrawal bounds. A preview is not authorization; prepare and explicitly approve the exact next stage separately.",
      "name": "writers.close_preview",
      "status": "wired_unsigned_preview"
    },
    {
      "cli": [
        "petri",
        "--json",
        "writers",
        "close-status",
        "--close-request",
        "<close_request_pubkey>"
      ],
      "description": "Read one staged writer-close request and the next permitted stage. MCP never advances it; direct CLI/TUI requires current action-specific native admission.",
      "name": "writers.close_status",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "liquidity",
        "--owner",
        "<owner_pubkey>"
      ],
      "description": "Read owner-bound manager-liquidity positions; the current hosted route fails closed when it cannot prove a non-empty row's canonical Market.",
      "name": "liquidity.positions",
      "status": "wired_read_fail_closed_upstream_limited"
    },
    {
      "cli": [
        "petri",
        "--json",
        "history",
        "<owner_pubkey>"
      ],
      "description": "Read authoritative untyped wallet history and recent wallet activity; category filters are unavailable.",
      "name": "history.read",
      "status": "wired_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "sources",
        "propose",
        "<sku_or_row>",
        "--category",
        "<public_source_type>",
        "--locator",
        "<url>",
        "--definition",
        "<definition>",
        "--stake",
        "<amount>"
      ],
      "description": "Create a local source proposal draft without submitting it.",
      "name": "source.draft_submission",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "sources",
        "support",
        "<sku_or_row>",
        "--stake",
        "<amount>"
      ],
      "description": "Create a local source support/backing draft without submitting it.",
      "name": "source.draft_support",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "sources",
        "challenge",
        "<sku>",
        "--reason",
        "<reason>",
        "--archive-url",
        "<url>",
        "--stake",
        "<amount>",
        "[--comparison-source",
        "<source_id>]"
      ],
      "description": "Create a local source/update challenge draft; source differentiation challenges use --comparison-source.",
      "name": "source.draft_challenge",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "prints",
        "opening",
        "<sku>",
        "--raw-value",
        "<value>",
        "--timestamp",
        "<iso8601>",
        "--archive-url",
        "<https://web.archive.org/web/...>",
        "--canonical-locator",
        "<url_or_locator>",
        "--source-definition",
        "<definition>",
        "--stake",
        "<amount>"
      ],
      "description": "Create a pending opening claim with https://web.archive.org/web/<14-digit UTC>/<exact canonical URL> (max 384 UTF-8 bytes); capture time must equal source time and the spread program derives the evidence hash.",
      "name": "opening.draft_print",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "prints",
        "challenge",
        "<sku>",
        "--reason",
        "<reason>",
        "--corrected-value",
        "<value>",
        "--timestamp",
        "<iso8601_or_unix>",
        "--archive-url",
        "<https://web.archive.org/web/...>",
        "--canonical-locator",
        "<url_or_locator>",
        "--source-definition",
        "<definition>",
        "--stake",
        "<amount>"
      ],
      "description": "Challenge one pending opening claim with https://web.archive.org/web/<14-digit UTC>/<exact canonical URL> (max 384 UTF-8 bytes); no caller evidence hash is accepted.",
      "name": "opening.draft_challenge",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "updates",
        "commit",
        "--source",
        "<source_id>",
        "--claim-id",
        "<hex_or_draft_id>",
        "--commit-hash",
        "<hex32>",
        "--stake",
        "<amount>"
      ],
      "description": "Create a local hidden source-local update commitment draft.",
      "name": "update.commit_claim",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "updates",
        "reveal",
        "--source",
        "<source_id>",
        "--claim-id",
        "<hex_or_draft_id>",
        "--prior-state",
        "<current_state>",
        "--raw-value",
        "<value>",
        "--timestamp",
        "<source_time>",
        "--archive-url",
        "<wayback_url>",
        "--secret-salt",
        "<hex32>"
      ],
      "description": "Create a local reveal draft for a hidden source-local update commitment using archive evidence.",
      "name": "update.reveal_claim",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "updates",
        "expire",
        "--source",
        "<source_id>",
        "--claim-id",
        "<claim_id>",
        "--claimant",
        "<claimant_address>"
      ],
      "description": "Settle the bond of a claimant-scoped update commitment that missed its reveal deadline.",
      "name": "update.expire_commitment",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "updates",
        "challenge",
        "<sku>",
        "--reason",
        "<reason>",
        "--archive-url",
        "<url>",
        "--claim-id",
        "<hex_or_draft_id>",
        "--claimant",
        "<claimant_address>",
        "--corrected-value",
        "<positive_state>"
      ],
      "description": "Challenge one revealed claimant-scoped update claim using its exact claim id, original claimant address, backend-safe positive integer corrected state, and archive evidence.",
      "name": "update.challenge_claim",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "emergency",
        "commit",
        "--dispute-id",
        "<hex32>",
        "--commit-hash",
        "<hex32>",
        "--lock-amount",
        "<amount>"
      ],
      "description": "Create a local hidden emergency vote commitment draft.",
      "name": "emergency.commit_vote",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "emergency",
        "reveal",
        "--dispute-id",
        "<hex32>",
        "--choice",
        "<choice>",
        "--salt",
        "<hex32>"
      ],
      "description": "Create a local emergency vote reveal draft.",
      "name": "emergency.reveal_vote",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "rewards",
        "claim",
        "--kind",
        "<source_proposer|source_support|opening|update>"
      ],
      "description": "Create a local current USDC oracle reward claim draft without submitting it.",
      "name": "oracle.reward.claim",
      "status": "wired_local_draft"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "stakes",
        "settle",
        "--kind",
        "<listing-bond|support-stake|source-challenge|opening-claim|opening-challenge|update-claim|update-challenge|emergency-vote>",
        "[--subject-pda",
        "<canonical_record_from_oracle_latest>]",
        "[--source-id",
        "<listing_bond_source_only>]"
      ],
      "description": "Create a permissionless terminal stake/bond settlement draft; the spread program derives refund or slash and the caller cannot choose the disposition.",
      "name": "oracle.stake.settle",
      "status": "wired"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "amba",
        "deposit",
        "--amount",
        "<amount>"
      ],
      "description": "Create a local AMBA deposit draft for oracle voting custody.",
      "name": "amba.deposit",
      "status": "wired_local_draft"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "amba",
        "withdraw",
        "--amount",
        "<amount>"
      ],
      "description": "Create a local AMBA withdraw draft for available oracle voting custody.",
      "name": "amba.withdraw",
      "status": "wired_local_draft"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "drafts",
        "list",
        "--limit",
        "<optional_limit>"
      ],
      "description": "List locally queued semantic Oracle drafts without preparing, signing, or sending a transaction.",
      "name": "oracle.drafts.list",
      "status": "wired_semantic_read_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "drafts",
        "validate",
        "<selector>"
      ],
      "description": "Validate selected semantic oracle drafts without preparing, signing, or sending a transaction.",
      "name": "oracle.drafts.validate",
      "status": "wired_semantic_validation_only"
    },
    {
      "cli": [
        "petri",
        "--json",
        "oracle",
        "drafts",
        "show",
        "latest"
      ],
      "description": "Show one semantic oracle draft without preparing, signing, or sending a transaction.",
      "name": "oracle.drafts.show",
      "status": "wired_semantic_read_only"
    }
  ]
}"#;

pub(crate) fn mcp_manifest() -> Value {
    let mut manifest: Value =
        serde_json::from_str(MCP_MANIFEST_JSON).expect("bundled MCP manifest JSON must be valid");
    manifest["currentRelease"]["packageBuildIdentity"] =
        ameba_sdk::current_sdk_package_build_identity_v1().unwrap_or(Value::Null);
    manifest["currentRelease"]["packageWriteCapable"] =
        Value::Bool(crate::current_release::require_current_write_release().is_ok());
    manifest["currentRelease"]["actionSchema"] = crate::writer_action_mask::action_schema();
    // Dated gate observations never become an agent's live permission predicate.
    let release: Value =
        serde_json::from_str(include_str!("../release/current-governance-status.json"))
            .expect("release metadata");
    manifest["currentRelease"]["governanceStatus"] =
        release["runtimePermission"]["gateStatus"].clone();
    manifest["currentRelease"]["observedAt"] = release["runtimePermission"]["observedAt"].clone();
    manifest["currentRelease"]["governanceEpoch"] =
        release["liveGovernance"]["pinnedObservation"]["epoch"].clone();
    manifest["humanWorkflows"]["writerClose"]["releaseContract"]["governanceStatus"] =
        release["runtimePermission"]["gateStatus"].clone();
    if let Some(tools) = manifest["tools"].as_array_mut() {
        tools.push(serde_json::json!({"name":"operations.list","description":"Inspect local recovery references for an explicit owner; never resolves a signer or resubmits.","cli":["petri","--json","operations","list","--owner","<OWNER>"],"status":"wired_read_only"}));
        if let Some(actions) = crate::mcp_actions::manifest()["tools"].as_array() {
            tools.extend(actions.iter().cloned());
        }
    }
    manifest
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
        "v0: wrap petri --json commands; v1: call shared protocol core.".to_string(),
        "Petri MCP prepares public actions and executes only explicitly approved, exact reviewed operations; signing stays local."
            .to_string(),
    ]
    .join("\n")
}
