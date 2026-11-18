//! Diff, graph and review-thread state and loads.

use std::collections::{HashMap, HashSet};

use crate::app::{App, rt};
use crate::core::{self, Pane};
use crate::{github, model};
use crate::app::auth_failure;

/// Diff tab state: fetched per-issue file lists, the line cursor and its
/// render bookkeeping, folded-file state, and the fetch triplet.
pub(crate) struct DiffView {
    pub(crate) files: HashMap<u64, Vec<model::DiffFile>>,
    pub(crate) cursor: usize,
    /// Active diff line as an absolute body-buffer row. `None` until the
    /// next render anchors it to the first file header. Steps one row at a
    /// time so wrapped rows just work; render resolves the owning file and
    /// auto-expands collapsed files from the fresh capture.
    pub(crate) line: Option<usize>,
    /// Set when input moved the line cursor: render then resolves the file
    /// and expands a landed-on collapsed file. Stays false for the lazy
    /// init anchor so opening a Diff never unfolds the first file.
    pub(crate) touched: bool,
    /// Owning file per visible body row, parallel to `body_rows` and only
    /// meaningful on the Diff tab. Rows before the first header map to
    /// `None`; blank/rule rows inherit the previous header's file.
    pub(crate) row_files: Vec<Option<usize>>,
    /// (issue, filename) pairs with unfolded hunks; the rest stays folded.
    pub(crate) expanded_files: HashSet<(u64, String)>,
    /// Issue numbers whose sidebar row shows its detail block. Keyed by
    /// number, so the state is sticky across cursor moves and shared by the
    /// Issues, PRs, and Refs panes.
    pub(crate) expanded_rows: HashSet<u64>,
    /// Visible buffer rows of the diff file headers, in file order.
    pub(crate) headers: Vec<usize>,
    /// A `J`/`K` jump awaiting the next render: target file index and
    /// direction. Only render knows the body layout after the jump's own
    /// fold change, so it resolves the scroll offset (`finish_diff_jump`).
    pub(crate) pending_jump: Option<(usize, isize)>,
    pub(crate) pending: HashSet<u64>,
    pub(crate) tx: std::sync::mpsc::Sender<(u64, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(u64, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) split: bool,
}

impl DiffView {
    pub(crate) fn new(
        tx: std::sync::mpsc::Sender<(u64, Result<Vec<model::DiffFile>, String>)>,
        rx: std::sync::mpsc::Receiver<(u64, Result<Vec<model::DiffFile>, String>)>,
    ) -> Self {
        Self { files: HashMap::new(), cursor: 0, line: None, touched: false,
            row_files: vec![], expanded_files: HashSet::new(), expanded_rows: HashSet::new(),
            headers: vec![], pending_jump: None, pending: HashSet::new(), tx, rx, split: false }
    }
}

/// Graph tab state: fetched per-issue graph payloads, the fetch triplet,
/// and the cursor into the current graph order.
pub(crate) struct GraphView {
    pub(crate) data: HashMap<u64, model::GraphData>,
    pub(crate) pending: HashSet<u64>,
    pub(crate) tx: std::sync::mpsc::Sender<(u64, Result<model::GraphData, String>)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(u64, Result<model::GraphData, String>)>,
    /// Cursor into `graph_order_numbers` for the current issue. Reset on
    /// issue switch; tab switches keep it so going back restores place.
    pub(crate) cursor: usize,
}

impl GraphView {
    pub(crate) fn new(
        tx: std::sync::mpsc::Sender<(u64, Result<model::GraphData, String>)>,
        rx: std::sync::mpsc::Receiver<(u64, Result<model::GraphData, String>)>,
    ) -> Self {
        Self { data: HashMap::new(), pending: HashSet::new(), tx, rx, cursor: 0 }
    }
}

/// REST sub-issue loads for the opt-in sidebar tree: per-parent session
/// state (loading or loaded children) plus the fetch channel.
pub(crate) struct SubIssueLoads {
    pub(crate) state: HashMap<u64, model::LazyChildren>,
    pub(crate) tx: std::sync::mpsc::Sender<(u64, Result<Vec<model::IssueData>, String>)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(u64, Result<Vec<model::IssueData>, String>)>,
}

impl SubIssueLoads {
    pub(crate) fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { state: HashMap::new(), tx, rx }
    }
}

