#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "..");
const LOCK_PATH = path.join(REPO_ROOT, "Cargo.lock");
const OUTPUT_PATH = path.join(REPO_ROOT, "THIRD_PARTY_LICENSES.md");
const MAX_LEGAL_FILE_BYTES = 8 * 1024 * 1024;
const LEGAL_FILE_NAME =
  /^(?:license|licence|copying|notice|copyright|unlicense)(?:$|[._-])/i;

const FIRST_PARTY_PACKAGE = /^(?:amoeba|ameba)(?:[-_]|$)/;

const LOCAL_PACKAGES = new Map([
  [
    `amoeba@${readFileSync(path.join(REPO_ROOT, 'Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1]}`,
    {
      kind: "workspace package",
      manifest: "Cargo.toml",
      legalFiles: ["LICENSE"],
    },
  ],
  [
    "solana-ed25519-program@2.2.3",
    {
      kind: "vendored crates.io patch",
      manifest: "vendor/solana-ed25519-program-2.2.3/Cargo.toml",
      legalFiles: ["vendor/LICENSE-APACHE-2.0"],
      includeSpdxFallback: true,
    },
  ],
  [
    "solana-keypair@2.2.3",
    {
      kind: "vendored crates.io patch",
      manifest: "vendor/solana-keypair-2.2.3/Cargo.toml",
      legalFiles: ["vendor/LICENSE-APACHE-2.0"],
      includeSpdxFallback: true,
    },
  ],
  [
    "solana-signature@2.3.0",
    {
      kind: "vendored crates.io patch",
      manifest: "vendor/solana-signature-2.3.0/Cargo.toml",
      legalFiles: ["vendor/LICENSE-APACHE-2.0"],
      includeSpdxFallback: true,
    },
  ],
]);

// These donors are exact packages in Cargo.lock. They are used only when an
// upstream .crate archive contains no legal file of its own. The fallback is
// labeled as such in the generated corpus.
const CACHED_STANDARD_TEXTS = new Map([
  [
    "Apache-2.0",
    {
      package: "getrandom",
      version: "0.1.16",
      file: "LICENSE-APACHE",
    },
  ],
  [
    "MIT",
    {
      package: "wasi",
      version: "0.9.0+wasi-snapshot-preview1",
      file: "LICENSE-MIT",
    },
  ],
  [
    "BSD-3-Clause",
    {
      package: "alloc-no-stdlib",
      version: "2.0.4",
      file: "LICENSE",
    },
  ],
  [
    "Apache-2.0 WITH LLVM-exception",
    {
      package: "linux-raw-sys",
      version: "0.4.15",
      file: "LICENSE-Apache-2.0_WITH_LLVM-exception",
    },
  ],
  [
    "Unlicense",
    {
      package: "memchr",
      version: "2.8.0",
      file: "UNLICENSE",
    },
  ],
  [
    "ISC",
    {
      package: "untrusted",
      version: "0.9.0",
      file: "LICENSE.txt",
    },
  ],
]);

// Two locked crates declare standard licenses that no locked .crate archive
// carries as a legal file. Bootstrap from the license steward, pin the bytes,
// then reuse the checked-in corpus as the offline cache on subsequent runs.
const OFFICIAL_STANDARD_TEXTS = new Map([
  [
    "MPL-2.0",
    {
      url: "https://www.mozilla.org/media/MPL/2.0/index.f75d2927d3c1.txt",
      sha256: "3f3d9e0024b1921b067d6f7f88deb4a60cbe7a78e76c64e3f1d7fc3b779b9d04",
      steward: "Mozilla",
    },
  ],
  [
    "LGPL-2.1-or-later",
    {
      url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt",
      sha256: "20e50fe7aae3e56378ebf0417d9de904f55a0e61e4df315333e632a4d3555d95",
      steward: "Free Software Foundation",
    },
  ],
]);

