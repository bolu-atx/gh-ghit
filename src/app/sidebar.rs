//! Sidebar rows: sections fetch and merge, search, filters, sort, saved queries and auto-refresh.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ratatui::widgets::ListState;

use crate::app::{App, Focus, UndoAction};
use crate::core::{self, Layer};
use crate::{github, model};
use crate::app::poll::probe_says_changed;

/// What a filter-dialog query commit asks the caller to fetch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum QueryAction {
    Run(String),
    Refetch,
}

/// Background section feed: the recent/search listing channel plus its
/// refresh cadence, auto-refresh state, page progress, replace-vs-append
/// mode, and the merged-PR window.
pub(crate) struct SectionFeed {
    pub(crate) rx: Option<std::sync::mpsc::Receiver<github::SectionEvent>>,
    pub(crate) last_fetch: Instant,
    pub(crate) auto_refresh_interval: Duration,
    pub(crate) auto_refresh: bool,
    /// The next spawned fetch runs the change probe first.
    pub(crate) probe: bool,
    /// How the next spawned fetch runs: change-only, with a status
    /// refresh, or a full walk of every list.
    pub(crate) refresh: github::Refresh,
    /// When the last status refresh or full walk started.
    pub(crate) last_status: Instant,
    /// When the last full walk started.
    pub(crate) last_full: Instant,
    /// The lease holder's listing sync while a query shows: it writes the
    /// store and lands nothing, so the query result stays. Its own clock
    /// paces it, apart from the query's refresh.
    pub(crate) sync_rx: Option<std::sync::mpsc::Receiver<github::SectionEvent>>,
    pub(crate) last_sync: Instant,
    /// Section page progress: rows fetched vs the row cap, while pages
    /// remain. Cleared when the terminal result lands.
    pub(crate) progress: Option<(usize, usize)>,
    /// The next section payload replaces (search) instead of appending.
    pub(crate) replace: bool,
    pub(crate) merged_window_hours: u64,
    /// A cap notice already showed for this fetch setting, so the periodic
    /// auto-refresh never repeats it.
    pub(crate) cap_notified: bool,
    /// The repo store's sync lease and change watch, opened at startup.
    /// `None` (no repo, or the store cannot open) syncs on its own.
    pub(crate) link: Option<github::StoreLink>,
    /// Store backfill pacing and progress.
    pub(crate) backfill: crate::app::backfill::BackfillFeed,
}

impl SectionFeed {
    pub(crate) fn new() -> Self {
        Self { rx: None, last_fetch: Instant::now(),
            auto_refresh_interval: auto_refresh_interval_default(),
            auto_refresh: false, probe: false, refresh: github::Refresh::Delta,
            last_status: Instant::now(), last_full: Instant::now(),
            sync_rx: None, last_sync: Instant::now(),
            progress: None, replace: false, merged_window_hours: 24,
            cap_notified: false, link: None,
            backfill: crate::app::backfill::BackfillFeed::new() }
    }
}

/// Format dialog picker lists: repository labels and milestones.
pub(crate) type RepoMeta = (Vec<String>, Vec<model::Milestone>);

/// Sidebar state: `/` search text and editing flag, client-side sort,
/// the `f` format dialog (open toggles, query draft/active, label and
/// milestone pickers, group-by, row fields), and the two pane list states.
pub(crate) struct Sidebar {
    pub(crate) show: bool,
    pub(crate) search: String,
    pub(crate) searching: bool,
    pub(crate) sort: core::SideSort,
    pub(crate) sort_desc: bool,
    pub(crate) show_open_prs: bool,
    pub(crate) show_merged_prs: bool,
    pub(crate) show_open_issues: bool,
    pub(crate) show_closed_issues: bool,
    /// `d` on the Filters pane: hide closed or fully done milestones.
    pub(crate) hide_done_milestones: bool,
    pub(crate) query_draft: String,
    pub(crate) query_active: String,
    /// Numbers a GitHub-run query returned. `Some` means the view shows
    /// GitHub results, re-read from the store on change; `None` means the
    /// store answered locally.
    pub(crate) query_github: Option<Vec<u64>>,
    pub(crate) saved_queries: Vec<github::SavedQuery>,
    pub(crate) name_draft: String,
    pub(crate) group_by: model::GroupBy,
    pub(crate) sub_issue_tree: bool,
    pub(crate) folded_issues: HashSet<u64>,
    /// Selected label pills. Empty means no label filter. Pins and
    /// requested rows skip it.
    pub(crate) label_filter: HashSet<String>,
    /// Selected milestone pills; `None` is the "(no milestone)" pill.
    /// Empty means no milestone filter.
    pub(crate) milestone_filter: HashSet<Option<String>>,
    /// Repository labels for the Format dialog picker.
    pub(crate) labels: Vec<String>,
    /// Repository milestones, in REST order, with due dates for grouping.
    pub(crate) milestones: Vec<model::Milestone>,
    /// Label/milestone fetch for the dialog; lands in `poll_repo_meta`.
    pub(crate) meta_rx: Option<std::sync::mpsc::Receiver<Result<RepoMeta, String>>>,
    /// Repo whose pickers are loaded or in flight, so reopening the
    /// dialog does not refetch.
    pub(crate) meta_repo: Option<String>,
    /// True once a picker fetch landed, so an empty list is trusted.
    pub(crate) meta_loaded: bool,
    /// Row fields per pane, toggled in the dialog Row fields section.
    pub(crate) row_issue: model::RowFields,
    pub(crate) row_pr: model::RowFields,
    /// Active widget in the dialog, in `format_widgets` order.
    pub(crate) format_focus: usize,
    pub(crate) format_columns: bool,
    pub(crate) format_rows: Vec<usize>,
    pub(crate) issue_state: ListState,
    pub(crate) pr_state: ListState,
}