impl App {
    /// Fold/unfold the file under the active diff line. No-op without
    /// loaded files. Folding acts on the body, so it takes body focus
    /// with it. Collapsing re-anchors the line cursor to the file header
    /// (the header row itself never moves); expanding leaves the cursor
    /// where it is because new rows only appear below the header.
    pub(crate) fn toggle_diff_file(&mut self) {
        self.set_diff_file(None);
    }

    /// Fold (`Some(false)`) or unfold (`Some(true)`) the file under the
    /// line cursor; `None` flips it. A file already in that state stays.
    pub(crate) fn set_diff_file(&mut self, open: Option<bool>) {
        let Some(n) = self.nav.current else { return };
        self.nav.pane = Pane::Main;
        let Some(files) = self.diff.files.get(&n) else { return };
        let count = files.len().min(crate::core::MAX_DIFF_FILES);
        if count == 0 {
            return;
        }
        let fi = self.diff_file_at_cursor().unwrap_or(self.diff.cursor).min(count - 1);
        self.diff.cursor = fi;
        self.diff.touched = false;
        let key = (n, files[fi].filename.clone());
        let is_open = self.diff.expanded_files.contains(&key);
        if open == Some(is_open) {
            return;
        }
        if is_open {
            self.diff.expanded_files.remove(&key);
            self.anchor_cursor_to_header(fi);
        } else {
            self.diff.expanded_files.insert(key);
        }
    }

    /// `J`/`K` jumps between diff file headers, landing the target header
    /// at the top of the viewport. The jump unfolds the target and records
    /// it in `diff.pending_jump`; the next render measures the header with
    /// the widget and width the body uses, so the landing matches exactly.
    /// Clamped at the first and last file.
    pub(crate) fn jump_diff_file(&mut self, delta: isize) {
        if !self.visual_diff() {
            return;
        }
        let Some(n) = self.nav.current else { return };
        let count = self.diff.files.get(&n).map(|f| f.len().min(crate::core::MAX_DIFF_FILES)).unwrap_or(0);
        if count == 0 {
            return;
        }
        let cur = match self.diff.pending_jump {
            Some((fi, _)) => fi,
            None => self.diff_file_at_cursor().unwrap_or(self.diff.cursor),
        }
        .min(count - 1);
        let next = (cur as isize + delta).clamp(0, count as isize - 1) as usize;
        if next == cur {
            return;
        }
        // Update the file state first: the summary names the cursor file,
        // so the measured offsets must come from the post-jump text.
        self.diff.cursor = next;
        self.nav.pane = Pane::Main;
        if let Some(files) = self.diff.files.get(&n) {
            if let Some(f) = files.get(next) {
                self.diff.expanded_files.insert((n, f.filename.clone()));
            }
        }
        self.diff.pending_jump = Some((next, delta));
    }

    /// Land a pending `J`/`K` jump on the header offset `render` measured.
    /// When render found none (cap hit, header cannot fit), fall back to
    /// the next visible header in this direction (headers run in file
    /// order, so it owns the adjacent file), else a page scroll toward it.
    /// The next render re-resolves the file from the line.
    pub(crate) fn finish_diff_jump(&mut self, delta: isize, offset: Option<u16>) {
        if let Some(off) = offset {
            self.nav.scroll = off;
            self.diff.line = Some(usize::from(off));
            self.diff.touched = true;
            return;
        }
        let base = usize::from(self.hits.body_base);
        let cur_vis = self.diff.line.map(|abs| abs.saturating_sub(base));
        let mut headers = self.diff.headers.clone();
        headers.sort_unstable();
        let target = if delta > 0 {
            cur_vis.and_then(|v| headers.iter().copied().find(|&h| h > v))
        } else {
            cur_vis.and_then(|v| headers.iter().copied().rev().find(|&h| h < v))
        };
        if let Some(h) = target {
            self.diff.line = Some(base + h);
            self.diff.touched = true;
        } else {
            let page = self.hits.body_area.height;
            if delta > 0 {
                self.nav.scroll = self.nav.scroll.saturating_add(page);
            } else {
                self.nav.scroll = self.nav.scroll.saturating_sub(page);
            }
        }
    }

