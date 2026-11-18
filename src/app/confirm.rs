//! Confirm dialogs, undo toasts and auth re-prompt.

use std::io::Stdout;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, Focus, NavIntent, spawn_bg};
use crate::github;
use crate::model::IssueData;

/// One button of a generic confirm dialog: the key that picks it plus
/// its label. `Esc` always cancels and never needs a listed choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmChoice {
    pub(crate) key: char,
    pub(crate) label: String,
}

/// What a generic confirm dialog decides. The renderer and key handling
/// only see title/body/choices; each variant owns its choice indexes
/// in `confirm_choose`. New uses add a variant plus its constructor
/// and one dispatch arm — no renderer or input changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmKind {
    /// Leaving a PR with unsubmitted review comments. Choice 0 submits
    /// then navigates, choice 1 navigates leaving them pending, choice
    /// 2 (and `Esc`) stays put.
    PendingNav { intent: NavIntent },
    /// Re-auth question for a dead or under-scoped token. Choice 0
    /// re-auths now, choice 1 (and `Esc`) dismisses to a status hint.
    Reauth,
    /// `Esc` on a non-empty compose draft. Choice 0 (and `Esc`) keeps
    /// editing, choice 1 discards the draft.
    DiscardDraft,
    /// Quit with an unsent draft or unsubmitted review comments.
    /// Choice 0 (and `Esc`) stays, choice 1 quits.
    Quit,
}

/// What `u` restores while its toast is up. Each variant carries exactly
/// what the removal dropped.
#[derive(Clone, Debug)]
pub(crate) enum UndoAction {
    Evict {
        number: u64,
        visited_at: Option<usize>,
        recent_at: Option<usize>,
        data: Option<Box<IssueData>>,
        statuses: Option<Vec<(String, Option<String>)>>,
        row_expanded: bool,
    },
    Query { index: usize, query: github::SavedQuery },
}

/// One live undo: the action, when it was armed, and the toast text.
#[derive(Clone, Debug)]
pub(crate) struct PendingUndo {
    pub(crate) action: UndoAction,
    pub(crate) at: Instant,
    pub(crate) message: String,
}

/// One line of a generic confirm body. Plain lines render as body
/// text; an emphasized line names the actionable fact in amber bold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmLine {
    pub(crate) text: String,
    pub(crate) emphasis: bool,
}

impl ConfirmLine {
    pub(crate) fn plain(text: String) -> Self {
        Self { text, emphasis: false }
    }

    pub(crate) fn strong(text: String) -> Self {
        Self { text, emphasis: true }
    }
}

/// Generic confirm modal: title, message lines, and explicit choices.
/// Specializations build one with a constructor like
/// `open_pending_nav` and interpret choices in `confirm_choose`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmDialog {
    pub(crate) title: String,
    pub(crate) body: Vec<ConfirmLine>,
    pub(crate) choices: Vec<ConfirmChoice>,
    pub(crate) kind: ConfirmKind,
}

/// Auth state: the startup bootstrap channel, the re-auth probe channel,
/// and the dismissal/re-probe bookkeeping. The pending re-auth
/// question itself lives in the generic confirm modal.
#[derive(Default)]
pub(crate) struct AuthState {
    pub(crate) bootstrap_rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    /// Early auth probe in flight (`bootstrap` started it).
    pub(crate) rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// Set by input; the event loop runs the refresh outside the TUI.
    pub(crate) reauth_requested: bool,
    /// The user dismissed the prompt: later auth failures notify only,
    /// never re-modal. Cleared by a successful re-auth.
    pub(crate) dismissed: bool,
    /// An auth problem was seen and no re-auth has fixed it yet. The
    /// status bar shows a badge while set; only a successful re-auth
    /// clears it, because a clean probe cannot prove every permission.
    pub(crate) degraded: bool,
    /// The repo-scoped access probe fired once the repo resolved.
    pub(crate) repo_probed: bool,
}

/// How long `u` can still undo; matches the toast lifetime.
const UNDO_WINDOW: std::time::Duration = std::time::Duration::from_secs(4);

