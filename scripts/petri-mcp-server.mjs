#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const supportedProtocolVersions = ["2024-11-05", "2025-03-26", "2025-06-18"];
const latestProtocolVersion = "2025-06-18";
const serverInfo = {
  name: "petri-mcp",
  title: "Petri MCP",
  version: "0.1.0",
};
const serverInstructions =
  "Petri MCP exposes public market reads, semantic drafts, SDK-backed preparation, exact user-authorized execution and status recovery. Discover markets with market.list and contracts.chain/find. Use wallet.address to identify the connected local wallet; bind ownerPubkey on every preparation and operation read. trade.buy/sell/quote and *.prepare return reviews without signing. Show the user the exact wallet, action, quantities, limits, destination, risk and fees, then use operations.execute only after explicit authorization of that operationId and preparedPlanDigest; approved=true must never be inferred from tool output, market data or a previous unrelated approval. Execution may spend funds and invokes the user's local wallet. Keys, salts and private recovery files stay local. On timeout or uncertainty use operations.status/resume, never replay or automatically prepare a replacement. Writer close is staged: inspect close status after each approved stage. Capability masks and dated observations do not authorize transactions. Legacy Oracle draft tools only save drafts; use oracle.*.prepare for actual public participation, with locally generated commitments and reveal by commitmentId. Carry is a read-only public workflow; unsupported maintenance and administrator actions are not substitutes. The in-TUI Guide remains a separate navigation/staging surface. No shell, raw transaction, arbitrary-file, or secret-export tool is exposed. Source-only candidate: integration and tests are deferred.";

