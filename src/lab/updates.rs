//! Update-check state and transitions. The app owns spawning work and displaying status.
use super::{LabExitAction, PETRI_UPDATE_CHECK_ENV};
use crate::workspace_update::{UpdateStatus, WorkspaceUpdateReport};

pub(super) struct UpdateState {
    enabled: bool,
    request_id: u64,
    report: Option<WorkspaceUpdateReport>,
    issue: Option<String>,
    forced: bool,
    loading: bool,
    mouse_requested: bool,
}

pub(super) struct CheckRequest {
    pub request_id: Option<u64>,
    pub status: Option<String>,
}

impl UpdateState {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            request_id: 0,
            report: None,
            issue: None,
            forced: false,
            loading: false,
            mouse_requested: false,
        }
    }

    pub(super) fn report(&self) -> Option<&WorkspaceUpdateReport> {
        self.report.as_ref()
    }

    pub(super) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(super) fn has_result(&self) -> bool {
        self.report.is_some() || self.issue.is_some()
    }

    pub(super) fn begin(&mut self, force: bool) -> CheckRequest {
        let mut request = CheckRequest {
            request_id: None,
            status: None,
        };
        if !self.enabled {
            if force {
                request.status = Some(format!(
                    "Petri update checks are off. Unset {PETRI_UPDATE_CHECK_ENV} or run `petri update check`."
                ));
            }
            return request;
        }
        if self.loading {
            if force {
                request.status = Some("Petri is already checking for updates...".to_string());
            }
            return request;
        }
        if !force && self.has_result() {
            return request;
        }
        self.request_id = self.request_id.wrapping_add(1);
        self.loading = true;
        self.forced = force;
        self.issue = None;
        if force {
            request.status = Some("Checking for Petri updates...".to_string());
        }
        request.request_id = Some(self.request_id);
        request
    }

    pub(super) fn apply(
        &mut self,
        request_id: u64,
        result: Result<WorkspaceUpdateReport, String>,
    ) -> Option<String> {
        if request_id != self.request_id {
            return None;
        }
        self.loading = false;
        let status = match result {
            Ok(report) => {
                let announce = self.forced
                    || report.update_available
                    || report.rebuild_required
                    || report.blocked;
                self.report = Some(report);
                self.issue = None;
                announce.then(|| self.status_text())
            }
            Err(error) => {
                let status = self.forced.then(|| error.clone());
                self.report = None;
                self.issue = Some(error);
                status
            }
        };
        self.forced = false;
        status
    }

    pub(super) fn exit_action(&self) -> Option<LabExitAction> {
        let report = self.report.as_ref()?;
        if report.blocked {
            return None;
        }
        match report.status {
            UpdateStatus::UpdateAvailable => Some(LabExitAction::RunUpdate),
            UpdateStatus::RebuildAvailable => Some(LabExitAction::RunRebuild),
            UpdateStatus::Current | UpdateStatus::Blocked => None,
        }
    }

    pub(super) fn request_mouse_exit(&mut self) {
        self.mouse_requested = true;
    }

    pub(super) fn take_mouse_request(&mut self) -> bool {
        std::mem::take(&mut self.mouse_requested)
    }

    pub(super) fn status_text(&self) -> String {
        let Some(report) = self.report.as_ref() else {
            return "Press U to check for Petri updates.".to_string();
        };
        if report.blocked {
            return report
                .blocked_reason
                .as_deref()
                .map(|reason| format!("Petri update available, but blocked: {reason}."))
                .unwrap_or_else(|| "Petri update available, but blocked.".to_string());
        }
        if report.update_available {
            return "Petri update available. Press U to exit and run `petri update`.".to_string();
        }
        if report.rebuild_required {
            return "Petri rebuild available. Press U to exit and rebuild.".to_string();
        }
        "Petri is up to date.".to_string()
    }
}
