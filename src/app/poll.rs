//! Per-frame polls that land background fetches into `App`, plus startup bootstrap.

use std::collections::HashSet;
use std::time::Instant;

use crate::app::{App, Focus, rt, spawn_bg};
use crate::core::self;
use crate::{github, model};
use crate::app::auth_failure;
use crate::app::sidebar::{RECENT_REFRESH_MAX_INTERVAL, auto_refresh_interval_default};

/// Fetch missing board card details together without blocking the event loop.
pub(super) fn kick_off_details(app: &mut App) {
    let Some(board) = &app.board.data else { return };
    let mut cards = Vec::new();
    for item in &board.items {
        let (Some(n), Some(repo)) = (item.number, item.repo.clone()) else { continue };
        if app.board.details.contains_key(&n) || app.board.detail_pending.contains(&n) { continue; }
        if let Some(d) = app.issues.data.get(&n) {
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
    let ready: Vec<(u64, Result<model::BoardDetail, String>)> = app.board.detail_rx.try_iter().collect();
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
pub(crate) fn bootstrap(app: &mut App, numbers: &[u64]) {
    app.initial_numbers = numbers.to_vec();
    let repo = app.repo_flag.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.bootstrap_rx = Some(rx);
    // `git` and `gh` subprocesses block: keep them off the async workers.
    rt().spawn_blocking(move || {
        let mut worker = App::new(repo);
        worker.resolve_repo();
        let _ = crate::github::token();
        let _ = tx.send(worker.repo_slug);
    });
    // Early auth probe: one cheap round-trip validates the token so a
    // dead one prompts before the first real fetch fails into an empty
    // sidebar. Scopes need a repo and follow in `poll_bootstrap`.
    let (atx, arx) = std::sync::mpsc::channel();
    app.auth.rx = Some(arx);
    spawn_bg(async move {
        let _ = atx.send(github::check_auth().await);
    });
}

pub(super) fn poll_bootstrap(app: &mut App) -> bool {
    let Some(result) = app.auth.bootstrap_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    app.auth.bootstrap_rx = None;
    app.repo_slug = result;
    // Repo-scoped access probe, once: the boot probe only checks the
    // token itself, while scopes need a repo to read against.
    if !app.auth.repo_probed {
        app.auth.repo_probed = true;
        if let Some(repo) = app.repo_flag.clone().or_else(|| app.repo_slug.clone()) {
            let (tx, rx) = std::sync::mpsc::channel();
            app.auth.rx = Some(rx);
            spawn_bg(async move {
                let _ = tx.send(github::check_repo_access(&repo).await);
            });
        }
    }
    let numbers = std::mem::take(&mut app.initial_numbers);
    app.apply_prefs();
    if let Some(key) = app.repo_key() {
        let mut pins = github::load_pins(&key);
        pins.retain(|n| !numbers.contains(n));
        app.nav.pinned = pins.iter().copied().collect();
        app.nav.pin_order = pins.clone();
        // The requested issue goes first; pins wait behind it.
        let opened = !numbers.is_empty();
        for n in numbers {
            app.ensure_visited(n);
            app.load_issue(n, false);
        }
        for n in pins {
            if !opened {
                app.load_issue(n, false);
            } else if !app.issues.data.contains_key(&n) || app.issues.partial.contains(&n) {
                app.queue_issue(n, false);
            }
        }
        if app.sidebar.query_active.trim().is_empty() {
            app.hydrate_cached_sections();
            app.merge_recent_sections();
        } else {
            let q = app.sidebar.query_active.clone();
            app.run_gh_search(q);
        }
        app.ensure_repo_meta();
    } else {
        for n in numbers { app.load_issue(n, false); }
    }
    true
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

pub(crate) fn poll_sections(app: &mut App) -> bool {
    let Some(event) = app.sections.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    match event {
        github::SectionEvent::Progress { done, total } => {
            app.sections.progress = Some((done, total));
        }
        // The fetch that hit the cap still lands its rows through Done;
        // the notice only has to name the cap, once per fetch setting.
        github::SectionEvent::Capped => {
            if !app.sections.cap_notified {
                app.sections.cap_notified = true;
                app.notify(cap_notice());
            }
        }
        github::SectionEvent::Unchanged => {
            app.sections.rx = None;
            app.sections.progress = None;
            if std::mem::take(&mut app.sections.auto_refresh) {
                app.sections.auto_refresh_interval = auto_refresh_interval_default();
            }
        }
        github::SectionEvent::Done(result) => {
            app.sections.rx = None;
            app.sections.progress = None;
            app.sections.last_full = Instant::now();
            let auto = std::mem::take(&mut app.sections.auto_refresh);
            if auto {
                app.sections.auto_refresh_interval = if result.is_ok() {
                    auto_refresh_interval_default()
                } else {
                    (app.sections.auto_refresh_interval * 2).min(RECENT_REFRESH_MAX_INTERVAL)
                };
            }
            let replace = std::mem::take(&mut app.sections.replace);
            if replace {
                app.drop_unvisited_background();
            }
            match result {
                Ok(rows) => {
                    if !replace {
                        evict_vanished_rows(app, &rows);
                    }
                    let mut last = None;
                    for row in rows {
                        let n = row.issue.number;
                        if let Some(d) = app.issues.data.get_mut(&n) {
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
                                d.labels = row.issue.labels;
                                d.updated_at = row.issue.updated_at;
                                app.issues.statuses.insert(n, row.statuses);
                                app.board.details.insert(n, row.detail);
                            }
                        } else {
                            if !row.complete { app.issues.partial.insert(n); }
                            app.issues.data.insert(n, row.issue);
                            app.issues.statuses.insert(n, row.statuses);
                            app.board.details.insert(n, row.detail);
                            app.ensure_recent(n);
                            last = Some(n);
                        }
                    }
                    if !auto && app.nav.current.is_none() && app.nav.requested.is_none() {
                        if let Some(n) = last { app.load_issue(n, false); }
                    }
                },
                // Auto-refresh failures notify too: a silent empty
                // sidebar is how under-scoped tokens went unnoticed.
                // The backoff above still bounds the retry rate.
                Err(e) => { auth_failure(app, &e); app.notify(e); },
            }
        }
    }
    true
}

/// Window landing: evict listed background rows the fresh payload no
/// longer carries. Visited, pinned, current, and pending-open rows stay
/// so the open view and its thread survive a refresh; search mode never
/// lands here and keeps `drop_unvisited_background` replace semantics.
fn evict_vanished_rows(app: &mut App, rows: &[crate::github::Row]) {
    let fresh: HashSet<u64> = rows.iter().map(|r| r.issue.number).collect();
    let keep: HashSet<u64> = app.nav.visited.iter().copied()
        .chain(app.nav.pin_order.iter().copied())
        .chain(app.nav.pinned.iter().copied())
        .chain(app.nav.current)
        .chain(app.nav.requested)
        .collect();
    let gone: HashSet<u64> = app.nav.recent.iter().copied()
        .filter(|n| !fresh.contains(n) && !keep.contains(n))
        .collect();
    if gone.is_empty() { return; }
    app.nav.recent.retain(|n| !gone.contains(n));
    for n in gone { app.drop_cached(n); }
}

pub(crate) fn poll_issues(app: &mut App) -> bool {
    let ready: Vec<_> = app.issues.rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, event) in ready {
        match event {
            github::IssueEvent::Progress { done, total } => {
                app.issues.progress.insert(number, (done, total));
            }
            github::IssueEvent::Done(result) => {
                let result = result.map(|bundle| *bundle);
                app.issues.progress.remove(&number);
                app.issues.pending.remove(&number);
                // An h/l step owns this number until it lands: remember
                // before clearing so a 404 still toasts below.
                let stepped = app.nav.step_pending == Some(number);
                if stepped {
                    app.nav.step_pending = None;
                }
                match result {
                    Ok((data, statuses)) => {
                        // Completing the full fetch never files the row:
                        // viewing a recent row must not relocate it to
                        // requested. Explicit actions file through
                        // `ensure_visited` at their own site.
                        app.issues.partial.remove(&number);
                        // REST `issues/{n}` has no parent link: keep the
                        // sub-issue fields the sections row already carried.
                        let mut data = data;
                        if let Some(old) = app.issues.data.get(&number) {
                            data.parent = data.parent.or(old.parent);
                            data.sub_summary = data.sub_summary.or(old.sub_summary);
                        }
                        app.issues.data.insert(number, data);
                        app.issues.statuses.insert(number, statuses);
                        if app.nav.current == Some(number) {
                            app.refresh_refs();
                            if app.visual_graph() { app.load_graph(number, false); }
                            if app.visual_diff() { app.load_diff(number, false); }
                        }
                        if app.nav.requested == Some(number) {
                            app.show_issue(number);
                            app.bar.clear();
                            app.nav.focus = Focus::Main;
                            app.message.clear();
                        }
                    }
                    Err(e) => {
                        // A stepped-to number that truly does not exist
                        // toasts "no #N" instead of the raw fetch error.
                        // reqwest folds the status into the error text, so
                        // a 404 substring marks absence; any other error
                        // keeps the existing error path unchanged and never
                        // claims "no more".
                        if stepped && e.contains("404") {
                            app.notify(format!("no #{number}"));
                        } else {
                            auth_failure(app, &e);
                            app.notify(e);
                        }
                    }
                }
            }
        }
    }
    landed
}
