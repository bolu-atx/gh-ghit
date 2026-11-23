//! The sidebar listing: refresh cadence, store watch, local queries and row landing.

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;

use std::collections::HashSet;
use std::time::Instant;

use super::{Session, SessionEvent};
use super::sections::refresh_plan;
use crate::backend::IssueBackend;
use crate::core::{self, Kind};
use crate::fetch_queue::View;
use crate::github;
use crate::model::{ItemRef, Source};

/// What the front end shows that the listing rules depend on.
pub struct Held {
    /// Background rows the sidebar lists, in fetch order.
    pub recent: Vec<ItemRef>,
    /// Rows a listing never evicts: visited, pinned, open, pending-open.
    pub keep: HashSet<ItemRef>,
}

/// What landing listing rows changed for the front end.
#[derive(Debug, Default)]
pub struct Landed {
    /// Rows now cached that were new: file them under recent.
    pub filed: Vec<ItemRef>,
    /// Background rows the listing dropped, with their cached payloads.
    pub gone: Vec<ItemRef>,
}

/// What a poll needs to know about the front end.
pub struct PollCtx {
    pub now: Instant,
    /// A `/` query is active: the listing re-reads by query, not by window.
    pub searching: bool,
    /// Visible lists in section order: open PRs, merged PRs, open issues, closed issues.
    pub show: [bool; 4],
    /// No dialog is up: auto-refresh may start.
    pub idle: bool,
    pub view: View,
    pub held: Held,
}

impl Session {
    /// True when this process syncs the repo's listing: it holds the
    /// store's sync lease, or has no store link and syncs on its own.
    pub fn sync_holder(&self) -> bool {
        self.sections.link.as_ref().is_none_or(github::StoreLink::holding)
    }

