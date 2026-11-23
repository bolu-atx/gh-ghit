//! Diff and review-thread state and loads.

use std::collections::{HashMap, HashSet};

use crate::app::{App, spawn_bg};
use ghit_core::core::{self, Pane};
use ghit_core::{github, model};
use crate::app::auth_failure;
use ghit_core::model::ItemRef;

/// Diff tab state: fetched per-issue file lists, the line cursor and its
/// render bookkeeping, folded-file state, and the fetch triplet.
pub(crate) struct DiffView {
    pub(crate) files: HashMap<ItemRef, Vec<model::DiffFile>>,
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
    pub(crate) expanded_files: HashSet<(ItemRef, String)>,
    /// Issue numbers whose sidebar row shows its detail block. Keyed by
    /// number, so the state is sticky across cursor moves and shared by the
    /// Issues, PRs, and Refs panes.
    pub(crate) expanded_rows: HashSet<ItemRef>,
    /// Visible buffer rows of the diff file headers, in file order.
    pub(crate) headers: Vec<usize>,
    /// A `J`/`K` jump awaiting the next render: target file index and
    /// direction. Only render knows the body layout after the jump's own
    /// fold change, so it resolves the scroll offset (`finish_diff_jump`).
    pub(crate) pending_jump: Option<(usize, isize)>,
    pub(crate) pending: HashSet<ItemRef>,
    pub(crate) tx: std::sync::mpsc::Sender<(ItemRef, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(ItemRef, Result<Vec<model::DiffFile>, String>)>,
    pub(crate) split: bool,
    /// Expand-all mode, sticky across PRs once `a` set it: `Some(true)`
    /// opens every small file of each PR on first view, `Some(false)`
    /// folds them. `None` until the first press.
    pub(crate) expand_all: Option<bool>,
    /// PRs that already took the current expand-all mode. Cleared on
    /// each press so other PRs pick the new mode up on their next view.
    pub(crate) synced: HashSet<ItemRef>,
    /// Index into `model::CONTEXT_STEPS`: context lines around changes.
    pub(crate) context: usize,
    /// Patches as GitHub sent them; `files` holds them widened to `context`.
    pub(crate) raw: HashMap<ItemRef, Vec<model::DiffFile>>,
    /// Head-side file lines per (PR, filename), for widening. `None`
    /// marks a failed fetch, so it is not retried this session.
    pub(crate) heads: HashMap<(ItemRef, String), Option<Vec<String>>>,
    pub(crate) heads_pending: HashSet<(ItemRef, String)>,
    pub(crate) heads_tx: std::sync::mpsc::Sender<HeadFiles>,
    pub(crate) heads_rx: std::sync::mpsc::Receiver<HeadFiles>,
}

/// One head-files fetch result: the PR, the names asked for, and the bodies.
pub(crate) type HeadFiles = (
    ItemRef,
    Vec<String>,
    Result<Vec<(String, Result<String, String>)>, String>,
);

impl DiffView {
    pub(crate) fn new(
        tx: std::sync::mpsc::Sender<(ItemRef, Result<Vec<model::DiffFile>, String>)>,
        rx: std::sync::mpsc::Receiver<(ItemRef, Result<Vec<model::DiffFile>, String>)>,
    ) -> Self {
        let (heads_tx, heads_rx) = std::sync::mpsc::channel();
        Self { files: HashMap::new(), cursor: 0, line: None, touched: false,
            row_files: vec![], expanded_files: HashSet::new(), expanded_rows: HashSet::new(),
            headers: vec![], pending_jump: None, pending: HashSet::new(), tx, rx, split: false,
            expand_all: None, synced: HashSet::new(), context: 0, raw: HashMap::new(),
            heads: HashMap::new(), heads_pending: HashSet::new(), heads_tx, heads_rx }
    }
}

/// REST sub-issue loads for the opt-in sidebar tree: per-parent session
/// state (loading or loaded children) plus the fetch channel.
pub(crate) struct SubIssueLoads {
    pub(crate) state: HashMap<ItemRef, model::LazyChildren>,
    pub(crate) tx: std::sync::mpsc::Sender<(ItemRef, Result<Vec<model::IssueData>, String>)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(ItemRef, Result<Vec<model::IssueData>, String>)>,
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
        let count = files.len().min(ghit_core::core::MAX_DIFF_FILES);
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

