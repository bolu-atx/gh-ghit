//! App: state, transitions, event loop, background service wiring.
//!
//! `App` owns the data (issues, pins, board, channels) and coordinates
//! fetching through `github`. It never draws (`ui`) and never parses input
//! (`input`); pure rules live in `core`. Network work runs on one shared
//! tokio runtime and lands via channels polled once per frame, so the loop
//! never blocks.

use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::time::Duration;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::ExecutableCommand;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::{api, cache, core, github, model};
use crate::model::{BoardData, IssueData};

/// The one executor for all background fetches. Tasks are cheap, so callers
/// spawn tasks per request and let the runtime schedule them. Built
/// lazily so tests that never fetch pay nothing, and never dropped.
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Details,
    Visual,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Main,
    Bar,
}

/// Which pane `j`/`k` act on. Tab cycles Issues → PRs → Refs, skipping
/// Refs while it is hidden. Text input (`Focus::Bar`), the board, and the
/// filter dialog keep their own keys. The body keeps its dedicated keys
/// (arrows move the diff line, 1/2/3 switch tabs) so it never takes Tab focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Issues,
    PRs,
    Main,
    Refs,
}

/// What a filter-dialog query commit asks the caller to fetch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum QueryAction {
    Run(String),
    Refetch,
}

pub(crate) struct App {
    pub(crate) repo_flag: Option<String>,
    pub(crate) repo_slug: Option<String>,
    pub(crate) bootstrap_rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    pub(crate) initial_numbers: Vec<u64>,
    pub(crate) section_rx: Option<std::sync::mpsc::Receiver<Result<Vec<api::Row>, String>>>,
    pub(crate) partial: HashSet<u64>,
    pub(crate) issue_tx: std::sync::mpsc::Sender<(u64, Result<github::IssueBundle, String>)>,
    pub(crate) issue_rx: std::sync::mpsc::Receiver<(u64, Result<github::IssueBundle, String>)>,
    pub(crate) issue_pending: HashSet<u64>,
    pub(crate) requested: Option<u64>,
    pub(crate) data: HashMap<u64, IssueData>,
    pub(crate) statuses: HashMap<u64, Vec<(String, Option<String>)>>,
    /// Numbers the user explicitly asked for or clicked, in first-seen
    /// order. Merely viewing a recent row never lands here.
    pub(crate) visited: Vec<u64>,
    /// Background-fetched recent rows (window listing or GitHub search),
    /// in fetch order.
    pub(crate) recent: Vec<u64>,
    pub(crate) pinned: HashSet<u64>,
    pub(crate) pin_order: Vec<u64>,
    pub(crate) current: Option<u64>,
    pub(crate) tab: Tab,
    pub(crate) focus: Focus,
    pub(crate) pane: Pane,
    pub(crate) bar: String,
    /// `/` search text. Persists after commit so matches stay highlighted;
    /// `searching` is true only while editing it in the bar.
    pub(crate) search: String,
    pub(crate) searching: bool,
    /// Client-side sort for both sidebar panes: key plus direction.
    pub(crate) sort: core::SideSort,
    pub(crate) sort_desc: bool,
    /// Filter dialog (`f`) state.
    pub(crate) shortcuts_open: bool,
    pub(crate) filter_open: bool,
    pub(crate) query_edit: bool,
    pub(crate) query_draft: String,
    pub(crate) query_active: String,
    pub(crate) show_open_prs: bool,
    pub(crate) show_merged_prs: bool,
    pub(crate) show_open_issues: bool,
    pub(crate) show_closed_issues: bool,
    /// The next section payload replaces (search) instead of appending.
    pub(crate) section_replace: bool,
    pub(crate) scroll: u16,
    pub(crate) message: String,
    pub(crate) message_at: Option<std::time::Instant>,
    pub(crate) issue_state: ListState,
    pub(crate) pr_state: ListState,
    pub(crate) quit: bool,
    pub(crate) body_area: Rect,
    pub(crate) body_rows: Vec<String>,
    /// Title/label block rendered above the content tabs. Clicks here
    /// resolve issue refs and URLs with the same mapping as the body.
    pub(crate) header_area: Rect,
    pub(crate) header_rows: Vec<String>,
    /// Scroll offset `body_rows` was captured at; selections add to this.
    pub(crate) body_base: u16,
    pub(crate) board_loading: bool,
    pub(crate) board_rx: Option<std::sync::mpsc::Receiver<Result<BoardData, String>>>,
    pub(crate) board_states: Vec<ListState>,
    pub(crate) board_expanded: Option<(usize, usize)>,
    pub(crate) merged_window_hours: u64,
    pub(crate) board_details: HashMap<u64, model::BoardDetail>,
    pub(crate) detail_pending: HashSet<u64>,
    pub(crate) detail_tx: std::sync::mpsc::Sender<(u64, Result<model::BoardDetail, String>)>,
    pub(crate) detail_rx: std::sync::mpsc::Receiver<(u64, Result<model::BoardDetail, String>)>,
    pub(crate) diffs: HashMap<u64, Vec<model::DiffFile>>,
    pub(crate) diff_cursor: usize,
    /// Active diff line as an absolute body-buffer row. `None` until the
    /// next render anchors it to the first file header. Steps one row at a
    /// time so wrapped rows just work; render resolves the owning file and
    /// auto-expands collapsed files from the fresh capture.
    pub(crate) diff_line: Option<usize>,
    /// Set when input moved the line cursor: render then resolves the file
    /// and expands a landed-on collapsed file. Stays false for the lazy
    /// init anchor so opening a Diff never unfolds the first file.
    pub(crate) diff_touched: bool,
    /// Owning file per visible body row, parallel to `body_rows` and only
    /// meaningful on the Diff tab. Rows before the first header map to
    /// `None`; blank/rule rows inherit the previous header's file.
    pub(crate) diff_row_files: Vec<Option<usize>>,
    /// (issue, filename) pairs with unfolded hunks; the rest stays folded.
    pub(crate) expanded_files: HashSet<(u64, String)>,
    /// Issue numbers whose sidebar row shows its detail block. Keyed by
    /// number, so the state is sticky across cursor moves and shared by
    /// the Issues, PRs, and Refs panes.
    pub(crate) expanded_rows: HashSet<u64>,
    /// Visible buffer rows of the diff file headers, in file order.
    pub(crate) diff_headers: Vec<usize>,
    pub(crate) diff_pending: HashSet<u64>,
    pub(crate) diff_tx: std::sync::mpsc::Sender<(u64, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) diff_rx: std::sync::mpsc::Receiver<(u64, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) diff_split: bool,
    pub(crate) graphs: HashMap<u64, model::GraphData>,
    pub(crate) graph_pending: HashSet<u64>,
    pub(crate) graph_tx: std::sync::mpsc::Sender<(u64, Result<model::GraphData, String>)>,
    pub(crate) graph_rx: std::sync::mpsc::Receiver<(u64, Result<model::GraphData, String>)>,
    /// Cursor into `graph_order_numbers` for the current issue. Reset on
    /// issue switch; tab switches keep it so going back restores place.
    pub(crate) graph_cursor: usize,
    // Last-rendered hit areas for mouse clicks. The row maps hold the
    // display order per pane, `None` marking a divider slot.
    pub(crate) side_issue_area: Rect,
    pub(crate) side_pr_area: Rect,
    pub(crate) side_issue_rows: Vec<model::SideRow>,
    pub(crate) side_pr_rows: Vec<model::SideRow>,
    /// Related issues mentioned in the current body. Hidden when empty.
    pub(crate) show_refs: bool,
    pub(crate) ref_numbers: Vec<u64>,
    /// Will-close counterparts of the current issue/PR, listed under a
    /// `Paired` divider. Kept disjoint from `ref_numbers` so a row has one
    /// behavior: paired rows open, mention rows only highlight.
    pub(crate) ref_paired: Vec<u64>,
    /// Click-selected refs row: highlights its source line instead of opening it.
    pub(crate) ref_highlight: Option<u64>,
    pub(crate) refs_area: Rect,
    pub(crate) ref_rows: Vec<model::SideRow>,
    pub(crate) refs_state: ListState,
    pub(crate) tabs_area: Rect,
    pub(crate) bar_area: Rect,
    pub(crate) board_cells: Vec<(Rect, usize, usize)>,
    // Board modal.
    pub(crate) board: Option<BoardData>,
    pub(crate) board_col: usize,
    pub(crate) board_row: usize,
}

