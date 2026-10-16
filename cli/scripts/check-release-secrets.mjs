#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  assertCredentialScannerSelfTest,
  assertNoKnownCompromisedCredential,
} from "./credential-scan.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
assertCredentialScannerSelfTest();

const files = execFileSync("git", [
  "-C",
  root,
  "ls-files",
  "--cached",
  "--others",
  "--exclude-standard",
  "-z",
])
  .toString("utf8")
  .split("\0")
  .filter(Boolean)
  .sort();

let scannedFiles = 0;
for (const file of files) {
  const absolute = path.join(root, file);
  if (!existsSync(absolute)) continue;
  if (!lstatSync(absolute).isFile()) continue;
  scannedFiles += 1;
  assertNoKnownCompromisedCredential(
    readFileSync(absolute),
    file,
    "current tracked source",
  );
}

console.log(`Release credential scan passed (${scannedFiles} tracked/untracked files).`);