    /// Files expand-all would open: shown, with a patch, not large.
    fn small_files(&self, n: ItemRef) -> Vec<&model::DiffFile> {
        let files = self.diff.files.get(&n).map(|f| f.as_slice()).unwrap_or(&[]);
        files.iter().take(ghit_core::core::MAX_DIFF_FILES).filter(|f| !f.is_large() && f.has_patch()).collect()
    }

    /// The next expand-all press opens files: some small file is folded.
    pub(crate) fn diff_will_expand(&self) -> bool {
        let Some(n) = self.nav.current else { return false };
        self.small_files(n).iter().any(|f| !self.diff.expanded_files.contains(&(n, f.filename.clone())))
    }

    /// Open every small file, or fold every file when all small files are
    /// already open. Like `J`/`K`, it keeps the file under the line cursor
    /// in view: it queues a jump to that file's header for the next render.
    pub(crate) fn diff_expand_all(&mut self) {
        let Some(n) = self.nav.current else { return };
        let count = self.diff.files.get(&n).map_or(0, |f| f.len().min(ghit_core::core::MAX_DIFF_FILES));
        if count == 0 {
            return;
        }
        self.nav.pane = Pane::Main;
        let fi = self.diff_file_at_cursor().unwrap_or(self.diff.cursor).min(count - 1);
        let expand = self.diff_will_expand();
        self.diff.expand_all = Some(expand);
        self.diff.synced.clear();
        self.diff.synced.insert(n);
        let msg = if expand {
            let names: Vec<String> = self.small_files(n).iter().map(|f| f.filename.clone()).collect();
            let large = count - names.len();
            let opened = names.len();
            self.diff.expanded_files.extend(names.into_iter().map(|name| (n, name)));
            let mut msg = format!("expanded {opened} files");
            if large > 0 {
                msg.push_str(&format!(" · {large} left folded"));
            }
            msg
        } else {
            self.diff.expanded_files.retain(|(i, _)| *i != n);
            "folded all files".to_string()
        };
        self.diff.cursor = fi;
        self.diff.touched = false;
        self.diff.pending_jump = Some((fi, if expand { 1 } else { -1 }));
        self.notify(msg);
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
        let count = self.diff.files.get(&n).map(|f| f.len().min(ghit_core::core::MAX_DIFF_FILES)).unwrap_or(0);
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
            // J/K already opened the target; expand-all's fold must stay folded.
            self.diff.touched = false;
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
            self.diff.touched = false;
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

    /// Step the context lines around each change through
    /// `model::CONTEXT_STEPS`, wrapping back to 3. Steps past 3 fetch the
    /// head side of open files (`sync_diff`) and widen once they land.
    pub(crate) fn cycle_diff_context(&mut self) {
        self.diff.context = (self.diff.context + 1) % model::CONTEXT_STEPS.len();
        let numbers: Vec<ItemRef> = self.diff.raw.keys().copied().collect();
        for n in numbers {
            self.rewiden(n);
        }
        self.diff.line = None;
        self.diff.touched = false;
        self.notify(format!("diff context: {}", self.diff_context_label()));
    }

    /// The context step as shown: a line count, or `all`.
    pub(crate) fn diff_context_label(&self) -> &'static str {
        ["3 lines", "5 lines", "25 lines", "100 lines", "all lines"][self.diff.context]
    }

    /// Rebuild PR `n`'s shown patches from its raw ones at the current
    /// context. Files without a fetched head side keep the raw patch.
    fn rewiden(&mut self, n: ItemRef) {
        let Some(raw) = self.diff.raw.get(&n) else { return };
        let ctx = model::CONTEXT_STEPS[self.diff.context];
        let files = raw.iter().map(|f| {
            let mut f = f.clone();
            if ctx > model::CONTEXT_STEPS[0] {
                if let Some(Some(lines)) = self.diff.heads.get(&(n, f.filename.clone())) {
                    f.patch = model::widen_patch(&f.patch, lines, ctx);
                }
            }
            f
        }).collect();
        self.diff.files.insert(n, files);
    }

