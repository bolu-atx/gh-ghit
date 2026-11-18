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
    /// When the last full section fetch landed.
    pub(crate) last_full: Instant,
    /// Section page progress: rows fetched vs the row cap, while pages
    /// remain. Cleared when the terminal result lands.
    pub(crate) progress: Option<(usize, usize)>,
    /// The next section payload replaces (search) instead of appending.
    pub(crate) replace: bool,
    pub(crate) merged_window_hours: u64,
    /// A cap notice already showed for this fetch setting, so the periodic
    /// auto-refresh never repeats it.
    pub(crate) cap_notified: bool,
}

impl SectionFeed {
    pub(crate) fn new() -> Self {
        Self { rx: None, last_fetch: Instant::now(),
            auto_refresh_interval: auto_refresh_interval_default(),
            auto_refresh: false, probe: false, last_full: Instant::now(),
            progress: None, replace: false, merged_window_hours: 24,
            cap_notified: false }
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
    pub(crate) query_draft: String,
    pub(crate) query_active: String,
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
            show_open_issues: true, show_closed_issues: true,
            query_draft: String::new(), query_active: String::new(),
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
/// Auto-refreshes inside this span since the last full fetch run the
/// free change probe first; past it a full fetch runs anyway, so CI and
/// project-status changes (which leave `updated_at` alone) still land.
pub(super) const FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(300);

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
    /// changes when rows first appear. Missing or expired cache is a
    /// no-op and startup behaves as before. Startup-only: refreshes must
    /// never resurrect evicted rows, so nothing else calls this.
    pub(super) fn hydrate_cached_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        let Some(rows) = github::cached_sections(&repo, self.sections.merged_window_hours) else { return };
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
        let probe = std::mem::take(&mut self.sections.probe);
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        self.enqueue(core::Kind::Sections, None, async move {
            if probe && !probe_says_changed(&repo, &tx).await { return; }
            let result = github::fetch_recent_sections(&repo, hours, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
    }

    pub(super) fn auto_refresh_recent(&mut self, now: Instant) {
        if now.duration_since(self.sections.last_fetch) < self.sections.auto_refresh_interval
            || self.sections.rx.is_some()
            || self.focus().layer != Layer::Base
            || github::rate_limited()
        {
            return;
        }
        self.sections.probe = now.duration_since(self.sections.last_full) < FULL_REFRESH_INTERVAL;
        if self.sidebar.query_active.trim().is_empty() {
            self.merge_recent_sections();
        } else {
            // Periodic refresh: no "searching" toast on every pass.
            self.search_fetch(self.sidebar.query_active.clone());
        }
        self.sections.probe = false;
        if self.sections.rx.is_some() {
            self.sections.auto_refresh = true;
        }
    }

    /// Window change: drop unopened background rows and refetch from
    /// scratch so a narrower window actually shrinks the list. The active
    /// query keeps its text; the window only changes the search's
    /// `updated:` token.
    pub(crate) fn refetch_recent(&mut self) {
        self.sections.cap_notified = false;
        self.drop_unvisited_background();
        if self.sidebar.query_active.trim().is_empty() {
            self.merge_recent_sections();
        } else {
            self.run_gh_search(self.sidebar.query_active.clone());
        }
    }

    /// Set the merged/closed window and refetch the active query path.
    pub(crate) fn set_window(&mut self, hours: u64) {
        self.sections.merged_window_hours = hours;
        self.refetch_recent();
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
        self.refetch_recent();
        self.persist_prefs();
        self.notify("recent: default query and window".to_string());
    }

    /// Server-side GitHub search scoped to the repo. Results replace the
    /// recent list when they land; see `sections.replace`. The window
    /// rides in as an `updated:` token at fetch time, so the query text
    /// stays exactly what the user typed.
    pub(crate) fn run_gh_search(&mut self, query: String) {
        self.sections.cap_notified = false;
        self.notify(format!("searching: {query}"));
        self.search_fetch(query);
    }

    /// Spawn the search fetch. Callers own the status-line announcement:
    /// user searches name the query, periodic refreshes stay quiet.
    fn search_fetch(&mut self, query: String) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else {
            self.notify("search needs a repo: use -R or run inside one".to_string());
            return;
        };
        self.sections.last_fetch = Instant::now();
        self.sections.auto_refresh = false;
        self.sections.replace = true;
        let search = core::search_query(&query, self.sections.merged_window_hours, chrono::Utc::now());
        let probe = std::mem::take(&mut self.sections.probe);
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.rx = Some(rx);
        self.enqueue(core::Kind::Sections, None, async move {
            if probe && !probe_says_changed(&repo, &tx).await { return; }
            let result = github::search_repo(&repo, &search, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
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
