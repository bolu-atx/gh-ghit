//! Per-frame polls that land background fetches into `App`, plus startup bootstrap.


use crate::app::{App, Focus};
use ghit_core::github;
use crate::app::auth_failure;
use ghit_core::model::ItemRef;
use ghit_core::session::{Held, Landed, PollCtx, SessionEvent};

// ---------------------------------------------------------------------------
// TUI runner
// ---------------------------------------------------------------------------
/// Shared startup: file the requested numbers and resolve the repo
/// off the async workers. Both the terminal and drive loops call this.
pub(crate) fn bootstrap(app: &mut App, numbers: &[ItemRef]) {
    app.session.start(numbers);
}

/// The repo resolved: load prefs and pins, open the requested items and
/// start the first listing.
fn land_bootstrap(app: &mut App, numbers: Vec<ItemRef>) {
    app.apply_prefs();
    if let Some(key) = app.session.repo_key() {
        app.session.load_pins(&key, &numbers);
        let pins = app.session.pins().to_vec();
        // The requested issue goes first; pins wait behind it.
        let opened = !numbers.is_empty();
        for n in numbers {
            app.ensure_visited(n);
            app.load_issue(n, false);
        }
        for n in pins {
            if !opened {
                app.load_issue(n, false);
            } else if !app.session.has_item(n) || app.session.is_partial(n) {
                app.queue_issue(n, false);
            }
        }
        app.session.open_store(&key);
        if app.sidebar.query_active.trim().is_empty() {
            app.hydrate_cached_sections();
            // Another process holds the sync lease: its syncs land through
            // `watch_store`.
            if app.session.sync_holder() {
                app.merge_recent_sections();
            }
        } else {
            let q = app.sidebar.query_active.clone();
            app.run_gh_search(q);
        }
        app.ensure_repo_meta();
    } else {
        for n in numbers { app.load_issue(n, false); }
    }
}

/// Status-line notice for a fetch that stopped at the shared row cap.
pub(crate) fn cap_notice() -> String {
    format!(
        "{} row cap reached: narrow the query or window",
        github::MAX_SECTION_ROWS
    )
}

/// Pick up the Format dialog's label and milestone lists. True when a
/// result landed.
pub(crate) fn poll_repo_meta(app: &mut App) -> bool {
    let Some(result) = app.sidebar.meta_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
        return false;
    };
    app.land_repo_meta(result);
    true
}

/// Land what the session finished and turn each event into view changes.
/// True when anything landed.
/// Land every finished `Session` fetch at once.
#[cfg(test)]
pub(crate) fn poll_session(app: &mut App) -> bool {
    let ctx = app.poll_ctx(std::time::Instant::now());
    let events = app.session.poll_frame(&ctx);
    apply_session(app, events)
}

/// Apply `Session` events to view state. Returns whether any landed.
pub(crate) fn apply_session(app: &mut App, events: Vec<SessionEvent>) -> bool {
    let landed = !events.is_empty();
    for event in events {
        apply_event(app, event);
    }
    landed
}

