//! TUI adapter: source and installed-app checks share presentation, not trust or
//! execution paths. Release JSON is emitted directly by main, without Git fields.
use crate::{
    backend::CliError,
    release_update,
    workspace_update::{self, UpdateStatus, WorkspaceUpdateReport},
};

pub fn check_for_tui(no_fetch: bool) -> Result<WorkspaceUpdateReport, CliError> {
    if !release_update::enabled() {
        return workspace_update::check_workspace_update(no_fetch);
    }
    if no_fetch {
        return Err(CliError::new(
            "--no-fetch applies only to source-checkout updates.",
        ));
    }
    let report = release_update::check().map_err(CliError::new)?;
    Ok(WorkspaceUpdateReport {
        ok: report.ok,
        action: "release_update_check".into(),
        status: if report.status == "blocked" {
            UpdateStatus::Blocked
        } else if report.update_available {
            UpdateStatus::UpdateAvailable
        } else {
            UpdateStatus::Current
        },
        repo_root: String::new(),
        branch: None,
        local_head: None,
        remote_head: None,
        built_commit: None,
        dirty: false,
        dirty_files: Vec::new(),
        fetch_performed: false,
        update_available: report.update_available,
        rebuild_required: false,
        should_fast_forward: false,
        blocked: report.status == "blocked",
        blocked_reason: (report.status == "blocked").then_some(report.message),
        steps: Vec::new(),
        next: Vec::new(),
    })
}
