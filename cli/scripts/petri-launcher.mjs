#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const args = process.argv.slice(2);
const parsed = parseArgs(args);
const TUI_UPDATE_REQUESTED_EXIT_CODE = 42;
const TUI_REBUILD_REQUESTED_EXIT_CODE = 43;
const UPDATE_COMMIT_ENV = "PETRI_TRUSTED_UPDATE_COMMIT";
const UPDATE_PROVENANCE_BLOCKED_REASON = "remote update commit is not explicitly trusted; verify origin/HEAD and set PETRI_TRUSTED_UPDATE_COMMIT to its exact commit";
const CANONICAL_ORIGIN_URLS = new Set([
  "https://github.com/amoeba-farm/petri.git",
  "git@github.com:amoeba-farm/petri.git",
  "ssh://git@github.com/amoeba-farm/petri.git",
  "https://github.com/SPACE999978/ameba_cli.git",
  "git@github.com:SPACE999978/ameba_cli.git",
  "ssh://git@github.com/SPACE999978/ameba_cli.git",
]);

const binaryName = process.platform === "win32" ? "petri.exe" : "petri";
const configuredBinary = existingPath(process.env.PETRI_BINARY);
const channelBinary = process.env.PETRI_DEV === "1"
  ? path.join(repoRoot, "target", "debug", binaryName)
  : path.join(repoRoot, "target", "release", binaryName);
const realBinary = configuredBinary || existingPath(channelBinary);

// New native-only forms must never fall through into the source-mutating path.
const nativeUpdateForm = parsed.command[0] === "update"
  && parsed.command.slice(1).some((arg) => ["info", "recover", "--yes", "--restart"].includes(arg));
if (!parsed.help && parsed.command[0] === "update" && !nativeUpdateForm) {
  try {
    runLauncherUpdate(parsed);
  } catch (error) {
    emitError(error);
  }
  process.exit(process.exitCode || 0);
}

if (realBinary && process.env.PETRI_FORCE_JS_LAUNCHER !== "1") {
  const binaryEnv = { ...process.env, PETRI_LAUNCHER: "1" };
  const result = spawnSync(realBinary, args, {
    stdio: "inherit",
    cwd: repoRoot,
    env: binaryEnv,
  });
  if (
    result.status === TUI_UPDATE_REQUESTED_EXIT_CODE
    || result.status === TUI_REBUILD_REQUESTED_EXIT_CODE
  ) {
    try {
      runLauncherUpdate({ ...parsed, command: ["update"] });
    } catch (error) {
      emitError(error);
    }
    process.exit(process.exitCode || 0);
  }
  process.exit(typeof result.status === "number" ? result.status : 1);
}

if (parsed.help || parsed.command.length === 0) {
  emit(parsed, launcherHelpPayload(), launcherHelpText());
} else {
  emit(parsed, binaryRequiredPayload(parsed.command), binaryRequiredText(parsed.command));
  process.exitCode = 2;
}

function launcherHelpPayload() {
  return {
    ok: false,
    code: "rust_binary_required",
    launcher: "installation-update-only",
    realBinary: realBinary || null,
    commands: ["petri update check", "petri update"],
  };
}

function launcherHelpText() {
  return [
    "Petri installation/update launcher",
    "",
    "The Rust Petri binary is not available in this checkout.",
    "This launcher does not implement product commands or backend responses.",
    "",
    "  petri update check   inspect source and binary state",
    "  petri update         fast-forward and build the release binary",
  ].join("\n");
}

function binaryRequiredPayload(command) {
  return {
    ok: false,
    code: "rust_binary_required",
    launcher: "installation-update-only",
    command,
    message: "This command requires the Rust Petri binary.",
    next: ["petri update"],
  };
}

function binaryRequiredText(command) {
  return [
    `Petri cannot run '${command.join(" ")}' without the Rust binary.`,
    "The JavaScript launcher only installs and updates Petri.",
    "Run: petri update",
  ].join("\n");
}

