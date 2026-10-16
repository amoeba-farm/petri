#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { spawnSync } from "node:child_process";
import { ResidentPetri } from './petri-action-bridge.mjs';
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
  "Petri MCP exposes read-only Amoeba futures and oracle reads. A futures position is a range of the RAMX or NANDX index that pays per point the index settles inside it; it never loses more than it cost. Discover markets with market.list and market.show, then read futures.book (the depth of one range), futures.trades, market.chart and futures.quote. futures.quote is a read-only price and never signs. Read the index oracle with oracle.read_months, oracle.read_sources, oracle.read_recipe and oracle.read_catalog; source candidates with oracle.read_candidates. Read futures.positions, wallet.balance, oracle.read_collateral and history.read only for an explicit ownerPubkey. Use wallet.address to identify the connected local wallet. Trades (petri long, short, close, cancel, deposit, withdraw) and oracle writes (petri oracle collateral, evidence, sources, prints, updates, stakes) sign with the user's local wallet and are run by the user in a terminal after reviewing them; no MCP tool signs, sends, places, cancels or closes anything. Research agents submit source candidates through the Petri research workspace (petri research mcp), not through this server. No shell, raw transaction, arbitrary-file, or secret-export tool is exposed.";

const allTools = [
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
    description: "List futures markets: price, 24h change, index, 24h volume, settlement date and status.",
    inputSchema: objectSchema({}),
    cli() {
      return ["markets"];
    },
  },
  {
    name: "market.show",
    description: "Read one futures market: price, index, suggested range, best prices, activity and settlement.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market symbol (ramx), name (ramx-nov26) or address"),
      expiryId: stringSchema("Optional expiry for a bare symbol, such as nov26"),
    }, ["marketId"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["markets", requiredSafeIdentifier(args, "marketId")];
      if (args.expiryId !== undefined) out.push("--expiry", requiredSafeIdentifier(args, "expiryId"));
      return out;
    },
  },
  {
    name: "market.chart",
    description: "Read a futures market's price chart: candles of the trades on its price line.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market symbol (ramx), name (ramx-nov26) or address"),
      expiryId: stringSchema("Optional expiry for a bare symbol, such as nov26"),
      window: {
        ...stringSchema("Time window, default 24h"),
        enum: ["1h", "24h", "7d", "30d", "all"],
      },
      height: boundedIntegerSchema("Chart height in terminal rows, default 14", 6, 30),
    }, ["marketId"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["chart", requiredSafeIdentifier(args, "marketId")];
      if (args.window !== undefined) out.push(requiredSafeIdentifier(args, "window"));
      if (args.expiryId !== undefined) out.push("--expiry", requiredSafeIdentifier(args, "expiryId"));
      if (args.height !== undefined) out.push("--height", String(requiredBoundedInteger(args, "height", 6, 30)));
      return out;
    },
  },
  {
    name: "futures.book",
    description: "Read the executable depth of one range: every price to buy it (sell rows) and to sell it (buy rows), with lots and cumulative lots. Defaults to the market's suggested range.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market symbol (ramx), name (ramx-nov26) or address"),
      range: stringSchema("Optional range in index points, LO-HI such as 90-120"),
      depth: positiveIntegerSchema("Price levels per side, default 10"),
    }, ["marketId"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["book", requiredSafeIdentifier(args, "marketId")];
      if (args.range !== undefined) out.push("--range", requiredSafeIdentifier(args, "range"));
      if (args.depth !== undefined) {
        out.push("--depth", String(requiredPositiveSafeInteger(args, "depth")));
      }
      return out;
    },
  },
  {
    name: "futures.trades",
    description: "Read a futures market's recent trades: time, taker side, lots, entry price and range.",
    inputSchema: objectSchema({
      marketId: stringSchema("Market symbol (ramx), name (ramx-nov26) or address"),
      limit: boundedIntegerSchema("Maximum trades, default 20", 1, 1000),
    }, ["marketId"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["trades", requiredSafeIdentifier(args, "marketId")];
      if (args.limit !== undefined) {
        out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 1000)));
      }
      return out;
    },
  },
  {
    name: "futures.positions",
    description: "Read a wallet's futures positions (range, lots, entry, cost, max payout, P&L at the index, what closing returns now), resting orders and cash in each market.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["positions", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "futures.quote",
    description: "Price a futures order without signing: the range (from leverage and the suggested range, or named), lots filled, average entry, what it costs and the most it pays. Read-only; it never places, signs, or sends an order.",
    inputSchema: objectSchema({
      side: {
        ...stringSchema("long (buy the range) or short (sell it)"),
        enum: ["long", "short"],
      },
      marketId: stringSchema("Market symbol (ramx), name (ramx-nov26) or address"),
      lots: {
        type: "integer",
        minimum: 1,
        maximum: Number.MAX_SAFE_INTEGER,
        description: "Lots; a lot pays $0.01 per index point inside its range",
      },
      leverage: {
        type: "integer",
        minimum: 1,
        maximum: Number.MAX_SAFE_INTEGER,
        description: "Optional leverage, default 10: sets where the range ends on the losing side",
      },
      to: stringSchema("Optional end of the range on the winning side, in index points"),
      range: stringSchema("Optional whole range LO-HI in index points, instead of leverage and to"),
      limit: stringSchema("Optional limit entry price, in index points"),
    }, ["side", "marketId", "lots"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const side = requiredString(args, "side");
      if (!["long", "short"].includes(side)) {
        throw new Error("side must be long or short");
      }
      const out = [
        "quote", side, requiredSafeIdentifier(args, "marketId"),
        String(requiredPositiveSafeInteger(args, "lots")),
      ];
      if (args.leverage !== undefined) out.push("--leverage", String(requiredPositiveSafeInteger(args, "leverage")));
      if (args.to !== undefined) out.push("--to", requiredSafeIdentifier(args, "to"));
      if (args.range !== undefined) out.push("--range", requiredSafeIdentifier(args, "range"));
      if (args.limit !== undefined) out.push("--limit", requiredSafeIdentifier(args, "limit"));
      return out;
    },
  },
  {
    name: "oracle.read_months",
    description: "Read the index oracle's months from the futures API: phase, source count, settlement status and month account, for every market or one.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx, or a month such as RAMX-202610"),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "months"];
      if (args.market) out.push(requiredSafeIdentifier(args, "market"));
      return out;
    },
  },
  {
    name: "oracle.read_latest",
    description: "Read the most recently updated oracle month.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "latest"];
      if (args.market) out.push(requiredSafeIdentifier(args, "market"));
      return out;
    },
  },
  {
    name: "oracle.read_history",
    description: "Read oracle months by last update, newest first.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "history"];
      if (args.market) out.push(requiredSafeIdentifier(args, "market"));
      return out;
    },
  },
  {
    name: "oracle.read_catalog",
    description: "Read every oracle period with its settlement time.",
    inputSchema: objectSchema({}),
    annotations: readOnlyAnnotations(),
    cli() {
      return ["oracle", "catalog"];
    },
  },
  {
    name: "oracle.read_sources",
    description: "Read a month's verified source inventory: source id, bucket, status, weight, current and baseline values, and the canonical URL.",
    inputSchema: objectSchema({
      market: stringSchema("Market id such as ramx, or a month such as RAMX-202610"),
      expiry: stringSchema("Optional month for a bare market id: oct26, 2026-10 or RAMX-202610"),
      query: stringSchema("Optional match on bucket, source id or address, status or URL"),
      limit: positiveIntegerSchema("Rows per month, default 40"),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "sources"];
      if (args.market) out.push(requiredSafeIdentifier(args, "market"));
      if (args.expiry) out.push("--expiry", requiredSafeIdentifier(args, "expiry"));
      if (args.query) out.push("--query", requiredSafeIdentifier(args, "query"));
      if (args.limit !== undefined) out.push("--limit", String(requiredPositiveSafeInteger(args, "limit")));
      return out;
    },
  },
  {
    name: "oracle.read_recipe",
    description: "Read a month's buckets with their weights, source counts and median current value.",
    inputSchema: objectSchema({
      market: stringSchema("Market id such as ramx, or a month such as RAMX-202610"),
      expiry: stringSchema("Optional month for a bare market id"),
      query: stringSchema("Optional match on bucket, source, status or URL"),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "recipe"];
      if (args.market) out.push(requiredSafeIdentifier(args, "market"));
      if (args.expiry) out.push("--expiry", requiredSafeIdentifier(args, "expiry"));
      if (args.query) out.push("--query", requiredSafeIdentifier(args, "query"));
      return out;
    },
  },
  {
    name: "oracle.read_candidates",
    description: "Read source candidates submitted by research agents. Candidates carry no oracle weight.",
    inputSchema: objectSchema({
      market: stringSchema("Optional market id such as ramx"),
      expiry: stringSchema("Optional month id such as RAMX-202610"),
      agentId: stringSchema("Optional 64-hex agent id"),
      limit: boundedIntegerSchema("Maximum candidates, default 50", 1, 100),
    }),
    annotations: readOnlyAnnotations(),
    cli(args) {
      const out = ["oracle", "candidates"];
      if (args.market) out.push("--market", requiredSafeIdentifier(args, "market"));
      if (args.expiry) out.push("--expiry", requiredSafeIdentifier(args, "expiry"));
      if (args.agentId) out.push("--agent", requiredSafeIdentifier(args, "agentId"));
      if (args.limit !== undefined) out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 100)));
      return out;
    },
  },
  {
    name: "oracle.read_collateral",
    description: "Read one wallet's USDC bond collateral in the oracle (available and locked).",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["oracle", "collateral", "--owner", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "wallet.balance",
    description: "Read a wallet's SOL, its USDC (the deployment's mint), its cash in each futures market and its oracle bond collateral.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
    }, ["ownerPubkey"]),
    annotations: readOnlyAnnotations(),
    cli(args) {
      return ["wallet", requiredSafeIdentifier(args, "ownerPubkey")];
    },
  },
  {
    name: "history.read",
    description: "Read a wallet's futures activity: trades, fills of its orders, orders, cancels, deposits, withdrawals, netting and settlements.",
    inputSchema: objectSchema({
      ownerPubkey: stringSchema("Exact owner public key"),
      limit: boundedIntegerSchema("Maximum activity rows", 1, 1000),
    }, ["ownerPubkey"]),
    cli(args) {
      const out = ["history", requiredSafeIdentifier(args, "ownerPubkey")];
      if (args.limit !== undefined) {
        out.push("--limit", String(requiredBoundedInteger(args, "limit", 1, 1000)));
      }
      return out;
    },
  },
];

