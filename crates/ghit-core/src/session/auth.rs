//! Auth state: the startup and probe channels plus the re-auth bookkeeping.

use std::sync::mpsc::Receiver;

use super::Session;
use crate::github;
use crate::runtime::spawn_bg;

/// Startup resolution and auth probes in flight, plus the dismissal and
/// re-probe bookkeeping. The pending re-auth question itself is view state.
#[derive(Default)]
pub(crate) struct AuthState {
    pub(crate) bootstrap_rx: Option<Receiver<Option<String>>>,
    /// Early auth probe in flight (`Session::start` began it).
    pub(crate) rx: Option<Receiver<Result<Vec<String>, String>>>,
    /// Set by input; the event loop runs the refresh outside the TUI.
    pub(crate) reauth_requested: bool,
    /// The user dismissed the prompt: later auth failures notify only,
    /// never re-modal. Cleared by a successful re-auth.
    pub(crate) dismissed: bool,
    /// An auth problem was seen and no re-auth has fixed it yet. Only a
    /// successful re-auth clears it, because a clean probe cannot prove
    /// every permission.
    pub(crate) degraded: bool,
    /// The repo-scoped access probe fired once the repo resolved.
    pub(crate) repo_probed: bool,
}

impl Session {
    /// The user dismissed the re-auth question: later auth failures
    /// notify only.
    pub fn dismiss_auth(&mut self) {
        self.auth.dismissed = true;
    }

    /// An auth problem was seen and no re-auth has fixed it yet.
    pub fn mark_auth_degraded(&mut self) {
        self.auth.degraded = true;
    }

    /// The refresh fixed the token: clear the warning and the dismissal.
    pub fn auth_recovered(&mut self) {
        self.auth.dismissed = false;
        self.auth.degraded = false;
    }

    /// Input asks for a re-auth: the event loop runs it off the TUI.
    pub fn request_reauth(&mut self) {
        self.auth.reauth_requested = true;
    }

    /// Take the pending re-auth request, so the loop runs it once.
    pub fn take_reauth_request(&mut self) -> bool {
        std::mem::take(&mut self.auth.reauth_requested)
    }

    /// Probe the token again after a refresh; the result lands as
    /// `SessionEvent::AuthChecked`.
    pub fn recheck_auth(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.auth.rx = Some(rx);
        spawn_bg(async move {
            let _ = tx.send(github::check_auth().await);
        });
    }
}