    /// Pull recent issues plus open/windowed-merged PRs into the store.
    /// Offline or out of a repo this is a no-op.
    pub fn refresh_recent(&mut self, show: [bool; 4], view: &View) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        self.sections.last_fetch = Instant::now();
        self.sections.auto_refresh = false;
        self.sections.replace = false;
        let hours = self.sections.merged_window_hours;
        let probe = std::mem::take(&mut self.sections.probe);
        let refresh = std::mem::take(&mut self.sections.refresh);
        let now = Instant::now();
        if refresh != github::Refresh::Delta { self.sections.last_status = now; }
        if refresh == github::Refresh::Full { self.sections.last_full = now; }
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        let backend = self.issue_backend.clone();
        self.enqueue(Kind::Sections, None, async move {
            if probe && backend.shares_github_sections()
                && !probe_says_changed(&repo, &tx).await { return; }
            let result = fetch_sections(backend, &repo, hours, show, refresh, tx.clone()).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        }, view);
    }

    /// Start the GitHub search fetch for `query`. False without a repo.
    pub fn search(&mut self, query: &str, view: &View) -> bool {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return false };
        self.sections.last_fetch = Instant::now();
        self.sections.auto_refresh = false;
        self.sections.replace = true;
        let search = core::search_query(query, self.sections.merged_window_hours, chrono::Utc::now());
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        self.enqueue(Kind::Sections, None, async move {
            let result = github::search_repo(&repo, &search, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        }, view);
        true
    }

    /// Query mode: the lease holder still runs the listing sync on the
    /// normal cadence, because every ghit instance of the repo reads the
    /// store it writes. The sync lands no rows here.
    fn sync_behind_query(&mut self, ctx: &PollCtx) {
        let now = ctx.now;
        if now.duration_since(self.sections.last_sync) < self.sections.auto_refresh_interval
            || self.sections.sync_rx.is_some()
            || !self.sync_holder()
            || !ctx.idle
            || github::rate_limited()
        {
            return;
        }
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        self.sections.last_sync = now;
        let refresh = refresh_plan(
            now.duration_since(self.sections.last_status),
            now.duration_since(self.sections.last_full),
        );
        if refresh != github::Refresh::Delta { self.sections.last_status = now; }
        if refresh == github::Refresh::Full { self.sections.last_full = now; }
        let probe = refresh == github::Refresh::Delta;
        let (hours, show) = (self.sections.merged_window_hours, ctx.show);
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.sync_rx = Some(rx);
        let backend = self.issue_backend.clone();
        self.enqueue(Kind::Sections, None, async move {
            if probe && backend.shares_github_sections()
                && !probe_says_changed(&repo, &tx).await { return; }
            let result = fetch_sections(backend, &repo, hours, show, refresh, tx.clone()).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        }, &ctx.view);
    }

    /// Per tick: refresh the listing on its cadence. Returns true when
    /// a listing fetch started.
    pub fn auto_refresh_recent(&mut self, ctx: &PollCtx) -> bool {
        let had_fetch = self.sections.rx.is_some();
        self.auto_refresh_inner(ctx);
        had_fetch != self.sections.rx.is_some()
    }

    fn auto_refresh_inner(&mut self, ctx: &PollCtx) {
        let now = ctx.now;
        if ctx.searching {
            self.sync_behind_query(ctx);
        }
        if now.duration_since(self.sections.last_fetch) < self.sections.auto_refresh_interval
            || self.sections.rx.is_some()
            || !ctx.idle
            || github::rate_limited()
        {
            return;
        }
        if ctx.searching {
            // A query never refreshes on GitHub by itself: it re-filters
            // when the store changes.
            self.sections.last_fetch = now;
            return;
        }
        // Another process holds the sync lease: its writes land
        // through the store watch, so this one sends no listing request.
        if !self.sync_holder() {
            return;
        }
        self.sections.refresh = refresh_plan(
            now.duration_since(self.sections.last_status),
            now.duration_since(self.sections.last_full),
        );
        self.sections.probe = self.sections.refresh == github::Refresh::Delta;
        self.refresh_recent(ctx.show, &ctx.view);
        if self.sections.rx.is_some() {
            self.sections.auto_refresh = true;
        }
    }

    /// Per tick: renew or try the sync lease. True when another process
    /// (or this one's own fetch) wrote a new sync that this instance has
    /// not read yet. An in-flight fetch skips the re-read; it lands the
    /// same rows itself.
    pub fn store_changed(&mut self, now: Instant) -> bool {
        let Some(link) = self.sections.link.as_mut() else { return false };
        link.tick(now);
        !(self.sections.rx.is_some() || !link.changed())
    }

    /// This instance's view of the stored listing: its window and list
    /// toggles filter the rows locally, with no request.
    pub fn window_rows(&self, show: [bool; 4]) -> Option<Vec<github::Row>> {
        let hours = self.sections.merged_window_hours;
        let rows = self.sections.link.as_ref().and_then(|link| link.rows(hours, show))?;
        Some(self.own_rows(rows))
    }

    /// `rows` without the GitHub issues a backend with its own issue
    /// sections never lists. The shared store may hold them.
    fn own_rows(&self, mut rows: Vec<github::Row>) -> Vec<github::Row> {
        if !self.issue_backend.shares_github_sections() {
            rows.retain(|row| row.issue.number.source != Source::GitHub || row.issue.is_pr);
        }
        rows
    }

    /// Stored rows for the numbers of a GitHub result set.
    pub fn stored_rows(&self, numbers: &[ItemRef]) -> Option<Vec<github::Row>> {
        self.sections.link.as_ref().map(|link| link.rows_by_number(numbers))
    }

    /// Store GitHub search hits (newer-only, statuses kept) and return
    /// the stored rows.
    pub fn save_search(&mut self, hits: Vec<github::Row>) -> Vec<github::Row> {
        match self.sections.link.as_mut() {
            Some(link) => link.save_search(hits),
            None => hits,
        }
    }

    /// The parsed query when the store alone answers it: every term is
    /// local, and the store holds every row the query could match (the
    /// window token counts as a term). `None` sends it to the server.
    fn local_query(&self, query: &str) -> Option<core::LocalQuery> {
        let link = self.sections.link.as_ref()?;
        let now = chrono::Utc::now();
        let search = core::search_query(query, self.sections.merged_window_hours, now);
        let parsed = core::LocalQuery::parse(&search)?;
        let (open_done, closed_done) = link.backfill_done();
        parsed.covered(now, self.sections.backfill.months, open_done, closed_done).then_some(parsed)
    }

    /// Stored rows a local query matches, most recently updated first.
    /// `None` when the query needs the server.
    pub fn local_query_rows(&self, query: &str) -> Option<Vec<github::Row>> {
        let parsed = self.local_query(query)?;
        let mut rows: Vec<github::Row> = self.own_rows(self.sections.link.as_ref()?.all_rows()?)
            .into_iter()
            .filter(|row| parsed.matches(&row.issue))
            .collect();
        rows.sort_by(|a, b| {
            (&b.issue.updated_at, b.issue.number).cmp(&(&a.issue.updated_at, a.issue.number))
        });
        Some(rows)
    }

    /// Open the repo's store link: the sync lease and change watch.
    pub fn open_store(&mut self, key: &str) {
        self.sections.link = github::StoreLink::open(key);
    }

    /// Take the sync lease from the holder, so a manual refresh works
    /// from any process.
    pub fn take_over_lease(&mut self) {
        if let Some(link) = self.sections.link.as_mut() {
            link.take_over();
        }
    }

    /// Map items and links when the store changed since `seen`.
    pub fn map_data(&self, seen: Option<i64>) -> Option<(i64, Vec<crate::model::MapItem>, Vec<crate::model::MapLink>)> {
        self.sections.link.as_ref()?.map_data(seen)
    }

    /// Stamp of the stored row a map card shows for `number`.
    pub fn card_stamp(&self, number: ItemRef) -> Option<String> {
        self.sections.link.as_ref().and_then(|link| link.card_stamp(number))
    }

    /// The stored row and body for a map card, at any age.
    pub fn item_card(&self, number: ItemRef) -> Option<(github::Row, Option<String>)> {
        self.sections.link.as_ref().and_then(|link| link.item_card(number))
    }

    /// Startup hydration: the last persisted sidebar, so a cold start
    /// shows previous state. Returns the rows it filed. Startup-only:
    /// refreshes must never resurrect evicted rows.
    pub fn hydrate_cached(&mut self, show: [bool; 4]) -> Vec<ItemRef> {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return vec![] };
        let Some(rows) = github::cached_sections(&repo, self.sections.merged_window_hours, show) else { return vec![] };
        let rows = self.own_rows(rows);
        let mut filed = Vec::new();
        for row in rows {
            let n = row.issue.number;
            if self.issues.data.contains_key(&n) {
                continue;
            }
            if !row.complete {
                self.issues.partial.insert(n);
            }
            self.issues.data.insert(n, row.issue);
            self.issues.statuses.insert(n, row.statuses);
            self.board.details.insert(n, row.detail);
            filed.push(n);
        }
        filed
    }

    /// Land listing rows: a window listing (`replace` false) first evicts
    /// the background rows it no longer carries, then known rows refresh
    /// display fields and new rows are cached for filing under recent.
    pub fn land_rows(&mut self, rows: Vec<github::Row>, replace: bool, held: &Held) -> Landed {
        let mut landed = Landed::default();
        if !replace {
            landed.gone = self.evict_vanished(&rows, held);
        }
        for row in rows {
            let n = row.issue.number;
            if let Some(d) = self.issues.data.get_mut(&n) {
                // Window listings refresh display fields only.
                // Body, comments, and review threads belong to
                // the full fetch and stay. Search hits carry no
                // statuses, so replace mode never touches known
                // rows; dropped unvisited rows reinsert below.
                // A listing older than what we hold (search index
                // lag, a cached page) never overwrites it: a
                // just-closed item must not flip back to open.
                if !replace && row.issue.updated_at >= d.updated_at {
                    d.title = row.issue.title;
                    d.state = row.issue.state;
                    d.checks = row.issue.checks;
                    d.labels = row.issue.labels;
                    d.updated_at = row.issue.updated_at;
                    self.issues.statuses.insert(n, row.statuses);
                    self.board.details.insert(n, row.detail);
                }
            } else {
                if !row.complete { self.issues.partial.insert(n); }
                self.issues.data.insert(n, row.issue);
                self.issues.statuses.insert(n, row.statuses);
                self.board.details.insert(n, row.detail);
                landed.filed.push(n);
            }
        }
        landed
    }

    /// Window landing: evict listed background rows the fresh payload no
    /// longer carries. Rows in `held.keep` stay so the open view and its
    /// thread survive a refresh.
    fn evict_vanished(&mut self, rows: &[github::Row], held: &Held) -> Vec<ItemRef> {
        let fresh: HashSet<ItemRef> = rows.iter().map(|r| r.issue.number).collect();
        let gone: Vec<ItemRef> = held
            .recent
            .iter()
            .copied()
            .filter(|n| !fresh.contains(n) && !held.keep.contains(n))
            .collect();
        for n in &gone {
            self.drop_cached(*n);
        }
        gone
    }

    /// The whole per-frame landing, in the order that keeps data right:
    /// feed results and listing rows first, then full item fetches, so a
    /// fetched item always wins over a stale listing row.
    pub fn poll_frame(&mut self, ctx: &PollCtx) -> Vec<SessionEvent> {
        let mut held = Held { recent: ctx.held.recent.clone(), keep: ctx.held.keep.clone() };
        let mut events = Vec::new();
        // Replace landings and bootstrap write rows from the front end,
        // after this returns. Full item fetches then wait one frame so
        // they still land after those rows and win over them.
        let mut front_end_rows = false;
        for event in self.poll_feed() {
            front_end_rows |= matches!(
                event,
                SessionEvent::SectionsDone { replace: true, .. } | SessionEvent::Bootstrapped { .. }
            );
            events.push(match event {
                SessionEvent::SectionsDone { result: Ok(rows), replace: false, auto } => {
                    let landed = self.land_rows(rows, false, &held);
                    held.apply(&landed);
                    SessionEvent::RowsLanded { landed, auto }
                }
                other => other,
            });
        }
        if self.store_changed(ctx.now) {
            if ctx.searching {
                events.push(SessionEvent::StoreChanged);
            } else if let Some(rows) = self.window_rows(ctx.show) {
                let landed = self.land_rows(rows, false, &held);
                held.apply(&landed);
                events.push(SessionEvent::RowsLanded { landed, auto: true });
            }
        }
        if self.auto_refresh_recent(ctx) {
            events.push(SessionEvent::SectionsStarted);
        }
        if self.drive_backfill(ctx.now, &ctx.view) {
            events.push(SessionEvent::BackfillLanded);
        }
        if !front_end_rows {
            events.extend(self.poll_items());
        }
        events
    }
}