    /// Bring the open PR's Diff tab in line with the sticky modes: apply
    /// the expand-all mode on its first view since the last press, and
    /// fetch the head side of open files when context exceeds 3. True
    /// when visible state changed.
    pub(crate) fn sync_diff(&mut self) -> bool {
        if !self.visual_diff() {
            return false;
        }
        let Some(n) = self.nav.current else { return false };
        if !self.diff.files.contains_key(&n) {
            return false;
        }
        let mut changed = false;
        if let Some(expand) = self.diff.expand_all {
            if self.diff.synced.insert(n) {
                if expand {
                    let names: Vec<String> = self.small_files(n).iter().map(|f| f.filename.clone()).collect();
                    self.diff.expanded_files.extend(names.into_iter().map(|name| (n, name)));
                } else {
                    self.diff.expanded_files.retain(|(i, _)| *i != n);
                }
                changed = true;
            }
        }
        if self.diff.context > 0 {
            let want: Vec<String> = self.diff.raw.get(&n).into_iter().flatten()
                .take(ghit_core::core::MAX_DIFF_FILES)
                .filter(|f| f.has_patch() && f.status != "removed")
                .map(|f| f.filename.clone())
                .filter(|name| {
                    let key = (n, name.clone());
                    self.diff.expanded_files.contains(&key)
                        && !self.diff.heads.contains_key(&key)
                        && !self.diff.heads_pending.contains(&key)
                })
                .collect();
            if !want.is_empty() {
                self.load_head_files(n, want);
            }
        }
        changed
    }

    /// Queue one fetch of the head side of `names` in PR `n`.
    fn load_head_files(&mut self, n: ItemRef, names: Vec<String>) {
        self.diff.heads_pending.extend(names.iter().map(|name| (n, name.clone())));
        let tx = self.diff.heads_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        self.enqueue(core::Kind::Diff, Some(n), async move {
            let result = github::fetch_head_files(&repo, n, names.clone()).await;
            let _ = tx.send((n, names, result));
        });
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
    pub(crate) fn load_diff(&mut self, number: ItemRef, force: bool) {
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

    /// Expand-time REST load for the opt-in tree: at most one fetch per
    /// parent per session, and only when the summary reports children
    /// the fetched rows do not cover. The row renders a loading child
    /// until `poll_sub_issues` lands the result.
    pub(crate) fn load_sub_issues(&mut self, number: ItemRef) {
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
        spawn_bg(async move {
            let result = github::submit_pending_review(&repo, n).await;
            let _ = tx.send((n, result));
        });
        self.notify("submitting pending review…".to_string());
    }
}

/// Land a pending-review submit: success refetches the issue so the
/// Diff tab shows submitted threads; failure just names the cause.
pub(crate) fn poll_review_submit(app: &mut App) -> bool {
    let ready: Vec<(ItemRef, Result<(), String>)> = app.compose.review_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        app.compose.submitting.remove(&n);
        match res {
            Ok(()) => {
                app.notify(format!(
                    "pending review submitted on {n}",
                    n = n.display("")
                ));
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
                // A refetch may move the head: drop its old file bodies.
                app.diff.heads.retain(|(i, _), _| *i != number);
                app.diff.raw.insert(number, files.clone());
                app.diff.files.insert(number, files);
            }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

/// Land head-side file bodies and re-widen their PR.
pub(crate) fn poll_head_files(app: &mut App) -> bool {
    let ready: Vec<HeadFiles> = app.diff.heads_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, names, result) in ready {
        for name in &names {
            app.diff.heads_pending.remove(&(n, name.clone()));
        }
        match result {
            Ok(bodies) => {
                for (name, body) in bodies {
                    let lines = body.ok().map(|b| b.lines().map(str::to_owned).collect());
                    app.diff.heads.insert((n, name), lines);
                }
            }
            Err(e) => {
                for name in names {
                    app.diff.heads.insert((n, name), None);
                }
                auth_failure(app, &e);
                app.notify(e);
            }
        }
        app.rewiden(n);
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