const tools = [...allTools];

const toolMap = new Map(tools.map((tool) => [tool.name, tool]));
let publicActionsLoaded = false;
const resident = new ResidentPetri({command:petriCommand, environment:() => petriChildEnvironment(process.env, true), cwd:repoRoot});

// The Rust registry is the single source for both the manifest and action schemas.
// An old binary must report an integration error, not silently advertise partial parity.
function loadPublicActions() {
  if (publicActionsLoaded) return;
  const manifest = runPetri(["mcp", "actions"]);
  if (manifest?.ok !== true || manifest.protocol !== "petri-public-actions.v1" || manifest.transport !== "resident-v1" || !Array.isArray(manifest.tools)) {
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
      async run(args) {
        const request = JSON.stringify(args);
        if (Buffer.byteLength(request, "utf8") > 24 * 1024) throw new Error("Public action request exceeds its size bound");
        const result = await resident.invoke(definition.name, args);
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
  rl.on("close", () => { void resident.close(); });
}

async function handleLine(line) {
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
    const result = await handleRequest(message);
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

async function callTool(params) {
  loadPublicActions();
  const name = requiredString(params, "name");
  const tool = toolMap.get(name);
  if (!tool) {
    throw new Error(`Unknown Petri MCP tool: ${name}`);
  }

  const args = params.arguments && typeof params.arguments === "object" ? params.arguments : {};
  rejectUnknownArguments(tool, args);
  const commandOrPayload = await (tool.native
      ? tool.native(args)
      : tool.run
        ? tool.run(args)
        : tool.cli(args));
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

function inferredAnnotations() {
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
    // Preserve the user's configured wallet only for wallet.address.
    if (publicAction && (name === "SOLANA_CONFIG" || name === "SOLANA_KEYPAIR")) continue;
    delete child[name];
  }
  child.PETRI_MCP_READ_ONLY = "1";
  // No inherited transaction-mode switch can elevate a read.
  delete child.PETRI_MCP_TRANSACTION_MODE;
  return child;
}

function petriCommand() {
  if (process.env.PETRI_MCP_PETRI_BIN) {
    return { bin: process.env.PETRI_MCP_PETRI_BIN, prefix: [] };
  }

  const binary = newestExistingPath([
    path.join(repoRoot, "target", "release", process.platform === "win32" ? "petri.exe" : "petri"),
    path.join(repoRoot, "target", "debug", process.platform === "win32" ? "petri.exe" : "petri"),
  ]);
  if (binary) {
    return { bin: binary, prefix: [] };
  }

  return {
    bin: process.execPath,
    prefix: [path.join(scriptDir, "petri-launcher.mjs")],
  };
}

const sensitivePetriFlags = new Set([
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

function positiveIntegerSchema(description) {
  return {
    type: "integer",
    minimum: 1,
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