function runLauncherUpdate(parsed) {
  const updateArgs = parsed.command.slice(1);
  if (updateArgs.some((arg) => !["check", "--no-fetch", "--skip-shim"].includes(arg))
      || updateArgs.filter((arg) => arg === "check").length > 1) {
    throw new Error("Unsupported source update arguments. No source or installation was changed.");
  }
  const checkOnly = updateArgs.includes("check");
  const noFetch = updateArgs.includes("--no-fetch");
  const skipShim = updateArgs.includes("--skip-shim");
  const steps = [];
  if (!noFetch) ensureCanonicalOrigin();
  const fetchPerformed = noFetch
    ? false
    : runLauncherStep("fetch_refs", "git", ["fetch", "--prune", "origin"], steps);
  let state = inspectLauncherWorkspace(fetchPerformed);
  let plan = planLauncherUpdate(state);

  if (checkOnly) {
    const report = launcherReport("update_check", state, plan, steps, true);
    emit(parsed, report, renderLauncherUpdateReport(report));
    return;
  }

  if (plan.blocked) {
    const report = launcherReport("update", state, plan, steps, false);
    emit(parsed, report, renderLauncherUpdateReport(report));
    process.exitCode = 1;
    return;
  }

  let rebuilt = false;
  if (plan.shouldFastForward) {
    const trustedCommit = trustedLauncherCommit(state.remoteHead);
    if (!trustedCommit) {
      plan = {
        ...plan,
        status: "blocked",
        shouldFastForward: false,
        blocked: true,
        blockedReason: UPDATE_PROVENANCE_BLOCKED_REASON,
      };
      const report = launcherReport("update", state, plan, steps, false);
      emit(parsed, report, renderLauncherUpdateReport(report));
      process.exitCode = 1;
      return;
    }
    ensureCanonicalOrigin();
    runLauncherStep("fast_forward", "git", ["merge", "--ff-only", trustedCommit], steps);
    state = inspectLauncherWorkspace(fetchPerformed);
    if (state.localHead !== trustedCommit) {
      throw new Error("updated checkout did not land on the explicitly trusted commit");
    }
    plan = planLauncherUpdate(state);
    if (plan.blocked) {
      const report = launcherReport("update", state, plan, steps, false);
      emit(parsed, report, renderLauncherUpdateReport(report));
      process.exitCode = 1;
      return;
    }
  }

  if (plan.rebuildRequired) {
    runLauncherStep(
      "build_release_binary",
      "cargo",
      ["build", "--release", "--locked", "--bin", "petri"],
      steps,
    );
    rebuilt = true;
  }

  if (process.platform === "win32" && !skipShim) {
    runLauncherStep(
      "install_windows_shim",
      "powershell.exe",
      ["-ExecutionPolicy", "Bypass", "-File", "scripts/install-petri.ps1", "-SkipBuild"],
      steps,
    );
  }

  const finalState = inspectLauncherWorkspace(fetchPerformed);
  if (rebuilt) finalState.builtCommit = finalState.localHead;
  const finalPlan = planLauncherUpdate(finalState);
  const report = launcherReport("update", finalState, finalPlan, steps, !finalPlan.blocked);
  emit(parsed, report, renderLauncherUpdateReport(report));
  if (!report.ok) process.exitCode = 1;
}

function inspectLauncherWorkspace(fetchPerformed) {
  const localHead = optionalLauncherStdout("git", ["rev-parse", "HEAD"]);
  const remoteHead = optionalLauncherStdout("git", ["rev-parse", "origin/HEAD"]);
  const dirtyFiles = requiredLauncherStdout("git", ["status", "--short"])
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter((line) => line.trim().length > 0);
  const fastForwardable = Boolean(
    localHead
      && remoteHead
      && runLauncherCommand("git", ["merge-base", "--is-ancestor", "HEAD", "origin/HEAD"]).success,
  );
  const localAhead = Boolean(
    localHead
      && remoteHead
      && localHead !== remoteHead
      && runLauncherCommand("git", ["merge-base", "--is-ancestor", "origin/HEAD", "HEAD"]).success,
  );

  return {
    repoRoot,
    branch: optionalLauncherStdout("git", ["branch", "--show-current"]) || null,
    localHead,
    remoteHead,
    builtCommit: readBuiltCommitFromRustBinary(),
    dirty: dirtyFiles.length > 0,
    dirtyFiles,
    fastForwardable,
    localAhead,
    fetchPerformed,
  };
}