fn apply_event(app: &mut App, event: SessionEvent) {
    match event {
        SessionEvent::Bootstrapped { numbers } => land_bootstrap(app, numbers),
        SessionEvent::AuthChecked(result) => match result {
            Ok(missing) if missing.is_empty() => {}
            Ok(missing) => app.open_reauth("token lacks scopes ghit needs".to_string(), missing, true),
            // Only auth-class failures rise to the modal: re-auth can fix
            // those, while a rate limit or outage just needs a status line.
            Err(e) if github::is_auth_error(&e) => app.open_reauth(e, vec![], false),
            Err(e) => app.notify(e),
        },
        SessionEvent::RowsLanded { landed, auto } => app.apply_landed(landed, auto),
        SessionEvent::StoreChanged => { app.requery_after_store_change(); }
        SessionEvent::SectionsStarted | SessionEvent::BackfillLanded => {}
        SessionEvent::SectionsProgress | SessionEvent::SectionsUnchanged | SessionEvent::IssueProgress(_) => {}
        SessionEvent::SectionsCapped => app.notify(cap_notice()),
        SessionEvent::SectionsDone { result, replace, auto } => {
            if replace {
                app.drop_unvisited_background();
            }
            match result {
                Ok(rows) if replace => app.land_github(rows),
                Ok(rows) => app.land_rows(rows, replace, auto),
                // Auto-refresh failures notify too: a silent empty
                // sidebar is how under-scoped tokens went unnoticed.
                // The backoff in the session still bounds the retry rate.
                Err(e) => { auth_failure(app, &e); app.notify(e); },
            }
        }
        SessionEvent::SyncFailed(e) => auth_failure(app, &e),
        SessionEvent::IssueLoaded(number) => {
            // An h/l step owns this number until it lands.
            if app.nav.step_pending == Some(number) {
                app.nav.step_pending = None;
            }
            if app.nav.current == Some(number) {
                app.refresh_refs();
                if app.visual_diff() { app.load_diff(number, false); }
            }
            if app.nav.requested == Some(number) {
                app.show_issue(number);
                app.bar.clear();
                app.nav.focus = Focus::Main;
                app.message.clear();
            }
        }
        SessionEvent::IssueFailed { number, error } => {
            // A stepped-to number that truly does not exist toasts
            // "no #N" instead of the raw fetch error. reqwest folds the
            // status into the error text, so a 404 substring marks
            // absence; any other error keeps the existing error path
            // and never claims "no more".
            let stepped = app.nav.step_pending == Some(number);
            if stepped {
                app.nav.step_pending = None;
            }
            if stepped && error.contains("404") {
                app.notify(format!("no {number}", number = number.display("")));
            } else {
                auth_failure(app, &error);
                app.notify(error);
            }
        }
        SessionEvent::BoardLoaded => {
            app.board.col = 0;
            app.board.row = 0;
            app.board.states.clear();
            app.board.expanded = None;
            app.prune_board_milestones();
            let view = app.fetch_view();
            app.session.load_board_details(&view);
        }
        SessionEvent::BoardFailed(e) => {
            // A failed load closes the board, as it did when "open"
            // meant "has data or is loading".
            app.close_board();
            auth_failure(app, &e);
            app.notify(e);
        }
        SessionEvent::BoardDetail(_) | SessionEvent::DiffLoaded(_) => {}
        SessionEvent::BoardDetailFailed(e) | SessionEvent::DiffFailed(e) => { auth_failure(app, &e); app.notify(e); }
        SessionEvent::HeadFilesLanded(n) => app.rewiden(n),
        SessionEvent::HeadFilesFailed { number, error } => {
            auth_failure(app, &error);
            app.notify(error);
            app.rewiden(number);
        }
        SessionEvent::DraftSent { number, result } => super::compose::land_draft_sent(app, number, result),
        SessionEvent::ReviewSubmitted { number, result } => match result {
            Ok(()) => {
                app.notify(format!("pending review submitted on {n}", n = number.display("")));
                app.load_issue(number, true);
            }
            Err(e) => {
                auth_failure(app, &e);
                app.notify(e);
            }
        },
        SessionEvent::StateChanged { number, open, result } => super::confirm::land_state_changed(app, number, open, result),
        SessionEvent::SubIssuesLoaded(_) => {}
        SessionEvent::SubIssuesFailed(e) => { auth_failure(app, &e); app.notify(e); }
    }
}

impl App {
    /// What the listing rules read from the view: the rows the sidebar
    /// holds and the rows no listing may evict.
    pub(crate) fn held_rows(&self) -> Held {
        let keep = self
            .nav
            .visited
            .iter()
            .copied()
            .chain(self.session.pins().iter().copied())
            .chain(self.session.pinned().iter().copied())
            .chain(self.nav.current)
            .chain(self.nav.requested)
            .collect();
        Held { recent: self.nav.recent.clone(), keep }
    }

    /// Everything the session's poll reads from the view.
    pub(crate) fn poll_ctx(&self, now: std::time::Instant) -> PollCtx {
        PollCtx {
            now,
            searching: !self.sidebar.query_active.trim().is_empty(),
            show: self.show_mask(),
            idle: self.focus().layer == ghit_core::core::Layer::Base,
            view: self.fetch_view(),
            held: self.held_rows(),
        }
    }

    /// Land listing rows in the session, then file and drop the rows it
    /// reports. A user fetch (`auto` false) with nothing open opens the
    /// last new row.
    pub(crate) fn land_rows(&mut self, rows: Vec<github::Row>, replace: bool, auto: bool) {
        let held = self.held_rows();
        let landed = self.session.land_rows(rows, replace, &held);
        self.apply_landed(landed, auto);
    }

    fn apply_landed(&mut self, landed: Landed, auto: bool) {
        self.nav.recent.retain(|n| !landed.gone.contains(n));
        for n in &landed.filed {
            self.ensure_recent(*n);
        }
        if !auto && self.nav.current.is_none() && self.nav.requested.is_none() {
            if let Some(n) = landed.filed.last().copied() { self.load_issue(n, false); }
        }
    }
}