impl Sidebar {
    pub(crate) fn new() -> Self {
        Self { show: true, search: String::new(), searching: false, sort: core::SideSort::Default,
            sort_desc: false, show_open_prs: true, show_merged_prs: true,
            show_open_issues: true, show_closed_issues: true, hide_done_milestones: false,
            query_draft: String::new(), query_active: String::new(), query_github: None,
            saved_queries: Vec::new(), name_draft: String::new(),
            group_by: model::GroupBy::None, sub_issue_tree: false,
            folded_issues: HashSet::new(), label_filter: HashSet::new(),
            milestone_filter: HashSet::new(),
            labels: vec![], milestones: vec![],
            meta_rx: None, meta_repo: None, meta_loaded: false,
            row_issue: model::RowFields::default(), row_pr: model::RowFields::default(),
            format_focus: 0, format_columns: false, format_rows: vec![],
            issue_state: ListState::default(), pr_state: ListState::default() }
    }
}

/// One selectable row of the Labels pane: a milestone pill (`None` is
/// "(no milestone)") or a label name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilterItem {
    Milestone(Option<String>),
    Label(String),
}

/// Labels pane view state: the cursor row and the first drawn line.
#[derive(Default)]
pub(crate) struct LabelsPane {
    pub(crate) cursor: usize,
    pub(crate) offset: usize,
}

pub(super) const RECENT_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const RECENT_REFRESH_MAX_INTERVAL: Duration = Duration::from_secs(60);
/// CI and project-status changes leave `updated_at` alone, so neither the
/// probe nor the change-only search sees them. Past this span since the
/// last status refresh, an auto-refresh re-reads the status of the open
/// rows, probe or no probe.
pub(super) const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
/// Backstop: past this span since the last full walk, an auto-refresh
/// walks every list so rows the change-only search missed converge.
pub(super) const FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(1800);

/// Which refresh an auto-refresh runs, given the time since the last
/// status refresh and since the last full walk.
pub(super) fn refresh_plan(since_status: Duration, since_full: Duration) -> github::Refresh {
    if since_full >= FULL_REFRESH_INTERVAL {
        github::Refresh::Full
    } else if since_status >= STATUS_REFRESH_INTERVAL {
        github::Refresh::Status
    } else {
        github::Refresh::Delta
    }
}

/// Auto-refresh cadence, overridable for deterministic runs: the e2e
/// harness stretches it past any scenario lifetime so each scenario
/// issues exactly one sections fetch per boot instead of hammering
/// the API every 30s. Zero or garbage falls back to the default.
pub(super) fn auto_refresh_interval_default() -> Duration {
    refresh_interval_from(std::env::var("GHIT_AUTO_REFRESH_SECS").ok().as_deref())
}

/// Parse the override value. Pure, so tests never mutate the process
/// env, which `App::new` in parallel tests also reads.
pub(super) fn refresh_interval_from(raw: Option<&str>) -> Duration {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(RECENT_REFRESH_INTERVAL)
}

impl App {
    /// File `number` under recent rows (background fetch). Requested rows
    /// never repeat here; see `sections`.
    pub(crate) fn ensure_recent(&mut self, number: u64) {
        if !self.nav.recent.contains(&number) {
            self.nav.recent.push(number);
        }
    }

