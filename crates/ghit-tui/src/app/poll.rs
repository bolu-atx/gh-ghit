//! Per-frame polls that land background fetches into `App`, plus startup bootstrap.

use std::collections::HashSet;

use crate::app::{App, Focus};
use ghit_core::core::self;
use ghit_core::{github, model};
use crate::app::auth_failure;
use ghit_core::model::ItemRef;
use ghit_core::session::SessionEvent;

/// Fetch missing board card details together without blocking the event loop.
pub(super) fn kick_off_details(app: &mut App) {
    let Some(board) = &app.board.data else { return };
    let mut cards = Vec::new();
    for item in &board.items {
        let (Some(n), Some(repo)) = (item.number, item.repo.clone()) else { continue };
        if app.board.details.contains_key(&n) || app.board.detail_pending.contains(&n) { continue; }
        if let Some(d) = app.session.issues.data.get(&n) {
            app.board.details.insert(n, model::BoardDetail::from_issue(d));
            continue;
        }
        app.board.detail_pending.insert(n);
        cards.push((repo, n));
    }
    if cards.is_empty() { return; }
    let tx = app.board.detail_tx.clone();
    app.enqueue(core::Kind::Other, None, async move {
        match github::fetch_board_details(&cards).await {
            Ok(details) => {
                let mut missing = Vec::new();
                for ((repo, n), detail) in cards.into_iter().zip(details) {
                    match detail {
                        Some(detail) => { let _ = tx.send((n, Ok(detail))); }
                        None => missing.push((repo, n)),
                    }
                }
                for (repo, n) in missing {
                    let _ = tx.send((n, github::fetch_board_detail(&repo, n).await));
                }
            }
            Err(e) => for (_, n) in cards { let _ = tx.send((n, Err(e.clone()))); },
        }
    });
}

pub(crate) fn poll_details(app: &mut App) -> bool {
    let ready: Vec<(ItemRef, Result<model::BoardDetail, String>)> =
        app.board.detail_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        app.board.detail_pending.remove(&n);
        match res {
            Ok(d) => { app.board.details.insert(n, d); }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

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
        let pins = app.session.pins.order.clone();
        // The requested issue goes first; pins wait behind it.
        let opened = !numbers.is_empty();
        for n in numbers {
            app.ensure_visited(n);
            app.load_issue(n, false);
        }
        for n in pins {
            if !opened {
                app.load_issue(n, false);
            } else if !app.session.issues.data.contains_key(&n) || app.session.issues.partial.contains(&n) {
                app.queue_issue(n, false);
            }
        }
        app.session.sections.link = github::StoreLink::open(&key);
        if app.sidebar.query_active.trim().is_empty() {
            app.hydrate_cached_sections();
            // Another process holds the sync lease: its syncs land through
            // `watch_store`.
            if app.sync_holder() {
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

/// Run the change probe inside a section task. False means the task is
/// over: `Unchanged` (nothing new) or `Done(Err)` (probe failed, so the
/// usual backoff applies) is already sent.
pub(super) async fn probe_says_changed(repo: &str, tx: &std::sync::mpsc::Sender<github::SectionEvent>) -> bool {
    match github::repo_changed(repo).await {
        Ok(true) => true,
        Ok(false) => { let _ = tx.send(github::SectionEvent::Unchanged); false }
        Err(e) => { let _ = tx.send(github::SectionEvent::Done(Err(e))); false }
    }
}

/// Land what the session finished and turn each event into view changes.
/// True when anything landed.
/// Land every finished `Session` fetch at once.
#[cfg(test)]
pub(crate) fn poll_session(app: &mut App) -> bool {
    let events = app.session.poll();
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
        SessionEvent::SectionsProgress | SessionEvent::SectionsUnchanged | SessionEvent::IssueProgress(_) => {}
        SessionEvent::SectionsCapped => app.notify(cap_notice()),
        SessionEvent::SectionsDone { result, replace, auto } => {
            if replace {
                app.drop_unvisited_background();
            }
            match result {
                Ok(rows) if replace => app.land_github(rows),
                Ok(rows) => land_rows(app, rows, replace, auto),
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
        SessionEvent::SubIssuesLoaded(_) => {}
        SessionEvent::SubIssuesFailed(e) => { auth_failure(app, &e); app.notify(e); }
    }
}

/// Land listing rows: a window listing (`replace` false) first evicts the
/// background rows it no longer carries, then known rows refresh display
/// fields and new rows file under recent. A user fetch (`auto` false) with
/// nothing open opens the last new row.
pub(super) fn land_rows(app: &mut App, rows: Vec<ghit_core::github::Row>, replace: bool, auto: bool) {
    if !replace {
        evict_vanished_rows(app, &rows);
    }
    let mut last = None;
    for row in rows {
        let n = row.issue.number;
        if let Some(d) = app.session.issues.data.get_mut(&n) {
            // Window listings refresh display fields only.
            // Body, comments, and review threads belong to
            // the full fetch and stay. Search hits carry no
            // statuses, so replace mode never touches known
            // rows — dropped unvisited rows reinsert below.
            // A listing older than what we hold (search index
            // lag, a cached page) never overwrites it: a
            // just-closed item must not flip back to open.
            if !replace && row.issue.updated_at >= d.updated_at {
                d.title = row.issue.title;
                d.state = row.issue.state;
                d.checks = row.issue.checks;
                d.labels = row.issue.labels;
                d.updated_at = row.issue.updated_at;
                app.session.issues.statuses.insert(n, row.statuses);
                app.board.details.insert(n, row.detail);
            }
        } else {
            if !row.complete { app.session.issues.partial.insert(n); }
            app.session.issues.data.insert(n, row.issue);
            app.session.issues.statuses.insert(n, row.statuses);
            app.board.details.insert(n, row.detail);
            app.ensure_recent(n);
            last = Some(n);
        }
    }
    if !auto && app.nav.current.is_none() && app.nav.requested.is_none() {
        if let Some(n) = last { app.load_issue(n, false); }
                    }
}

/// Window landing: evict listed background rows the fresh payload no
/// longer carries. Visited, pinned, current, and pending-open rows stay
/// so the open view and its thread survive a refresh; search mode never
/// lands here and keeps `drop_unvisited_background` replace semantics.
fn evict_vanished_rows(app: &mut App, rows: &[ghit_core::github::Row]) {
    let fresh: HashSet<ItemRef> = rows.iter().map(|r| r.issue.number).collect();
    let keep: HashSet<ItemRef> = app
        .nav
        .visited
        .iter()
        .copied()
        .chain(app.session.pins.order.iter().copied())
        .chain(app.session.pins.set.iter().copied())
        .chain(app.nav.current)
        .chain(app.nav.requested)
        .collect();
    let gone: HashSet<ItemRef> = app
        .nav
        .recent
        .iter()
        .copied()
        .filter(|n| !fresh.contains(n) && !keep.contains(n))
        .collect();
    if gone.is_empty() { return; }
    app.nav.recent.retain(|n| !gone.contains(n));
    for n in gone { app.drop_cached(n); }
}