const FALLBACK_EXPRESSIONS = new Map([
  ["Apache-2.0", ["Apache-2.0"]],
  ["MIT", ["MIT"]],
  ["BSD-3-Clause", ["BSD-3-Clause"]],
  ["MIT OR Apache-2.0", ["MIT", "Apache-2.0"]],
  ["MIT/Apache-2.0", ["MIT", "Apache-2.0"]],
  ["Apache-2.0/MIT", ["Apache-2.0", "MIT"]],
  ["MPL-2.0", ["MPL-2.0"]],
  ["Unlicense", ["Unlicense"]],
  ["Apache-2.0 OR ISC OR MIT", ["Apache-2.0", "ISC", "MIT"]],
  [
    "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
    ["MIT", "Apache-2.0", "LGPL-2.1-or-later"],
  ],
  [
    "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
    ["Apache-2.0 WITH LLVM-exception", "Apache-2.0", "MIT"],
  ],
]);

// Git repositories whose README licenses every crate in them while some of
// those crates declare nothing and ship no legal file. Used only for such a
// crate, with the standard texts, and labelled with the statement it rests on.
const REPOSITORY_LICENSE_STATEMENTS = new Map([
  [
    "https://github.com/tlsnotary/mpz2",
    {
      expression: "MIT OR Apache-2.0",
      basis:
        "README.md at the locked revision: \"All crates in this repository are licensed under either of Apache License, Version 2.0 or MIT license\"",
    },
  ],
  [
    "https://github.com/tlsnotary/tlsn",
    {
      expression: "MIT OR Apache-2.0",
      basis:
        "README.md at the locked revision: \"All crates in this repository are licensed under either of Apache License, Version 2.0 or MIT license\"",
    },
  ],
]);

// This locked archive contains a legal file, but the file is MIT text while
// the manifest declares Apache-2.0. Preserve the upstream file and also ship
// the declared standard text rather than silently choosing either one.
const FORCED_FALLBACK_PACKAGES = new Map([
  [
    "aligned-sized@1.1.0",
    "packaged LICENSE text conflicts with the declared Apache-2.0 expression",
  ],
]);

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function compareStrings(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function packageKey(pkg) {
  // Mainnet and historical SDK modules intentionally resolve distinct revisions
  // of the same Rust package. Never collapse their provenance or license files.
  return `${pkg.name}@${pkg.version}${pkg.source?.startsWith("git+") ? ` (${pkg.source})` : ""}`;
}

function parseArgs(argv) {
  if (argv.length === 0) {
    return { check: false };
  }
  if (argv.length === 1 && argv[0] === "--check") {
    return { check: true };
  }
  fail("Usage: node scripts/generate-third-party-licenses.mjs [--check]");
}

function tomlStringValue(section, key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = section.match(
    new RegExp(`^${escaped}\\s*=\\s*("(?:[^"\\\\]|\\\\.)*"|'[^']*')\\s*$`, "m"),
  );
  if (!match) {
    return "";
  }
  const value = match[1];
  if (value.startsWith('"')) {
    try {
      return JSON.parse(value);
    } catch {
      fail(`Unable to parse TOML string for ${key}: ${value}`);
    }
  }
  return value.slice(1, -1);
}

function tomlStringArray(section, key) {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = section.match(
    new RegExp(`^${escaped}\\s*=\\s*\\[([\\s\\S]*?)\\]`, "m"),
  );
  if (!match) {
    return [];
  }
  const values = [];
  const strings = match[1].matchAll(/"(?:[^"\\]|\\.)*"|'[^']*'/g);
  for (const stringMatch of strings) {
    const raw = stringMatch[0];
    if (raw.startsWith('"')) {
      try {
        values.push(JSON.parse(raw));
      } catch {
        fail(`Unable to parse TOML array value for ${key}: ${raw}`);
      }
    } else {
      values.push(raw.slice(1, -1));
    }
  }
  return values;
}