    /// Startup hydration: paint the last persisted sidebar instantly so a
    /// cold start shows previous state. The background refresh landing in
    /// `poll_sections` updates these rows in place, so hydration only
    /// changes when rows first appear. A store with no sync is a
    /// no-op and startup behaves as before. Startup-only: refreshes must
    /// never resurrect evicted rows, so nothing else calls this.
    pub(super) fn hydrate_cached_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        let Some(rows) = github::cached_sections(&repo, self.sections.merged_window_hours, self.show_mask()) else { return };
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
            self.ensure_recent(n);
        }
    }

    /// Pull recent issues plus open/windowed-merged PRs into the sidebar.
    /// Offline or out of a repo this is a no-op: clicked rows still list.
    pub(crate) fn merge_recent_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        self.sections.last_fetch = Instant::now();
        self.sections.auto_refresh = false;
        self.sections.replace = false;
        let hours = self.sections.merged_window_hours;
        let show = self.show_mask();
        let probe = std::mem::take(&mut self.sections.probe);
        let refresh = std::mem::take(&mut self.sections.refresh);
        let now = Instant::now();
        if refresh != github::Refresh::Delta { self.sections.last_status = now; }
        if refresh == github::Refresh::Full { self.sections.last_full = now; }
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        self.enqueue(core::Kind::Sections, None, async move {
            if probe && !probe_says_changed(&repo, &tx).await { return; }
            let result = github::fetch_recent_sections(&repo, hours, show, refresh, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
    }

    /// Visible lists in section order: open PRs, merged PRs, open issues,
    /// closed issues. Hidden lists are not fetched.
    pub(crate) fn show_mask(&self) -> [bool; 4] {
        [self.sidebar.show_open_prs, self.sidebar.show_merged_prs, self.sidebar.show_open_issues, self.sidebar.show_closed_issues]
    }

    /// Flip one list (0 open PRs, 1 merged PRs, 2 open issues, 3 closed
    /// issues). The mask filters the stored rows, so a plain listing
    /// re-reads the store and sends no request; an active search ignores
    /// the toggles server-side.
    pub(crate) fn toggle_list(&mut self, index: usize) {
        let value = match index {
            0 => &mut self.sidebar.show_open_prs,
            1 => &mut self.sidebar.show_merged_prs,
            2 => &mut self.sidebar.show_open_issues,
            _ => &mut self.sidebar.show_closed_issues,
        };
        *value = !*value;
        self.persist_prefs();
        if self.sidebar.query_active.trim().is_empty() {
            self.reread_sections();
        }
    }

    /// True when this process syncs the repo's listing: it holds the
    /// store's sync lease, or has no store link and syncs on its own.
    pub(super) fn sync_holder(&self) -> bool {
        self.sections.link.as_ref().is_none_or(github::StoreLink::holding)
    }

    /// Per tick: renew or try the sync lease, and when another process (or
    /// this one's own fetch) wrote a new sync, re-read this instance's view
    /// of the stored rows. An in-flight fetch skips the re-read; it lands
    /// the same rows itself. A local query re-runs its filter; a GitHub
    /// result set is re-read by number. True when rows landed.
    pub(super) fn watch_store(&mut self, now: Instant) -> bool {
        let Some(link) = self.sections.link.as_mut() else { return false };
        link.tick(now);
        if self.sections.rx.is_some() || !link.changed() {
            return false;
        }
        let query = self.sidebar.query_active.clone();
        if query.trim().is_empty() {
            return self.reread_sections();
        }
        if self.sidebar.query_github.is_some() {
            return self.reread_github(true);
        }
        let Some(rows) = self.local_query_rows(&query) else { return false };
        self.land_local_query(rows, true);
        true
    }

    /// Land this instance's view of the stored rows: its window and list
    /// toggles filter them locally, with no request. Rows the view no
    /// longer shows leave unless visited or pinned. True when rows landed.
    fn reread_sections(&mut self) -> bool {
        let hours = self.sections.merged_window_hours;
        let show = self.show_mask();
        let Some(rows) = self.sections.link.as_ref().and_then(|link| link.rows(hours, show)) else { return false };
        crate::app::poll::land_rows(self, rows, false, true);
        true
    }

    /// Query mode: the lease holder still runs the listing sync on the
    /// normal cadence, because every ghit instance of the repo reads the
    /// store it writes. The sync lands no rows here. A local query
    /// re-filters when the store changes (`watch_store`).
    fn sync_behind_query(&mut self, now: Instant) {
        if now.duration_since(self.sections.last_sync) < self.sections.auto_refresh_interval
            || self.sections.sync_rx.is_some()
            || !self.sync_holder()
            || self.focus().layer != Layer::Base
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
        let (hours, show) = (self.sections.merged_window_hours, self.show_mask());
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.sync_rx = Some(rx);
        self.enqueue(core::Kind::Sections, None, async move {
            if probe && !probe_says_changed(&repo, &tx).await { return; }
            let result = github::fetch_recent_sections(&repo, hours, show, refresh, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
    }

    pub(super) fn auto_refresh_recent(&mut self, now: Instant) {
        if !self.sidebar.query_active.trim().is_empty() {
            self.sync_behind_query(now);
        }
        if now.duration_since(self.sections.last_fetch) < self.sections.auto_refresh_interval
            || self.sections.rx.is_some()
            || self.focus().layer != Layer::Base
            || github::rate_limited()
        {
            return;
        }
        let search = !self.sidebar.query_active.trim().is_empty();
        if !search {
            // Another process holds the sync lease: its writes land
            // through `watch_store`, so this one sends no listing request.
            if !self.sync_holder() {
                return;
            }
            self.sections.refresh = refresh_plan(
                now.duration_since(self.sections.last_status),
                now.duration_since(self.sections.last_full),
            );
            self.sections.probe = self.sections.refresh == github::Refresh::Delta;
            self.merge_recent_sections();
        } else {
            // A query never refreshes on GitHub by itself: it re-filters
            // when the store changes (`watch_store`).
            self.sections.last_fetch = now;
            return;
        }
        if self.sections.rx.is_some() {
            self.sections.auto_refresh = true;
        }
    }

    /// Window change or back to the listing: re-read the stored rows
    /// through the new window, so a narrower window shrinks the list with
    /// no request. The active query keeps its text and refetches; the
    /// window only changes the search's `updated:` token.
    pub(crate) fn reload_recent(&mut self) {
        self.sections.cap_notified = false;
        if self.sidebar.query_active.trim().is_empty() {
            // A search still in flight must not replace the listing later.
            if std::mem::take(&mut self.sections.replace) {
                self.sections.rx = None;
                self.sections.progress = None;
                self.sections.auto_refresh = false;
            }
            self.reread_sections();
        } else {
            self.drop_unvisited_background();
            self.run_gh_search(self.sidebar.query_active.clone());
        }
    }

    /// Set the merged/closed window and reload the active view.
    pub(crate) fn set_window(&mut self, hours: u64) {
        self.sections.merged_window_hours = hours;
        self.reload_recent();
        self.persist_prefs();
        self.notify(format!("merged window: {}", self.window_label()));
    }

    /// `0` in the Format dialog: back to the recency default, an empty
    /// query and the default window. Filters and grouping stay put.
    pub(crate) fn reset_recent_default(&mut self) {
        self.sidebar.query_active.clear();
        self.sidebar.query_draft.clear();
        self.set_format_edit(crate::app::FormatEdit::None);
        self.sections.merged_window_hours = core::DEFAULT_WINDOW_HOURS;
        self.reload_recent();
        self.persist_prefs();
        self.notify("recent: default query and window".to_string());
    }

    /// Run a `/` query against the local store. A query the store cannot
    /// answer lands no rows and names the GitHub action instead of
    /// calling the network.
    pub(crate) fn run_gh_search(&mut self, query: String) {
        self.sections.cap_notified = false;
        self.sidebar.query_github = None;
        if let Some(rows) = self.local_query_rows(&query) {
            self.notify(format!("query: {query} ({} local)", rows.len()));
            self.land_local_query(rows, false);
            return;
        }
        self.land_local_query(vec![], false);
        self.notify(format!("the local copy cannot answer: {query} · {} searches GitHub", self.github_key()));
    }

    /// Re-read the GitHub result set from the store, with no request.
    pub(crate) fn reread_github(&mut self, auto: bool) -> bool {
        let Some(numbers) = self.sidebar.query_github.clone() else { return false };
        let Some(rows) = self.sections.link.as_ref().map(|link| link.rows_by_number(&numbers)) else { return false };
        self.land_local_query(rows, auto);
        true
    }

    /// Key name of the Search GitHub action, for notices.
    pub(crate) fn github_key(&self) -> String {
        crate::keymap::describe(crate::keymap::Action::SearchGithub)
            .map_or_else(String::new, |binding| binding.key_name())
    }

    /// Where the active query's rows came from, for the status line.
    pub(crate) fn query_source(&self) -> &'static str {
        if self.sidebar.query_github.is_some() { "GitHub" } else { "local" }
    }

    /// Re-run the active query once on GitHub. The hits land in the store
    /// (`land_github`); nothing re-runs it until the user asks again.
    pub(crate) fn search_github(&mut self) {
        let query = self.sidebar.query_active.trim().to_string();
        if query.is_empty() {
            self.notify("no query: F writes one, then H searches GitHub".to_string());
            return;
        }
        self.sections.cap_notified = false;
        self.notify(format!("searching GitHub: {query}"));
        self.search_fetch(query);
    }

    /// Land GitHub search hits: store them (newer-only, statuses kept),
    /// show the stored rows, and remember their numbers so a store change
    /// keeps the same result set on screen.
    pub(super) fn land_github(&mut self, hits: Vec<github::Row>) {
        let rows = match self.sections.link.as_mut() {
            Some(link) => link.save_search(hits),
            None => hits,
        };
        let numbers = rows.iter().map(|row| row.issue.number).collect();
        self.land_local_query(rows, false);
        self.sidebar.query_github = Some(numbers);
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
    fn local_query_rows(&self, query: &str) -> Option<Vec<github::Row>> {
        let parsed = self.local_query(query)?;
        let mut rows: Vec<github::Row> = self.sections.link.as_ref()?.all_rows()?
            .into_iter()
            .filter(|row| parsed.matches(&row.issue))
            .collect();
        rows.sort_by(|a, b| {
            (&b.issue.updated_at, b.issue.number).cmp(&(&a.issue.updated_at, a.issue.number))
        });
        Some(rows)
    }

    /// Land local query rows as a search result: replace semantics, and any
    /// fetch in flight is abandoned so it cannot land over them.
    fn land_local_query(&mut self, rows: Vec<github::Row>, auto: bool) {
        self.sections.rx = None;
        self.sections.progress = None;
        self.sections.replace = false;
        self.sections.auto_refresh = false;
        self.sections.last_fetch = Instant::now();
        self.drop_unvisited_background();
        let numbers: Vec<u64> = rows.iter().map(|row| row.issue.number).collect();
        crate::app::poll::land_rows(self, rows, true, auto);
        // Complete rows already held are not re-filed by the landing.
        for n in numbers {
            self.ensure_recent(n);
        }
    }

    /// Spawn the GitHub search fetch; the caller announces it.
    fn search_fetch(&mut self, query: String) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else {
            self.notify("search needs a repo: use -R or run inside one".to_string());
            return;
        };
        self.sections.last_fetch = Instant::now();
        self.sections.auto_refresh = false;
        self.sections.replace = true;
        let search = core::search_query(&query, self.sections.merged_window_hours, chrono::Utc::now());
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        self.enqueue(core::Kind::Sections, None, async move {
            let result = github::search_repo(&repo, &search, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
    }

    /// Sidebar loading body for one pane: the scope the fetch covers.
    pub(crate) fn pane_loading_label(&self, want_pr: bool) -> String {
        let (open, done) = if want_pr {
            (self.sidebar.show_open_prs, self.sidebar.show_merged_prs)
        } else {
            (self.sidebar.show_open_issues, self.sidebar.show_closed_issues)
        };
        core::pane_loading_label(want_pr, open, done, self.sections.merged_window_hours,
            &self.sidebar.query_active)
    }

    pub(crate) fn window_label(&self) -> String {
        core::window_label(self.sections.merged_window_hours)
    }

    /// Sidebar split: pinned | requested | recent, pins never duplicated.
    pub(crate) fn sections(&self) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        core::section_split(&self.nav.pin_order, &self.nav.visited, &self.nav.recent, &self.nav.pinned)
    }

    /// Row title for search, empty when the row is still loading.
    fn row_title(&self, n: u64) -> String {
        self.issues.data.get(&n).map(|d| d.title.clone()).unwrap_or_default()
    }

    /// Client-side `/` row match over already-fetched rows: title,
    /// number, label names, and author login. Rows still loading fall
    /// back to title/number only.
    fn row_matches(&self, n: u64, query: &str) -> bool {
        match self.issues.data.get(&n) {
            Some(d) => {
                let labels: Vec<&str> = d.labels.iter().map(|l| l.name.as_str()).collect();
                core::matches_search(n, &d.title, &labels, &d.author.login, query)
            }
            None => core::matches_search(n, &self.row_title(n), &[], "", query),
        }
    }

    /// True for issues, false for PRs and rows still loading (filed with
    /// issues until their data arrives and re-files them).
    pub(super) fn is_pr(&self, n: u64) -> bool {
        self.issues.data.get(&n).map(|d| d.is_pr).unwrap_or(false)
    }

    /// Recent-row visibility under the phase-2 filters: the state
    /// toggles, label pills, and milestone pills. Pins and requested rows
    /// always show: filters thin the firehose, never rows the user
    /// explicitly opened.
    fn recent_visible(&self, n: u64) -> bool {
        let Some(d) = self.issues.data.get(&n) else { return true };
        let closed = matches!(d.state.as_str(), "CLOSED" | "MERGED");
        let state_ok = if d.is_pr {
            if closed { self.sidebar.show_merged_prs } else { self.sidebar.show_open_prs }
        } else if closed {
            self.sidebar.show_closed_issues
        } else {
            self.sidebar.show_open_issues
        };
        state_ok && self.label_visible(d) && self.milestone_visible(d)
    }

    /// Label pills match when the row carries any selected label. Empty
    /// selection means no filter.
    fn label_visible(&self, d: &model::IssueData) -> bool {
        self.sidebar.label_filter.is_empty()
            || d.labels.iter().any(|l| self.sidebar.label_filter.contains(&l.name))
    }

    /// Milestone pills match the row's milestone exactly; `None` is the
    /// "(no milestone)" pill. Empty selection means no filter.
    fn milestone_visible(&self, d: &model::IssueData) -> bool {
        self.sidebar.milestone_filter.is_empty()
            || self.sidebar.milestone_filter.contains(&d.milestone)
    }

    /// True when any phase-2 filter narrows the recent rows.
    pub(crate) fn filters_active(&self) -> bool {
        !(self.sidebar.show_open_prs && self.sidebar.show_merged_prs
            && self.sidebar.show_open_issues && self.sidebar.show_closed_issues)
            || !self.sidebar.label_filter.is_empty()
            || !self.sidebar.milestone_filter.is_empty()
    }

    /// (pinned, requested, recent) for one pane. When `filtered`, the `/`
    /// query narrows rows; the committed highlight in `search` always
    /// applies at render through `section_list`. Each group keeps its
    /// section order but is sorted by the active client-side key.
    pub(crate) fn pane_groups(&self, want_pr: bool, filtered: bool) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        let (p, v, r) = self.sections();
        let q = self.sidebar.search.clone();
        let pick = |list: Vec<u64>| -> Vec<u64> {
            list.into_iter()
                .filter(|n| {
                    self.is_pr(*n) == want_pr
                        && (!filtered || q.trim().is_empty() || self.row_matches(*n, &q))
                })
                .collect()
        };
        let sorted = |list: Vec<u64>| -> Vec<u64> {
            core::sort_rows(list, self.sidebar.sort, self.sidebar.sort_desc, &self.issues.data)
        };
        let pinned = sorted(pick(p));
        let requested = sorted(pick(v));
        let mut recent = sorted(pick(r));
        recent.retain(|n| self.recent_visible(*n));
        (pinned, requested, recent)
    }

    /// Display rows for one pane. Pinned and requested keep the ordinary
    /// section layout; milestone grouping splits only the filtered recent
    /// rows, so rows the user asked for never move under a group.
    fn pane_rows(&self, want_pr: bool, filtered: bool) -> Vec<model::SideRow> {
        let (p, v, r) = self.pane_groups(want_pr, filtered);
        if !want_pr && self.sidebar.sub_issue_tree {
            let mut out = core::pane_order(&p, &v, &[]);
            if self.sidebar.group_by == model::GroupBy::Milestone {
                let visible: HashSet<u64> = r.iter().copied().collect();
                let milestone_of = |mut n: u64| {
                    let mut seen = HashSet::new();
                    while seen.insert(n) {
                        match self.issues.data.get(&n).and_then(|d| d.parent) {
                            Some(parent) if visible.contains(&parent) => n = parent,
                            _ => break,
                        }
                    }
                    self.issues.data.get(&n).and_then(|d| d.milestone.clone())
                };
                let due_of = |title: &str| self.sidebar.milestones.iter()
                    .find(|m| m.title == title).and_then(|m| m.due_on.clone());
                for (label, rows) in core::milestone_groups(&r, &milestone_of, &due_of) {
                    out.push(model::SideRow::Divider(format!("{label} ({})", rows.len())));
                    out.extend(core::tree_rows(
                        &rows, &self.issues.data, &self.sidebar.folded_issues, &self.subs.state));
                }
            } else {
                if !out.is_empty() && !r.is_empty() {
                    out.push(model::SideRow::Divider("recent".into()));
                }
                out.extend(core::tree_rows(
                    &r, &self.issues.data, &self.sidebar.folded_issues, &self.subs.state));
            }
            return out;
        }
        if self.sidebar.group_by != model::GroupBy::Milestone {
            return core::pane_order(&p, &v, &r);
        }
        let milestone_of = |n: u64| {
            self.issues.data.get(&n).and_then(|d| d.milestone.clone())
        };
        let due_of = |title: &str| {
            self.sidebar.milestones.iter()
                .find(|m| m.title == title)
                .and_then(|m| m.due_on.clone())
        };
        let groups = core::milestone_groups(&r, &milestone_of, &due_of);
        core::grouped_pane_order(&p, &v, &groups)
    }

    /// One key cycles the sort key; each key starts in its natural
    /// direction (updated newest first, text A→Z).
    pub(crate) fn cycle_sort(&mut self) {
        self.sidebar.sort = self.sidebar.sort.next();
        self.sidebar.sort_desc = self.sidebar.sort.natural_desc();
        self.persist_prefs();
    }

    /// Flip the active sort direction. Default order has nothing to flip.
    pub(crate) fn toggle_sort_dir(&mut self) {
        if self.sidebar.sort != core::SideSort::Default {
            self.sidebar.sort_desc = !self.sidebar.sort_desc;
            self.persist_prefs();
        }
    }

    /// `/` bar hint label for the active sort.
    pub(crate) fn sort_label(&self) -> String {
        self.sidebar.sort.label(self.sidebar.sort_desc)
    }

    /// Display rows for the Issues pane, dividers marking group slots.
    pub(crate) fn issue_rows(&self) -> Vec<model::SideRow> {
        self.pane_rows(false, self.sidebar.searching)
    }

    /// Display rows for the Pull requests pane, dividers marking groups.
    pub(crate) fn pr_rows(&self) -> Vec<model::SideRow> {
        self.pane_rows(true, self.sidebar.searching)
    }

    /// Scoped j/k order for the Issues pane: exactly the displayed rows.
    pub(crate) fn issue_numbers(&self) -> Vec<u64> {
        self.issue_rows().iter().filter_map(model::SideRow::number).collect()
    }

    /// Scoped j/k order for the PRs pane: exactly the displayed rows.
    pub(crate) fn pr_numbers(&self) -> Vec<u64> {
        self.pr_rows().iter().filter_map(model::SideRow::number).collect()
    }

    /// Flat order limited to `/` search hits, for Enter / n / N jumps.
    /// Uses the displayed order, so it follows sort and grouping.
    pub(crate) fn search_hits(&self) -> Vec<u64> {
        if self.sidebar.search.trim().is_empty() {
            return vec![];
        }
        let q = self.sidebar.search.clone();
        self.issue_rows().iter().chain(self.pr_rows().iter())
            .filter_map(model::SideRow::number)
            .filter(|n| self.row_matches(*n, &q))
            .collect()
    }

    /// Commit `/` search: jump to the first hit, keep the highlight.
    /// An explicit open, so the row files under requested.
    pub(crate) fn commit_search(&mut self) {
        self.sidebar.searching = false;
        self.nav.focus = Focus::Main;
        if let Some(n) = self.search_hits().first().copied() {
            self.open_issue(n);
        }
    }

    /// Start editing the GitHub query in the format dialog.
    pub(crate) fn start_query_edit(&mut self) {
        self.sidebar.query_draft = self.sidebar.query_active.clone();
        self.set_format_edit(crate::app::FormatEdit::Query);
    }

    /// What committing the query draft means. Pure for tests: the caller
    /// performs the fetch. Non-empty runs a search, empty clears back to
    /// the window listing.
    pub(crate) fn commit_query_draft(&mut self) -> QueryAction {
        self.close(crate::app::Slot::Format);
        self.nav.focus = Focus::Main;
        let q = self.sidebar.query_draft.trim().to_string();
        self.sidebar.query_active = q.clone();
        if q.is_empty() { QueryAction::Refetch } else { QueryAction::Run(q) }
    }

    pub(crate) fn query_tag(&self) -> Option<&str> {
        let query = self.sidebar.query_active.trim();
        if query.is_empty() { return None; }
        Some(self.sidebar.saved_queries.iter().find(|saved| saved.query == query)
            .map_or("query", |saved| saved.name.as_str()))
    }

    pub(crate) fn start_name_edit(&mut self) {
        if self.sidebar.query_active.trim().is_empty() {
            self.notify("no active query to save".to_string());
            return;
        }
        self.sidebar.name_draft = self.sidebar.saved_queries.iter()
            .find(|saved| saved.query == self.sidebar.query_active.trim())
            .map_or_else(String::new, |saved| saved.name.clone());
        self.set_format_edit(crate::app::FormatEdit::Name);
    }

    pub(crate) fn save_named_query(&mut self) {
        let name = self.sidebar.name_draft.trim().to_string();
        if name.is_empty() {
            self.notify("query name is empty".to_string());
            return;
        }
        let query = self.sidebar.query_active.trim().to_string();
        if let Some(saved) = self.sidebar.saved_queries.iter_mut().find(|saved| saved.name == name) {
            saved.query = query;
        } else {
            self.sidebar.saved_queries.push(github::SavedQuery { name, query });
        }
        self.set_format_edit(crate::app::FormatEdit::None);
        self.persist_prefs();
    }

    pub(crate) fn delete_named_query(&mut self) {
        let focus = self.sidebar.format_focus;
        if (1..=self.sidebar.saved_queries.len()).contains(&focus) {
            let query = self.sidebar.saved_queries.remove(focus - 1);
            self.sidebar.format_focus = focus.min(self.format_widgets() - 1);
            self.persist_prefs();
            let message = format!("deleted query {} · u undo", query.name);
            self.arm_undo(UndoAction::Query { index: focus - 1, query }, message);
        }
    }

    /// Fetch labels and milestones for the dialog once per repo. The
    /// meta channel carries the result to `poll_repo_meta`.
    pub(crate) fn ensure_repo_meta(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        if self.sidebar.meta_repo.as_deref() == Some(repo.as_str()) {
            return;
        }
        self.sidebar.meta_repo = Some(repo.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.sidebar.meta_rx = Some(rx);
        self.enqueue(core::Kind::Other, None, async move {
            let _ = tx.send(github::fetch_repo_meta(&repo).await);
        });
    }

    /// Land a picker fetch: keep the lists, drop saved pills the repo no
    /// longer has, clamp the focus, and persist the pruned set. A failed
    /// fetch clears `meta_repo` so the next dialog open retries.
    pub(crate) fn land_repo_meta(&mut self, result: Result<RepoMeta, String>) {
        self.sidebar.meta_rx = None;
        let (labels, milestones) = match result {
            Ok(meta) => meta,
            Err(e) => {
                self.sidebar.meta_repo = None;
                self.notify(e);
                return;
            }
        };
        self.sidebar.labels = labels;
        self.sidebar.milestones = milestones;
        self.sidebar.meta_loaded = true;
        let live_labels: HashSet<String> = self.sidebar.labels.iter().cloned().collect();
        self.sidebar.label_filter.retain(|l| live_labels.contains(l));
        let live_milestones: HashSet<Option<String>> =
            std::iter::once(None)
                .chain(self.sidebar.milestones.iter().map(|m| Some(m.title.clone())))
                .collect();
        self.sidebar.milestone_filter.retain(|m| live_milestones.contains(m));
        self.sidebar.format_focus =
            self.sidebar.format_focus.min(self.format_widgets().saturating_sub(1));
        self.persist_prefs();
    }

    /// Group counts for one sidebar pane, matching the displayed rows.
    pub(crate) fn side_counts(&self, want_pr: bool) -> (usize, usize, usize) {
        let (p, v, r) = self.pane_groups(want_pr, self.sidebar.searching);
        (p.len(), v.len(), r.len())
    }

    /// Title subtitle for Issues/PRs: row count, recent mode, filters, and sort.
    pub(crate) fn side_subtitle(&self, want_pr: bool) -> String {
        let (state, sort) = self.side_subtitle_parts(want_pr);
        format!("{state} · {sort}")
    }

    /// The subtitle in its two clickable parts: list state (opens Format)
    /// and sort label (click cycles, the arrow reverses).
    pub(crate) fn side_subtitle_parts(&self, want_pr: bool) -> (String, String) {
        let (p, v, r) = self.side_counts(want_pr);
        let query = self.sidebar.query_active.trim();
        let context = if query.is_empty() { " · recent" } else { "" };
        let filtered = if self.filters_active() { " · filtered" } else { "" };
        (format!("{}{context}{filtered}", p + v + r), self.sort_label())
    }

    /// Footer for Issues/PRs: per-group counts.
    pub(crate) fn side_footer(&self, want_pr: bool) -> String {
        let (p, v, r) = self.side_counts(want_pr);
        let query = self.sidebar.query_active.trim();
        let suffix = if query.is_empty() { String::new() } else { format!(" · {query}") };
        format!("pinned ({p}) · requested ({v}) · recent ({r}){suffix}")
    }
}