    /// Owning file of the active diff line from the last render capture.
    /// `None` when the cursor is unset, stale, or on a pre-header row.
    pub(crate) fn diff_file_at_cursor(&self) -> Option<usize> {
        let abs = self.diff.line?;
        let vis = abs.checked_sub(self.hits.body_base as usize)?;
        self.diff.row_files.get(vis).copied().flatten()
    }

    /// Move the line cursor to file `fi`'s header when it is on screen.
    /// Used after collapsing: rows below the header vanish, so a cursor
    /// inside the old content would point at the wrong region.
    fn anchor_cursor_to_header(&mut self, fi: usize) {
        let base = self.hits.body_base as usize;
        let found = match self.nav.current.and_then(|n| self.diff.files.get(&n)) {
            Some(files) => self.hits.body_rows.iter().enumerate().find_map(|(vis, line)| {
                (model::header_file_index(line, files) == Some(fi)).then_some(vis)
            }),
            None => return,
        };
        if let Some(vis) = found {
            self.diff.line = Some(base + vis);
        }
    }

    /// Flip the Diff tab between unified and side-by-side rendering,
    /// back at the top so the new layout starts visible.
    pub(crate) fn toggle_diff_split(&mut self) {
        self.diff.split = !self.diff.split;
        self.diff.line = None;
        self.diff.touched = false;
        self.nav.scroll = 0;
        self.notify(if self.diff.split { "diff: split".to_string() } else { "diff: unified".to_string() });
    }

    /// Queue a diff fetch for PR `number`. No-op for issues, cached rows,
    /// and rows already in flight.
    pub(crate) fn load_diff(&mut self, number: u64, force: bool) {
        let Some(d) = self.issues.data.get(&number) else { return };
        if !d.is_pr {
            return;
        }
        if !force && self.diff.files.contains_key(&number) {
            return;
        }
        if !self.diff.pending.insert(number) {
            return;
        }
        let tx = self.diff.tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        self.enqueue(core::Kind::Diff, Some(number), async move {
            let _ = tx.send((number, github::fetch_diff(&repo, number, force).await));
        });
    }

    /// Queue a graph fetch for `number`. No-op for cached rows and rows
    /// already in flight. One cache entry per issue, so switching issues
    /// refetches per issue and the tab shows a skeleton meanwhile.
    pub(crate) fn load_graph(&mut self, number: u64, force: bool) {
        if !force && self.graph.data.contains_key(&number) {
            return;
        }
        if !self.graph.pending.insert(number) {
            return;
        }
        let tx = self.graph.tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        self.enqueue(core::Kind::Graph, Some(number), async move {
            let _ = tx.send((number, github::fetch_graph(&repo, number, force).await));
        });
    }

    /// Expand-time REST load for the opt-in tree: at most one fetch per
    /// parent per session, and only when the summary reports children
    /// the fetched rows do not cover. The row renders a loading child
    /// until `poll_sub_issues` lands the result.
    pub(crate) fn load_sub_issues(&mut self, number: u64) {
        if !self.sidebar.sub_issue_tree
            || !core::needs_lazy_children(number, &self.issues.data, &self.subs.state)
        {
            return;
        }
        self.subs.state.insert(number, model::LazyChildren::Loading);
        let tx = self.subs.tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        self.enqueue(core::Kind::Other, None, async move {
            let _ = tx.send((number, github::fetch_sub_issues(&repo, number).await));
        });
    }

