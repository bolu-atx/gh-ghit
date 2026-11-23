//! Auth state: the startup and probe channels plus the re-auth bookkeeping.

use std::sync::mpsc::Receiver;

/// Startup resolution and auth probes in flight, plus the dismissal and
/// re-probe bookkeeping. The pending re-auth question itself is view state.
#[derive(Default)]
pub struct AuthState {
    pub bootstrap_rx: Option<Receiver<Option<String>>>,
    /// Early auth probe in flight (`Session::start` began it).
    pub rx: Option<Receiver<Result<Vec<String>, String>>>,
    /// Set by input; the event loop runs the refresh outside the TUI.
    pub reauth_requested: bool,
    /// The user dismissed the prompt: later auth failures notify only,
    /// never re-modal. Cleared by a successful re-auth.
    pub dismissed: bool,
    /// An auth problem was seen and no re-auth has fixed it yet. Only a
    /// successful re-auth clears it, because a clean probe cannot prove
    /// every permission.
    pub degraded: bool,
    /// The repo-scoped access probe fired once the repo resolved.
    pub repo_probed: bool,
}