impl App {
    /// Arm one undo and show its toast. Replaces any earlier undo.
    pub(super) fn arm_undo(&mut self, action: UndoAction, message: String) {
        self.notify(message.clone());
        self.undo = Some(PendingUndo { action, at: Instant::now(), message });
    }

    /// An undo is armed, inside the toast window, and its toast is still
    /// the visible message: a later toast hides the offer, so `u` stops.
    pub(crate) fn undo_live(&self) -> bool {
        self.undo.as_ref().is_some_and(|u| u.at.elapsed() < UNDO_WINDOW && self.message == u.message)
    }

    /// Restore what the last `x`/delete removed, if its window is open.
    pub(crate) fn undo_last(&mut self) {
        if !self.undo_live() {
            self.undo = None;
            return;
        }
        let Some(undo) = self.undo.take() else { return };
        match undo.action {
            UndoAction::Evict { number: n, visited_at, recent_at, data, statuses, row_expanded } => {
                if let Some(i) = visited_at {
                    self.nav.visited.insert(i.min(self.nav.visited.len()), n);
                }
                if let Some(i) = recent_at {
                    self.nav.recent.insert(i.min(self.nav.recent.len()), n);
                }
                if let Some(d) = data {
                    self.issues.data.insert(n, *d);
                }
                if let Some(st) = statuses {
                    self.issues.statuses.insert(n, st);
                }
                if row_expanded {
                    self.diff.expanded_rows.insert(n);
                }
                self.show_issue(n);
                self.notify(format!("restored #{n}"));
            }
            UndoAction::Query { index, query } => {
                let name = query.name.clone();
                self.sidebar.saved_queries.insert(index.min(self.sidebar.saved_queries.len()), query);
                self.persist_prefs();
                self.notify(format!("restored query {name}"));
            }
        }
    }

    /// `Esc` in compose: an empty draft closes at once, a typed one asks.
    pub(crate) fn request_discard_compose(&mut self) {
        let typed = self.draft().is_some_and(|d| !d.body.is_empty());
        if !typed {
            self.discard_compose();
            return;
        }
        self.open_confirm(ConfirmDialog {
            title: "Discard draft".to_string(),
            body: vec![ConfirmLine::plain("The comment draft has text that will be lost.".to_string())],
            choices: vec![
                ConfirmChoice { key: 'k', label: "keep editing".to_string() },
                ConfirmChoice { key: 'd', label: "discard".to_string() },
            ],
            kind: ConfirmKind::DiscardDraft,
        });
    }

    /// Quit now, unless a typed draft or unsubmitted review comments
    /// would be lost: then ask first.
    pub(crate) fn request_quit(&mut self) {
        let draft = self.draft().is_some_and(|d| !d.body.is_empty());
        let pending = self.pending_threads();
        if !draft && pending == 0 {
            self.quit = true;
            return;
        }
        let line = if draft {
            "An unsent comment draft will be lost.".to_string()
        } else {
            format!("{pending} unsubmitted review comment thread{} stay pending on GitHub.", if pending == 1 { "" } else { "s" })
        };
        self.open_confirm(ConfirmDialog {
            title: "Quit ghit".to_string(),
            body: vec![ConfirmLine::plain(line)],
            choices: vec![
                ConfirmChoice { key: 'c', label: "cancel".to_string() },
                ConfirmChoice { key: 'q', label: "quit".to_string() },
            ],
            kind: ConfirmKind::Quit,
        });
    }

    /// A confirm modal is up, whatever it decides.
    pub(crate) fn confirm_open(&self) -> bool {
        self.confirm().is_some()
    }