    /// Flat navigation order for the graph tab: the center first, then
    /// neighbors section by section. Empty without a loaded graph.
    pub(crate) fn graph_order_numbers(&self) -> Vec<u64> {
        let Some(n) = self.nav.current else { return vec![] };
        let Some(data) = self.graph.data.get(&n) else { return vec![] };
        model::graph_order(data).into_iter().take(8).map(|i| data.nodes[i].number).collect()
    }

    /// Move the graph cursor, clamped at the ends. No-op without a graph.
    pub(crate) fn graph_move(&mut self, delta: isize) {
        let order = self.graph_order_numbers();
        if let Some(next) = core::graph_select(order.len(), Some(self.graph.cursor), delta) {
            self.graph.cursor = next;
        }
    }

    /// Open the graph node under the cursor in the viewer.
    pub(crate) fn open_graph_selection(&mut self) {
        let order = self.graph_order_numbers();
        if let Some(&n) = order.get(self.graph.cursor) {
            self.open_issue(n);
        }
    }

    /// Pending review threads on the current PR, if any.
    pub(crate) fn pending_threads(&self) -> usize {
        let Some(n) = self.nav.current else { return 0 };
        let Some(d) = self.issues.data.get(&n) else { return 0 };
        d.review_threads.iter().filter(|t| t.has_pending()).count()
    }

    /// Submit the viewer's pending review so pending line comments go
    /// live. No pending threads means a status-line note, never a send.
    pub(crate) fn submit_pending_review(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if self.compose.submitting.contains(&n) {
            return;
        }
        if self.pending_threads() == 0 {
            self.notify("no pending comments to submit".to_string());
            return;
        }
        self.compose.submitting.insert(n);
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        let tx = self.compose.review_tx.clone();
        rt().spawn(async move {
            let result = github::submit_pending_review(&repo, n).await;
            let _ = tx.send((n, result));
        });
        self.notify("submitting pending review…".to_string());
    }
}

/// Land a pending-review submit: success refetches the issue so the
/// Diff tab shows submitted threads; failure just names the cause.
pub(crate) fn poll_review_submit(app: &mut App) -> bool {
    let ready: Vec<(u64, Result<(), String>)> = app.compose.review_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        app.compose.submitting.remove(&n);
        match res {
            Ok(()) => {
                app.notify(format!("pending review submitted on #{n}"));
                app.load_issue(n, true);
            }
            Err(e) => {
                auth_failure(app, &e);
                app.notify(e);
            }
        }
    }
    landed
}

pub(crate) fn poll_diffs(app: &mut App) -> bool {
    let ready: Vec<_> = app.diff.rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, result) in ready {
        app.diff.pending.remove(&number);
        match result {
            Ok(files) => {
                app.diff.files.insert(number, files);
            }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

pub(crate) fn poll_graphs(app: &mut App) -> bool {
    let ready: Vec<_> = app.graph.rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, result) in ready {
        app.graph.pending.remove(&number);
        match result {
            Ok(data) => {
                app.graph.data.insert(number, data);
                if app.nav.current == Some(number) {
                    app.graph.cursor = app.graph.cursor.min(app.graph_order_numbers().len().saturating_sub(1));
                }
            }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

pub(crate) fn poll_sub_issues(app: &mut App) -> bool {
    let ready: Vec<_> = app.subs.rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (parent, result) in ready {
        match result {
            Ok(list) => {
                let mut numbers = Vec::with_capacity(list.len());
                for mut data in list {
                    numbers.push(data.number);
                    data.parent = Some(parent);
                    // An outside child is a slim row: opening it runs the
                    // full fetch. A number the query already fetched keeps
                    // its richer payload.
                    if !app.issues.data.contains_key(&data.number) {
                        app.issues.partial.insert(data.number);
                        app.issues.data.insert(data.number, data);
                    }
                }
                app.subs.state.insert(parent, model::LazyChildren::Loaded(numbers));
            }
            Err(e) => {
                // Drop the state so the next explicit expand can retry.
                app.subs.state.remove(&parent);
                auth_failure(app, &e);
                app.notify(e);
            }
        }
    }
    landed
}