function packageManifestSection(manifestPath) {
  const manifest = readFileSync(manifestPath, "utf8");
  const marker = manifest.match(/^\[package\]\s*$/m);
  if (!marker || marker.index === undefined) {
    fail(`Missing [package] section: ${manifestPath}`);
  }
  const remainder = manifest.slice(marker.index + marker[0].length);
  const nextSection = remainder.search(/^\[/m);
  return nextSection === -1 ? remainder : remainder.slice(0, nextSection);
}

function readManifestMetadata(manifestPath) {
  const section = packageManifestSection(manifestPath);
  return {
    name: tomlStringValue(section, "name"),
    version: tomlStringValue(section, "version"),
    license: tomlStringValue(section, "license"),
    licenseFile: tomlStringValue(section, "license-file"),
    repository: tomlStringValue(section, "repository"),
    homepage: tomlStringValue(section, "homepage"),
    authors: tomlStringArray(section, "authors"),
  };
}

function parseCargoLock() {
  const lock = readFileSync(LOCK_PATH, "utf8");
  const packages = [];
  const blocks = lock.split(/\r?\n(?=\[\[package\]\]\r?\n)/);
  for (const block of blocks) {
    if (!block.startsWith("[[package]]")) {
      continue;
    }
    const name = block.match(/^name\s*=\s*"([^"]+)"\s*$/m)?.[1];
    const version = block.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
    const source = block.match(/^source\s*=\s*"([^"]+)"\s*$/m)?.[1] ?? "";
    if (!name || !version) {
      fail(`Unable to parse Cargo.lock package block:\n${block.slice(0, 200)}`);
    }
    packages.push({ name, version, source });
  }
  if (packages.length === 0) {
    fail("Cargo.lock did not contain any package entries");
  }
  packages.sort(
    (a, b) =>
      compareStrings(a.name, b.name) ||
      compareStrings(a.version, b.version) ||
      compareStrings(a.source, b.source),
  );
  return { bytes: Buffer.from(lock), packages };
}

function resolvedGitPackages() {
  const result = spawnSync(
    "cargo",
    ["metadata", "--locked", "--format-version", "1"],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      windowsHide: true,
      maxBuffer: 32 * 1024 * 1024,
    },
  );
  if (result.status !== 0) {
    fail(`cargo metadata failed while resolving Git package licenses: ${result.error?.message ?? result.stderr}`);
  }
  const metadata = JSON.parse(result.stdout);
  // Git packages, and Amoeba's own crates that this monorepo links by path
  // (../sdk, ../futures, ../oracle, ../governance; source-less in Cargo.lock).
  const monorepoRoot = `${path.resolve(REPO_ROOT, "..")}${path.sep}`;
  return new Map(
    metadata.packages
      .filter(
        (pkg) =>
          pkg.source?.startsWith("git+") ||
          (!pkg.source &&
            FIRST_PARTY_PACKAGE.test(pkg.name) &&
            pkg.manifest_path.startsWith(monorepoRoot) &&
            !pkg.manifest_path.startsWith(`${REPO_ROOT}${path.sep}`)),
      )
      .map((pkg) => [
        packageKey(pkg),
        {
          source: pkg.source ?? "",
          manifestPath: pkg.manifest_path,
          metadata: {
            name: pkg.name,
            version: pkg.version,
            license: pkg.license ?? "",
            licenseFile: pkg.license_file ?? "",
            repository: pkg.repository ?? "",
            homepage: pkg.homepage ?? "",
            authors: pkg.authors ?? [],
          },
        },
      ]),
  );
}

function assertReadableLegalBytes(bytes, label) {
  if (bytes.length > MAX_LEGAL_FILE_BYTES) {
    fail(`Legal file exceeds ${MAX_LEGAL_FILE_BYTES} bytes: ${label}`);
  }
  if (bytes.includes(0)) {
    fail(`Legal file contains NUL bytes: ${label}`);
  }
  try {
    new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    fail(`Legal file is not valid UTF-8: ${label}`);
  }
}

function canonicalLegalBytes(bytes, label) {
  assertReadableLegalBytes(bytes, label);
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  return Buffer.from(text.replace(/\r\n?/g, "\n"), "utf8");
}

function walkLegalFiles(root) {
  const result = [];
  const visit = (directory) => {
    const entries = readdirSync(directory, { withFileTypes: true }).sort((a, b) =>
      compareStrings(a.name, b.name),
    );
    for (const entry of entries) {
      if (entry.name === ".git" || entry.name === "target") {
        continue;
      }
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute);
      } else if (entry.isFile() && LEGAL_FILE_NAME.test(entry.name)) {
        result.push(absolute);
      }
    }
  };
  visit(root);
  return result;
}

