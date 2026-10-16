#!/usr/bin/env node
// Checks which deployment a petri binary talks to with no flags, environment
// or saved settings: `production` for a release build (no dev-chain feature),
// `dev` for the dev build (scripts/install-dev.sh). Reads only
// `petri config --json`; nothing is sent anywhere.
//
//   node scripts/check-petri-defaults.mjs <petri binary> production|dev
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [binary, profile] = process.argv.slice(2);
const expected = {
  production: {
    profile: "production",
    cluster: "mainnet-beta",
    apiUrl: "https://api.amoeba.farm",
    rpcUrl: "https://api.mainnet-beta.solana.com",
  },
  dev: {
    profile: "dev",
    cluster: "localnet",
    apiUrl: "http://127.0.0.1:4500",
    rpcUrl: "http://127.0.0.1:8899",
  },
}[profile];
if (!binary || !expected) {
  console.error("usage: check-petri-defaults.mjs <petri binary> production|dev");
  process.exit(2);
}

const scratch = mkdtempSync(join(tmpdir(), "petri-defaults-"));
try {
  const env = { ...process.env, PETRI_CONFIG_PATH: join(scratch, "config.json") };
  for (const name of ["PETRI_PROFILE", "PETRI_API_URL", "PETRI_RPC_URL", "PETRI_DEPLOYMENT"]) {
    delete env[name];
  }
  const shown = JSON.parse(execFileSync(binary, ["config", "--json"], { env, encoding: "utf8" }));
  for (const [key, value] of Object.entries(expected)) {
    if (shown[key] !== value) {
      throw new Error(`${binary} defaults to ${key} ${JSON.stringify(shown[key])}, not ${JSON.stringify(value)}`);
    }
  }
  for (const key of ["profileSource", "apiUrlSource", "rpcUrlSource"]) {
    if (shown[key] !== "default") {
      throw new Error(`${binary}: ${key} is ${JSON.stringify(shown[key])}, not the default`);
    }
  }
  if (shown.deployment !== null) {
    throw new Error(`${binary} names a deployment manifest by default: ${shown.deployment}`);
  }
  console.log(`petri defaults to ${profile}: ${expected.apiUrl}, ${expected.cluster}.`);
} catch (error) {
  console.error(`check-petri-defaults: ${error.message}`);
  process.exitCode = 1;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
