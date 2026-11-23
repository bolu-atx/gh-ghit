//! Sidebar rows: sections fetch and merge, search, filters, sort, saved queries and auto-refresh.

use std::collections::HashSet;
use std::time::Instant;

use ratatui::widgets::ListState;

use crate::app::{App, Focus, UndoAction};
use ghit_core::core;
use ghit_core::{github, model};
use ghit_core::model::ItemRef;

/// What a filter-dialog query commit asks the caller to fetch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum QueryAction {
    Run(String),
    Refetch,
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
    pub(crate) query_github: Option<Vec<ItemRef>>,
    pub(crate) saved_queries: Vec<github::SavedQuery>,
    pub(crate) name_draft: String,
    pub(crate) group_by: model::GroupBy,
    pub(crate) sub_issue_tree: bool,
    pub(crate) folded_issues: HashSet<ItemRef>,
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

impl App {
    /// File `number` under recent rows (background fetch). Requested rows
    /// never repeat here; see `sections`.
    pub(crate) fn ensure_recent(&mut self, number: ItemRef) {
        if !self.nav.recent.contains(&number) {
            self.nav.recent.push(number);
        }
    }

    /// Startup hydration: paint the last persisted sidebar instantly so a
    /// cold start shows previous state. The background refresh landing in
    /// `poll_session` updates these rows in place, so hydration only
    /// changes when rows first appear. A store with no sync is a
    /// no-op and startup behaves as before. Startup-only: refreshes must
    /// never resurrect evicted rows, so nothing else calls this.
    pub(super) fn hydrate_cached_sections(&mut self) {
        let show = self.show_mask();
        for n in self.session.hydrate_cached(show) {
            self.ensure_recent(n);
        }
    }