function ensureCanonicalOrigin() {
  const origin = requiredLauncherStdout("git", ["remote", "get-url", "origin"]);
  if (!CANONICAL_ORIGIN_URLS.has(origin)) {
    throw new Error(`Petri updates require the canonical origin URL; found ${origin}`);
  }
}

function trustedLauncherCommit(remoteHead) {
  const configured = process.env[UPDATE_COMMIT_ENV]?.trim();
  if (!remoteHead || !configured || !/^[0-9a-f]{40}$/i.test(configured)) return null;
  return configured.toLowerCase() === remoteHead.toLowerCase() ? remoteHead : null;
}

function readBuiltCommitFromRustBinary() {
  if (!realBinary || !fs.existsSync(realBinary)) return null;
  const result = spawnSync(realBinary, ["--json", "update", "--no-fetch", "check"], {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
  if (result.status !== 0 || !result.stdout) return null;
  try {
    const payload = JSON.parse(result.stdout);
    return typeof payload.builtCommit === "string" && payload.builtCommit
      ? payload.builtCommit
      : null;
  } catch {
    return null;
  }
}

function planLauncherUpdate(state) {
  const remoteAhead = remoteIsAhead(state);
  const diverged = headsDiverged(state);
  if (remoteAhead || diverged) {
    if (state.dirtyFiles.length > 0) {
      return {
        status: "blocked",
        updateAvailable: true,
        rebuildRequired: false,
        shouldFastForward: false,
        blocked: true,
        blockedReason: "worktree has uncommitted changes",
      };
    }

    if (diverged) {
      return {
        status: "blocked",
        updateAvailable: true,
        rebuildRequired: false,
        shouldFastForward: false,
        blocked: true,
        blockedReason: "local branch and origin/HEAD have diverged; merge or rebase manually",
      };
    }
    return {
      status: "update_available",
      updateAvailable: true,
      rebuildRequired: true,
      shouldFastForward: true,
      blocked: false,
      blockedReason: null,
    };
  }
  if (binaryIsStale(state)) {
    return {
      status: "rebuild_available",
      updateAvailable: false,
      rebuildRequired: true,
      shouldFastForward: false,
      blocked: false,
      blockedReason: null,
    };
  }
  return {
    status: "current",
    updateAvailable: false,
    rebuildRequired: false,
    shouldFastForward: false,
    blocked: false,
    blockedReason: null,
  };
}

function launcherReport(action, state, plan, steps, ok) {
  return {
    ok,
    action,
    status: plan.status,
    repoRoot: state.repoRoot,
    branch: state.branch,
    localHead: state.localHead,
    remoteHead: state.remoteHead,
    builtCommit: state.builtCommit,
    dirty: state.dirty,
    dirtyFiles: state.dirtyFiles,
    fetchPerformed: state.fetchPerformed,
    updateAvailable: plan.updateAvailable,
    rebuildRequired: plan.rebuildRequired,
    shouldFastForward: plan.shouldFastForward,
    blocked: plan.blocked,
    blockedReason: plan.blockedReason,
    steps,
    next: launcherNextSteps(action, plan),
  };
}

function renderLauncherUpdateReport(report) {
  const lines = [
    report.action === "update" ? "Petri update" : "Petri update check",
    `status=${report.status}`,
    `repo=${report.repoRoot}`,
    `branch=${report.branch || "detached"} | local=${shortCommit(report.localHead)} | remote=${shortCommit(report.remoteHead)} | binary=${shortCommit(report.builtCommit)}`,
  ];
  if (report.fetchPerformed) lines.push("fetched=origin refs refreshed");
  if (report.blocked) {
    lines.push(`blocked=${report.blockedReason || "update cannot run automatically"}`);
    for (const dirty of report.dirtyFiles.slice(0, 5)) lines.push(`dirty=${dirty}`);
  } else if (report.updateAvailable) {
    lines.push("remote update available; Petri can fast-forward then rebuild.");
  } else if (report.rebuildRequired) {
    lines.push("source is current, but the release binary needs a rebuild.");
  } else {
    lines.push("Petri is up to date.");
  }
  for (const step of report.steps) {
    lines.push(`step=${step.name} | success=${step.success} | ${step.detail}`);
  }
  if (report.next.length) {
    lines.push("next:");
    for (const item of report.next) lines.push(`  ${item}`);
  }
  return lines.join("\n");
}

function launcherNextSteps(action, plan) {
  if (plan.blocked) {
    if (plan.blockedReason === "worktree has uncommitted changes") {
      return [
        "Commit or stash local changes before running petri update.",
        "Run petri update again after the worktree is clean.",
      ];
    }
    if (plan.blockedReason === UPDATE_PROVENANCE_BLOCKED_REASON) {
      return [
        "Verify the exact origin/HEAD commit through the trusted release channel.",
        `Set ${UPDATE_COMMIT_ENV} to that full commit, then run petri update again.`,
      ];
    }
    return [
      "Merge or rebase the local branch with origin/HEAD manually.",
      "Run petri update again after the branch histories are reconciled.",
    ];
  }
  if (action === "update_check" && (plan.updateAvailable || plan.rebuildRequired)) {
    return ["Run: petri update"];
  }
  if (action === "update" && !plan.updateAvailable && !plan.rebuildRequired) {
    return ["Run: petri --version"];
  }
  return [];
}

function runLauncherStep(name, program, commandArgs, steps) {
  const result = runLauncherCommand(program, commandArgs);
  steps.push({
    name,
    command: [program, ...commandArgs].join(" "),
    success: result.success,
    detail: launcherStepDetail(result),
  });
  if (!result.success) {
    throw new Error(`${[program, ...commandArgs].join(" ")} failed: ${launcherStepDetail(result)}`);
  }
  return true;
}

function requiredLauncherStdout(program, commandArgs) {
  const result = runLauncherCommand(program, commandArgs);
  if (!result.success) {
    throw new Error(`${[program, ...commandArgs].join(" ")} failed: ${launcherStepDetail(result)}`);
  }
  return result.stdout.trim();
}

function optionalLauncherStdout(program, commandArgs) {
  const result = runLauncherCommand(program, commandArgs);
  if (!result.success) return null;
  const output = result.stdout.trim();
  return output.length ? output : null;
}

function runLauncherCommand(program, commandArgs) {
  const result = spawnSync(program, commandArgs, {
    cwd: repoRoot,
    encoding: "utf8",
    windowsHide: true,
    maxBuffer: 1024 * 1024 * 16,
  });
  return {
    success: result.status === 0,
    status: result.status,
    stdout: result.stdout || "",
    stderr: result.stderr || "",
    error: result.error || null,
  };
}

function launcherStepDetail(result) {
  if (result.error) return result.error.message;
  const output = (result.stderr || result.stdout || "").trim();
  return output ? output.split(/\r?\n/)[0].trim() : "completed";
}

function remoteIsAhead(state) {
  return Boolean(
    state.localHead
      && state.remoteHead
      && state.localHead !== state.remoteHead
      && state.fastForwardable,
  );
}

function headsDiverged(state) {
  return Boolean(
    state.localHead
      && state.remoteHead
      && state.localHead !== state.remoteHead
      && !state.fastForwardable
      && !state.localAhead,
  );
}

function binaryIsStale(state) {
  if (!state.localHead) return false;
  return !state.builtCommit || state.localHead !== state.builtCommit;
}

function shortCommit(commit) {
  return commit ? String(commit).slice(0, 7) : "unknown";
}

function parseArgs(argv) {
  const command = [];
  let json = false;
  let help = false;
  for (const arg of argv) {
    if (arg === "--json" || arg === "--output=json") {
      json = true;
      continue;
    }
    if (arg === "--help" || arg === "-h" || arg === "help") {
      help = true;
      continue;
    }
    if (arg === "--output") continue;
    if (arg.toLowerCase() === "json") {
      json = true;
      continue;
    }
    command.push(arg.toLowerCase());
  }
  return { command, json, help };
}

function emit(parsed, payload, output) {
  if (parsed.json) {
    console.log(JSON.stringify(payload, null, 2));
  } else {
    console.log(output);
  }
}

function emitError(error) {
  console.error(`error: ${error.message || error}`);
  process.exitCode = 1;
}

function existingPath(candidate) {
  return candidate && fs.existsSync(candidate) ? candidate : null;
}