impl App {
    pub(crate) fn new(repo_flag: Option<String>) -> Self {
        let (detail_tx, detail_rx) = std::sync::mpsc::channel();
        let (issue_tx, issue_rx) = std::sync::mpsc::channel();
        let (diff_tx, diff_rx) = std::sync::mpsc::channel();
        let (graph_tx, graph_rx) = std::sync::mpsc::channel();
        Self {
            repo_flag,
            repo_slug: None,
            bootstrap_rx: None,
            initial_numbers: vec![],
            section_rx: None,
            partial: HashSet::new(),
            issue_tx, issue_rx, issue_pending: HashSet::new(), requested: None,
            data: HashMap::new(),
            statuses: HashMap::new(),
            visited: vec![],
            recent: vec![],
            pinned: HashSet::new(),
            pin_order: vec![],
            current: None,
            tab: Tab::Details,
            focus: Focus::Main,
            pane: Pane::Issues,
            bar: String::new(),
            search: String::new(),
            searching: false,
            sort: core::SideSort::Default,
            sort_desc: false,
            shortcuts_open: false,
            filter_open: false,
            query_edit: false,
            query_draft: String::new(),
            query_active: String::new(),
            show_open_prs: true,
            show_merged_prs: true,
            show_open_issues: true,
            show_closed_issues: true,
            section_replace: false,
            scroll: 0,
            message: String::new(),
            message_at: None,
            issue_state: ListState::default(),
            pr_state: ListState::default(),
            board: None,
            board_col: 0,
            board_row: 0,
            board_loading: false,
            board_rx: None,
            board_states: Vec::new(),
            board_expanded: None,
            merged_window_hours: 24,
            board_details: HashMap::new(),
            detail_pending: HashSet::new(),
            detail_tx,
            detail_rx,
            diffs: HashMap::new(),
            diff_cursor: 0,
            diff_line: None,
            diff_touched: false,
            diff_row_files: vec![],
            expanded_files: HashSet::new(),
            expanded_rows: HashSet::new(),
            diff_headers: vec![],
            diff_pending: HashSet::new(),
            diff_tx,
            diff_rx,
            diff_split: false,
            graphs: HashMap::new(),
            graph_pending: HashSet::new(),
            graph_tx,
            graph_rx,
            graph_cursor: 0,
            quit: false,
            body_area: Rect::default(),
            body_rows: Vec::new(),
            header_area: Rect::default(),
            header_rows: vec![],
            body_base: 0,
            side_issue_area: Rect::default(),
            side_pr_area: Rect::default(),
            side_issue_rows: vec![],
            side_pr_rows: vec![],
            show_refs: true,
            ref_numbers: vec![],
            ref_paired: vec![],
            ref_highlight: None,
            refs_area: Rect::default(),
            ref_rows: vec![],
            refs_state: ListState::default(),
            tabs_area: Rect::default(),
            bar_area: Rect::default(),
            board_cells: Vec::new(),
        }
    }

    /// Status-line message that expires after a few seconds so the
    /// shortcut hints always come back.
    pub(crate) fn notify(&mut self, msg: String) {
        self.message = msg;
        self.message_at = Some(std::time::Instant::now());
    }

    pub(crate) fn message_visible(&self) -> bool {
        match (self.message.is_empty(), self.message_at) {
            (false, Some(at)) => at.elapsed() < std::time::Duration::from_secs(4),
            _ => false,
        }
    }

    pub(crate) fn repo_key(&self) -> Option<String> {
        let r = self
            .repo_flag
            .clone()
            .or_else(|| self.repo_slug.clone())?;
        Some(r.to_lowercase())
    }

    fn resolve_repo(&mut self) {
        self.repo_slug = github::resolve_repo(self.repo_flag.as_deref());
    }

    /// File `number` under requested rows. Call this only for explicit
    /// user actions (typing, clicking, search commit): background fetches
    /// and cursor movement must not relocate rows between groups.
    pub(crate) fn ensure_visited(&mut self, number: u64) {
        if !self.visited.contains(&number) {
            self.visited.push(number);
        }
        self.sync_selection();
    }

    /// File `number` under recent rows (background fetch). Requested rows
    /// never repeat here; see `sections`.
    pub(crate) fn ensure_recent(&mut self, number: u64) {
        if !self.recent.contains(&number) {
            self.recent.push(number);
        }
    }