function resolveInside(root, relative, label) {
  const absolute = path.resolve(root, relative);
  const rootWithSeparator = `${path.resolve(root)}${path.sep}`;
  if (absolute !== path.resolve(root) && !absolute.startsWith(rootWithSeparator)) {
    fail(`${label} escapes its package root: ${relative}`);
  }
  return absolute;
}

function legalFilesForDirectory(directory, metadata) {
  const files = new Set(walkLegalFiles(directory));
  if (metadata.licenseFile) {
    const declared = resolveInside(directory, metadata.licenseFile, "license-file");
    if (existsSync(declared) && lstatSync(declared).isFile()) {
      files.add(declared);
    }
  }
  return [...files]
    .map((absolute) => {
      const relative = path.relative(directory, absolute).split(path.sep).join("/");
      const bytes = canonicalLegalBytes(
        readFileSync(absolute),
        `${directory}/${relative}`,
      );
      return { relative, bytes, hash: sha256(bytes) };
    })
    .sort((a, b) => compareStrings(a.relative, b.relative));
}

function registryCacheRoots() {
  const cargoHome = process.env.CARGO_HOME
    ? path.resolve(process.env.CARGO_HOME)
    : path.join(os.homedir(), ".cargo");
  const registrySource = path.join(cargoHome, "registry", "src");
  if (!existsSync(registrySource)) {
    fail(
      `Missing Cargo registry source cache: ${registrySource}. Run cargo fetch --locked first.`,
    );
  }
  return readdirSync(registrySource, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(registrySource, entry.name))
    .sort(compareStrings);
}

function cacheFingerprint(info) {
  return sha256(
    Buffer.from(
      JSON.stringify({
        metadata: info.metadata,
        legalFiles: info.legalFiles.map((file) => ({
          relative: file.relative,
          hash: file.hash,
        })),
      }),
    ),
  );
}

function findRegistryPackage(roots, name, version) {
  const leaf = `${name}-${version}`;
  const candidates = roots
    .map((root) => path.join(root, leaf))
    .filter((candidate) => existsSync(candidate) && lstatSync(candidate).isDirectory())
    .map((directory) => {
      const metadata = readManifestMetadata(path.join(directory, "Cargo.toml"));
      if (metadata.name !== name || metadata.version !== version) {
        fail(`Cargo cache identity mismatch for ${name}@${version}: ${directory}`);
      }
      const legalFiles = legalFilesForDirectory(directory, metadata);
      return { directory, metadata, legalFiles };
    });
  if (candidates.length === 0) {
    fail(
      `Missing extracted Cargo cache package ${name}@${version}. Run cargo fetch --locked first.`,
    );
  }
  const fingerprints = new Set(candidates.map(cacheFingerprint));
  if (fingerprints.size !== 1) {
    fail(`Ambiguous cached contents for ${name}@${version}`);
  }
  return candidates[0];
}

function addLegalText(texts, bytes, source) {
  bytes = canonicalLegalBytes(bytes, source.label);
  const hash = sha256(bytes);
  const existing = texts.get(hash);
  if (existing) {
    if (!existing.bytes.equals(bytes)) {
      fail(`SHA-256 collision while processing legal text ${hash}`);
    }
    existing.sources.push(source);
    return hash;
  }
  texts.set(hash, { hash, bytes, sources: [source] });
  return hash;
}