    /// Pick choice `index` of the open dialog, then run what the
    /// specialization decides. Out-of-range indexes are ignored so a
    /// stale key never acts on the wrong button.
    pub(crate) fn confirm_choose(&mut self, index: usize) {
        let Some(dialog) = self.take_confirm() else { return };
        match dialog.kind.clone() {
            ConfirmKind::PendingNav { intent } => match index {
                0 => {
                    self.submit_pending_review();
                    self.run_nav(intent);
                }
                1 => self.run_nav(intent),
                _ => {}
            },
            ConfirmKind::DiscardDraft => {
                if index == 1 {
                    self.discard_compose();
                }
            }
            ConfirmKind::Quit => {
                if index == 1 {
                    self.quit = true;
                }
            }
            ConfirmKind::Reauth => match index {
                0 => {
                    self.auth.reauth_requested = true;
                    self.open_confirm(dialog);
                }
                1 => self.dismiss_reauth(),
                _ => {}
            },
        }
    }

    /// Drop the open dialog: the deferred navigation never runs. A
    /// dropped re-auth question counts as dismissed, like its `n`
    /// choice, so later auth failures notify only.
    pub(crate) fn confirm_cancel(&mut self) {
        let kind = self.take_confirm().map(|d| d.kind);
        if matches!(kind, Some(ConfirmKind::Reauth)) {
            self.dismiss_reauth();
        }
    }

    /// Dismiss the re-auth question: later auth failures notify only,
    /// never re-modal. Names the by-hand command on the status line.
    fn dismiss_reauth(&mut self) {
        let cmd = crate::github::reauth_command();
        self.auth.dismissed = true;
        self.nav.focus = Focus::Main;
        self.notify(format!("auth dismissed — re-auth by hand: {cmd}"));
    }

    /// Run a deferred navigation after a confirm resolves. Never guards:
    /// the guard already ran when the intent was stored.
    fn run_nav(&mut self, intent: NavIntent) {
        match intent {
            NavIntent::Issue(n) => self.open_issue_inner(n),
            NavIntent::Tab(tab) => self.set_tab_inner(tab),
            NavIntent::Step(delta) => self.step_issue_inner(delta),
            NavIntent::Back(steps) => self.go_back_inner(steps),
            NavIntent::Forward => self.go_forward_inner(),
            NavIntent::Evict => self.evict_current_inner(),
        }
    }

    /// Specialization: leaving a PR with unsubmitted review comments.
    /// Choice order is Submit, Discard, Cancel; the renderer shows the
    /// keys. Discard only navigates — the comments stay pending on
    /// GitHub, nothing is deleted.
    pub(crate) fn open_pending_nav(&mut self, intent: NavIntent) {
        let Some(n) = self.nav.current else { return };
        let count = self.pending_threads();
        self.open_confirm(ConfirmDialog {
            title: "Unsubmitted comments".to_string(),
            body: vec![
                ConfirmLine::plain(format!("#{n} has {count} unsubmitted comment{s}.", s = if count == 1 { "" } else { "s" })),
                ConfirmLine::plain("Discard leaves them pending on GitHub.".to_string()),
            ],
            choices: vec![
                ConfirmChoice { key: 's', label: "Submit them".to_string() },
                ConfirmChoice { key: 'd', label: "Discard".to_string() },
                ConfirmChoice { key: 'c', label: "Cancel".to_string() },
            ],
            kind: ConfirmKind::PendingNav { intent },
        });
    }

    /// Specialization: the re-auth question for a dead or
    /// under-scoped token. `scopes_known` is true only when a probe
    /// read the granted scopes; fetch failures cannot name the gap, so
    /// an empty missing list there means unknown, not clean. An open
    /// dialog keeps its turn: the question waits, and the next auth
    /// failure re-raises it.
    pub(crate) fn open_reauth(&mut self, cause: String, missing: Vec<String>, scopes_known: bool) {
        self.auth.degraded = true;
        if self.confirm_open() {
            return;
        }
        let mut body = vec![ConfirmLine::plain(cause)];
        if !scopes_known {
            body.push(ConfirmLine::plain(
                "scopes could not be read from this failure — the error above names the fix".to_string(),
            ));
        } else if missing.is_empty() {
            body.push(ConfirmLine::plain("the token is expired, revoked, or invalid".to_string()));
        } else {
            body.push(ConfirmLine::strong(format!("missing scopes: {}", missing.join(", "))));
            body.push(ConfirmLine::plain(format!("needed: {}", crate::github::REQUIRED_SCOPES.join(", "))));
        }
        body.push(ConfirmLine::plain(crate::github::reauth_command()));
        self.open_confirm(ConfirmDialog {
            title: "GitHub auth".to_string(),
            body,
            choices: vec![
                ConfirmChoice { key: 'y', label: "re-auth now".to_string() },
                ConfirmChoice { key: 'n', label: "dismiss".to_string() },
            ],
            kind: ConfirmKind::Reauth,
        });
    }

