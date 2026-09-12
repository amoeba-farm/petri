#!/usr/bin/env node

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdtempSync, writeFileSync, unlinkSync, rmdirSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const root = path.resolve(path.dirname(scriptPath), "..");
const binaryFlag = process.argv.indexOf("--binary");
const defaultBinary = path.join(
  root,
  "target",
  "debug",
  process.platform === "win32" ? "petri.exe" : "petri",
);
let binary = binaryFlag >= 0 ? path.resolve(process.argv[binaryFlag + 1] ?? "") : defaultBinary;
if (process.platform === "win32" && binary && !existsSync(binary) && existsSync(`${binary}.exe`)) {
  binary = `${binary}.exe`;
}

if (!binary || !existsSync(binary)) {
  throw new Error(`Petri binary not found: ${binary || "<missing --binary value>"}`);
}

const temporary = mkdtempSync(path.join(os.tmpdir(), "petri-v3-refusal-"));
const configPath = path.join(temporary, "config.yml");
writeFileSync(configPath, "commitment: finalized\n", {mode:0o600});
const requests = [];
const server = createServer((request, response) => {
  requests.push({method:request.method, url:request.url});
  response.writeHead(200, {"Content-Type":"application/json"});
  response.end(JSON.stringify({ok:false,error:"uninitialized fixture"}));
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const pubkey = "11111111111111111111111111111111";
const shared = [
  "--backend-url",
  `http://127.0.0.1:${server.address().port}`,
  "--solana-config",
  configPath,
  "--keypair",
  path.join(temporary, "missing-keypair.json"),
  "--no-color",
];

const cases = [
  ["trades prepare", ["trades", "prepare", "--market", pubkey, "--direction", "quote-for-option", "--amount-in", "1", "--minimum-amount-out", "1", "--limit-bin-id", "0"]],
  ["trades submit", ["trades", "submit", "--market", pubkey, "--direction", "option-for-quote", "--amount-in", "1", "--minimum-amount-out", "1", "--limit-bin-id", "0"]],
  ["writers deposit", ["writers", "deposit", "--sleeve", pubkey, "--amount", "1"]],
  ["writers bid", ["writers", "bid", "--auction", pubkey, "--series-index", "0", "--price", "1", "--amount", "1"]],
  ["writers close start", ["writers", "close", "--sleeve", pubkey, "--amount", "1", "--minimum-withdrawal", "0"]],
  ["writers close advance", ["writers", "close", "--close-request", pubkey]],
  ["writers close cancel", ["writers", "close", "--close-request", pubkey, "--cancel"]],
  ["writers claim flat", ["writers", "claim", "--sleeve", pubkey, "--variant", "flat-residual", "--amount", "1"]],
  ["writers claim long", ["writers", "claim", "--sleeve", pubkey, "--variant", "collective-long", "--series-index", "0", "--amount", "1"]],
  ["writers transfer Flat", ["writers", "transfer-flat", "--sleeve", pubkey, "--destination", pubkey, "--amount", "1"]],
  ["staking stake", ["staking", "stake", "--amount", "1"]],
  ["staking activate", ["staking", "activate"]],
  ["staking cancel", ["staking", "cancel"]],
  ["staking unstake", ["staking", "unstake", "--amount", "1"]],
  ["staking claim", ["staking", "claim"]],
];

try {
  for (const [label, args] of cases) {
    const result = await new Promise((resolve, reject) => {
      const child = spawn(binary, [...shared, ...args], {env:{...process.env,NO_COLOR:"1"},windowsHide:true});
      let stdout = "", stderr = "";
      child.stdout.on("data", chunk => stdout += chunk);
      child.stderr.on("data", chunk => stderr += chunk);
      const timer = setTimeout(() => {child.kill(); reject(new Error(`${label}: timeout`));},15000);
      child.on("error", error => {clearTimeout(timer); reject(error);});
      child.on("close", status => {clearTimeout(timer); resolve({status,stdout,stderr});});
    });
    if (result.status !== 1 || result.stdout.trim())
      throw new Error(`${label}: expected scoped refusal without output: ${JSON.stringify(result)}`);
    if (!/could not verify|not_wired|not wired|ACTION_NOT_RELEASED|action.mask|CURRENT_PROGRAM_|not released|unavailable/iu.test(result.stderr))
      throw new Error(`${label}: unexpected refusal: ${result.stderr}`);
    if (/failed to (?:read|open).*keypair|keypair.*(?:not found|does not exist)|hardware wallet/iu.test(result.stderr))
      throw new Error(`${label}: crossed signer boundary before runtime admission: ${result.stderr}`);
  }
  if (requests.some(request => request.method !== "GET" || !request.url.includes("identity")))
    throw new Error(`Uninitialized/mismatched service reached preparation or mutation: ${JSON.stringify(requests)}`);
  console.log(`V3 runtime refusal verified for ${cases.length} CLI mutation forms: no preparation, signer access or submission.`);
} finally {
  await new Promise(resolve => server.close(resolve));
  unlinkSync(configPath);
  rmdirSync(temporary);
}
