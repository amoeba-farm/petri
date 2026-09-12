use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use serde::Serialize;

use crate::backend::CliError;

const BUILT_COMMIT: Option<&str> = option_env!("PETRI_BUILD_COMMIT");
const BUILD_MANIFEST_DIR: Option<&str> = option_env!("CARGO_MANIFEST_DIR");
const DIRTY_WORKTREE_BLOCKED_REASON: &str = "worktree has uncommitted changes";
const DIVERGED_BRANCH_BLOCKED_REASON: &str =
    "local branch and origin/HEAD have diverged; merge or rebase manually";
const UPDATE_COMMIT_ENV: &str = "PETRI_TRUSTED_UPDATE_COMMIT";
const UPDATE_PROVENANCE_BLOCKED_REASON: &str = "remote update commit is not explicitly trusted; verify origin/HEAD and set PETRI_TRUSTED_UPDATE_COMMIT to its exact commit";
const CANONICAL_ORIGIN_URLS: [&str; 6] = [
    "https://github.com/amoeba-farm/petri.git",
    "git@github.com:amoeba-farm/petri.git",
    "ssh://git@github.com/amoeba-farm/petri.git",
    "https://github.com/SPACE999978/ameba_cli.git",
    "git@github.com:SPACE999978/ameba_cli.git",
    "ssh://git@github.com/SPACE999978/ameba_cli.git",
];

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Current,
    UpdateAvailable,
    RebuildAvailable,
    Blocked,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUpdateState {
    pub repo_root: String,
    pub branch: Option<String>,
    pub local_head: Option<String>,
    pub remote_head: Option<String>,
    pub built_commit: Option<String>,
    pub dirty_files: Vec<String>,
    pub fast_forwardable: bool,
    pub local_ahead: bool,
    pub fetch_performed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUpdatePlan {
    pub status: UpdateStatus,
    pub update_available: bool,
    pub rebuild_required: bool,
    pub should_fast_forward: bool,
    pub blocked: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUpdateStep {
    pub name: String,
    pub command: String,
    pub success: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUpdateReport {
    pub ok: bool,
    pub action: String,
    pub status: UpdateStatus,
    pub repo_root: String,
    pub branch: Option<String>,
    pub local_head: Option<String>,
    pub remote_head: Option<String>,
    pub built_commit: Option<String>,
    pub dirty: bool,
    pub dirty_files: Vec<String>,
    pub fetch_performed: bool,
    pub update_available: bool,
    pub rebuild_required: bool,
    pub should_fast_forward: bool,
    pub blocked: bool,
    pub blocked_reason: Option<String>,
    pub steps: Vec<WorkspaceUpdateStep>,
    pub next: Vec<String>,
}

pub fn check_workspace_update(no_fetch: bool) -> Result<WorkspaceUpdateReport, CliError> {
    let repo_root = locate_repo_root()?;
    let mut steps = vec![];
    let fetch_performed = maybe_fetch(&repo_root, no_fetch, &mut steps)?;
    let state = inspect_workspace(&repo_root, fetch_performed)?;
    let plan = plan_update(&state);

    Ok(report_from_plan("update_check", &state, &plan, steps, true))
}

pub fn run_workspace_update(
    no_fetch: bool,
    skip_shim: bool,
) -> Result<WorkspaceUpdateReport, CliError> {
    let repo_root = locate_repo_root()?;
    let mut steps = vec![];
    let fetch_performed = maybe_fetch(&repo_root, no_fetch, &mut steps)?;
    let mut state = inspect_workspace(&repo_root, fetch_performed)?;
    let mut plan = plan_update(&state);

    if plan.blocked {
        return Ok(report_from_plan("update", &state, &plan, steps, false));
    }

    let mut rebuilt = false;
    if plan.should_fast_forward {
        let trusted_commit = match trusted_update_commit(state.remote_head.as_deref()) {
            Ok(commit) => commit,
            Err(reason) => {
                block_plan(&mut plan, reason);
                return Ok(report_from_plan("update", &state, &plan, steps, false));
            }
        };
        ensure_canonical_origin(&repo_root)?;
        run_step(
            &repo_root,
            "fast_forward",
            "git",
            &["merge", "--ff-only", &trusted_commit],
            &mut steps,
        )?;
        state = inspect_workspace(&repo_root, fetch_performed)?;
        if state.local_head.as_deref() != Some(trusted_commit.as_str()) {
            return Err(CliError::new(
                "updated checkout did not land on the explicitly trusted commit",
            ));
        }
        plan = plan_update(&state);
        if plan.blocked {
            return Ok(report_from_plan("update", &state, &plan, steps, false));
        }
    }

    if plan.rebuild_required {
        run_step(
            &repo_root,
            "build_release_binary",
            "cargo",
            &["build", "--release", "--locked", "--bin", "petri"],
            &mut steps,
        )?;
        rebuilt = true;
    }

    if cfg!(windows) && !skip_shim {
        let current_process_id = std::process::id().to_string();
        run_step(
            &repo_root,
            "install_windows_shim",
            "powershell.exe",
            &[
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "scripts/install-petri.ps1",
                "-SkipBuild",
                "-WaitForPid",
                &current_process_id,
            ],
            &mut steps,
        )?;
    }

    let mut final_state = inspect_workspace(&repo_root, fetch_performed)?;
    if rebuilt {
        final_state.built_commit = final_state.local_head.clone();
    }
    let final_plan = plan_update(&final_state);

    Ok(report_from_plan(
        "update",
        &final_state,
        &final_plan,
        steps,
        !final_plan.blocked,
    ))
}

pub fn render_update_report(report: &WorkspaceUpdateReport) -> String {
    let mut lines = vec![
        match report.action.as_str() {
            "update" => "Petri update".to_string(),
            _ => "Petri update check".to_string(),
        },
        format!("status={}", status_slug(report.status)),
        format!("repo={}", report.repo_root),
        format!(
            "branch={} | local={} | remote={} | binary={}",
            report.branch.as_deref().unwrap_or("detached"),
            short_commit(report.local_head.as_deref()),
            short_commit(report.remote_head.as_deref()),
            short_commit(report.built_commit.as_deref())
        ),
    ];

    if report.fetch_performed {
        lines.push("fetched=origin refs refreshed".to_string());
    }

    if report.blocked {
        lines.push(format!(
            "blocked={}",
            report
                .blocked_reason
                .as_deref()
                .unwrap_or("update cannot run automatically")
        ));
        for dirty in report.dirty_files.iter().take(5) {
            lines.push(format!("dirty={dirty}"));
        }
    } else if report.update_available {
        lines.push("remote update available; Petri can fast-forward then rebuild.".to_string());
    } else if report.rebuild_required {
        lines.push("source is current, but the release binary needs a rebuild.".to_string());
    } else {
        lines.push("Petri is up to date.".to_string());
    }

    for step in &report.steps {
        lines.push(format!(
            "step={} | success={} | {}",
            step.name, step.success, step.detail
        ));
    }

    if !report.next.is_empty() {
        lines.push("next:".to_string());
        lines.extend(report.next.iter().map(|item| format!("  {item}")));
    }

    lines.join("\n")
}

pub fn plan_update(state: &WorkspaceUpdateState) -> WorkspaceUpdatePlan {
    let remote_ahead = remote_is_ahead(state);
    let diverged = heads_diverged(state);
    if remote_ahead || diverged {
        if !state.dirty_files.is_empty() {
            return WorkspaceUpdatePlan {
                status: UpdateStatus::Blocked,
                update_available: true,
                rebuild_required: false,
                should_fast_forward: false,
                blocked: true,
                blocked_reason: Some(DIRTY_WORKTREE_BLOCKED_REASON.to_string()),
            };
        }

        if diverged {
            return WorkspaceUpdatePlan {
                status: UpdateStatus::Blocked,
                update_available: true,
                rebuild_required: false,
                should_fast_forward: false,
                blocked: true,
                blocked_reason: Some(DIVERGED_BRANCH_BLOCKED_REASON.to_string()),
            };
        }

        return WorkspaceUpdatePlan {
            status: UpdateStatus::UpdateAvailable,
            update_available: true,
            rebuild_required: true,
            should_fast_forward: true,
            blocked: false,
            blocked_reason: None,
        };
    }

    if binary_is_stale(state) {
        return WorkspaceUpdatePlan {
            status: UpdateStatus::RebuildAvailable,
            update_available: false,
            rebuild_required: true,
            should_fast_forward: false,
            blocked: false,
            blocked_reason: None,
        };
    }

    WorkspaceUpdatePlan {
        status: UpdateStatus::Current,
        update_available: false,
        rebuild_required: false,
        should_fast_forward: false,
        blocked: false,
        blocked_reason: None,
    }
}

fn report_from_plan(
    action: &str,
    state: &WorkspaceUpdateState,
    plan: &WorkspaceUpdatePlan,
    steps: Vec<WorkspaceUpdateStep>,
    ok: bool,
) -> WorkspaceUpdateReport {
    WorkspaceUpdateReport {
        ok,
        action: action.to_string(),
        status: plan.status,
        repo_root: state.repo_root.clone(),
        branch: state.branch.clone(),
        local_head: state.local_head.clone(),
        remote_head: state.remote_head.clone(),
        built_commit: state.built_commit.clone(),
        dirty: !state.dirty_files.is_empty(),
        dirty_files: state.dirty_files.clone(),
        fetch_performed: state.fetch_performed,
        update_available: plan.update_available,
        rebuild_required: plan.rebuild_required,
        should_fast_forward: plan.should_fast_forward,
        blocked: plan.blocked,
        blocked_reason: plan.blocked_reason.clone(),
        steps,
        next: next_steps(action, plan),
    }
}

fn next_steps(action: &str, plan: &WorkspaceUpdatePlan) -> Vec<String> {
    if plan.blocked {
        if plan.blocked_reason.as_deref() == Some(DIRTY_WORKTREE_BLOCKED_REASON) {
            return vec![
                "Commit or stash local changes before running petri update.".to_string(),
                "Run petri update again after the worktree is clean.".to_string(),
            ];
        }
        if plan.blocked_reason.as_deref() == Some(UPDATE_PROVENANCE_BLOCKED_REASON) {
            return vec![
                "Verify the exact origin/HEAD commit through the trusted release channel."
                    .to_string(),
                format!(
                    "Set {UPDATE_COMMIT_ENV} to that full commit, then run petri update again."
                ),
            ];
        }
        return vec![
            "Merge or rebase the local branch with origin/HEAD manually.".to_string(),
            "Run petri update again after the branch histories are reconciled.".to_string(),
        ];
    }
    if action == "update_check" && (plan.update_available || plan.rebuild_required) {
        return vec!["Run: petri update".to_string()];
    }
    if action == "update" && !plan.update_available && !plan.rebuild_required {
        return vec!["Run: petri --version".to_string()];
    }
    vec![]
}

fn maybe_fetch(
    repo_root: &Path,
    no_fetch: bool,
    steps: &mut Vec<WorkspaceUpdateStep>,
) -> Result<bool, CliError> {
    if no_fetch {
        return Ok(false);
    }
    ensure_canonical_origin(repo_root)?;
    run_step(
        repo_root,
        "fetch_refs",
        "git",
        &["fetch", "--prune", "origin"],
        steps,
    )?;
    Ok(true)
}

fn ensure_canonical_origin(repo_root: &Path) -> Result<(), CliError> {
    let origin = required_stdout(repo_root, "git", &["remote", "get-url", "origin"])?;
    if CANONICAL_ORIGIN_URLS.contains(&origin.as_str()) {
        Ok(())
    } else {
        Err(CliError::new(format!(
            "Petri updates require the canonical origin URL; found {origin}"
        )))
    }
}

fn trusted_update_commit(remote_head: Option<&str>) -> Result<String, String> {
    validate_trusted_update_commit(env::var(UPDATE_COMMIT_ENV).ok().as_deref(), remote_head)
}

fn validate_trusted_update_commit(
    configured_commit: Option<&str>,
    remote_head: Option<&str>,
) -> Result<String, String> {
    let remote_head = remote_head.ok_or_else(|| UPDATE_PROVENANCE_BLOCKED_REASON.to_string())?;
    let configured_commit = configured_commit
        .map(str::trim)
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| UPDATE_PROVENANCE_BLOCKED_REASON.to_string())?;
    if !configured_commit.eq_ignore_ascii_case(remote_head) {
        return Err(UPDATE_PROVENANCE_BLOCKED_REASON.to_string());
    }
    Ok(remote_head.to_string())
}

fn block_plan(plan: &mut WorkspaceUpdatePlan, reason: String) {
    plan.status = UpdateStatus::Blocked;
    plan.should_fast_forward = false;
    plan.blocked = true;
    plan.blocked_reason = Some(reason);
}

fn inspect_workspace(
    repo_root: &Path,
    fetch_performed: bool,
) -> Result<WorkspaceUpdateState, CliError> {
    let branch = optional_stdout(repo_root, "git", &["branch", "--show-current"])?;
    let local_head = optional_stdout(repo_root, "git", &["rev-parse", "HEAD"])?;
    let remote_head = optional_stdout(repo_root, "git", &["rev-parse", "origin/HEAD"])?;
    let dirty_files = required_stdout(repo_root, "git", &["status", "--short"])?
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(ToString::to_string)
        .collect::<Vec<String>>();
    let fast_forwardable = if local_head.is_some() && remote_head.is_some() {
        run_process(
            repo_root,
            "git",
            &["merge-base", "--is-ancestor", "HEAD", "origin/HEAD"],
        )?
        .success
    } else {
        false
    };
    let local_ahead = if local_head != remote_head && local_head.is_some() && remote_head.is_some()
    {
        run_process(
            repo_root,
            "git",
            &["merge-base", "--is-ancestor", "origin/HEAD", "HEAD"],
        )?
        .success
    } else {
        false
    };

    Ok(WorkspaceUpdateState {
        repo_root: repo_root.display().to_string(),
        branch: branch.filter(|value| !value.is_empty()),
        local_head,
        remote_head,
        built_commit: BUILT_COMMIT
            .filter(|value| !value.is_empty() && *value != "unknown")
            .map(ToString::to_string),
        dirty_files,
        fast_forwardable,
        local_ahead,
        fetch_performed,
    })
}

fn locate_repo_root() -> Result<PathBuf, CliError> {
    let mut candidates = vec![];
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir);
    }
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            candidates.push(parent.to_path_buf());
        }
        if let Ok(resolved_exe) = fs::canonicalize(&current_exe) {
            if let Some(parent) = resolved_exe.parent() {
                candidates.push(parent.to_path_buf());
            }
        }
    }
    if let Some(manifest_dir) = BUILD_MANIFEST_DIR {
        if !manifest_dir.is_empty() {
            candidates.push(PathBuf::from(manifest_dir));
        }
    }

    for candidate in candidates {
        for ancestor in candidate.ancestors() {
            if is_cli_repo_root(ancestor) {
                return fs::canonicalize(ancestor).map_err(|error| {
                    CliError::new(format!(
                        "failed to resolve Petri repo root {}: {error}",
                        ancestor.display()
                    ))
                });
            }
        }
    }

    Err(CliError::new(
        "could not locate ameba_cli repo root from current directory or executable path",
    ))
}