    /// Pull recent issues plus open/windowed-merged PRs into the sidebar.
    /// Offline or out of a repo this is a no-op: clicked rows still list.
    pub(crate) fn merge_recent_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        let hours = self.merged_window_hours;
        let (tx, rx) = std::sync::mpsc::channel();
        self.section_rx = Some(rx);
        rt().spawn(async move {
            let _ = tx.send(github::fetch_recent_sections(&repo, hours).await);
        });
    }

    /// Window change: drop unopened background rows and refetch from
    /// scratch so a narrower window actually shrinks the list. Explicitly
    /// opened rows keep their data and stay listed.
    pub(crate) fn refetch_recent(&mut self) {
        self.drop_unvisited_background();
        self.query_active.clear();
        self.query_draft.clear();
        self.merge_recent_sections();
    }

    /// Server-side GitHub search scoped to the repo. Results replace the
    /// recent list when they land; see `section_replace`.
    pub(crate) fn run_gh_search(&mut self, query: String) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else {
            self.notify("search needs a repo: use -R or run inside one".to_string());
            return;
        };
        self.section_replace = true;
        self.notify(format!("searching: {query}"));
        let (tx, rx) = std::sync::mpsc::channel();
        self.section_rx = Some(rx);
        rt().spawn(async move {
            let _ = tx.send(github::search_repo(&repo, &query).await);
        });
    }

    /// Drop background rows the user never opened, with their cached data.
    fn drop_unvisited_background(&mut self) {
        let keep: HashSet<u64> = self.visited.iter().copied()
            .chain(self.pin_order.iter().copied())
            .chain(self.pinned.iter().copied())
            .collect();
        self.recent.retain(|n| keep.contains(n));
        for n in self.partial.iter().copied().collect::<Vec<_>>() {
            if !keep.contains(&n) {
                self.drop_cached(n);
            }
        }
    }

    /// Remove every cached payload for `n` so a revisit fetches again.
    fn drop_cached(&mut self, n: u64) {
        self.partial.remove(&n);
        self.data.remove(&n);
        self.statuses.remove(&n);
        self.board_details.remove(&n);
        self.diffs.remove(&n);
        self.diff_pending.remove(&n);
        self.graphs.remove(&n);
        self.graph_pending.remove(&n);
    }

    /// Evict the highlighted sidebar row: drop it from the pane lists and
    /// every cached payload, then put the cursor on the next row (falling
    /// back to the previous one at the end). Pinned rows resist until
    /// unpinned with `p`; a revisit refetches from the network.
    pub(crate) fn evict_current(&mut self) {
        if !matches!(self.pane, Pane::Issues | Pane::PRs) {
            return;
        }
        let Some(n) = self.current else {
            self.notify("no row to evict".to_string());
            return;
        };
        if self.pinned.contains(&n) || self.pin_order.contains(&n) {
            self.notify(format!("#{n} is pinned — unpin with p first"));
            return;
        }
        let order = match self.pane {
            Pane::Issues => self.issue_numbers(),
            Pane::PRs => self.pr_numbers(),
            _ => return,
        };
        let Some(pos) = order.iter().position(|x| *x == n) else { return };
        self.visited.retain(|x| *x != n);
        self.recent.retain(|x| *x != n);
        self.expanded_rows.remove(&n);
        if self.ref_highlight == Some(n) {
            self.ref_highlight = None;
        }
        self.drop_cached(n);
        let next = order
            .get(pos + 1)
            .copied()
            .or_else(|| pos.checked_sub(1).and_then(|i| order.get(i)).copied());
        match next {
            Some(next) => self.show_issue(next),
            None => {
                self.current = None;
                self.requested = None;
                self.ref_numbers.clear();
                self.ref_paired.clear();
                self.scroll = 0;
                self.sync_selection();
            }
        }
        self.notify(format!("evicted #{n}"));
    }

    pub(crate) fn window_label(&self) -> String {
        core::window_label(self.merged_window_hours)
    }

    pub(crate) fn load_issue(&mut self, number: u64, force: bool) {
        self.requested = Some(number);
        if !force && self.data.contains_key(&number) && !self.partial.contains(&number) {
            self.show_issue(number);
            self.bar.clear();
            self.focus = Focus::Main;
            return;
        }
        if !self.issue_pending.insert(number) { return; }
        let tx = self.issue_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        rt().spawn(async move {
            let _ = tx.send((number, github::fetch_number(&repo, number, force).await));
        });
    }

    /// Sidebar split: pinned | requested | recent, pins never duplicated.
    pub(crate) fn sections(&self) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        core::section_split(&self.pin_order, &self.visited, &self.recent, &self.pinned)
    }

    /// Row title for search, empty when the row is still loading.
    fn row_title(&self, n: u64) -> String {
        self.data.get(&n).map(|d| d.title.clone()).unwrap_or_default()
    }

    /// Client-side `/` row match over already-fetched rows: title,
    /// number, label names, and author login. Rows still loading fall
    /// back to title/number only.
    fn row_matches(&self, n: u64, query: &str) -> bool {
        match self.data.get(&n) {
            Some(d) => {
                let labels: Vec<&str> = d.labels.iter().map(|l| l.name.as_str()).collect();
                core::matches_search(n, &d.title, &labels, &d.author.login, query)
            }
            None => core::matches_search(n, &self.row_title(n), &[], "", query),
        }
    }

    /// True for issues, false for PRs and rows still loading (filed with
    /// issues until their data arrives and re-files them).
    fn is_pr(&self, n: u64) -> bool {
        self.data.get(&n).map(|d| d.is_pr).unwrap_or(false)
    }

    /// Recent-row visibility under the `f` state toggles. Pins and
    /// requested rows always show: filters thin the firehose, never rows
    /// the user explicitly opened.
    fn recent_visible(&self, n: u64) -> bool {
        let Some(d) = self.data.get(&n) else { return true };
        let closed = matches!(d.state.as_str(), "CLOSED" | "MERGED");
        if d.is_pr {
            if closed { self.show_merged_prs } else { self.show_open_prs }
        } else if closed {
            self.show_closed_issues
        } else {
            self.show_open_issues
        }
    }

    /// (pinned, requested, recent) for one pane. When `filtered`, the `/`
    /// query narrows rows; the committed highlight in `search` always
    /// applies at render through `section_list`. Each group keeps its
    /// section order but is sorted by the active client-side key.
    pub(crate) fn pane_groups(&self, want_pr: bool, filtered: bool) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
        let (p, v, r) = self.sections();
        let q = self.search.clone();
        let pick = |list: Vec<u64>| -> Vec<u64> {
            list.into_iter()
                .filter(|n| {
                    self.is_pr(*n) == want_pr
                        && (!filtered || q.trim().is_empty() || self.row_matches(*n, &q))
                })
                .collect()
        };
        let sorted = |list: Vec<u64>| -> Vec<u64> {
            core::sort_rows(list, self.sort, self.sort_desc, &self.data)
        };
        let pinned = sorted(pick(p));
        let requested = sorted(pick(v));
        let mut recent = sorted(pick(r));
        recent.retain(|n| self.recent_visible(*n));
        (pinned, requested, recent)
    }

    /// One key cycles the sort key; each key starts in its natural
    /// direction (updated newest first, text A→Z).
    pub(crate) fn cycle_sort(&mut self) {
        self.sort = self.sort.next();
        self.sort_desc = self.sort.natural_desc();
    }

    /// Flip the active sort direction. Default order has nothing to flip.
    pub(crate) fn toggle_sort_dir(&mut self) {
        if self.sort != core::SideSort::Default {
            self.sort_desc = !self.sort_desc;
        }
    }

    /// `/` bar hint label for the active sort.
    pub(crate) fn sort_label(&self) -> String {
        self.sort.label(self.sort_desc)
    }

    /// Display rows for the Issues pane, dividers marking group slots.
    pub(crate) fn issue_rows(&self) -> Vec<model::SideRow> {
        let (p, v, r) = self.pane_groups(false, self.searching);
        core::pane_order(&p, &v, &r)
    }

    /// Display rows for the Pull requests pane, dividers marking groups.
    pub(crate) fn pr_rows(&self) -> Vec<model::SideRow> {
        let (p, v, r) = self.pane_groups(true, self.searching);
        core::pane_order(&p, &v, &r)
    }

    /// Scoped j/k order for the Issues pane: pinned, requested, recent.
    pub(crate) fn issue_numbers(&self) -> Vec<u64> {
        let (p, v, r) = self.pane_groups(false, false);
        p.into_iter().chain(v).chain(r).collect()
    }

    /// Scoped j/k order for the PRs pane: pinned, requested, recent.
    pub(crate) fn pr_numbers(&self) -> Vec<u64> {
        let (p, v, r) = self.pane_groups(true, false);
        p.into_iter().chain(v).chain(r).collect()
    }

    /// Flat order limited to `/` search hits, for Enter / n / N jumps.
    pub(crate) fn search_hits(&self) -> Vec<u64> {
        if self.search.trim().is_empty() {
            return vec![];
        }
        let (ip, iv, ir) = self.pane_groups(false, false);
        let (pp, pv, pr) = self.pane_groups(true, false);
        let q = self.search.clone();
        ip.into_iter()
            .chain(iv).chain(ir).chain(pp).chain(pv).chain(pr)
            .filter(|n| self.row_matches(*n, &q))
            .collect()
    }

    /// Commit `/` search: jump to the first hit, keep the highlight.
    /// An explicit open, so the row files under requested.
    pub(crate) fn commit_search(&mut self) {
        self.searching = false;
        self.focus = Focus::Main;
        if let Some(n) = self.search_hits().first().copied() {
            self.ensure_visited(n);
            self.load_issue(n, false);
        }
    }

    /// Start editing the GitHub query in the filter dialog.
    pub(crate) fn start_query_edit(&mut self) {
        self.query_draft = self.query_active.clone();
        self.query_edit = true;
    }

    /// What committing the query draft means. Pure for tests: the caller
    /// performs the fetch. Non-empty runs a search, empty clears back to
    /// the window listing.
    pub(crate) fn commit_query_draft(&mut self) -> QueryAction {
        self.query_edit = false;
        self.filter_open = false;
        self.focus = Focus::Main;
        let q = self.query_draft.trim().to_string();
        self.query_active = q.clone();
        if q.is_empty() { QueryAction::Refetch } else { QueryAction::Run(q) }
    }

    /// Recent pane subtitle: window label, or the active search query.
    pub(crate) fn recent_detail(&self) -> String {
        let n = self.issue_rows().len() + self.pr_rows().len();
        if self.query_active.trim().is_empty() {
            format!("{} · {n}", self.window_label())
        } else {
            format!("\"{}\" · {n}", self.query_active)
        }
    }

    /// Position of `current` in display rows, +1 for the top spacer row.
    fn selected_index(rows: &[model::SideRow], current: Option<u64>) -> Option<usize> {
        current
            .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
            .map(|p| p + 1)
    }

    fn sync_selection(&mut self) {
        let issue_rows = self.issue_rows();
        let pr_rows = self.pr_rows();
        match self.current {
            Some(n) if issue_rows.iter().any(|r| r.number() == Some(n)) => {
                self.issue_state.select(Self::selected_index(&issue_rows, self.current));
                self.pr_state.select(None);
            }
            Some(n) if pr_rows.iter().any(|r| r.number() == Some(n)) => {
                self.issue_state.select(None);
                self.pr_state.select(Self::selected_index(&pr_rows, self.current));
            }
            _ => {}
        }
    }

    pub(crate) fn show_issue(&mut self, n: u64) {
        self.requested = Some(n);
        if self.partial.contains(&n) { self.load_issue(n, false); }
        self.current = Some(n);
        self.scroll = 0;
        self.diff_cursor = 0;
        self.diff_line = None;
        self.diff_touched = false;
        self.ref_highlight = None;
        self.graph_cursor = 0;
        self.sync_selection();
        self.load_diff(n, false);
        if self.visual_graph() {
            self.load_graph(n, false);
        }
        self.refresh_refs();
    }

    /// Select a refs row: highlight where it is mentioned in the body
    /// instead of opening it. Clicking the same row again clears it.
    /// Tab cycles Issues → PRs → Refs, so the pane keeps focus until then.
    pub(crate) fn select_ref(&mut self, n: u64) {
        if self.ref_highlight == Some(n) {
            self.ref_highlight = None;
            return;
        }
        self.ref_highlight = Some(n);
        // Scroll the body pane so the first mentioning row sits at the top.
        // `body_rows` is the visible slice taken at `body_base`.
        let repo = self.repo_key();
        if let Some(row) = self.body_rows.iter().position(|line| {
            model::links_in_line(line, repo.as_deref())
                .iter()
                .any(|(_, t)| *t == model::LinkTarget::Issue(n))
        }) {
            self.scroll = self.body_base.saturating_add(row as u16);
        }
    }

    /// Toggle the detail block on the focused pane's selected row. The set
    /// is keyed by issue number, so expansion sticks across cursor moves.
    /// The body has no row selection, so there the key is a no-op.
    pub(crate) fn toggle_row_expansion(&mut self) {
        let selected = match self.pane {
            Pane::Issues => self.current.filter(|n| self.issue_numbers().contains(n)),
            Pane::PRs => self.current.filter(|n| self.pr_numbers().contains(n)),
            Pane::Refs => self.ref_highlight,
            Pane::Main => None,
        };
        let Some(n) = selected else { return };
        if !self.expanded_rows.remove(&n) {
            self.expanded_rows.insert(n);
        }
    }

    /// Fold/unfold the file under the active diff line. No-op without
    /// loaded files. Folding acts on the body, so it takes body focus
    /// with it. Collapsing re-anchors the line cursor to the file header
    /// (the header row itself never moves); expanding leaves the cursor
    /// where it is because new rows only appear below the header.
    pub(crate) fn toggle_diff_file(&mut self) {
        let Some(n) = self.current else { return };
        self.pane = Pane::Main;
        let Some(files) = self.diffs.get(&n) else { return };
        let count = files.len().min(crate::ui::MAX_DIFF_FILES);
        if count == 0 {
            return;
        }
        let fi = self.diff_file_at_cursor().unwrap_or(self.diff_cursor).min(count - 1);
        self.diff_cursor = fi;
        self.diff_touched = false;
        let key = (n, files[fi].filename.clone());
        if !self.expanded_files.remove(&key) {
            self.expanded_files.insert(key);
        } else {
            self.anchor_cursor_to_header(fi);
        }
    }

    /// Owning file of the active diff line from the last render capture.
    /// `None` when the cursor is unset, stale, or on a pre-header row.
    pub(crate) fn diff_file_at_cursor(&self) -> Option<usize> {
        let abs = self.diff_line?;
        let vis = abs.checked_sub(self.body_base as usize)?;
        self.diff_row_files.get(vis).copied().flatten()
    }

    /// Move the line cursor to file `fi`'s header when it is on screen.
    /// Used after collapsing: rows below the header vanish, so a cursor
    /// inside the old content would point at the wrong region.
    fn anchor_cursor_to_header(&mut self, fi: usize) {
        let base = self.body_base as usize;
        let found = match self.current.and_then(|n| self.diffs.get(&n)) {
            Some(files) => self.body_rows.iter().enumerate().find_map(|(vis, line)| {
                (model::header_file_index(line, files) == Some(fi)).then_some(vis)
            }),
            None => return,
        };
        if let Some(vis) = found {
            self.diff_line = Some(base + vis);
        }
    }

    /// The right refs pane shows only when toggled on and non-empty.
    pub(crate) fn refs_visible(&self) -> bool {
        self.show_refs && !(self.ref_numbers.is_empty() && self.ref_paired.is_empty())
    }

    /// Display rows for the refs pane: mentioned numbers, then a `Paired`
    /// divider with the will-close counterparts. Empty sections collapse.
    pub(crate) fn refs_rows(&self) -> Vec<model::SideRow> {
        let mut rows: Vec<model::SideRow> = self.ref_numbers.iter().copied().map(model::SideRow::Item).collect();
        if !self.ref_paired.is_empty() {
            rows.push(model::SideRow::Divider("Paired"));
            rows.extend(self.ref_paired.iter().copied().map(model::SideRow::Item));
        }
        rows
    }

    /// j/k order for the refs pane, skipping the divider.
    pub(crate) fn ref_order(&self) -> Vec<u64> {
        self.ref_numbers.iter().chain(self.ref_paired.iter()).copied().collect()
    }

    /// Enter on a refs row: paired rows open like sidebar rows; mention
    /// rows stay highlight-only, so they are a no-op here.
    pub(crate) fn open_selected_ref(&mut self) {
        let Some(n) = self.ref_highlight else { return };
        if !self.ref_paired.contains(&n) {
            return;
        }
        self.ensure_visited(n);
        self.show_issue(n);
    }

    /// Flip the right refs pane, keeping the current issue in place.
    pub(crate) fn toggle_refs(&mut self) {
        self.show_refs = !self.show_refs;
        if !self.show_refs && self.pane == Pane::Refs {
            self.pane = Pane::Main;
        }
        self.notify(if self.show_refs { "refs: shown".to_string() } else { "refs: hidden".to_string() });
    }

    /// Tab cycles Issues → PRs → Refs, skipping Refs while it is hidden.
    /// The body keeps its dedicated keys so it never takes Tab focus.
    pub(crate) fn cycle_pane(&mut self) {
        self.pane = match self.pane {
            Pane::Issues => Pane::PRs,
            Pane::PRs if self.refs_visible() => Pane::Refs,
            _ => Pane::Issues,
        };
    }

    /// Recompute refs from the current body and pairs, then queue
    /// background fetches for rows we have never loaded. Never changes
    /// `current`.
    pub(crate) fn refresh_refs(&mut self) {
        let Some(n) = self.current else {
            self.ref_numbers.clear();
            self.ref_paired.clear();
            return;
        };
        let Some(d) = self.data.get(&n) else {
            self.ref_numbers.clear();
            self.ref_paired.clear();
            return;
        };
        // Pairs win over mentions so each row has one behavior: the
        // pairing survives, and Enter on it opens the counterpart.
        let paired: Vec<u64> = d.paired.iter().map(|p| p.number).filter(|m| *m != n).collect();
        let repo = self.repo_key();
        let nums: Vec<u64> = model::referenced_numbers(&d.body, repo.as_deref())
            .into_iter()
            .filter(|m| *m != n && !paired.contains(m))
            .collect();
        self.ref_numbers = nums;
        self.ref_paired = paired;
        let tx = self.issue_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        for m in self.ref_order() {
            if self.data.contains_key(&m) || !self.issue_pending.insert(m) {
                continue;
            }
            let tx = tx.clone();
            let repo = repo.clone();
            rt().spawn(async move {
                let _ = tx.send((m, github::fetch_number(&repo, m, false).await));
            });
        }
    }

    pub(crate) fn visual_diff(&self) -> bool {
        self.tab == Tab::Visual && self.current.and_then(|n| self.data.get(&n)).is_some_and(|d| model::visual_content(d.is_pr) == model::VisualContent::Diff)
    }

    pub(crate) fn visual_graph(&self) -> bool {
        self.tab == Tab::Visual && self.current.and_then(|n| self.data.get(&n)).is_some_and(|d| model::visual_content(d.is_pr) == model::VisualContent::Graph)
    }

    /// Switch tabs, resetting scroll. Selecting Diff on a PR or Graph on
    /// any issue starts its background fetch so the pane fills in without
    /// blocking; first visit fetches, later visits reuse the cache.
    pub(crate) fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.scroll = 0;
        self.diff_line = None;
        self.diff_touched = false;
        if self.visual_diff() {
            if let Some(n) = self.current {
                self.load_diff(n, false);
            }
        }
        if self.visual_graph() {
            if let Some(n) = self.current {
                self.load_graph(n, false);
            }
        }
    }

    /// Flip the Diff tab between unified and side-by-side rendering,
    /// back at the top so the new layout starts visible.
    pub(crate) fn toggle_diff_split(&mut self) {
        self.diff_split = !self.diff_split;
        self.diff_line = None;
        self.diff_touched = false;
        self.scroll = 0;
        self.notify(if self.diff_split { "diff: split".to_string() } else { "diff: unified".to_string() });
    }

    /// Queue a diff fetch for PR `number`. No-op for issues, cached rows,
    /// and rows already in flight.
    pub(crate) fn load_diff(&mut self, number: u64, force: bool) {
        let Some(d) = self.data.get(&number) else { return };
        if !d.is_pr {
            return;
        }
        if !force && self.diffs.contains_key(&number) {
            return;
        }
        if !self.diff_pending.insert(number) {
            return;
        }
        let tx = self.diff_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        rt().spawn(async move {
            let _ = tx.send((number, github::fetch_diff(&repo, number, force).await));
        });
    }

    /// Queue a graph fetch for `number`. No-op for cached rows and rows
    /// already in flight. One cache entry per issue, so switching issues
    /// refetches per issue and the tab shows a skeleton meanwhile.
    pub(crate) fn load_graph(&mut self, number: u64, force: bool) {
        if !force && self.graphs.contains_key(&number) {
            return;
        }
        if !self.graph_pending.insert(number) {
            return;
        }
        let tx = self.graph_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        rt().spawn(async move {
            let _ = tx.send((number, github::fetch_graph(&repo, number, force).await));
        });
    }

    /// Flat navigation order for the graph tab: the center first, then
    /// neighbors section by section. Empty without a loaded graph.
    pub(crate) fn graph_order_numbers(&self) -> Vec<u64> {
        let Some(n) = self.current else { return vec![] };
        let Some(data) = self.graphs.get(&n) else { return vec![] };
        model::graph_order(data).into_iter().take(8).map(|i| data.nodes[i].number).collect()
    }

    /// Move the graph cursor, clamped at the ends. No-op without a graph.
    pub(crate) fn graph_move(&mut self, delta: isize) {
        let order = self.graph_order_numbers();
        if let Some(next) = core::graph_select(order.len(), Some(self.graph_cursor), delta) {
            self.graph_cursor = next;
        }
    }

    /// Open the graph node under the cursor in the viewer.
    pub(crate) fn open_graph_selection(&mut self) {
        let order = self.graph_order_numbers();
        if let Some(&n) = order.get(self.graph_cursor) {
            self.ensure_visited(n);
            self.load_issue(n, false);
        }
    }

    pub(crate) fn toggle_pin(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let Some(key) = self.repo_key() else {
            self.notify("pinning needs a repo: use -R or run inside one".to_string());
            return;
        };
        let loaded = cache::load_pins(&key);
        let was_pinned = loaded.contains(&n);
        let pins = core::toggle_pin(&loaded, n);
        if was_pinned {
            self.pinned.remove(&n);
            self.notify(format!("unpinned #{n}"));
        } else {
            self.pinned.insert(n);
            self.notify(format!("pinned #{n}"));
        }
        self.pin_order = pins.clone();
        cache::save_pins(&key, &pins);
        self.show_issue(n);
    }

    pub(crate) fn refresh_current(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone());
        if let Some(owner) = github::board_owner(repo.as_deref()) {
            cache::cache_drop(&format!("board-{}", owner.to_lowercase()));
        }
        self.load_issue(n, true);
        self.load_diff(n, true);
        self.load_graph(n, true);
        // A search listing refreshes from the server; a window listing
        // re-appends (idempotent through the contains checks in polling).
        if self.query_active.trim().is_empty() {
            self.merge_recent_sections();
        } else {
            let q = self.query_active.clone();
            self.run_gh_search(q);
        }
        self.notify(format!("refreshed #{n}"));
    }

    pub(crate) fn open_number(&mut self, n: u64) {
        if let Some(d) = self.data.get(&n) {
            open::that(&d.url).ok();
        } else if let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) {
            let url = format!("https://github.com/{repo}/issues/{n}");
            open::that(&url).ok();
        } else {
            open::that(format!("https://github.com/issues/{n}")).ok();
        }
        self.notify(format!("opened #{n} in browser"));
    }

    /// Full yank text for the current context, without side effects.
    /// Refs highlight wins, then the Diff file cursor, else the current
    /// issue/PR. `None` means `yank` should explain instead of copying.
    pub(crate) fn yank_text(&self) -> Option<String> {
        let current = self.current?;
        let d = self.data.get(&current)?;
        let repo = self.repo_key();
        if self.pane == Pane::Refs {
            let t = self.ref_highlight?;
            let target = self.data.get(&t);
            if let Some(text) = core::yank_ref_text(
                current,
                t,
                &d.body,
                repo.as_deref(),
                target.map(|x| x.title.as_str()),
                target.map(|x| x.url.as_str()),
            ) {
                return Some(text);
            }
            if let Some(td) = target {
                return Some(core::yank_issue_text(t, &td.title, &td.url));
            }
            return Some(format!("#{t}"));
        }
        if self.tab == Tab::Visual && d.is_pr {
            let files = self.diffs.get(&current)?;
            if files.is_empty() {
                return None;
            }
            // A patch content row under the active line yanks that line;
            // headers, blanks, and rules fall back to the file summary.
            if let Some(abs) = self.diff_line {
                let vis = abs.saturating_sub(self.body_base as usize);
                if let (Some(line), Some(fi)) =
                    (self.body_rows.get(vis), self.diff_file_at_cursor())
                {
                    let is_header = line.starts_with("▸ ") || line.starts_with("▾ ");
                    let trimmed = line.trim();
                    let is_meta = trimmed.is_empty()
                        || trimmed.chars().all(|c| c == '─')
                        || trimmed.starts_with("… and");
                    if !is_header && !is_meta {
                        if let Some(file) = files.get(fi) {
                            return Some(core::yank_diff_line_text(
                                current,
                                &d.title,
                                &d.url,
                                &file.filename,
                                line,
                            ));
                        }
                    }
                }
            }
            let file = &files[self.diff_cursor.min(files.len() - 1)];
            let expanded = self.expanded_files.contains(&(current, file.filename.clone()));
            let hunk = if expanded {
                core::first_hunk(&file.patch)
            } else {
                None
            };
            return Some(core::yank_diff_text(
                current,
                &d.title,
                &d.url,
                &file.filename,
                &file.status,
                file.additions,
                file.deletions,
                hunk.as_deref(),
            ));
        }
        Some(core::yank_issue_text(current, &d.title, &d.url))
    }

    /// Copy the context yank to the clipboard. Every path toasts: success
    /// shows the yanked text trimmed to fit, failures name the reason.
    pub(crate) fn yank(&mut self) {
        match self.yank_text() {
            Some(text) => {
                if crate::clipboard::copy(&text) {
                    self.notify(format!("yanked {}", core::toast_snippet(&text, 80)));
                } else {
                    self.notify("copy failed".to_string());
                }
            }
            None => {
                if self.current.is_none() {
                    self.notify("view an issue first".to_string());
                } else if self.pane == Pane::Refs {
                    self.notify("no ref selected".to_string());
                } else if self.tab == Tab::Visual {
                    self.notify("no diff loaded yet".to_string());
                } else {
                    self.notify("nothing to yank".to_string());
                }
            }
        }
    }

    pub(crate) fn submit_bar(&mut self) {
        match core::parse_bar(&self.bar.clone()) {
            core::BarCommand::Numbers(numbers) => {
                for n in numbers {
                    self.ensure_visited(n);
                    self.load_issue(n, false);
                }
            }
            core::BarCommand::OpenCurrent => {
                if let Some(n) = self.current {
                    self.open_number(n);
                } else {
                    self.notify("view an issue first".to_string());
                }
            }
            core::BarCommand::Open(n) => self.open_number(n),
            core::BarCommand::Quit => {}
            core::BarCommand::Dismiss => {
                self.focus = Focus::Main;
            }
            core::BarCommand::Hint => self.message = "type an issue number, o, or q".to_string(),
        }
    }

    /// Modal opens instantly with a skeleton; the network fetch lands in
    /// the background.
    pub(crate) fn open_board(&mut self) {
        if board_open(self) {
            return;
        }
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone());
        let Some(owner) = github::board_owner(repo.as_deref()) else {
            self.notify("board needs a repo: use -R or run inside one".to_string());
            return;
        };
        self.board_loading = true;
        self.board_col = 0;
        self.board_row = 0;
        let (tx, rx) = std::sync::mpsc::channel();
        self.board_rx = Some(rx);
        // `gh` subprocesses block: keep them off the async workers.
        rt().spawn_blocking(move || {
            let _ = tx.send(board_via_gh(&owner));
        });
    }

    /// Snapshot the visible body text so clicks can land on `#123` refs.
    /// `body_base` records the scroll offset the snapshot was taken at, so
    /// selections map visible rows back without accumulating across clicks.
    pub(crate) fn capture_body_rows(&mut self, buf: &ratatui::buffer::Buffer) {
        let area = self.body_area;
        self.body_rows = (0..area.height)
            .map(|r| {
                let y = area.y + r;
                (area.x..area.x + area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        self.body_base = self.scroll;
    }
}

/// Board fetch while `gh::fetch_board` is deprecated. Single call site by
/// design: remove with the GraphQL board port.
#[allow(deprecated)]
fn board_via_gh(owner: &str) -> Result<BoardData, String> {
    crate::gh::fetch_board(owner)
}

/// The board modal is up, whether still fetching or fully loaded.
pub(crate) fn board_open(app: &App) -> bool {
    app.board.is_some() || app.board_loading
}

pub(crate) fn board_move(app: &mut App, dx: isize, dy: isize) {
    app.board_expanded = None;
    let Some(board) = &app.board else { return };
    let cols = model::board_columns(&board.items, &board.order);
    if cols.is_empty() {
        return;
    }
    let col = (app.board_col as isize + dx).clamp(0, cols.len() as isize - 1) as usize;
    let rows = cols[col].1.len();
    (app.board_col, app.board_row) =
        core::board_cursor(cols.len(), rows, app.board_col, app.board_row, dx, dy);
}

pub(crate) fn board_selected_number(app: &App) -> Option<u64> {
    let board = app.board.as_ref()?;
    let cols = model::board_columns(&board.items, &board.order);
    cols.get(app.board_col)?.1.get(app.board_row)?.number
}

/// Fetch missing board card details together without blocking the event loop.
fn kick_off_details(app: &mut App) {
    let Some(board) = &app.board else { return };
    let mut cards = Vec::new();
    for item in &board.items {
        let (Some(n), Some(repo)) = (item.number, item.repo.clone()) else { continue };
        if app.board_details.contains_key(&n) || app.detail_pending.contains(&n) { continue; }
        if let Some(d) = app.data.get(&n) {
            app.board_details.insert(n, model::BoardDetail::from_issue(d));
            continue;
        }
        app.detail_pending.insert(n);
        cards.push((repo, n));
    }
    if cards.is_empty() { return; }
    let tx = app.detail_tx.clone();
    rt().spawn(async move {
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

fn poll_details(app: &mut App) {
    let ready: Vec<(u64, Result<model::BoardDetail, String>)> = app.detail_rx.try_iter().collect();
    for (n, res) in ready {
        app.detail_pending.remove(&n);
        match res {
            Ok(d) => { app.board_details.insert(n, d); }
            Err(e) => app.notify(e),
        }
    }
}

fn poll_board(app: &mut App) {
    if !app.board_loading {
        return;
    }
    let res = app.board_rx.as_ref().and_then(|rx| rx.try_recv().ok());
    let Some(res) = res else { return };
    app.board_rx = None;
    app.board_loading = false;
    match res {
        Ok(data) => {
            app.board = Some(data);
            app.board_col = 0;
            app.board_row = 0;
            app.board_states.clear();
            app.board_expanded = None;
            kick_off_details(app);
        }
        Err(e) => app.notify(e),
    }
}

// ---------------------------------------------------------------------------
// TUI runner
// ---------------------------------------------------------------------------

pub(crate) fn run(app: &mut App, numbers: &[u64]) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen).map_err(|e| e.to_string())?;
    stdout.execute(EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    app.initial_numbers = numbers.to_vec();
    app.focus = Focus::Bar;
    let repo = app.repo_flag.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.bootstrap_rx = Some(rx);
    // `git` and `gh` subprocesses block: keep them off the async workers.
    rt().spawn_blocking(move || {
        let mut worker = App::new(repo);
        worker.resolve_repo();
        let _ = crate::api::token();
        let _ = tx.send(worker.repo_slug);
    });

    let result = event_loop(&mut terminal, app);

    disable_raw_mode().map_err(|e| e.to_string())?;
    terminal
        .backend_mut()
        .execute(DisableMouseCapture)
        .map_err(|e| e.to_string())?;
    terminal.backend_mut().execute(LeaveAlternateScreen).map_err(|e| e.to_string())?;
    terminal.show_cursor().map_err(|e| e.to_string())?;
    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        terminal.draw(|f| crate::ui::render(f, app)).map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) => crate::input::handle_key(app, key),
                Event::Mouse(m) => crate::input::handle_mouse(app, m),
                _ => {}
            };
        }
        poll_bootstrap(app);
        poll_sections(app);
        poll_issues(app);
        poll_diffs(app);
        poll_graphs(app);
        poll_board(app);
        poll_details(app);
        if app.quit {
            return Ok(());
        }
    }
}