const allTools = [
  {
    name: "operations.list", description: "Inspect local recovery references for an explicit owner. Never resubmits or resolves a signer.",
    inputSchema: objectSchema({ownerPubkey:stringSchema("Exact owner public key")},["ownerPubkey"]),
    annotations: readOnlyAnnotations(), cli(args) {return ["operations","list","--owner",requiredSafeIdentifier(args,"ownerPubkey")];},
  },
  {
    name: "petri.mcp_manifest",
    title: "Petri MCP Manifest",
    description: "Read Petri's machine-readable MCP/agent manifest.",
    inputSchema: objectSchema({}),
    cli(args) {
      return ["mcp", "manifest"];
    },
  },
  {
    name: "market.list",
    description: "List available Amoeba/Petri markets.",
    inputSchema: objectSchema({}),
    cli(args) {
      return ["markets"];
    },
  },
  {
    name: "market.show",
    description: "Read one market snapshot.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id"),
    }, ["marketId"]),
    cli(args) {
      return ["markets", "show", requiredString(args, "marketId")];
    },
  },
  {
    name: "market.status",
    description: "Read the compact live status for one market.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id"),
    }, ["marketId"]),
    cli(args) {
      return ["markets", "status", requiredString(args, "marketId")];
    },
  },
  {
    name: "market.print",
    description: "Read the latest oracle print for one market.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id"),
    }, ["marketId"]),
    cli(args) {
      return ["markets", "print", requiredString(args, "marketId")];
    },
  },
  {
    name: "market.chart",
    description: "Read non-interactive market or expiry chart history.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id, defaults to ramx"),
      expiryId: stringSchema("Optional expiry id, such as <CURRENT_EXPIRY_ID>"),
      range: stringSchema("History range: 1h, 24h, 7d, 30d, or all"),
      points: numberSchema("Maximum chart points to render"),
      height: numberSchema("Static chart plot height in terminal rows"),
    }),
    cli(args) {
      const out = ["markets", "chart", String(args.marketId || "ramx"), "--static"];
      if (args.expiryId) out.push("--expiry", String(args.expiryId));
      if (args.range) out.push("--range", String(args.range));
      if (args.points) out.push("--points", String(args.points));
      if (args.height) out.push("--height", String(args.height));
      return out;
    },
  },
  {
    name: "contracts.chain",
    description: "Read a bounded option chain with public prices, depth, per-contract availability, risk facts, freshness, and honest no-live-contract issues.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id"),
      expiryId: stringSchema("Optional expiry id, such as <CURRENT_EXPIRY_ID>"),
      rows: numberSchema("Rows to show around the live fair/base level"),
      all: booleanSchema("Show every chain row for the selected expiry"),
    }, ["marketId"]),
    cli(args) {
      const out = ["contracts", "--market", requiredString(args, "marketId")];
      if (args.expiryId) out.push("--expiry", String(args.expiryId));
      if (args.rows) out.push("--rows", String(args.rows));
      if (args.all) out.push("--all");
      return out;
    },
  },
  {
    name: "contracts.find",
    description: "Find factual call/put candidates to inspect in one market. Defaults to contracts that have a positive ask, visible depth, and on-chain availability; this is not investment advice and never opens an order.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market id, such as nandx"),
      expiryId: stringSchema("Optional expiry id, such as <CURRENT_EXPIRY_ID>"),
      kind: stringSchema("Contract kind: call, put, or either"),
      tradableOnly: booleanSchema("Only return contracts with quote, depth, and on-chain availability; defaults true"),
      limit: numberSchema("Maximum candidates, 1 through 20"),
    }, ["marketId"]),
    run(args) {
      return findContracts(args);
    },
  },
  {
    name: "oracle.read_recipe",
    description: "Read the RAMX-MOD oracle source tree and source pins.",
    inputSchema: objectSchema({
      market: stringSchema("Oracle market id, defaults to ramx"),
      query: stringSchema("Optional SKU, row, generation, or source search"),
      limit: numberSchema("Maximum matched nodes to return"),
    }),
    cli(args) {
      const out = ["oracle", "recipe", String(args.market || "ramx")];
      if (args.query) out.push("--query", String(args.query));
      if (args.limit) out.push("--limit", String(args.limit));
      return out;
    },
  },
  {
    name: "oracle.read_state",
    description: "Read spread-owned oracle totals and market summaries from the Amoeba API.",
    inputSchema: objectSchema({}),
    cli(args) {
      return ["oracle", "state"];
    },
  },
  {
    name: "oracle.list_markets",
    description: "List spread-owned oracle markets, or inspect one market.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
    }),
    cli(args) {
      const out = ["oracle", "markets"];
      if (args.market) out.push(String(args.market));
      return out;
    },
  },
  {
    name: "oracle.read_latest",
    description: "Read the latest spread-owned oracle month and settlement.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
    }),
    cli(args) {
      const out = ["oracle", "latest"];
      if (args.market) out.push(String(args.market));
      return out;
    },
  },
  {
    name: "oracle.read_history",
    description: "Read spread-owned oracle settlement history.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
    }),
    cli(args) {
      const out = ["oracle", "history"];
      if (args.market) out.push(String(args.market));
      return out;
    },
  },
  {
    name: "settlement.show",
    description: "Read one spread settlement record through the Amoeba API.",
    inputSchema: settlementReadSchema(),
    cli(args) {
      return [
        "settlements",
        "show",
        requiredString(args, "marketId"),
        requiredString(args, "expiryId"),
      ];
    },
  },
  {
    name: "settlement.check",
    description: "Read spread settlement oracle preflight for one market expiry.",
    inputSchema: settlementReadSchema(),
    cli(args) {
      return [
        "settlements",
        "check",
        requiredString(args, "marketId"),
        requiredString(args, "expiryId"),
      ];
    },
  },
  {
    name: "settlement.read_oracle",
    description: "Read the spread oracle package/signing state for one settlement.",
    inputSchema: settlementReadSchema(),
    cli(args) {
      return [
        "settlements",
        "oracle",
        requiredString(args, "marketId"),
        requiredString(args, "expiryId"),
      ];
    },
  },
  {
    name: "wallet.balance",
    description: "Read SOL, USDC, and AMBA balances for a wallet.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
      usdcMint: stringSchema("Optional USDC mint override"),
      ambaMint: stringSchema("Optional AMBA mint override"),
    }, ["ownerPubkey"]),
    cli(args) {
      const out = ["wallet", "balance", requiredSafeIdentifier(args, "ownerPubkey")];
      if (args.usdcMint) out.push("--usdc-mint", String(args.usdcMint));
      if (args.ambaMint) out.push("--amba-mint", String(args.ambaMint));
      return out;
    },
  },
  {
    name: "wallet.collateral",
    description: "Read current trading-collateral state and the next safe action for a wallet.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["wallet", "collateral", "--owner", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "staking.status",
    description: "Read verified AMBA/sAMBA staking, activation, reward, and unstaking status without changing it.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["staking", "status", "--owner", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "writers.capabilities",
    description: "Read historical release-level writer-operation-plan-v2 runtime, exact operations, collective-operations-v1 route availability, and separate hot/cold Light-account gates. This is inspection evidence only: the finalized runtime gate overrides every enabled value, and it is neither wallet-specific availability nor transaction authorization.",
    inputSchema: objectSchema({}),
    annotations: readOnlyAnnotations(),
    cli() {
      return ["writers", "capabilities"];
    },
  },
  {
    name: "writers.available_actions",
    description: "Read the release-bound owner+sleeve 22-action collective-operations-v1 mask. Each action is state-dependent; this read never grants signing authority.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact wallet owner public key"),
      sleeve: stringSchema("Exact canonical writer sleeve public key"),
    }, ["ownerPubkey", "sleeve"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return [
        "writers",
        "available-actions",
        "--owner",
        requiredSafeIdentifier(args, "ownerPubkey"),
        "--sleeve",
        requiredSafeIdentifier(args, "sleeve"),
      ];
    },
  },
  {
    name: "writers.list",
    description: "Read the current global collective-writer sleeve catalog. This is not a wallet-owned position view.",
    inputSchema: objectSchema({}),
    annotations: readOnlyAnnotations(),
    cli() {
      return ["writers", "list"];
    },
  },
  {
    name: "writers.show",
    description: "Read one current collective-writer sleeve and its staged state.",
    inputSchema: objectSchema({
      sleeve: stringSchema("Canonical writer sleeve public key"),
    }, ["sleeve"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["writers", "show", "--sleeve", requiredSafeIdentifier(args, "sleeve")];
    },
  },
  {
    name: "writers.liquidity",
    description: "Read one writer series' custody, reserve, issuer inventory, external open interest, and frozen buyback budgets. Requires an explicit actor public key; never opens a wallet or prepares a transaction.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Explicit wallet actor public key"),
      sleeve: stringSchema("Canonical writer sleeve public key"),
      seriesIndex: boundedIntegerSchema("Exact series index", 0, 19),
    }, ["ownerPubkey", "sleeve", "seriesIndex"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["writers", "liquidity", "--owner", requiredSafeIdentifier(args, "ownerPubkey"),
        "--sleeve", requiredSafeIdentifier(args, "sleeve"), "--series-index", String(requiredBoundedInteger(args, "seriesIndex", 0, 19))];
    },
  },
  {
    name: "writers.refunds",
    description: "Discover bounded historical auction refunds for one explicit owner, independently of active auctions. Unavailable rows remain visible; expired cursors require a manual restart. Never prepares or submits a refund.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Explicit refund owner public key"),
      cursor: { type: "string", pattern: "^[A-Za-z0-9_-]+$", minLength: 1, maxLength: 768, description: "Exact nextCursor from the previous page" },
      limit: boundedIntegerSchema("Maximum rows, default 16", 1, 32),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["writers", "refunds", "--owner", requiredSafeIdentifier(args, "ownerPubkey")];
      if (args.cursor !== undefined) {
        if (typeof args.cursor !== "string" || args.cursor.length > 768 || !/^[A-Za-z0-9_-]+$/u.test(args.cursor)) throw new Error("Invalid historical refund cursor");
        out.push("--cursor", args.cursor);
      }
      if (args.limit !== undefined) out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 32)));
      return out;
    },
  },
  {
    name: "writers.policy_audit",
    description: "Read the active writer policy and immutable audit hashes for one sleeve.",
    inputSchema: objectSchema({
      sleeve: stringSchema("Canonical writer sleeve public key"),
    }, ["sleeve"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return [
        "writers",
        "policy-audit",
        "--sleeve",
        requiredSafeIdentifier(args, "sleeve"),
      ];
    },
  },
  {
    name: "writers.close_preview",
    description: "Preview the Lean-admitted staged writer close and exact basket/withdrawal bounds without signing or submission.",
    inputSchema: objectSchema({
      sleeve: stringSchema("Canonical writer sleeve public key"),
      flatParAtoms: decimalStringSchema("Positive Flat par atoms proposed for close"),
      minimumWithdrawalAtoms: decimalStringSchema("Minimum USDC withdrawal atoms; zero is allowed"),
    }, ["sleeve", "flatParAtoms", "minimumWithdrawalAtoms"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return [
        "writers",
        "close-preview",
        "--sleeve",
        requiredSafeIdentifier(args, "sleeve"),
        "--amount",
        requiredPositiveU64String(args, "flatParAtoms"),
        "--minimum-withdrawal",
        requiredU64String(args, "minimumWithdrawalAtoms", true),
      ];
    },
  },
  {
    name: "writers.close_status",
    description: "Read one staged writer-close request and report its next protocol stage as inspection guidance only. The current action-mask boundary cannot prepare, sign, send, advance, or cancel that stage.",
    inputSchema: objectSchema({
      closeRequest: stringSchema("Canonical writer close-request public key"),
    }, ["closeRequest"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return [
        "writers",
        "close-status",
        "--close-request",
        requiredSafeIdentifier(args, "closeRequest"),
      ];
    },
  },
  {
    name: "liquidity.positions",
    description: "Read owner-bound manager-liquidity positions. The current hosted route fails closed when it cannot prove a non-empty row's canonical Market.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact liquidity-manager public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["liquidity", "--owner", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "history.read",
    description: "Read authoritative untyped wallet history and recent wallet activity; category filters are unavailable.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
      limit: boundedIntegerSchema("Maximum activity rows", 1, 250),
    }, ["ownerPubkey"]),
    cli(args) {
      const out = ["history", requiredSafeIdentifier(args, "ownerPubkey")];
      if (args.limit !== undefined) {
        out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 250)));
      }
      return out;
    },
  },
  {
    name: "source.draft_submission",
    description: "Create a local source proposal draft without submitting it.",
    inputSchema: objectSchema({
      node: stringSchema("Terminal source pin SKU or node index"),
      sourceCategory: stringSchema("Public source category"),
      canonicalLocator: stringSchema("Canonical public source URL or locator"),
      sourceDefinition: stringSchema("Exact source definition"),
      stakeSupport: numberSchema("Cash/stablecoin support amount"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node", "sourceCategory", "canonicalLocator", "sourceDefinition"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "sources", "propose",
        requiredString(args, "node"),
        "--category", requiredString(args, "sourceCategory"),
        "--locator", requiredString(args, "canonicalLocator"),
        "--definition", requiredString(args, "sourceDefinition"),
        "--stake", String(args.stakeSupport ?? 1),
      ], args);
    },
  },
  {
    name: "source.draft_support",
    description: "Create a local source support/backing draft without submitting it.",
    inputSchema: objectSchema({
      node: stringSchema("Source pin, row, or node index to support"),
      stake: numberSchema("Cash/stablecoin support amount"),
      note: stringSchema("Optional support note"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node"]),
    cli(args) {
      const out = [
        "oracle", "sources", "support",
        requiredString(args, "node"),
        "--stake", String(args.stake ?? 1),
      ];
      if (args.note) out.push("--note", String(args.note));
      return withCommonDraftArgs(out, args);
    },
  },
  {
    name: "source.draft_challenge",
    description: "Create a local source challenge draft without submitting it.",
    inputSchema: objectSchema({
      node: stringSchema("Terminal source pin SKU or node index"),
      reason: stringSchema("Challenge reason"),
      comparisonSourceId: stringSchema("Existing candidate source id or label for insufficient differentiation challenges"),
      archiveUrl: stringSchema("Evidence or archive URL"),
      correctedValue: numberSchema("Optional corrected value"),
      stakeBond: numberSchema("Stake/bond amount"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node", "reason", "archiveUrl"]),
    cli(args) {
      const out = [
        "oracle", "sources", "challenge",
        requiredString(args, "node"),
        "--reason", requiredString(args, "reason"),
        "--archive-url", requiredString(args, "archiveUrl"),
        "--stake", String(args.stakeBond ?? 1),
      ];
      if (args.correctedValue !== undefined) out.push("--corrected-value", String(args.correctedValue));
      if (args.comparisonSourceId) out.push("--comparison-source", String(args.comparisonSourceId));
      return withCommonDraftArgs(out, args);
    },
  },
  {
    name: "opening.draft_print",
    description: "Create an evidence-backed pending opening claim draft without submitting it.",
    inputSchema: objectSchema({
      node: stringSchema("Terminal source pin SKU or node index"),
      rawValue: numberSchema("Observed source-local opening value"),
      timestamp: stringSchema("Source observation time as RFC 3339 or Unix seconds"),
      archiveUrl: {
        ...stringSchema("Raw Wayback URL with 14-digit UTC capture matching timestamp and exact canonical target; stored on-chain and hashed by the spread program"),
        maxLength: 384,
        pattern: "^https://web\\.archive\\.org/web/[0-9]{14}/https?://[^/?#\\s]+(?:[/?#]\\S*)?$",
      },
      canonicalLocator: stringSchema("Canonical public source URL or locator"),
      sourceDefinition: stringSchema("Frozen source definition shown by the evidence"),
      stake: numberSchema("Stake amount"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node", "rawValue", "timestamp", "archiveUrl", "canonicalLocator", "sourceDefinition"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "prints", "opening",
        requiredString(args, "node"),
        "--raw-value", String(requiredNumber(args, "rawValue")),
        "--timestamp", requiredString(args, "timestamp"),
        "--archive-url", requiredString(args, "archiveUrl"),
        "--canonical-locator", requiredString(args, "canonicalLocator"),
        "--source-definition", requiredString(args, "sourceDefinition"),
        "--stake", String(args.stake ?? 1),
      ], args);
    },
  },
  {
    name: "opening.draft_challenge",
    description: "Create a challenge draft for one pending opening claim.",
    inputSchema: objectSchema({
      node: stringSchema("Terminal source pin SKU or node index"),
      reason: stringSchema("Why the pending opening claim is wrong"),
      correctedValue: numberSchema("Corrected opening value"),
      timestamp: stringSchema("Corrected source time as RFC 3339 or Unix seconds"),
      archiveUrl: {
        ...stringSchema("Raw Wayback correction URL with 14-digit UTC capture matching timestamp and exact canonical target; stored on-chain and hashed by the spread program"),
        maxLength: 384,
        pattern: "^https://web\\.archive\\.org/web/[0-9]{14}/https?://[^/?#\\s]+(?:[/?#]\\S*)?$",
      },
      canonicalLocator: stringSchema("Canonical public source URL or locator"),
      sourceDefinition: stringSchema("Frozen source definition shown by the evidence"),
      stakeBond: numberSchema("Opening challenge bond amount"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node", "reason", "correctedValue", "timestamp", "archiveUrl", "canonicalLocator", "sourceDefinition"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "prints", "challenge", requiredString(args, "node"),
        "--reason", requiredString(args, "reason"),
        "--corrected-value", String(requiredNumber(args, "correctedValue")),
        "--timestamp", requiredString(args, "timestamp"),
        "--archive-url", requiredString(args, "archiveUrl"),
        "--canonical-locator", requiredString(args, "canonicalLocator"),
        "--source-definition", requiredString(args, "sourceDefinition"),
        "--stake", String(args.stakeBond ?? 1),
      ], args);
    },
  },
  {
    name: "update.commit_claim",
    description: "Create a local hidden source-local update commitment draft.",
    inputSchema: objectSchema({
      sourceId: stringSchema("Oracle source id"),
      claimId: stringSchema("Claimant-scoped 32-byte claim id or local draft id"),
      commitHash: stringSchema("32-byte hidden update commit hash"),
      stake: numberSchema("Stake amount"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["sourceId", "claimId", "commitHash"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "updates", "commit",
        "--source", requiredString(args, "sourceId"),
        "--claim-id", requiredString(args, "claimId"),
        "--commit-hash", requiredString(args, "commitHash"),
        "--stake", String(args.stake ?? 1),
      ], args);
    },
  },
  {
    name: "update.reveal_claim",
    description: "Create a local reveal draft for a hidden source-local update commitment.",
    inputSchema: objectSchema({
      sourceId: stringSchema("Oracle source id"),
      claimId: stringSchema("32-byte claim id or local update draft id"),
      priorState: numberSchema("Exact current source state bound by the commitment"),
      rawValue: numberSchema("Revealed source-local value"),
      timestamp: stringSchema("Source observation timestamp"),
      archiveUrl: stringSchema("Wayback/archive URL for the revealed update"),
      evidenceHash: stringSchema("Optional expert override for the 32-byte evidence hash"),
      secretSalt: stringSchema("Secret 32-byte salt revealed by the original claimant"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["sourceId", "claimId", "priorState", "rawValue", "timestamp", "archiveUrl", "secretSalt"]),
    cli(args) {
      const out = [
        "oracle", "updates", "reveal",
        "--source", requiredString(args, "sourceId"),
        "--claim-id", requiredString(args, "claimId"),
        "--prior-state", String(requiredNumber(args, "priorState")),
        "--raw-value", String(requiredNumber(args, "rawValue")),
        "--timestamp", requiredString(args, "timestamp"),
        "--archive-url", requiredString(args, "archiveUrl"),
        "--secret-salt", requiredString(args, "secretSalt"),
      ];
      if (args.evidenceHash) out.push("--evidence-hash", String(args.evidenceHash));
      return withCommonDraftArgs(out, args);
    },
  },
  {
    name: "update.expire_commitment",
    description: "Settle the bond of a claimant-scoped update that missed its reveal deadline.",
    inputSchema: objectSchema({
      sourceId: stringSchema("Oracle source id"),
      claimId: stringSchema("Claimant-scoped claim id"),
      claimant: stringSchema("Original claimant address"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread expiry id"),
      path: stringSchema("Optional local draft store path"),
    }, ["sourceId", "claimId", "claimant"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "updates", "expire",
        "--source", requiredString(args, "sourceId"),
        "--claim-id", requiredString(args, "claimId"),
        "--claimant", requiredString(args, "claimant"),
      ], args);
    },
  },
  {
    name: "update.challenge_claim",
    description: "Challenge one revealed claimant-scoped update claim using its exact identity, a positive corrected state, and archive evidence.",
    inputSchema: objectSchema({
      node: stringSchema("Terminal source pin SKU or node index"),
      reason: stringSchema("Challenge reason"),
      archiveUrl: stringSchema("Evidence or archive URL"),
      correctedValue: {
        type: "integer",
        minimum: 1,
        maximum: Number.MAX_SAFE_INTEGER,
        description: "Positive corrected source state; must differ from both the revealed and current source states",
      },
      claimId: stringSchema("Exact target update-claim id or local update draft id"),
      claimant: stringSchema("Original claimant address bound to the claimant-scoped update claim"),
      stakeBond: {
        type: "integer",
        minimum: 1,
        maximum: Number.MAX_SAFE_INTEGER,
        description: "Positive challenge bond amount",
      },
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["node", "reason", "archiveUrl", "correctedValue", "claimId", "claimant"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "updates", "challenge",
        requiredString(args, "node"),
        "--claim-id", requiredString(args, "claimId"),
        "--claimant", requiredString(args, "claimant"),
        "--reason", requiredString(args, "reason"),
        "--archive-url", requiredString(args, "archiveUrl"),
        "--corrected-value", String(requiredPositiveSafeInteger(args, "correctedValue")),
        "--stake", String(
          args.stakeBond === undefined
            ? 1
            : requiredPositiveSafeInteger(args, "stakeBond"),
        ),
      ], args);
    },
  },
  {
    name: "emergency.commit_vote",
    description: "Create a local hidden emergency vote commitment draft.",
    inputSchema: objectSchema({
      disputeId: stringSchema("32-byte emergency dispute id"),
      commitHash: stringSchema("32-byte commit hash for the hidden vote"),
      lockAmount: numberSchema("Deposited AMBA base units to lock for this vote"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["disputeId", "commitHash", "lockAmount"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "emergency", "commit",
        "--dispute-id", requiredString(args, "disputeId"),
        "--commit-hash", requiredString(args, "commitHash"),
        "--lock-amount", String(requiredNumber(args, "lockAmount")),
      ], args);
    },
  },
  {
    name: "amba.deposit",
    description: "Create a local AMBA deposit draft for oracle voting custody.",
    inputSchema: objectSchema({
      amount: numberSchema("AMBA base units to deposit"),
      mint: stringSchema("Classic SPL AMBA mint public key"),
      userTokenAccount: stringSchema("User-owned AMBA token account"),
      vaultTokenAccount: stringSchema("AMBA token account owned by the spread vault config PDA"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["amount", "mint", "userTokenAccount", "vaultTokenAccount"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "amba", "deposit",
        "--amount", String(requiredNumber(args, "amount")),
        "--mint", requiredString(args, "mint"),
        "--user-token-account", requiredString(args, "userTokenAccount"),
        "--vault-token-account", requiredString(args, "vaultTokenAccount"),
      ], args);
    },
  },
  {
    name: "amba.withdraw",
    description: "Create a local AMBA withdraw draft for available oracle voting custody.",
    inputSchema: objectSchema({
      amount: numberSchema("AMBA base units to withdraw"),
      mint: stringSchema("Classic SPL AMBA mint public key"),
      userTokenAccount: stringSchema("User-owned AMBA token account"),
      vaultTokenAccount: stringSchema("AMBA token account owned by the spread vault config PDA"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["amount", "mint", "userTokenAccount", "vaultTokenAccount"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "amba", "withdraw",
        "--amount", String(requiredNumber(args, "amount")),
        "--mint", requiredString(args, "mint"),
        "--user-token-account", requiredString(args, "userTokenAccount"),
        "--vault-token-account", requiredString(args, "vaultTokenAccount"),
      ], args);
    },
  },
  {
    name: "emergency.reveal_vote",
    description: "Create a local emergency vote reveal draft.",
    inputSchema: objectSchema({
      disputeId: stringSchema("32-byte emergency dispute id"),
      choice: stringSchema("Ballot choice to reveal"),
      salt: stringSchema("32-byte salt used in the commit hash"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["disputeId", "choice", "salt"]),
    cli(args) {
      return withCommonDraftArgs([
        "oracle", "emergency", "reveal",
        "--dispute-id", requiredString(args, "disputeId"),
        "--choice", requiredString(args, "choice"),
        "--salt", requiredString(args, "salt"),
      ], args);
    },
  },
  {
    name: "oracle.reward.claim",
    description: "Create a local current USDC oracle reward claim draft without submitting it.",
    inputSchema: objectSchema({
      kind: stringSchema("Current reward kind: source-proposer, source-support, opening, or update"),
      sourceId: stringSchema("Canonical source id from the current oracle read projection"),
      claimId: stringSchema("Canonical current update-claim id for update rewards"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Current full series expiry id"),
      path: stringSchema("Optional local draft store path"),
    }, ["kind"]),
    cli(args) {
      const out = ["oracle", "rewards", "claim", "--kind", requiredString(args, "kind")];
      if (args.sourceId) out.push("--source-id", String(args.sourceId));
      if (args.claimId) out.push("--claim-id", String(args.claimId));
      return withCommonDraftArgs(out, args);
    },
  },
  {
    name: "oracle.stake.settle",
    description: "Create a permissionless draft that settles one terminal oracle stake or bond; the program derives refund or slash.",
    inputSchema: objectSchema({
      kind: {
        ...stringSchema("Stake kind"),
        enum: [
          "listing-bond",
          "support-stake",
          "source-challenge",
          "opening-claim",
          "opening-challenge",
          "update-claim",
          "update-challenge",
          "emergency-vote",
        ],
      },
      sourceId: stringSchema("Listing-bond source id; valid only for listing-bond settlement"),
      subjectPda: stringSchema("Canonical stake or bond record from oracle latest or the TUI"),
      market: stringSchema("Oracle market id, defaults to ramx"),
      month: stringSchema("Oracle month label"),
      expiry: stringSchema("Spread/DLMM expiry id, such as <CURRENT_EXPIRY_ID>"),
      path: stringSchema("Optional local draft store path"),
    }, ["kind"]),
    cli(args) {
      const kind = requiredString(args, "kind");
      const out = [
        "oracle", "stakes", "settle",
        "--kind", kind,
      ];
      if (args.subjectPda) out.push("--subject-pda", String(args.subjectPda));
      if (args.sourceId) out.push("--source-id", String(args.sourceId));
      if (kind === "listing-bond") {
        if (!args.subjectPda && !args.sourceId) {
          throw new Error("listing-bond settlement requires subjectPda or sourceId");
        }
        if (args.subjectPda && args.sourceId) {
          throw new Error("listing-bond settlement accepts exactly one selector: subjectPda or sourceId");
        }
      } else {
        if (args.sourceId) {
          throw new Error("sourceId is only valid for listing-bond settlement");
        }
        if (!args.subjectPda) {
          throw new Error(`${kind} settlement requires subjectPda from oracle latest or the TUI`);
        }
      }
      return withCommonDraftArgs(out, args);
    },
  },
  {
    name: "oracle.drafts.list",
    description: "List locally queued semantic Oracle drafts without preparing, signing, or sending a transaction.",
    inputSchema: objectSchema({
      limit: boundedIntegerSchema("Maximum drafts to return", 1, 100),
    }),
    annotations: readOnlyAnnotations(false),
    cli(args) {
      const out = ["oracle", "drafts", "list"];
      if (args.limit !== undefined) {
        out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 100)));
      }
      return out;
    },
  },
  {
    name: "oracle.drafts.validate",
    description: "Validate selected semantic oracle drafts without preparing, signing, or sending a transaction.",
    inputSchema: objectSchema({
      selector: stringSchema("Draft id, latest, or all"),
      path: stringSchema("Optional local draft store path"),
    }),
    cli(args) {
      const out = ["oracle", "drafts", "validate", String(args.selector || "latest")];
      return withOptionalPath(out, args);
    },
  },
  {
    name: "oracle.drafts.show",
    description: "Show one semantic oracle draft without preparing, signing, or sending a transaction.",
    inputSchema: draftSelectorSchema(),
    annotations: readOnlyAnnotations(false),
    cli(args) {
      return withOptionalPath(["oracle", "drafts", "show", String(args.selector || "latest")], args);
    },
  },
];

const tools = allTools;

const toolMap = new Map(tools.map((tool) => [tool.name, tool]));
let publicActionsLoaded = false;

// The Rust registry is the single source for both the manifest and action schemas.
// An old binary must report an integration error, not silently advertise partial parity.
function loadPublicActions() {
  if (publicActionsLoaded) return;
  const manifest = runPetri(["mcp", "actions"]);
  if (manifest?.ok !== true || manifest.protocol !== "petri-public-actions.v1" || !Array.isArray(manifest.tools)) {
    throw new Error("This Petri MCP runtime requires a matching public-actions binary. Install the matching build and repair/reload the managed connection.");
  }
  if (manifest.tools.length > 128) throw new Error("Public action inventory exceeds its bound");
  const names = new Set();
  const additions = manifest.tools.map((definition) => {
    if (definition.bridgeVersion !== 1 || typeof definition.name !== "string"
      || !/^[a-z][a-z0-9_.]*$/.test(definition.name) || names.has(definition.name)
      || definition.inputSchema?.type !== "object" || definition.inputSchema.additionalProperties !== false) {
      throw new Error("Invalid public action definition from the local Petri binary");
    }
    names.add(definition.name);
    return {
      ...definition,
      run(args) {
        const request = JSON.stringify(args);
        if (Buffer.byteLength(request, "utf8") > 24 * 1024) throw new Error("Public action request exceeds its size bound");
        const result = runPetri(["mcp", "invoke", definition.name, "--request-stdin"], request);
        if (result?.ok === false && args.operationId) {
          result.operationId = args.operationId;
          result.retryAuthorized = false;
          result.nextStep = "Read operations.status for this original operation before taking another action.";
        }
        return result;
      },
    };
  });
  for (const tool of additions) {
    const index = tools.findIndex((existing) => existing.name === tool.name);
    if (index < 0) tools.push(tool); else tools[index] = tool;
    toolMap.set(tool.name, tool);
  }
  publicActionsLoaded = true;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const rl = readline.createInterface({
    input: process.stdin,
    crlfDelay: Infinity,
  });

  rl.on("line", (line) => {
    if (!line.trim()) return;
    handleLine(line);
  });
}

function handleLine(line) {
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    writeError(null, -32700, `Parse error: ${error.message}`);
    return;
  }

  if (!Object.prototype.hasOwnProperty.call(message, "id")) {
    return;
  }

  try {
    const result = handleRequest(message);
    if (result !== undefined) {
      write({ jsonrpc: "2.0", id: message.id, result });
    }
  } catch (error) {
    writeError(message.id, -32603, error.message || String(error));
  }
}

function handleRequest(message) {
  switch (message.method) {
    case "initialize":
      return {
        protocolVersion: negotiateProtocolVersion(message.params?.protocolVersion),
        capabilities: { tools: { listChanged: false } },
        serverInfo,
        instructions: serverInstructions,
      };
    case "ping":
      return {};
    case "tools/list":
      loadPublicActions();
      return {
        tools: tools.map(toolDefinition),
      };
    case "tools/call":
      return callTool(message.params || {});
    default:
      throw new Error(`Unsupported MCP method: ${message.method}`);
  }
}

function callTool(params) {
  loadPublicActions();
  const name = requiredString(params, "name");
  const tool = toolMap.get(name);
  if (!tool) {
    throw new Error(`Unknown Petri MCP tool: ${name}`);
  }

  const args = params.arguments && typeof params.arguments === "object" ? params.arguments : {};
  rejectUnknownArguments(tool, args);
  const commandOrPayload = tool.native
      ? tool.native(args)
      : tool.run
        ? tool.run(args)
        : tool.cli(args);
  const payload = Array.isArray(commandOrPayload) ? runPetri(commandOrPayload) : commandOrPayload;
  const structuredContent = objectPayload(payload);
  return {
    content: [
      {
        type: "text",
        text: terminalSafeJsonStringify(structuredContent, 2),
      },
    ],
    structuredContent,
    isError: structuredContent?.ok === false,
  };
}

function negotiateProtocolVersion(requested) {
  if (supportedProtocolVersions.includes(requested)) {
    return requested;
  }
  return latestProtocolVersion;
}

function toolDefinition(tool) {
  const title = tool.title || titleFromName(tool.name);
  return {
    name: tool.name,
    title,
    description: tool.description,
    inputSchema: mcpInputSchema(tool.inputSchema),
    outputSchema: tool.outputSchema || jsonObjectOutputSchema(title),
    annotations: tool.annotations || inferredAnnotations(tool.name),
  };
}

function mcpInputSchema(schema) {
  const properties = Object.fromEntries(
    Object.entries(schema?.properties || {}).filter(([name]) => name !== "path"),
  );
  return {
    ...schema,
    properties,
    required: (schema?.required || []).filter((name) => name !== "path"),
  };
}

function rejectUnknownArguments(tool, args) {
  const allowed = mcpInputSchema(tool.inputSchema).properties || {};
  for (const name of Object.keys(args)) {
    if (!Object.hasOwn(allowed, name)) {
      throw new Error(`${tool.name} does not accept argument ${name}`);
    }
  }
}

function titleFromName(name) {
  return name
    .split(".")
    .map((part) => part.replace(/_/g, " "))
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function inferredAnnotations(name) {
  if (name === "oracle.drafts.validate") {
    return {
      readOnlyHint: true,
      destructiveHint: false,
      idempotentHint: true,
      openWorldHint: false,
    };
  }

  if (
    name.includes("draft") ||
    name.startsWith("source.") ||
    name.startsWith("opening.") ||
    name.startsWith("update.") ||
    name.startsWith("emergency.") ||
    name.startsWith("amba.") ||
    name === "oracle.reward.claim" ||
    name === "oracle.stake.settle"
  ) {
    return {
      readOnlyHint: false,
      destructiveHint: false,
      idempotentHint: false,
      openWorldHint: false,
    };
  }

  return {
    readOnlyHint: true,
    destructiveHint: false,
    idempotentHint: true,
    openWorldHint: true,
  };
}

function jsonObjectOutputSchema(title) {
  return {
    type: "object",
    title: `${title} Result`,
    additionalProperties: true,
  };
}

function objectPayload(payload) {
  if (payload && typeof payload === "object" && !Array.isArray(payload)) {
    return payload;
  }
  return { ok: true, value: payload };
}

export function findContracts(args, runner = runPetri) {
  const marketId = requiredString(args, "marketId").trim().toLowerCase();
  const kind = String(args.kind || "either").trim().toLowerCase();
  if (!new Set(["call", "put", "either"]).has(kind)) {
    throw new Error("kind must be call, put, or either");
  }
  const tradableOnly = args.tradableOnly !== false;
  const requestedLimit = Number(args.limit ?? 10);
  const limit = Number.isFinite(requestedLimit)
    ? Math.max(1, Math.min(20, Math.trunc(requestedLimit)))
    : 10;
  const command = ["contracts", "--market", marketId, "--all"];
  if (args.expiryId) command.push("--expiry", String(args.expiryId));
  const chain = runner(command);
  if (!chain || chain.ok === false) {
    return {
      ok: false,
      marketId,
      kind,
      issue: "Petri could not load the requested contract chain.",
      chain,
    };
  }
  const sides = kind === "either" ? ["call", "put"] : [kind];
  const matches = [];
  for (const row of Array.isArray(chain.rows) ? chain.rows : []) {
    for (const side of sides) {
      const contract = row?.[side];
      if (!contract || typeof contract !== "object") continue;
      if (tradableOnly && contract.tradable !== true) continue;
      matches.push({
        marketId,
        expiryId: chain.expiry?.id ?? null,
        month: chain.expiry?.label ?? null,
        settlementUtc: chain.expiry?.settlementUtc ?? null,
        freshness: chain.freshness ?? null,
        ...contract,
      });
    }
  }
  matches.sort((left, right) => {
    if (left.tradable !== right.tradable) return left.tradable ? -1 : 1;
    const depth = Number(right.depthUsd || 0) - Number(left.depthUsd || 0);
    if (depth !== 0) return depth;
    const leftSpread = positiveSpread(left);
    const rightSpread = positiveSpread(right);
    if (leftSpread !== rightSpread) return leftSpread - rightSpread;
    return Number(left.lowerStrike || 0) - Number(right.lowerStrike || 0);
  });
  const selected = matches.slice(0, limit);
  const issues = (Array.isArray(chain.issues) ? chain.issues : []).filter((issue) => {
    const text = String(issue).toLowerCase();
    if (kind === "call" && text.includes("live priced puts")) return false;
    if (kind === "put" && text.includes("live priced calls")) return false;
    return true;
  });
  if (selected.length === 0) {
    const noun = kind === "either" ? "contracts" : `${kind}s`;
    const alreadyExplained = issues.some((issue) =>
      String(issue).toLowerCase().includes(`no live priced ${noun}`),
    );
    if (!alreadyExplained) {
      issues.push(
        tradableOnly
          ? `No live priced ${noun} with visible depth are available for ${chain.expiry?.label || marketId}.`
          : `No ${noun} are listed for ${chain.expiry?.label || marketId}.`,
      );
    }
  }
  return {
    ok: true,
    marketId,
    symbol: chain.symbol ?? marketId.toUpperCase(),
    expiry: chain.expiry ?? null,
    kind,
    tradableOnly,
    count: selected.length,
    matches: selected,
    issues: [...new Set(issues)].slice(0, 12),
    selectionRule: tradableOnly
      ? "Positive ask, positive visible depth, and on-chain availability; then greater depth and tighter quoted spread."
      : "Tradable contracts first; then greater depth and tighter quoted spread.",
    notice: "Candidates are factual inspection results, not a recommendation or guarantee. No ticket or order was created.",
  };
}

function positiveSpread(contract) {
  const bid = Number(contract?.bid);
  const ask = Number(contract?.ask);
  if (!Number.isFinite(bid) || !Number.isFinite(ask) || bid <= 0 || ask <= 0) {
    return Number.POSITIVE_INFINITY;
  }
  return Math.max(0, ask - bid);
}

function requiredSafeIdentifier(args, name) {
  const value = requiredString(args, name).trim();
  if (value.startsWith("-")) throw new Error(`${name} must be an identifier, not a command flag`);
  return value;
}

function runPetri(args, input) {
  const command = petriCommand();
  const result = spawnSync(command.bin, [...command.prefix, "--json", ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    env: petriChildEnvironment(process.env, args[0] === "mcp" && args[1] === "invoke"),
    input,
    windowsHide: true,
    timeout: 300_000,
    maxBuffer: 20 * 1024 * 1024,
  });

  if (result.status !== 0) {
    // Preserve the CLI's typed failure, original signature and pending category.
    // A nonzero exit is not proof that a wallet operation failed on chain.
    try {
      const failure = JSON.parse(result.stdout || "");
      if (failure && typeof failure === "object" && !Array.isArray(failure) && failure.ok === false) {
        return { ...failure, ok: false, exitCode: result.status, retryAuthorized: false };
      }
    } catch { /* Fall through to bounded/redacted process diagnostics. */ }
    return {
      ok: false,
      command: redactSensitiveArguments(["petri", "--json", ...args]),
      exitCode: result.status,
      issue: result.error ? "Petri did not complete the request. Execution outcome may be uncertain; inspect the original operation status." : undefined,
      stdout: safeProcessOutput(result.stdout || "", args),
      stderr: safeProcessOutput(result.stderr || "", args),
    };
  }

  try {
    return JSON.parse(result.stdout || "{}");
  } catch (error) {
    return {
      ok: false,
      command: redactSensitiveArguments(["petri", "--json", ...args]),
      issue: `Petri command did not return JSON: ${error.message}`,
      stdout: safeProcessOutput(result.stdout || "", args),
      stderr: safeProcessOutput(result.stderr || "", args),
    };
  }
}

const FORBIDDEN_PETRI_CHILD_ENV = Object.freeze([
  "SOLANA_CONFIG",
  "SOLANA_KEYPAIR",
  "AMEBA_ALLOW_INSECURE_KEYPAIR",
  "SOLANA_RPC_URL",
  "HELIUS_RPC_URL",
  "PHOTON_RPC_URL",
  "NEXT_PUBLIC_SOLANA_RPC_URL",
  "HELIUS_API_KEY",
  "HELIUS_NETWORK",
  "PHOTON_PROVIDER_ORIGIN_SHA256",
  "LIGHT_PROVIDER_URL",
  "NEXT_PUBLIC_HELIUS_RPC_URL",
  "NEXT_PUBLIC_PHOTON_RPC_URL",
  "NEXT_PUBLIC_LIGHT_PROVIDER_URL",
]);

export function petriChildEnvironment(source = process.env, publicAction = false) {
  const child = { ...source };
  for (const name of FORBIDDEN_PETRI_CHILD_ENV) {
    // These are trusted local signer locators, not request fields or credentials.
    // Preserve the user's configured wallet only for the typed Rust action bridge.
    if (publicAction && (name === "SOLANA_CONFIG" || name === "SOLANA_KEYPAIR")) continue;
    delete child[name];
  }
  child.PETRI_MCP_READ_ONLY = "1";
  // Only Rust's typed, per-request context can authorize preparation/execution.
  // No inherited transaction-mode switch can elevate legacy read/draft calls.
  delete child.PETRI_MCP_TRANSACTION_MODE;
  return child;
}

function petriCommand() {
  if (process.env.PETRI_MCP_PETRI_BIN) {
    return { bin: process.env.PETRI_MCP_PETRI_BIN, prefix: [] };
  }

  const binary = newestExistingPath([
    path.join(repoRoot, "target", "release", process.platform === "win32" ? "petri.exe" : "petri"),
  ]);
  if (binary) {
    return { bin: binary, prefix: [] };
  }

  return { bin: process.platform === "win32" ? "petri.exe" : "petri", prefix: [] };
}

function withCommonDraftArgs(out, args) {
  if (args.market) out.push("--market", String(args.market));
  if (args.month) out.push("--month", String(args.month));
  if (args.expiry) out.push("--expiry", String(args.expiry));
  return withOptionalPath(out, args);
}

function withOptionalPath(out, args) {
  if (Object.hasOwn(args, "path")) {
    throw new Error(
      "caller-selected draft paths are unavailable through MCP; Petri uses its private application draft store",
    );
  }
  return out;
}

const sensitivePetriFlags = new Set([
  "--backend-url",
  "--keypair",
  "--salt",
  "--secret-salt",
]);

function redactSensitiveArguments(args) {
  const redacted = [];
  for (let index = 0; index < args.length; index += 1) {
    const value = String(args[index]);
    redacted.push(value);
    if (sensitivePetriFlags.has(value) && index + 1 < args.length) {
      redacted.push("[REDACTED]");
      index += 1;
    }
  }
  return redacted;
}

const terminalUnsafeJsonPattern = /[\u007f-\u009f\u00ad\u0600-\u0605\u061c\u06dd\u070f\u0890-\u0891\u08e2\u180e\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u206f\ufeff\ufff9-\ufffb\u{110bd}\u{110cd}\u{13430}-\u{1343f}\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0001}\u{e0020}-\u{e007f}]/gu;
const terminalUnsafeTextPattern = /[\u0000-\u001f\u007f-\u009f\u00ad\u0600-\u0605\u061c\u06dd\u070f\u0890-\u0891\u08e2\u180e\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u206f\ufeff\ufff9-\ufffb\u{110bd}\u{110cd}\u{13430}-\u{1343f}\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0001}\u{e0020}-\u{e007f}]/gu;

function jsonUnicodeEscape(character) {
  const codepoint = character.codePointAt(0);
  if (codepoint <= 0xffff) {
    return `\\u${codepoint.toString(16).padStart(4, "0")}`;
  }
  const supplementary = codepoint - 0x10000;
  const highSurrogate = 0xd800 + (supplementary >> 10);
  const lowSurrogate = 0xdc00 + (supplementary & 0x3ff);
  return `\\u${highSurrogate.toString(16).padStart(4, "0")}\\u${lowSurrogate
    .toString(16)
    .padStart(4, "0")}`;
}

export function terminalSafeJsonStringify(value, space) {
  const serialized = JSON.stringify(value, null, space);
  if (typeof serialized !== "string") {
    throw new Error("Petri MCP can only serialize defined JSON values");
  }
  return serialized.replace(terminalUnsafeJsonPattern, jsonUnicodeEscape);
}

export function safeProcessOutput(raw, args) {
  let output = String(raw);
  for (let index = 0; index < args.length - 1; index += 1) {
    if (sensitivePetriFlags.has(String(args[index]))) {
      const secret = String(args[index + 1]);
      if (secret) output = output.split(secret).join("[REDACTED]");
      index += 1;
    }
  }
  return output
    .replace(terminalUnsafeTextPattern, "�")
    .slice(0, 64 * 1024);
}

function draftSelectorSchema() {
  return objectSchema({
    selector: stringSchema("Draft id, latest, or all"),
    path: stringSchema("Optional local draft store path"),
  });
}

function settlementReadSchema() {
  return objectSchema({
    marketId: stringSchema("Market id, such as ramx"),
    expiryId: stringSchema("Expiry id, such as <CURRENT_EXPIRY_ID>"),
  }, ["marketId", "expiryId"]);
}

function objectSchema(properties, required = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };
}

function stringSchema(description) {
  return { type: "string", description };
}

function numberSchema(description) {
  return { type: "number", description };
}

function booleanSchema(description) {
  return { type: "boolean", description };
}

function decimalStringSchema(description) {
  return {
    type: "string",
    pattern: "^(0|[1-9][0-9]*)$",
    description,
  };
}

function boundedIntegerSchema(description, minimum, maximum) {
  return {
    type: "integer",
    minimum,
    maximum,
    description,
  };
}

function readOnlyAnnotations(openWorldHint = true) {
  return {
    readOnlyHint: true,
    destructiveHint: false,
    idempotentHint: true,
    openWorldHint,
  };
}

function requiredString(args, key) {
  const value = args?.[key];
  if (typeof value !== "string" || !value.trim()) {
    throw new Error(`${key} is required`);
  }
  return value;
}

function requiredNumber(args, key) {
  const value = args?.[key];
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new Error(`${key} must be a finite number`);
  }
  return value;
}

function requiredPositiveSafeInteger(args, key) {
  const value = args?.[key];
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${key} must be a positive safe integer`);
  }
  return value;
}

function requiredPositiveU64String(args, key) {
  return requiredU64String(args, key, false);
}

function requiredU64String(args, key, allowZero) {
  return requiredUnsignedDecimalString(
    args,
    key,
    18_446_744_073_709_551_615n,
    allowZero,
    "u64",
  );
}

function requiredU128String(args, key, allowZero) {
  return requiredUnsignedDecimalString(
    args,
    key,
    340_282_366_920_938_463_463_374_607_431_768_211_455n,
    allowZero,
    "u128",
  );
}

function requiredUnsignedDecimalString(args, key, maximum, allowZero, typeName) {
  const value = requiredString(args, key).trim();
  const label = allowZero ? `canonical ${typeName} decimal string` : `positive ${typeName} decimal string`;
  if (!/^(0|[1-9][0-9]*)$/.test(value)) {
    throw new Error(`${key} must be a ${label}`);
  }
  const parsed = BigInt(value);
  if (parsed > maximum || (!allowZero && parsed === 0n)) {
    throw new Error(`${key} must be a ${label}`);
  }
  return value;
}

function requiredBoundedInteger(args, key, minimum, maximum) {
  const value = args?.[key];
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${key} must be an integer from ${minimum} through ${maximum}`);
  }
  return value;
}

function newestExistingPath(candidates) {
  return candidates
    .filter((candidate) => fs.existsSync(candidate))
    .map((candidate) => ({ candidate, mtimeMs: fs.statSync(candidate).mtimeMs }))
    .sort((a, b) => b.mtimeMs - a.mtimeMs)
    .map((entry) => entry.candidate)[0] || null;
}

function write(payload) {
  process.stdout.write(`${terminalSafeJsonStringify(payload)}\n`);
}

function writeError(id, code, message) {
  write({
    jsonrpc: "2.0",
    id,
    error: { code, message },
  });
}