    /// Pull recent issues plus open/windowed-merged PRs into the sidebar.
    /// Offline or out of a repo this is a no-op: clicked rows still list.
    pub(crate) fn merge_recent_sections(&mut self) {
        let (show, view) = (self.show_mask(), self.fetch_view());
        self.session.refresh_recent(show, &view);
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

    /// Re-run the active query after the stored listing changed: a
    /// GitHub result set is re-read by number, a local query re-filters.
    pub(super) fn requery_after_store_change(&mut self) -> bool {
        let query = self.sidebar.query_active.clone();
        if self.sidebar.query_github.is_some() {
            return self.reread_github(true);
        }
        let Some(rows) = self.session.local_query_rows(&query) else { return false };
        self.land_local_query(rows, true);
        true
    }

    /// Per tick, tests only: the store watch of one poll frame.
    #[cfg(test)]
    pub(super) fn watch_store(&mut self, now: Instant) -> bool {
        if !self.session.store_changed(now) {
            return false;
        }
        if self.sidebar.query_active.trim().is_empty() {
            return self.reread_sections();
        }
        self.requery_after_store_change()
    }

    /// Per tick, tests only: the auto-refresh of one poll frame.
    #[cfg(test)]
    pub(super) fn auto_refresh_recent(&mut self, now: Instant) {
        let ctx = self.poll_ctx(now);
        self.session.auto_refresh_recent(&ctx);
    }

    /// Land this instance's view of the stored rows: its window and list
    /// toggles filter them locally, with no request. Rows the view no
    /// longer shows leave unless visited or pinned. True when rows landed.
    pub(super) fn reread_sections(&mut self) -> bool {
        let Some(rows) = self.session.window_rows(self.show_mask()) else { return false };
        self.land_rows(rows, false, true);
        true
    }

    /// Window change or back to the listing: re-read the stored rows
    /// through the new window, so a narrower window shrinks the list with
    /// no request. The active query keeps its text and refetches; the
    /// window only changes the search's `updated:` token.
    pub(crate) fn reload_recent(&mut self) {
        self.session.sections.cap_notified = false;
        if self.sidebar.query_active.trim().is_empty() {
            // A search still in flight must not replace the listing later.
            if std::mem::take(&mut self.session.sections.replace) {
                self.session.sections.rx = None;
                self.session.sections.progress = None;
                self.session.sections.auto_refresh = false;
            }
            self.reread_sections();
        } else {
            self.drop_unvisited_background();
            self.run_gh_search(self.sidebar.query_active.clone());
        }
    }

    /// Set the merged/closed window and reload the active view.
    pub(crate) fn set_window(&mut self, hours: u64) {
        self.session.sections.merged_window_hours = hours;
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
        self.session.sections.merged_window_hours = core::DEFAULT_WINDOW_HOURS;
        self.reload_recent();
        self.persist_prefs();
        self.notify("recent: default query and window".to_string());
    }

    /// Run a `/` query against the local store. A query the store cannot
    /// answer lands no rows and names the GitHub action instead of
    /// calling the network.
    pub(crate) fn run_gh_search(&mut self, query: String) {
        self.session.sections.cap_notified = false;
        self.sidebar.query_github = None;
        if let Some(rows) = self.session.local_query_rows(&query) {
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
        let Some(rows) = self.session.stored_rows(&numbers) else { return false };
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
        self.session.sections.cap_notified = false;
        self.notify(format!("searching GitHub: {query}"));
        self.search_fetch(query);
    }

    /// Land GitHub search hits: store them (newer-only, statuses kept),
    /// show the stored rows, and remember their numbers so a store change
    /// keeps the same result set on screen.
    pub(super) fn land_github(&mut self, hits: Vec<github::Row>) {
        let rows = self.session.save_search(hits);
        let numbers = rows.iter().map(|row| row.issue.number).collect();
        self.land_local_query(rows, false);
        self.sidebar.query_github = Some(numbers);
    }

    /// Land local query rows as a search result: replace semantics, and any
    /// fetch in flight is abandoned so it cannot land over them.
    fn land_local_query(&mut self, rows: Vec<github::Row>, auto: bool) {
        self.session.sections.rx = None;
        self.session.sections.progress = None;
        self.session.sections.replace = false;
        self.session.sections.auto_refresh = false;
        self.session.sections.last_fetch = Instant::now();
        self.drop_unvisited_background();
        let numbers: Vec<ItemRef> = rows.iter().map(|row| row.issue.number).collect();
        self.land_rows(rows, true, auto);
        // Complete rows already held are not re-filed by the landing.
        for n in numbers {
            self.ensure_recent(n);
        }
    }

    /// Spawn the GitHub search fetch; the caller announces it.
    fn search_fetch(&mut self, query: String) {
        let view = self.fetch_view();
        if !self.session.search(&query, &view) {
            self.notify("search needs a repo: use -R or run inside one".to_string());
        }
    }

    /// Sidebar loading body for one pane: the scope the fetch covers.
    pub(crate) fn pane_loading_label(&self, want_pr: bool) -> String {
        let (open, done) = if want_pr {
            (self.sidebar.show_open_prs, self.sidebar.show_merged_prs)
        } else {
            (self.sidebar.show_open_issues, self.sidebar.show_closed_issues)
        };
        core::pane_loading_label(want_pr, open, done, self.session.sections.merged_window_hours,
            &self.sidebar.query_active)
    }

    pub(crate) fn window_label(&self) -> String {
        core::window_label(self.session.sections.merged_window_hours)
    }

    /// Sidebar split: pinned | requested | recent, pins never duplicated.
    pub(crate) fn sections(&self) -> (Vec<ItemRef>, Vec<ItemRef>, Vec<ItemRef>) {
        core::section_split(&self.session.pins.order, &self.nav.visited, &self.nav.recent, &self.session.pins.set)
    }

    /// Row title for search, empty when the row is still loading.
    fn row_title(&self, n: ItemRef) -> String {
        self.session.issues.data.get(&n).map(|d| d.title.clone()).unwrap_or_default()
    }

    /// Client-side `/` row match over already-fetched rows: title,
    /// number, label names, and author login. Rows still loading fall
    /// back to title/number only.
    fn row_matches(&self, n: ItemRef, query: &str) -> bool {
        match self.session.issues.data.get(&n) {
            Some(d) => {
                let labels: Vec<&str> = d.labels.iter().map(|l| l.name.as_str()).collect();
                core::matches_search(n, &d.title, &labels, &d.author.login, query)
            }
            None => core::matches_search(n, &self.row_title(n), &[], "", query),
        }
    }

    /// True for issues, false for PRs and rows still loading (filed with
    /// issues until their data arrives and re-files them).
    pub(super) fn is_pr(&self, n: ItemRef) -> bool {
        self.session.issues.data.get(&n).map(|d| d.is_pr).unwrap_or(false)
    }

    /// Recent-row visibility under the phase-2 filters: the state
    /// toggles, label pills, and milestone pills. Pins and requested rows
    /// always show: filters thin the firehose, never rows the user
    /// explicitly opened.
    fn recent_visible(&self, n: ItemRef) -> bool {
        let Some(d) = self.session.issues.data.get(&n) else { return true };
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
    pub(crate) fn pane_groups(
        &self,
        want_pr: bool,
        filtered: bool,
    ) -> (Vec<ItemRef>, Vec<ItemRef>, Vec<ItemRef>) {
        let (p, v, r) = self.sections();
        let q = self.sidebar.search.clone();
        let pick = |list: Vec<ItemRef>| -> Vec<ItemRef> {
            list.into_iter()
                .filter(|n| {
                    self.is_pr(*n) == want_pr
                        && (!filtered || q.trim().is_empty() || self.row_matches(*n, &q))
                })
                .collect()
        };
        let sorted = |list: Vec<ItemRef>| -> Vec<ItemRef> {
            core::sort_rows(list, self.sidebar.sort, self.sidebar.sort_desc, &self.session.issues.data)
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
                let visible: HashSet<ItemRef> = r.iter().copied().collect();
                let milestone_of = |mut n: ItemRef| {
                    let mut seen = HashSet::new();
                    while seen.insert(n) {
                        match self.session.issues.data.get(&n).and_then(|d| d.parent) {
                            Some(parent) if visible.contains(&parent) => n = parent,
                            _ => break,
                        }
                    }
                    self.session.issues.data.get(&n).and_then(|d| d.milestone.clone())
                };
                let due_of = |title: &str| self.sidebar.milestones.iter()
                    .find(|m| m.title == title).and_then(|m| m.due_on.clone());
                for (label, rows) in core::milestone_groups(&r, &milestone_of, &due_of) {
                    out.push(model::SideRow::Divider(format!("{label} ({})", rows.len())));
                    out.extend(core::tree_rows(
                        &rows, &self.session.issues.data, &self.sidebar.folded_issues, &self.session.subs.state));
                }
            } else {
                if !out.is_empty() && !r.is_empty() {
                    out.push(model::SideRow::Divider("recent".into()));
                }
                out.extend(core::tree_rows(
                    &r, &self.session.issues.data, &self.sidebar.folded_issues, &self.session.subs.state));
            }
            return out;
        }
        if self.sidebar.group_by != model::GroupBy::Milestone {
            return core::pane_order(&p, &v, &r);
        }
        let milestone_of = |n: ItemRef| {
            self.session.issues.data.get(&n).and_then(|d| d.milestone.clone())
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
    pub(crate) fn issue_numbers(&self) -> Vec<ItemRef> {
        self.issue_rows().iter().filter_map(model::SideRow::number).collect()
    }

    /// Scoped j/k order for the PRs pane: exactly the displayed rows.
    pub(crate) fn pr_numbers(&self) -> Vec<ItemRef> {
        self.pr_rows().iter().filter_map(model::SideRow::number).collect()
    }

    /// Flat order limited to `/` search hits, for Enter / n / N jumps.
    /// Uses the displayed order, so it follows sort and grouping.
    pub(crate) fn search_hits(&self) -> Vec<ItemRef> {
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
        let Some(repo) = self.session.repo_flag.clone().or_else(|| self.session.repo_slug.clone()) else { return };
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
