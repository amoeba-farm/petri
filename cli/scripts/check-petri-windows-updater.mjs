import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";

if (process.platform !== "win32") {
  console.log("Petri Windows updater integration check skipped on non-Windows.");
  process.exit(0);
}

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));
const updater = path.join(scriptsDir, "finish-petri-windows-update.ps1");
const root = mkdtempSync(path.join(tmpdir(), "petri-updater-check-"));
const updaterSource = readFileSync(updater, "utf8");
for (const requiredSecurityCheck of [
  "Get-AuthenticodeSignature -LiteralPath $PSCommandPath",
  "Get-AuthenticodeSignature -LiteralPath $StagedBinary",
  "$StagedSignature.SignerCertificate.Thumbprint -ine $ExpectedPublisherThumbprint",
  "Get-FileHash -LiteralPath $StagedBinary -Algorithm SHA256",
  "Get-AuthenticodeSignature -LiteralPath $AppBinary",
]) {
  assert.ok(
    updaterSource.includes(requiredSecurityCheck),
    `updater is missing its signed-update security check: ${requiredSecurityCheck}`,
  );
}

// The checked-in helper and release binaries are unsigned source inputs. Exercise the
// replacement state machine with a test-only Authenticode result while the assertions
// above pin every production signature and hash check in the original script.
const testUpdater = path.join(root, "finish-petri-windows-update.test.ps1");
const testSignatureFunction = [
  "function Get-AuthenticodeSignature {",
  "  param([string]$LiteralPath)",
  "  [pscustomobject]@{",
  "    Status = [System.Management.Automation.SignatureStatus]::Valid",
  "    SignerCertificate = [pscustomobject]@{ Thumbprint = 'PETRI-UPDATER-TEST' }",
  "  }",
  "}",
  "",
].join("\n");
const testUpdaterSource = updaterSource.replace(
  /\)\r?\n\r?\n(?=\$ErrorActionPreference)/,
  `)\n\n${testSignatureFunction}`,
);
assert.notEqual(testUpdaterSource, updaterSource, "could not instrument updater test copy");
writeFileSync(testUpdater, testUpdaterSource);
const powershellEnv = {
  ...process.env,
  PSModulePath: (process.env.PSModulePath ?? "")
    .split(path.delimiter)
    .filter((entry) => !entry.toLowerCase().includes("codex-primary-runtime"))
    .join(path.delimiter),
};

function runUpdater(args) {
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", testUpdater, ...args],
    { encoding: "utf8", windowsHide: true, env: powershellEnv },
  );
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

function quotePowerShell(value) {
  return `'${value.replaceAll("'", "''")}'`;
}

function waitForLine(child, expected) {
  return new Promise((resolve, reject) => {
    let output = "";
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      output += chunk;
      if (output.includes(expected)) resolve();
    });
    child.once("error", reject);
    child.once("exit", (code) => {
      if (!output.includes(expected)) reject(new Error(`locker exited ${code}: ${output}`));
    });
  });
}

try {
  const immediateDir = path.join(root, "immediate");
  const immediateApp = path.join(immediateDir, "petri.exe");
  const immediateStaged = path.join(immediateDir, "petri.exe.new");
  const immediateStatus = path.join(immediateDir, "update-status.json");
  mkdirSync(immediateDir, { recursive: true });
  writeFileSync(immediateApp, "old");
  writeFileSync(immediateStaged, "new");
  runUpdater([
    "-StagedBinary", immediateStaged,
    "-AppBinary", immediateApp,
    "-StatusFile", immediateStatus,
  ]);
  assert.equal(readFileSync(immediateApp, "utf8"), "new");
  assert.equal(JSON.parse(readFileSync(immediateStatus, "utf8")).status, "installed");

  const deferredDir = path.join(root, "deferred");
  const deferredApp = path.join(deferredDir, "petri.exe");
  const deferredStaged = path.join(deferredDir, "petri.exe.new");
  const deferredStatus = path.join(deferredDir, "update-status.json");
  mkdirSync(deferredDir, { recursive: true });
  writeFileSync(deferredApp, "old");
  writeFileSync(deferredStaged, "new");

  const lockerScript = [
    `$stream = [System.IO.File]::Open(${quotePowerShell(deferredApp)}, 'Open', 'ReadWrite', 'None')`,
    "[Console]::Out.WriteLine('locked')",
    "Start-Sleep -Milliseconds 1200",
    "$stream.Dispose()",
  ].join("; ");
  const locker = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", lockerScript], {
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
    env: powershellEnv,
  });
  await waitForLine(locker, "locked");

  runUpdater([
    "-StagedBinary", deferredStaged,
    "-AppBinary", deferredApp,
    "-WaitForPid", String(locker.pid),
    "-StatusFile", deferredStatus,
  ]);
  assert.equal(readFileSync(deferredApp, "utf8"), "new");
  assert.equal(JSON.parse(readFileSync(deferredStatus, "utf8")).status, "installed");

  console.log("Petri Windows updater integration checks passed.");
} finally {
  rmSync(root, { recursive: true, force: true });
}