function extractPinnedTextFromCorpus(hash) {
  if (!existsSync(OUTPUT_PATH)) {
    return null;
  }
  const corpus = readFileSync(OUTPUT_PATH, "utf8");
  const begin = `<!-- legal-text-begin sha256=${hash} -->\n`;
  const beginIndex = corpus.indexOf(begin);
  if (beginIndex === -1) {
    return null;
  }
  const fenceStart = beginIndex + begin.length;
  const fenceEnd = corpus.indexOf("\n", fenceStart);
  if (fenceEnd === -1) {
    return null;
  }
  const fenceLine = corpus.slice(fenceStart, fenceEnd);
  const fence = fenceLine.endsWith("text") ? fenceLine.slice(0, -4) : "";
  if (!/^`{3,}$/.test(fence)) {
    return null;
  }
  const endMarker = `\n${fence}\n<!-- legal-text-end sha256=${hash} -->`;
  const contentStart = fenceEnd + 1;
  const contentEnd = corpus.indexOf(endMarker, contentStart);
  if (contentEnd === -1) {
    return null;
  }
  const bytes = Buffer.from(corpus.slice(contentStart, contentEnd), "utf8");
  return sha256(bytes) === hash ? bytes : null;
}

async function officialStandardText(spdxId, descriptor) {
  const cached = extractPinnedTextFromCorpus(descriptor.sha256);
  if (cached) {
    return cached;
  }
  let response;
  try {
    response = await fetch(descriptor.url, {
      headers: { "user-agent": "petri-third-party-license-generator/1" },
    });
  } catch (error) {
    fail(
      `Unable to fetch pinned ${spdxId} text and no valid checked-in copy exists: ${error.message}`,
    );
  }
  if (!response.ok) {
    fail(`Unable to fetch pinned ${spdxId} text: HTTP ${response.status}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  if (sha256(bytes) !== descriptor.sha256) {
    fail(
      `Pinned ${spdxId} text hash mismatch: expected ${descriptor.sha256}, received ${sha256(bytes)}`,
    );
  }
  assertReadableLegalBytes(bytes, descriptor.url);
  return bytes;
}

function markdownCell(value) {
  if (!value) {
    return "—";
  }
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("|", "&#124;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\n", " ");
}

function code(value) {
  return `<code>${markdownCell(value).replaceAll("`", "&#96;")}</code>`;
}

function fenceFor(text) {
  let longest = 0;
  for (const match of text.matchAll(/`+/g)) {
    longest = Math.max(longest, match[0].length);
  }
  return "`".repeat(Math.max(3, longest + 1));
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const { bytes: lockBytes, packages } = parseCargoLock();
  const roots = registryCacheRoots();
  const packageByKey = new Map(packages.map((pkg) => [packageKey(pkg), pkg]));
  if (packageByKey.size !== packages.length) {
    fail("Cargo.lock contains duplicate exact package identities");
  }
  const cacheByKey = new Map();
  const texts = new Map();
  const mappings = [];
  const standardHashes = new Map();

  const registryInfo = (name, version) => {
    const key = `${name}@${version}`;
    if (!packageByKey.get(key)?.source.startsWith("registry+")) {
      fail(`SPDX donor is not an exact registry package in Cargo.lock: ${key}`);
    }
    if (!cacheByKey.has(key)) {
      cacheByKey.set(key, findRegistryPackage(roots, name, version));
    }
    return cacheByKey.get(key);
  };

  const loadStandardText = async (spdxId) => {
    if (standardHashes.has(spdxId)) {
      return standardHashes.get(spdxId);
    }
    const cached = CACHED_STANDARD_TEXTS.get(spdxId);
    if (cached) {
      const info = registryInfo(cached.package, cached.version);
      const legalFile = info.legalFiles.find((file) => file.relative === cached.file);
      if (!legalFile) {
        fail(
          `Pinned SPDX donor file is missing: ${cached.package}@${cached.version}/${cached.file}`,
        );
      }
      const hash = addLegalText(texts, legalFile.bytes, {
        kind: "spdx-fallback",
        label: `${spdxId} standard text from locked ${cached.package}@${cached.version}/${cached.file}`,
      });
      standardHashes.set(spdxId, hash);
      return hash;
    }
    const official = OFFICIAL_STANDARD_TEXTS.get(spdxId);
    if (official) {
      const bytes = await officialStandardText(spdxId, official);
      const hash = addLegalText(texts, bytes, {
        kind: "spdx-fallback",
        label: `${spdxId} standard text from ${official.steward}: ${official.url}`,
      });
      standardHashes.set(spdxId, hash);
      return hash;
    }
    fail(`No pinned standard text for SPDX identifier: ${spdxId}`);
  };

  const fallbackHashes = async (licenseExpression, key) => {
    if (!licenseExpression) {
      fail(`Package ${key} has no legal file and no declared license expression`);
    }
    const spdxIds = FALLBACK_EXPRESSIONS.get(licenseExpression);
    if (!spdxIds) {
      fail(
        `Package ${key} has no legal file and an unresolved license expression: ${licenseExpression}`,
      );
    }
    const textsForExpression = [];
    for (const spdxId of spdxIds) {
      textsForExpression.push({ spdxId, hash: await loadStandardText(spdxId) });
    }
    return textsForExpression.filter(
      (item, index, items) =>
        items.findIndex((candidate) => candidate.hash === item.hash) === index,
    );
  };

  const sourceLess = new Set();
  const gitPackages = resolvedGitPackages();
  for (const pkg of packages) {
    const key = packageKey(pkg);
    if (pkg.source.startsWith("registry+")) {
      const info = registryInfo(pkg.name, pkg.version);
      const files = info.legalFiles.map((file) => ({
        path: file.relative,
        hash: addLegalText(texts, file.bytes, {
          kind: "upstream-package",
          label: `${key}/${file.relative}`,
        }),
      }));
      let resolution = "upstream-packaged legal file";
      if (files.length === 0) {
        resolution = "SPDX-text fallback (upstream package contains no legal file)";
        for (const fallback of await fallbackHashes(info.metadata.license, key)) {
          files.push({
            path: `SPDX fallback: ${fallback.spdxId}`,
            hash: fallback.hash,
          });
        }
      } else if (FORCED_FALLBACK_PACKAGES.has(key)) {
        resolution = `upstream-packaged legal file + SPDX-text fallback (${FORCED_FALLBACK_PACKAGES.get(key)})`;
        for (const fallback of await fallbackHashes(info.metadata.license, key)) {
          files.push({
            path: `SPDX fallback: ${fallback.spdxId}`,
            hash: fallback.hash,
          });
        }
      }
      mappings.push({
        ...pkg,
        metadata: info.metadata,
        origin: "registry package",
        resolution,
        files,
      });
      continue;
    }

    if (pkg.source.startsWith("git+")) {
      const resolved = gitPackages.get(key);
      if (!resolved || resolved.source !== pkg.source) {
        fail(`Cargo metadata did not resolve the exact locked Git package: ${key}`);
      }
      const packageRoot = path.dirname(resolved.manifestPath);
      const files = legalFilesForDirectory(packageRoot, resolved.metadata).map((file) => ({
        path: file.relative,
        hash: addLegalText(texts, file.bytes, {
          kind: "upstream-git-package",
          label: `${key}/${file.relative}`,
        }),
      }));
      let resolution = "upstream Git legal file";
      if (files.length === 0 && !FIRST_PARTY_PACKAGE.test(pkg.name)) {
        // A crate of a multi-crate Git repository can carry no legal file of
        // its own while the repository's root licenses all of it (e.g.
        // tlsn-utils' LICENSE-APACHE and LICENSE-MIT): use the checkout's
        // root, labelled as such.
        const parts = packageRoot.split(path.sep);
        const at = parts.lastIndexOf("checkouts");
        if (at >= 0 && parts.length > at + 2) {
          const repoRoot = parts.slice(0, at + 3).join(path.sep);
          if (repoRoot !== packageRoot) {
            // Only the root's own files: a sibling crate's license is not
            // this crate's.
            for (const file of legalFilesForDirectory(repoRoot, resolved.metadata).filter(
              (f) => !f.relative.includes("/") && !f.relative.includes(path.sep),
            )) {
              files.push({
                path: `repository root: ${file.relative}`,
                hash: addLegalText(texts, file.bytes, {
                  kind: "upstream-git-repository",
                  label: `${key} repository root/${file.relative}`,
                }),
              });
            }
            if (files.length > 0) {
              resolution = "upstream Git repository-root legal file (the crate has none of its own)";
            }
          }
        }
      }
      if (files.length === 0 && !resolved.metadata.license && FIRST_PARTY_PACKAGE.test(pkg.name)) {
        // Amoeba's own program and client crates (futures, oracle) pinned by
        // revision. They are not third-party components.
        mappings.push({
          ...pkg,
          metadata: resolved.metadata,
          origin: "Git package",
          resolution: "first-party Amoeba crate; no third-party license applies",
          files,
        });
        continue;
      }
      const repository = pkg.source.replace(/^git\+/, "").split(/[?#]/)[0];
      const statement = REPOSITORY_LICENSE_STATEMENTS.get(repository);
      if (files.length === 0 && !resolved.metadata.license && statement) {
        resolution = `SPDX-text fallback per the repository's license statement (${statement.basis})`;
        for (const fallback of await fallbackHashes(statement.expression, key)) {
          files.push({
            path: `SPDX fallback: ${fallback.spdxId}`,
            hash: fallback.hash,
          });
        }
      }
      if (files.length === 0) {
        resolution = "SPDX-text fallback (upstream Git package contains no legal file)";
        for (const fallback of await fallbackHashes(resolved.metadata.license, key)) {
          files.push({
            path: `SPDX fallback: ${fallback.spdxId}`,
            hash: fallback.hash,
          });
        }
      }
      mappings.push({
        ...pkg,
        metadata: resolved.metadata,
        origin: "Git package",
        resolution,
        files,
      });
      continue;
    }

    const monorepo = LOCAL_PACKAGES.has(key) ? undefined : gitPackages.get(key);
    if (monorepo && monorepo.source === "") {
      // Amoeba's own crates from this monorepo, linked by path. They are not
      // third-party components.
      const manifest = path
        .relative(path.resolve(REPO_ROOT, ".."), monorepo.manifestPath)
        .split(path.sep)
        .join("/");
      mappings.push({
        ...pkg,
        source: `monorepo path package: ${manifest}`,
        metadata: monorepo.metadata,
        origin: "monorepo path package",
        resolution: "first-party Amoeba crate; no third-party license applies",
        files: [],
      });
      continue;
    }
    sourceLess.add(key);
    const local = LOCAL_PACKAGES.get(key);
    if (!local) {
      fail(`Cargo.lock contains an unreviewed source-less package: ${key}`);
    }
    const manifestPath = path.join(REPO_ROOT, local.manifest);
    const metadata = readManifestMetadata(manifestPath);
    if (metadata.name !== pkg.name || metadata.version !== pkg.version) {
      fail(`Local package identity mismatch for ${key}: ${local.manifest}`);
    }
    const files = [];
    for (const relative of local.legalFiles) {
      const absolute = resolveInside(REPO_ROOT, relative, "local legal file");
      if (!existsSync(absolute) || !lstatSync(absolute).isFile()) {
        fail(`Missing local legal file for ${key}: ${relative}`);
      }
      const bytes = canonicalLegalBytes(readFileSync(absolute), `${key}/${relative}`);
      files.push({
        path: relative,
        hash: addLegalText(texts, bytes, {
          kind: "local-package",
          label: `${key}/${relative}`,
        }),
      });
    }
    let resolution = "declared workspace legal file";
    if (local.includeSpdxFallback) {
      resolution = "vendored patch legal file + SPDX-text fallback";
      for (const fallback of await fallbackHashes(metadata.license, key)) {
        files.push({
          path: `SPDX fallback: ${fallback.spdxId}`,
          hash: fallback.hash,
        });
      }
    }
    mappings.push({
      ...pkg,
      source: `${local.kind}: ${local.manifest}`,
      metadata,
      origin: local.kind,
      resolution,
      files,
    });
  }

  for (const key of LOCAL_PACKAGES.keys()) {
    if (!sourceLess.has(key)) {
      fail(`Reviewed local package is no longer source-less in Cargo.lock: ${key}`);
    }
  }

  const sortedTexts = [...texts.values()].sort((a, b) => compareStrings(a.hash, b.hash));
  const textIdByHash = new Map(
    sortedTexts.map((text, index) => [text.hash, `L${String(index + 1).padStart(4, "0")}`]),
  );
  const registryCount = mappings.filter((mapping) => mapping.origin === "registry package").length;
  const gitCount = mappings.filter((mapping) => mapping.origin === "Git package").length;
  const fallbackCount = mappings.filter((mapping) => mapping.resolution.includes("SPDX-text fallback")).length;
  const directFileCount = mappings.reduce(
    (count, mapping) =>
      count + mapping.files.filter((file) => !file.path.startsWith("SPDX fallback:")).length,
    0,
  );

  const lines = [
    "# Third-party license corpus",
    "",
    "This file is generated from the exact `Cargo.lock` package set and the legal files in the extracted Cargo registry cache. UTF-8 legal text is normalized to LF line endings so the corpus is reproducible across build hosts. Do not edit it by hand.",
    "",
    "An **SPDX-text fallback** is used only when an upstream `.crate` archive contains no `LICENSE`, `LICENCE`, `COPYING`, `NOTICE`, `COPYRIGHT`, or `UNLICENSE` file, or when a reviewed packaged file conflicts with the manifest's declared license expression. The fallback is explicitly labeled, preserves the package's declared authors and repository, and supplies pinned standard license text; it is not represented as an upstream-packaged file.",
    "",
    `- Cargo.lock SHA-256: \`${sha256(lockBytes)}\``,
    `- Locked packages: ${mappings.length}`,
    `- Registry packages: ${registryCount}`,
    `- Git packages: ${gitCount}`,
    `- Workspace/path packages: ${mappings.length - registryCount - gitCount}`,
    `- Packages requiring an SPDX-text fallback: ${fallbackCount}`,
    `- Discovered legal files: ${directFileCount}`,
    `- Unique normalized legal texts: ${sortedTexts.length}`,
    "",
    "## Package-to-license mapping",
    "",
    "| Package | Version | Origin / locked source | Declared license | Authors | Repository | Resolution | Files and exact text IDs |",
    "| --- | --- | --- | --- | --- | --- | --- | --- |",
  ];

  for (const mapping of mappings) {
    const fileCell = mapping.files
      .map((file) => `${code(file.path)} → ${code(textIdByHash.get(file.hash))}`)
      .join("<br>");
    lines.push(
      `| ${markdownCell(mapping.name)} | ${markdownCell(mapping.version)} | ${markdownCell(mapping.source)} | ${markdownCell(mapping.metadata.license)} | ${markdownCell(mapping.metadata.authors.join("; "))} | ${markdownCell(mapping.metadata.repository || mapping.metadata.homepage)} | ${markdownCell(mapping.resolution)} | ${fileCell} |`,
    );
  }

  lines.push("", "## Exact legal texts", "");
  for (const text of sortedTexts) {
    const id = textIdByHash.get(text.hash);
    const decoded = new TextDecoder("utf-8", { fatal: true }).decode(text.bytes);
    const fence = fenceFor(decoded);
    const kinds = [...new Set(text.sources.map((source) => source.kind))]
      .sort(compareStrings)
      .join(", ");
    const firstSources = text.sources
      .map((source) => source.label)
      .sort(compareStrings)
      .slice(0, 5);
    const extraSources = text.sources.length - firstSources.length;
    lines.push(
      `### ${id}`,
      "",
      `- SHA-256: \`${text.hash}\``,
      `- Provenance class: ${markdownCell(kinds)}`,
      `- Observed source${text.sources.length === 1 ? "" : "s"}: ${firstSources.map(code).join(", ")}${extraSources > 0 ? `, plus ${extraSources} more mapped package file(s)` : ""}`,
      "",
      `<!-- legal-text-begin sha256=${text.hash} -->`,
      `${fence}text`,
      decoded,
      fence,
      `<!-- legal-text-end sha256=${text.hash} -->`,
      "",
    );
  }

  const rendered = `${lines.join("\n").replace(/\n+$/, "")}\n`;
  if (args.check) {
    if (!existsSync(OUTPUT_PATH)) {
      fail("THIRD_PARTY_LICENSES.md is missing; run the generator without --check");
    }
    const current = readFileSync(OUTPUT_PATH, "utf8");
    if (current !== rendered) {
      fail("THIRD_PARTY_LICENSES.md is stale; run the generator without --check");
    }
    console.log(
      `Third-party license corpus is current (${mappings.length} packages, ${sortedTexts.length} unique texts).`,
    );
    return;
  }
  writeFileSync(OUTPUT_PATH, rendered, "utf8");
  console.log(
    `Wrote THIRD_PARTY_LICENSES.md (${mappings.length} packages, ${sortedTexts.length} unique texts, ${fallbackCount} SPDX fallbacks).`,
  );
}

main().catch((error) => {
  console.error(`third-party license generation failed: ${error.message}`);
  process.exitCode = 1;
});