fn poll_bootstrap(app: &mut App) {
    let Some(result) = app.bootstrap_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
    app.bootstrap_rx = None;
    app.repo_slug = result;
    let numbers = std::mem::take(&mut app.initial_numbers);
    if let Some(key) = app.repo_key() {
        let mut pins = cache::load_pins(&key);
        pins.retain(|n| !numbers.contains(n));
        app.pinned = pins.iter().copied().collect();
        app.pin_order = pins.clone();
        for n in pins { app.load_issue(n, false); }
        for n in numbers {
            app.ensure_visited(n);
            app.load_issue(n, false);
        }
        app.merge_recent_sections();
    } else {
        for n in numbers { app.load_issue(n, false); }
    }
}

pub(crate) fn poll_sections(app: &mut App) {
    let Some(result) = app.section_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
    app.section_rx = None;
    if std::mem::take(&mut app.section_replace) {
        app.drop_unvisited_background();
    }
    match result {
        Ok(rows) => {
            let mut last = None;
            for row in rows {
                let n = row.issue.number;
                if !app.data.contains_key(&n) {
                    if !row.complete { app.partial.insert(n); }
                    app.data.insert(n, row.issue);
                    app.statuses.insert(n, row.statuses);
                    app.board_details.insert(n, row.detail);
                    app.ensure_recent(n);
                    last = Some(n);
                }
            }
            if app.current.is_none() && app.requested.is_none() {
                if let Some(n) = last { app.load_issue(n, false); }
            }
        },
        Err(e) => app.notify(e),
    }
}