    /// Reopen the re-auth question after dismissal left auth degraded.
    /// No-op unless degraded with no dialog up, so `R` never steals a
    /// healthy key or stacks a second modal.
    pub(crate) fn reopen_reauth(&mut self) {
        if !self.auth.degraded || self.confirm_open() {
            return;
        }
        self.open_reauth("auth still failing after dismissal".to_string(), vec![], false);
    }

    /// Guard one navigation: with pending comments on the current issue
    /// the intent waits in a confirm dialog, otherwise it runs at once.
    pub(super) fn guard_nav(&mut self, intent: NavIntent) -> bool {
        if self.pending_threads() == 0 {
            return false;
        }
        self.open_pending_nav(intent);
        true
    }
}

/// Land an auth probe: a dead token or missing scopes raises the
/// re-auth modal; a covered token stays silent.
pub(crate) fn poll_auth(app: &mut App) -> bool {
    let Some(result) = app.auth.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    app.auth.rx = None;
    match result {
        Ok(missing) if missing.is_empty() => {}
        Ok(missing) => {
            app.open_reauth("token lacks scopes ghit needs".to_string(), missing, true);
        }
        // Only auth-class failures rise to the modal: re-auth can fix
        // those, while a rate limit or outage just needs a status line.
        Err(e) if crate::github::is_auth_error(&e) => {
            app.open_reauth(e, vec![], false);
        }
        Err(e) => app.notify(e),
    }
    true
}

/// Route an auth-class fetch failure at the re-auth modal: the early
/// probes cannot see every token kind or repo, so the first failing
/// fetch gets the same question. A dismissed prompt never re-rises;
/// the status-bar badge carries the warning, and the caller still
/// notifies as before.
pub(crate) fn auth_failure(app: &mut App, e: &str) {
    if !crate::github::is_auth_error(e) {
        return;
    }
    app.auth.degraded = true;
    if app.auth.dismissed || app.confirm_open() {
        return;
    }
    app.open_reauth(e.to_string(), vec![], false);
}

/// Run `gh auth refresh` with the required scopes on the real terminal:
/// the TUI suspends (raw mode off, alternate screen left), the command
/// inherits stdio so it can prompt, then the TUI resumes and the probe
/// re-runs. Drive mode has no terminal to suspend; it just names the
/// command instead.
pub(crate) fn run_reauth(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    use crossterm::ExecutableCommand;
    let cmd = crate::github::reauth_command();
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    println!("ghit needs these scopes: {}", crate::github::REQUIRED_SCOPES.join(", "));
    let status = std::process::Command::new("gh")
        .args(["auth", "refresh"])
        .args(crate::github::REQUIRED_SCOPES.iter().flat_map(|s| ["-s", s]))
        .status();
    match status {
        Ok(s) if s.success() => {
            crate::github::forget_token();
            app.auth.dismissed = false;
            app.auth.degraded = false;
            app.notify("re-auth ok: re-checking token…".to_string());
        }
        _ => app.notify(format!("re-auth failed — run by hand: {cmd}")),
    }
    let _ = enable_raw_mode();
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = terminal.clear();
    if matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::Reauth)) {
        app.take_confirm();
    }
    app.auth.reauth_requested = false;
    // Re-probe so the modal returns if the refresh did not cover it.
    let (atx, arx) = std::sync::mpsc::channel();
    app.auth.rx = Some(arx);
    spawn_bg(async move {
        let _ = atx.send(github::check_auth().await);
    });
}
