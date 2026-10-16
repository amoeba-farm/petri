import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const repoRoot = new URL("..", import.meta.url);
const launcherSource = readFileSync(new URL("petri-launcher.mjs", import.meta.url), "utf8");

for (const requiredSource of [
  'process.env.PETRI_DEV === "1"',
  'path.join(repoRoot, "target", "release", binaryName)',
  'path.join(repoRoot, "target", "debug", binaryName)',
  'launcher: "installation-update-only"',
  'runLauncherUpdate(parsed)',
  "TUI_UPDATE_REQUESTED_EXIT_CODE",
  "TUI_REBUILD_REQUESTED_EXIT_CODE",
  "headsDiverged(state)",
  '["merge-base", "--is-ancestor", "origin/HEAD", "HEAD"]',
]) {
  if (!launcherSource.includes(requiredSource)) {
    throw new Error(`launcher is missing installation/update behavior: ${requiredSource}`);
  }
}

for (const forbiddenSource of [
  'from "node:http"',
  'from "node:https"',
  "AMEBA_BACKEND_URL",
  "backendUrl",
  "catalog_fallback",
  "catalogPath",
  "listDishes",
  "showDish",
  "extractDishes",
  "renderDishList",
  "renderDishDetail",
  "mcpManifest",
  "getJson",
  "maybeRewriteBinaryParseError",
]) {
  if (launcherSource.includes(forbiddenSource)) {
    throw new Error(`launcher still owns product/backend behavior: ${forbiddenSource}`);
  }
}

const launcherCases = [
  {
    args: ["update", "info"], status: 2,
    includes: ["without the Rust binary"], excludes: ["fetch_refs", "fast_forward", "build_release_binary"],
  },
  {
    args: ["update", "recover", "--yes"], status: 2,
    includes: ["without the Rust binary"], excludes: ["fetch_refs", "fast_forward", "build_release_binary"],
  },
  {
    args: ["update", "unknown-form"], status: 1,
    includes: ["Unsupported source update arguments"], excludes: ["fetch_refs", "fast_forward", "build_release_binary"],
  },
  {
    args: [],
    status: 0,
    includes: [
      "Petri installation/update launcher",
      "does not implement product commands or backend responses",
      "petri update check",
      "petri update",
    ],
    excludes: ["RAMX", "petri markets", "mcp manifest", "AMEBA_BACKEND_URL"],
  },
  {
    args: ["--help"],
    status: 0,
    includes: ["Petri installation/update launcher", "Rust Petri binary is not available"],
    excludes: ["Amoeba market terminal", "Common commands", "market.list"],
  },
  {
    args: ["help", "update"],
    status: 0,
    includes: ["Petri installation/update launcher", "petri update check"],
    excludes: ["Petri update\n", "step=fetch_refs"],
  },
  {
    args: ["--json"],
    status: 0,
    includes: [
      '"code": "rust_binary_required"',
      '"launcher": "installation-update-only"',
      '"petri update check"',
    ],
    excludes: ['"backend"', '"markets"', '"mcp"'],
  },
  {
    args: ["markets"],
    status: 2,
    includes: ["cannot run 'markets' without the Rust binary", "only installs and updates Petri"],
    excludes: ["RAMX", "NANDX", "HTTP", "saved market catalog"],
  },
  {
    args: ["mcp", "manifest"],
    status: 2,
    includes: ["cannot run 'mcp manifest' without the Rust binary"],
    excludes: ["petri-agent-protocol", "market.list", "settlement.check"],
  },
  {
    args: ["config", "show"],
    status: 2,
    includes: ["cannot run 'config show' without the Rust binary"],
    excludes: ["backend", "javascript-setup"],
  },
  {
    args: ["--json", "markets", "status", "ramx"],
    status: 2,
    includes: [
      '"code": "rust_binary_required"',
      '"launcher": "installation-update-only"',
      '"markets"',
      '"ramx"',
    ],
    excludes: ['"symbol": "RAMX"', '"source": "live"'],
  },
  {
    args: ["update", "check", "--no-fetch"],
    status: 0,
    includes: ["Petri update check", "repo=", "branch="],
  },
  {
    args: ["--json", "update", "check", "--no-fetch"],
    status: 0,
    includes: ['"action": "update_check"', '"repoRoot"'],
  },
];

for (const testCase of launcherCases) {
  const result = runLauncher(testCase.args);
  if (result.status !== testCase.status) {
    throw new Error(
      `launcher exited ${result.status}, expected ${testCase.status} for ${testCase.args.join(" ")}\n${result.stdout}\n${result.stderr}`,
    );
  }
  const output = `${result.stdout}\n${result.stderr}`;
  for (const expected of testCase.includes) {
    if (!output.includes(expected)) {
      throw new Error(`missing '${expected}' for ${testCase.args.join(" ")}\n${output}`);
    }
  }
  for (const forbidden of testCase.excludes || []) {
    if (output.includes(forbidden)) {
      throw new Error(`unexpected '${forbidden}' for ${testCase.args.join(" ")}\n${output}`);
    }
  }
}

const tempRoot = mkdtempSync(path.join(tmpdir(), "petri-launcher-delegation-"));
try {
  let fakeBinary;
  let delegatedArgs;
  if (process.platform === "win32") {
    fakeBinary = path.join(tempRoot, "petri.exe");
    copyFileSync(
      path.join(
        process.env.SystemRoot || "C:\\Windows",
        "System32",
        "WindowsPowerShell",
        "v1.0",
        "powershell.exe",
      ),
      fakeBinary,
    );
    const freshTime = new Date(Date.now() + 60_000);
    utimesSync(fakeBinary, freshTime, freshTime);
    delegatedArgs = [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      "[Console]::Out.Write('delegated=' + $env:PETRI_LAUNCHER_ENV_PROBE)",
    ];
  } else {
    fakeBinary = path.join(tempRoot, "petri");
    writeFileSync(
      fakeBinary,
      "#!/bin/sh\nprintf 'delegated=%s\\n' \"$PETRI_LAUNCHER_ENV_PROBE\"\nprintf 'args=%s\\n' \"$*\"\n",
      "utf8",
    );
    chmodSync(fakeBinary, 0o755);
    delegatedArgs = ["book", "ramx", "--depth", "5"];
  }
  const explicitProbe = path.join(tempRoot, "caller-owned-probe.json");
  const delegated = spawnSync(
    process.execPath,
    ["scripts/petri-launcher.mjs", ...delegatedArgs],
    {
      cwd: repoRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        PETRI_BINARY: fakeBinary,
        PETRI_FORCE_JS_LAUNCHER: "0",
        PETRI_LAUNCHER_ENV_PROBE: explicitProbe,
      },
    },
  );
  if (delegated.status !== 0 || !delegated.stdout.includes(`delegated=${explicitProbe}`)) {
    throw new Error(`launcher did not transparently delegate: ${delegated.stdout || delegated.stderr}`);
  }
  if (process.platform !== "win32" && !delegated.stdout.includes("args=book ramx --depth 5")) {
    throw new Error(`launcher changed delegated arguments: ${delegated.stdout}`);
  }
} finally {
  rmSync(tempRoot, { recursive: true, force: true });
}

console.log("Petri installation/update launcher checks passed.");

function runLauncher(caseArgs) {
  return spawnSync(process.execPath, ["scripts/petri-launcher.mjs", ...caseArgs], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PETRI_FORCE_JS_LAUNCHER: "1",
      PETRI_BINARY: "",
      PETRI_API_URL: "http://127.0.0.1:9",
    },
  });
}
