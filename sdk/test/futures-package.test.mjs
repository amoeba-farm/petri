// Package surface: the published entry points resolve, the futures module
// imports only web3.js and buffer, and it bundles for browsers.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const manifest = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));

test("ameba-sdk and ameba-sdk/futures resolve through the export map to the same API", async () => {
  assert.deepEqual(Object.keys(manifest.exports).sort(), [".", "./futures", "./package.json"]);
  const root = await import("ameba-sdk");
  const futures = await import("ameba-sdk/futures");
  assert.deepEqual(Object.keys(root).sort(), Object.keys(futures).sort());
  for (const name of ["FuturesClient", "FuturesMarket", "FuturesEarnVault", "FuturesBook", "decodeProposal",
    "decodeFuturesInstruction", "decodeVault", "findRegistryAddress", "findPeriodAddress", "transactionErrorName",
    "GovernanceGateBinding"]) {
    assert.equal(typeof futures[name], "function", name);
  }
  for (const name of ["FuturesDeployment", "decodeInstrument", "parseFuturesEvents", "bindClearingGovernance"]) {
    assert.equal(futures[name], undefined, `${name} (the removed clearing surface)`);
  }
  // The product is futures: no export is named for "range" (the 2026-10-15 rename).
  assert.deepEqual(Object.keys(futures).filter((name) => /range/iu.test(name)), []);
  assert.deepEqual(Object.keys(manifest.dependencies).sort(), ["@solana/web3.js", "buffer"]);
});

test("the futures module imports nothing but its own files, @solana/web3.js and buffer", () => {
  const dir = new URL("../dist/futures/", import.meta.url);
  const specifiers = new Set();
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".js"))) {
    const source = readFileSync(new URL(file, dir), "utf8");
    for (const m of source.matchAll(/(?:^|\n)\s*(?:import|export)\b[^"']*?from\s*["']([^"']+)["']/gu)) specifiers.add(m[1]);
    for (const m of source.matchAll(/\bimport\(\s*["']([^"']+)["']\s*\)/gu)) specifiers.add(m[1]);
  }
  const external = [...specifiers].filter((s) => !s.startsWith("./")).sort();
  assert.deepEqual(external, ["@solana/web3.js", "buffer"]);
});

test("the futures entry bundles for the browser", async () => {
  const result = await build({
    entryPoints: [fileURLToPath(new URL("../dist/futures/index.js", import.meta.url))],
    bundle: true,
    platform: "browser",
    format: "esm",
    write: false,
    logLevel: "silent",
    metafile: true,
  });
  assert.equal(result.errors.length, 0);
  const inputs = Object.keys(result.metafile.inputs);
  assert(inputs.some((p) => p.replaceAll("\\", "/").includes("dist/futures/index.js")));
  assert(!inputs.some((p) => /^node:/u.test(p)));
});