fn is_cli_repo_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("config/options_catalog.json").is_file()
}

fn required_stdout(repo_root: &Path, program: &str, args: &[&str]) -> Result<String, CliError> {
    let output = run_process(repo_root, program, args)?;
    if output.success {
        Ok(output.stdout.trim().to_string())
    } else {
        Err(command_error(program, args, &output))
    }
}

fn optional_stdout(
    repo_root: &Path,
    program: &str,
    args: &[&str],
) -> Result<Option<String>, CliError> {
    let output = run_process(repo_root, program, args)?;
    if output.success {
        let text = output.stdout.trim().to_string();
        Ok(if text.is_empty() { None } else { Some(text) })
    } else {
        Ok(None)
    }
}

fn run_step(
    repo_root: &Path,
    name: &str,
    program: &str,
    args: &[&str],
    steps: &mut Vec<WorkspaceUpdateStep>,
) -> Result<(), CliError> {
    let output = run_process(repo_root, program, args)?;
    let detail = step_detail(&output);
    steps.push(WorkspaceUpdateStep {
        name: name.to_string(),
        command: shell_words(program, args),
        success: output.success,
        detail,
    });
    if output.success {
        Ok(())
    } else {
        Err(command_error(program, args, &output))
    }
}

fn run_process(repo_root: &Path, program: &str, args: &[&str]) -> Result<ProcessOutput, CliError> {
    let output = ProcessCommand::new(program)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            CliError::new(format!(
                "failed to run {}: {error}",
                shell_words(program, args)
            ))
        })?;

    Ok(ProcessOutput {
        success: output.status.success(),
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[derive(Debug)]
struct ProcessOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn command_error(program: &str, args: &[&str], output: &ProcessOutput) -> CliError {
    let detail = step_detail(output);
    CliError::new(format!(
        "{} failed{}: {}",
        shell_words(program, args),
        output
            .status_code
            .map(|code| format!(" with exit code {code}"))
            .unwrap_or_default(),
        detail
    ))
}

fn step_detail(output: &ProcessOutput) -> String {
    let text = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if text.is_empty() {
        "completed".to_string()
    } else {
        text.lines()
            .next()
            .unwrap_or("completed")
            .trim()
            .to_string()
    }
}

fn shell_words(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<&str>>()
        .join(" ")
}

fn remote_is_ahead(state: &WorkspaceUpdateState) -> bool {
    match (&state.local_head, &state.remote_head) {
        (Some(local), Some(remote)) => local != remote && state.fast_forwardable,
        _ => false,
    }
}

fn heads_diverged(state: &WorkspaceUpdateState) -> bool {
    match (&state.local_head, &state.remote_head) {
        (Some(local), Some(remote)) => {
            local != remote && !state.fast_forwardable && !state.local_ahead
        }
        _ => false,
    }
}

fn binary_is_stale(state: &WorkspaceUpdateState) -> bool {
    match (&state.local_head, &state.built_commit) {
        (Some(local), Some(built)) => local != built,
        (Some(_), None) => true,
        _ => false,
    }
}

fn status_slug(status: UpdateStatus) -> &'static str {
    match status {
        UpdateStatus::Current => "current",
        UpdateStatus::UpdateAvailable => "update_available",
        UpdateStatus::RebuildAvailable => "rebuild_available",
        UpdateStatus::Blocked => "blocked",
    }
}

fn short_commit(commit: Option<&str>) -> String {
    commit
        .map(|value| value.chars().take(7).collect::<String>())
        .unwrap_or_else(|| "unknown".to_string())
}
