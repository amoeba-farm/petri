import { spawn } from "node:child_process";
import { once } from "node:events";

// Drives scripts/petri-mcp-server.mjs over stdio against unreachable
// endpoints: the protocol, the read-only tool inventory, argument validation,
// terminal-safe output and the child-environment scrub. Needs a built
// target/debug/petri (or PETRI_MCP_PETRI_BIN).

const terminalAttack =
  "unsafe\u001b]8;;https://example.test\u0007link\u007f\u0085­؜​‮⁠⁦﻿\u{e0001}tail";
const terminalAttackControls = [
  "\u001b", "\u0007", "\u007f", "\u0085", "­", "؜",
  "​", "‮", "⁠", "⁦", "﻿", "\u{e0001}",
];
const owner = "G2f6Fv477ZyFbmRFtVr21jf9VufXpiRCxf1e91J6SxxT";

const child = spawn(process.execPath, ["scripts/petri-mcp-server.mjs"], {
  cwd: new URL("..", import.meta.url),
  stdio: ["pipe", "pipe", "pipe"],
  env: {
    ...process.env,
    PETRI_API_URL: "http://127.0.0.1:9",
    PETRI_RPC_URL: "http://127.0.0.1:9",
  },
});

let stdout = "";
let stderr = "";
child.stdout.setEncoding("utf8");
child.stderr.setEncoding("utf8");
child.stdout.on("data", (chunk) => { stdout += chunk; });
child.stderr.on("data", (chunk) => { stderr += chunk; });

send(1, "initialize", {
  protocolVersion: "2025-06-18",
  capabilities: {},
  clientInfo: { name: "petri-mcp-check", version: "0.1.0" },
});
send(2, "tools/list", {});
send(3, "tools/call", { name: "petri.mcp_manifest", arguments: {} });
send(4, "tools/call", { name: "futures.book", arguments: { marketId: "ramx", ownerPubkey: owner } });
send(5, "tools/call", { name: "futures.quote", arguments: { side: "long", marketId: "ramx", lots: 0, leverage: 5 } });
send(6, "tools/call", { name: "futures.quote", arguments: { side: "buy", marketId: "ramx", lots: 1, leverage: 5 } });
send(7, "tools/call", { name: "futures.quote", arguments: { side: "short", marketId: "ramx", lots: 1, leverage: 0 } });
send(8, "tools/call", { name: "wallet.balance", arguments: {} });
send(9, "tools/call", { name: "oracle.read_collateral", arguments: {} });
send(10, "tools/call", { name: "wallet.address", arguments: { unexpected: true } });
send(11, "tools/call", { name: "market.show", arguments: { marketId: "ramx", expiryId: "--yes" } });
send(12, "tools/call", { name: "oracle.read_sources", arguments: { market: "ramx", query: "--simulate" } });
send(13, "tools/call", { name: "oracle.read_months", arguments: {} });
send(14, "tools/call", { name: "futures.book", arguments: { marketId: "ramx", depth: 5 } });
for (const [id, name] of [[15, "futures.buy"], [16, "trade.prepare"], [17, "settlement.check"], [18, "futures.account"], [19, "operations.execute"], [20, "wallet.collateral"]]) {
  send(id, "tools/call", { name, arguments: {} });
}
send(21, terminalAttack, {});
child.stdin.end();

const timeout = setTimeout(() => { child.kill("SIGKILL"); }, 60_000);
await once(child, "exit");
clearTimeout(timeout);

const responses = stdout.trim().split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line));
for (const control of terminalAttackControls) {
  assert(!stdout.includes(control), "MCP JSON wire output neutralizes terminal controls");
}
const terminalWireError = responses.find((item) => item.id === 21);
assert(terminalWireError?.error?.message.includes(terminalAttack), "wire escaping preserves parsed JSON error semantics");

const initialize = responseById(1);
assert(initialize.result?.serverInfo?.name === "petri-mcp", "initialize returned serverInfo");
assert(initialize.result?.protocolVersion === "2025-06-18", "initialize negotiated the latest protocol");
const instructions = initialize.result?.instructions || "";
assert(instructions.includes("read-only") && instructions.includes("run by the user in a terminal"),
  "instructions keep signing out of MCP");

