//! Automatic updates are restricted to installed preview builds. All network
//! and archive work runs away from terminal input; restart waits for safe idle.
use crate::release_update::{self, Release};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

enum Reply {
    Checked(Result<Option<Release>, String>),
    Prepared(Result<(), String>),
}
pub enum Action {
    Restart,
    Error(String),
}
pub struct Updates {
    enabled: bool,
    next_check: Instant,
    reply: Option<mpsc::Receiver<Reply>>,
    pending: Option<Release>,
    scheduled: bool,
    installing: bool,
}
impl Updates {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: enabled
                && release_update::enabled()
                && std::env::var_os("PETRI_MCP_READ_ONLY").is_none()
                && std::env::var("PETRI_UPDATE_CHECK").ok().as_deref() != Some("0"),
            next_check: Instant::now(),
            reply: None,
            pending: None,
            scheduled: false,
            installing: false,
        }
    }
    pub fn installing(&self) -> bool {
        self.installing
    }
    pub fn tick(&mut self, safe: bool) -> Option<Action> {
        if let Some(reply) = &self.reply {
            match reply.try_recv() {
                Ok(value) => {
                    self.reply = None;
                    match value {
                        Reply::Checked(Ok(release)) => self.pending = release,
                        Reply::Checked(Err(_)) => {} // Offline checks do not interrupt research.
                        Reply::Prepared(Ok(())) => self.scheduled = true,
                        Reply::Prepared(Err(error)) => {
                            self.installing = false;
                            return Some(Action::Error(error));
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.reply = None;
                    if self.installing {
                        self.installing = false;
                        return Some(Action::Error(
                            "Update worker stopped. Try petri update later.".into(),
                        ));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.scheduled && safe {
            return Some(Action::Restart);
        }
        if self.reply.is_some() || self.scheduled || !self.enabled {
            return None;
        }
        if safe {
            if let Some(release) = self.pending.take() {
                self.installing = true;
                let (send, receive) = mpsc::channel();
                self.reply = Some(receive);
                thread::spawn(move || {
                    let _ = send.send(Reply::Prepared(
                        release_update::prepare(&release, true).map(|_| ()),
                    ));
                });
                return None;
            }
        }
        if Instant::now() >= self.next_check {
            self.next_check = Instant::now() + Duration::from_secs(6 * 60 * 60);
            let (send, receive) = mpsc::channel();
            self.reply = Some(receive);
            thread::spawn(move || {
                let _ = send.send(Reply::Checked(release_update::check().map(|report| {
                    if report.update_available {
                        report.release
                    } else {
                        None
                    }
                })));
            });
        }
        None
    }
}

/// Running bots don't hold an update back: quitting takes their ladders down
/// and the new petri starts them again from `bots/.running`.
pub fn safe_to_restart(app: &super::model::App, idle: Duration) -> bool {
    app.modal.is_none()
        && app.markets.ticket.is_none()
        && app.pending.is_none()
        && !app.mutation_inflight
        && !app.busy
        && idle >= Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_waits_for_forms_pending_saves_network_and_idle() {
        let mut app = super::super::model::App::new("petri".into(), "http://127.0.0.1:4185".into());
        let idle = Duration::from_secs(30);
        assert!(safe_to_restart(&app, idle));
        assert!(!safe_to_restart(&app, Duration::ZERO));
        app.modal = Some(super::super::model::Modal::Name("draft".into()));
        assert!(!safe_to_restart(&app, idle));
        app.modal = None;
        app.pending = Some(super::super::model::Post {
            path: "/save",
            body: serde_json::json!({}),
        });
        assert!(!safe_to_restart(&app, idle));
        app.pending = None;
        app.mutation_inflight = true;
        assert!(!safe_to_restart(&app, idle));
        app.mutation_inflight = false;
        app.busy = true;
        assert!(!safe_to_restart(&app, idle));
    }
    #[test]
    fn disabled_session_never_checks_or_installs() {
        let mut updates = Updates::new(false);
        assert!(updates.tick(true).is_none());
        assert!(updates.reply.is_none());
        assert!(!updates.installing());
    }

    #[test]
    fn failed_install_unfreezes_input_and_does_not_retry_in_a_loop() {
        let (send, reply) = mpsc::channel();
        send.send(Reply::Prepared(Err("offline".into()))).unwrap();
        let mut updates = Updates {
            enabled: true,
            next_check: Instant::now() + Duration::from_secs(600),
            reply: Some(reply),
            pending: None,
            scheduled: false,
            installing: true,
        };
        assert!(matches!(updates.tick(true), Some(Action::Error(_))));
        assert!(!updates.installing());
        assert!(updates.tick(true).is_none());
        assert!(updates.reply.is_none());
    }

    #[test]
    fn prepared_update_requests_restart_after_safe_handoff() {
        let (send, reply) = mpsc::channel();
        send.send(Reply::Prepared(Ok(()))).unwrap();
        let mut updates = Updates {
            enabled: true,
            next_check: Instant::now() + Duration::from_secs(600),
            reply: Some(reply),
            pending: None,
            scheduled: false,
            installing: true,
        };
        assert!(updates.tick(false).is_none());
        assert!(matches!(updates.tick(true), Some(Action::Restart)));
    }
}
