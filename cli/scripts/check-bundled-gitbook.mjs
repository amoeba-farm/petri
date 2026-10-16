#!/usr/bin/env node

import { access, copyFile, lstat, mkdir, readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const bundledRoot = path.join(repoRoot, "assets", "gitbook");
const authoringRoot = path.resolve(repoRoot, "..", "docs-gitbook");
const rustSourcePath = path.join(repoRoot, "src", "gitbook.rs");

const bundledFiles = [
  "SUMMARY.md",
  "README.md",
  "docs/synthetics/01-about-synthetics.md",
  "docs/synthetics/02-what-synthetics-offer-individuals.md",
  "docs/overview/01-why-amoeba-farm.md",
  "docs/overview/02-markets-ramx-nandx-future.md",
  "docs/getting-started/01-where-to-begin.md",
  "docs/getting-started/02-cli.md",
  "docs/getting-started/03-web-client.md",
  "docs/getting-started/04-claude-code-codex.md",
  "docs/trading/01-about-capped-futures.md",
  "docs/trading/02-fees.md",
  "docs/oracle/01-how-it-is-structured.md",
  "docs/oracle/02-token-holders-and-contributors.md",
  "docs/oracle/03-reward-collection.md",
  "docs/developer/01-api.md",
  "docs/04-products-indexes-buckets.md",
  "docs/lifecycle/01-market-lifecycle.md",
  "docs/contributors/02-evidence-rules.md",
  "docs/markets/03-risk-disclosures.md",
  "docs/reference/glossary.md",
];

const args = new Set(process.argv.slice(2));
const sync = args.delete("--sync");
const requireAuthoringSource = args.delete("--require-authoring-source");
if (args.size > 0) {
  throw new Error(`Unknown argument(s): ${[...args].join(", ")}`);
}

function portable(relativePath) {
  return relativePath.split(path.sep).join("/");
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return (
    relative === "" ||
    (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`))
  );
}

async function exists(candidate) {
  try {
    await access(candidate);
    return true;
  } catch {
    return false;
  }
}

async function listFiles(root, prefix = "") {
  if (!(await exists(root))) return [];
  const entries = await readdir(root, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const relative = path.join(prefix, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFiles(path.join(root, entry.name), relative)));
    } else {
      files.push(portable(relative));
    }
  }
  return files.sort();
}

async function syncFromAuthoringSource() {
  if (!(await exists(authoringRoot))) {
    throw new Error(
      `Cannot sync: the private authoring checkout is not available at ${authoringRoot}`,
    );
  }
  for (const relative of bundledFiles) {
    const source = path.join(authoringRoot, relative);
    const destination = path.join(bundledRoot, relative);
    if (!isInside(authoringRoot, source) || !isInside(bundledRoot, destination)) {
      throw new Error(`Refusing unsafe bundled docs path: ${relative}`);
    }
    const sourceStat = await lstat(source);
    if (!sourceStat.isFile() || sourceStat.isSymbolicLink()) {
      throw new Error(`Refusing non-file authoring source: ${relative}`);
    }
    if ((await exists(destination)) && (await lstat(destination)).isSymbolicLink()) {
      throw new Error(`Refusing symbolic-link sync destination: ${relative}`);
    }
    await mkdir(path.dirname(destination), { recursive: true });
    await copyFile(source, destination);
  }
}

async function checkRustCompileInputs(errors) {
  const source = await readFile(rustSourcePath, "utf8");
  const inputs = [...source.matchAll(/include_str!\("([^"]+)"\)/g)].map((match) => match[1]);
  const resolvedInputs = inputs.map((input) => path.resolve(path.dirname(rustSourcePath), input));

  for (const [index, resolved] of resolvedInputs.entries()) {
    if (!isInside(repoRoot, resolved)) {
      errors.push(`src/gitbook.rs compile input escapes ameba_cli: ${inputs[index]}`);
    } else if (!isInside(bundledRoot, resolved)) {
      errors.push(`src/gitbook.rs compile input is outside assets/gitbook: ${inputs[index]}`);
    }
  }

  const actual = resolvedInputs
    .filter((resolved) => isInside(bundledRoot, resolved))
    .map((resolved) => portable(path.relative(bundledRoot, resolved)));
  if (new Set(actual).size !== actual.length) {
    errors.push("src/gitbook.rs contains duplicate bundled documentation inputs");
  }
  if (JSON.stringify(actual) !== JSON.stringify(bundledFiles)) {
    errors.push("src/gitbook.rs bundled input manifest does not match the checked-in guide manifest");
  }
}

async function checkBundledTree(errors) {
  const actualFiles = await listFiles(bundledRoot);
  const expectedFiles = [...bundledFiles].sort();
  if (JSON.stringify(actualFiles) !== JSON.stringify(expectedFiles)) {
    errors.push(
      `assets/gitbook file manifest differs (expected ${expectedFiles.length}, found ${actualFiles.length})`,
    );
  }

  for (const relative of bundledFiles) {
    const candidate = path.join(bundledRoot, relative);
    if (!isInside(bundledRoot, candidate) || !(await exists(candidate))) {
      errors.push(`missing bundled guide file: ${relative}`);
      continue;
    }
    const stat = await lstat(candidate);
    if (stat.isSymbolicLink()) {
      errors.push(`bundled guide file must not be a symbolic link: ${relative}`);
      continue;
    }
    const contents = await readFile(candidate);
    if (contents.length === 0) errors.push(`bundled guide file is empty: ${relative}`);
  }

  if (await exists(path.join(bundledRoot, "SUMMARY.md"))) {
    const summary = await readFile(path.join(bundledRoot, "SUMMARY.md"), "utf8");
    const linkedPages = [...summary.matchAll(/^\* \[[^\]]+\]\(([^)]+)\)$/gm)].map(
      (match) => match[1],
    );
    if (JSON.stringify(linkedPages) !== JSON.stringify(bundledFiles.slice(1))) {
      errors.push("assets/gitbook/SUMMARY.md links do not match the bundled page manifest");
    }
  }
}

async function checkAuthoringParity(errors) {
  const sourceAvailable = await exists(authoringRoot);
  if (!sourceAvailable) {
    if (requireAuthoringSource) {
      errors.push(`private authoring checkout is required but missing: ${authoringRoot}`);
    }
    return false;
  }

  for (const relative of bundledFiles) {
    const source = path.join(authoringRoot, relative);
    const bundled = path.join(bundledRoot, relative);
    if (!(await exists(source))) {
      errors.push(`private authoring source is missing: ${relative}`);
      continue;
    }
    if (!(await exists(bundled))) continue;
    const [sourceBytes, bundledBytes] = await Promise.all([readFile(source), readFile(bundled)]);
    if (!sourceBytes.equals(bundledBytes)) {
      errors.push(`bundled guide is out of sync with ../docs-gitbook: ${relative}`);
    }
  }
  return true;
}

if (sync) await syncFromAuthoringSource();

const errors = [];
await checkRustCompileInputs(errors);
await checkBundledTree(errors);
const authoringSourceChecked = await checkAuthoringParity(errors);

if (errors.length > 0) {
  for (const error of errors) console.error(`ERROR: ${error}`);
  process.exitCode = 1;
} else {
  const action = sync ? "synced and checked" : "checked";
  const parity = authoringSourceChecked ? "authoring source matched" : "standalone mode";
  console.log(`Bundled GitBook ${action}: ${bundledFiles.length} files (${parity}).`);
}