const tools = responseById(2).result?.tools || [];
const names = tools.map((tool) => tool.name).sort();
for (const name of [
  "petri.mcp_manifest", "market.list", "market.show", "market.chart", "futures.book", "futures.trades",
  "futures.positions", "futures.quote", "oracle.read_months", "oracle.read_latest", "oracle.read_history",
  "oracle.read_catalog", "oracle.read_sources", "oracle.read_recipe", "oracle.read_candidates",
  "oracle.read_collateral", "wallet.balance", "history.read", "wallet.address",
]) {
  assert(names.includes(name), `tools/list includes ${name}`);
}
for (const tool of tools) {
  assert(tool.annotations?.readOnlyHint === true && tool.annotations?.destructiveHint === false,
    `${tool.name} is read-only`);
  assert(tool.inputSchema?.additionalProperties === false, `${tool.name} rejects undeclared arguments`);
  assert(tool.outputSchema?.type === "object", `${tool.name} declares an output schema`);
}
assert(!names.some((name) => /prepare|execute|draft|staking|stake|amba|emergency|reward|operations|settlement/u.test(name)),
  "no write, draft or retired tool is listed");
for (const retired of ["futures.account", "wallet.collateral", "market.status", "market.print"]) {
  assert(!names.includes(retired), `retired tool ${retired} is gone`);
}

const manifest = responseById(3).result?.structuredContent;
const manifestNames = (manifest?.tools || []).map((tool) => tool.name).sort();
assert(JSON.stringify(names) === JSON.stringify(manifestNames),
  "the Rust manifest and stdio tools/list expose the same tool names");
assert(manifest?.safety?.signing?.includes("No MCP tool signs or sends"), "manifest states the signing boundary");

const refused = (id, pattern, label) => {
  const response = responses.find((item) => item.id === id);
  const text = response?.error?.message || JSON.stringify(response?.result?.structuredContent || {});
  assert(response?.error || response?.result?.isError === true, `${label} is refused`);
  if (pattern) assert(pattern.test(text), `${label} names its reason: ${text}`);
};
refused(4, /does not accept argument ownerPubkey/u, "an undeclared book filter");
refused(5, /lots must be a positive safe integer/u, "zero quote lots");
refused(6, /side must be long or short/u, "an unknown quote side");
refused(7, /leverage must be a positive safe integer/u, "zero leverage");
refused(8, /ownerPubkey is required/u, "a balance read without owner");
refused(9, /ownerPubkey is required/u, "a collateral read without owner");
refused(10, null, "wallet.address with undeclared fields");
refused(11, /must be an identifier, not a command flag/u, "a flag-like expiry");
refused(12, /must be an identifier, not a command flag/u, "a flag-like oracle query");
for (const id of [15, 16, 17, 18, 19, 20]) refused(id, /Unknown Petri MCP tool/u, `retired tool ${id}`);
// Reads against an unreachable API fail as typed results, not protocol errors.
for (const id of [13, 14]) {
  const result = responseById(id).result;
  assert(result?.isError === true && result.structuredContent?.ok === false, `read ${id} reports the unreachable API`);
}
assert(!stderr.trim(), `server stderr should be empty, got: ${stderr}`);

const mcpModule = await import(`./petri-mcp-server.mjs?read-check=${Date.now()}`);
const childOutput = mcpModule.safeProcessOutput(terminalAttack, []);
const displayJson = mcpModule.terminalSafeJsonStringify({ field: terminalAttack }, 2);
for (const control of terminalAttackControls) {
  assert(!childOutput.includes(control), "child output neutralizes terminal controls");
  assert(!displayJson.includes(control), "display JSON neutralizes terminal controls");
}
assert(JSON.parse(displayJson).field === terminalAttack, "display JSON escaping preserves values");
const scrubbed = ["SOLANA_CONFIG", "SOLANA_KEYPAIR", "AMEBA_ALLOW_INSECURE_KEYPAIR", "SOLANA_RPC_URL", "HELIUS_RPC_URL", "HELIUS_API_KEY"];
const source = Object.fromEntries(scrubbed.map((name) => [name, `test-only-${name}`]));
source.PETRI_API_URL = "http://127.0.0.1:9";
source.PETRI_MCP_READ_ONLY = "attacker-controlled";
const environment = mcpModule.petriChildEnvironment(source);
for (const name of scrubbed) {
  assert(!Object.hasOwn(environment, name), `the child environment strips ${name}`);
  assert(source[name] === `test-only-${name}`, `stripping ${name} leaves the source intact`);
}
assert(environment.PETRI_API_URL === "http://127.0.0.1:9", "the child keeps the futures API setting");
assert(environment.PETRI_MCP_READ_ONLY === "1", "the child is forced read-only");
console.log("Petri MCP server checks passed.");

function send(id, method, params) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
}

function responseById(id) {
  const response = responses.find((item) => item.id === id);
  assert(response, `response ${id} exists`);
  assert(!response.error, `response ${id} has no error`);
  return response;
}

function assert(condition, message) {
  if (!condition) {
    console.error("stdout:");
    console.error(stdout);
    console.error("stderr:");
    console.error(stderr);
    throw new Error(message);
  }
}