impl Held {
    /// Fold a landing into the held rows, so a second landing in the same
    /// frame sees them.
    pub fn apply(&mut self, landed: &Landed) {
        self.recent.retain(|n| !landed.gone.contains(n));
        for n in &landed.filed {
            if !self.recent.contains(n) {
                self.recent.push(*n);
            }
        }
    }
}

/// Run the change probe inside a section task. False means the task is
/// over: `Unchanged` (nothing new) or `Done(Err)` (probe failed, so the
/// usual backoff applies) is already sent.
pub async fn probe_says_changed(repo: &str, tx: &std::sync::mpsc::Sender<github::SectionEvent>) -> bool {
    match github::repo_changed(repo).await {
        Ok(true) => true,
        Ok(false) => { let _ = tx.send(github::SectionEvent::Unchanged); false }
        Err(e) => { let _ = tx.send(github::SectionEvent::Done(Err(e))); false }
    }
}

/// Shared backends preserve the single GitHub fetch, including its progress events.
async fn fetch_sections(
    backend: IssueBackend,
    repo: &str,
    hours: u64,
    show: [bool; 4],
    refresh: github::Refresh,
    progress: std::sync::mpsc::Sender<github::SectionEvent>,
) -> Result<Vec<github::Row>, String> {
    if backend.shares_github_sections() {
        return github::fetch_recent_sections(repo, hours, show, refresh, Some(progress)).await;
    }
    let (prs, issues) = split_section_flags(show);
    let prs = github::fetch_recent_sections(repo, hours, prs, refresh, Some(progress.clone())).await?;
    let issues = backend.fetch_sections(repo, hours, issues, refresh, Some(progress)).await?;
    Ok(merge_section_rows(prs, issues))
}

fn split_section_flags(show: [bool; 4]) -> ([bool; 4], [bool; 2]) {
    ([show[0], show[1], false, false], [show[2], show[3]])
}

fn merge_section_rows(
    mut prs: Vec<github::Row>, issues: Vec<github::Row>,
) -> Vec<github::Row> {
    prs.extend(issues);
    prs
}