pub(crate) fn poll_issues(app: &mut App) {
    let ready: Vec<_> = app.issue_rx.try_iter().collect();
    for (number, result) in ready {
        app.issue_pending.remove(&number);
        match result {
            Ok((data, statuses)) => {
                // Completing the full fetch never files the row: viewing a
                // recent row must not relocate it to requested. Explicit
                // actions file through `ensure_visited` at their own site.
                app.partial.remove(&number);
                app.data.insert(number, data);
                app.statuses.insert(number, statuses);
                if app.current == Some(number) {
                    app.refresh_refs();
                    if app.visual_graph() { app.load_graph(number, false); }
                    if app.visual_diff() { app.load_diff(number, false); }
                }
                if app.requested == Some(number) {
                    app.show_issue(number);
                    app.bar.clear();
                    app.focus = Focus::Main;
                    app.message.clear();
                }
            }
            Err(e) => app.notify(e),
        }
    }
}

pub(crate) fn poll_diffs(app: &mut App) {
    let ready: Vec<_> = app.diff_rx.try_iter().collect();
    for (number, result) in ready {
        app.diff_pending.remove(&number);
        match result {
            Ok(files) => {
                app.diffs.insert(number, files);
            }
            Err(e) => app.notify(e),
        }
    }
}

pub(crate) fn poll_graphs(app: &mut App) {
    let ready: Vec<_> = app.graph_rx.try_iter().collect();
    for (number, result) in ready {
        app.graph_pending.remove(&number);
        match result {
            Ok(data) => {
                app.graphs.insert(number, data);
                if app.current == Some(number) {
                    app.graph_cursor = app.graph_cursor.min(app.graph_order_numbers().len().saturating_sub(1));
                }
            }
            Err(e) => app.notify(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    fn issue(number: u64, updated: &str) -> IssueData {
        IssueData {
            number,
            title: format!("issue {number}"),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![],
            assignees: vec![],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated.to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            body: String::new(),
            comments: vec![],
            review_threads: vec![],
            paired: vec![],
            is_pr: false,
        }
    }

    fn sidebar_app(numbers: &[u64]) -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for &n in numbers {
            app.data.insert(n, issue(n, "2026-01-05T00:00:00Z"));
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.current = numbers.first().copied();
        app.sync_selection();
        app
    }

    #[test]
    fn toggle_row_expansion_is_sticky_and_keyed_by_number() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.visited = vec![7, 9, 11];
        app.current = Some(7);
        app.pane = Pane::Issues;

        app.toggle_row_expansion();
        assert!(app.expanded_rows.contains(&7), "space expands the selected row");
        app.toggle_row_expansion();
        assert!(!app.expanded_rows.contains(&7), "space collapses it again");

        // Expansion sticks across cursor moves and is keyed by number:
        // expanding one row never touches another.
        app.toggle_row_expansion();
        app.show_issue(9);
        assert!(app.expanded_rows.contains(&7), "cursor move keeps expansion");
        app.toggle_row_expansion();
        assert!(app.expanded_rows.contains(&9));
        assert!(app.expanded_rows.contains(&7), "rows expand independently");

        // The refs pane toggles its highlighted row.
        app.pane = Pane::Refs;
        app.ref_highlight = Some(11);
        app.toggle_row_expansion();
        assert!(app.expanded_rows.contains(&11));

        // The body pane has no row selection.
        app.pane = Pane::Main;
        let before = app.expanded_rows.clone();
        app.toggle_row_expansion();
        assert_eq!(app.expanded_rows, before);
    }

    #[test]
    fn sort_cycle_orders_rows_and_restores_default() {
        let mut app = sidebar_app(&[7, 9, 11]);
        app.data.get_mut(&7).unwrap().updated_at = "2026-01-05T00:00:00Z".to_string();
        app.data.get_mut(&9).unwrap().updated_at = "2026-01-09T00:00:00Z".to_string();
        app.data.get_mut(&11).unwrap().updated_at = "2026-01-01T00:00:00Z".to_string();
        assert_eq!(app.issue_numbers(), vec![7, 9, 11]);
        app.cycle_sort();
        assert_eq!(app.sort, core::SideSort::Updated);
        assert_eq!(app.issue_numbers(), vec![9, 7, 11], "newest first");
        app.toggle_sort_dir();
        assert_eq!(app.issue_numbers(), vec![11, 7, 9], "direction flips");
        for _ in 0..4 {
            app.cycle_sort();
        }
        assert_eq!(app.sort, core::SideSort::Default);
        assert_eq!(app.issue_numbers(), vec![7, 9, 11], "default order is back");
    }

    #[test]
    fn sort_applies_to_the_prs_pane_too() {
        let mut app = sidebar_app(&[7]);
        for (n, updated) in [(13, "2026-01-03T00:00:00Z"), (15, "2026-01-08T00:00:00Z")] {
            let mut pr = issue(n, updated);
            pr.is_pr = true;
            app.data.insert(n, pr);
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        assert_eq!(app.pr_numbers(), vec![13, 15]);
        app.cycle_sort();
        assert_eq!(app.pr_numbers(), vec![15, 13], "PRs sort newest first");
        assert_eq!(app.issue_numbers(), vec![7], "panes stay separate");
    }

    #[test]
    fn evict_drops_row_and_cached_data_then_moves_cursor() {
        let mut app = sidebar_app(&[7, 9]);
        app.current = Some(7);
        app.expanded_rows.insert(7);
        app.diffs.insert(7, vec![]);
        app.graphs.insert(7, model::GraphData::default());
        app.evict_current();
        assert!(!app.visited.contains(&7));
        assert!(!app.data.contains_key(&7), "data dropped");
        assert!(!app.statuses.contains_key(&7), "status dropped");
        assert!(!app.diffs.contains_key(&7), "diff dropped");
        assert!(!app.graphs.contains_key(&7), "graph dropped");
        assert!(!app.expanded_rows.contains(&7), "expansion dropped");
        assert_eq!(app.current, Some(9), "cursor falls to the next row");
        assert_eq!(app.issue_numbers(), vec![9]);
    }

    #[test]
    fn evict_refuses_pinned_rows() {
        let mut app = sidebar_app(&[7]);
        app.current = Some(7);
        app.pinned.insert(7);
        app.pin_order.push(7);
        app.evict_current();
        assert!(app.data.contains_key(&7), "pinned row stays");
        assert!(app.visited.contains(&7));
        assert_eq!(app.current, Some(7));
        assert!(app.message.contains("pinned"), "{}", app.message);
    }

    #[test]
    fn evict_falls_back_to_previous_row_at_the_end() {
        let mut app = sidebar_app(&[7, 9]);
        app.current = Some(9);
        app.evict_current();
        assert_eq!(app.current, Some(7));
        assert_eq!(app.issue_numbers(), vec![7]);
    }

    #[test]
    fn evicted_row_can_be_refetched_on_revisit() {
        let mut app = sidebar_app(&[7, 9]);
        app.current = Some(7);
        app.evict_current();
        assert!(!app.data.contains_key(&7), "cache dropped so a revisit fetches");
        assert!(!app.issue_pending.contains(&7), "no stale in-flight marker");
        app.ensure_visited(7);
        assert!(app.issue_numbers().contains(&7), "row is listable again");
        // The refetch lands through the normal poll path.
        let fresh = issue(7, "2026-01-06T00:00:00Z");
        app.issue_tx.send((7, Ok((fresh, vec![])))).unwrap();
        poll_issues(&mut app);
        assert!(app.data.contains_key(&7));
    }
}
