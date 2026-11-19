//! Focus, panes, history and layout: where the user is and how they move.

use std::collections::HashSet;

use ratatui::layout::Rect;

use crate::app::{App, Tab, Focus, NavIntent, Focused, Columns, open_in_browser};
use crate::core::{self, Layer, Pane};
use crate::model;

/// Navigation state: where the user is and how they got there. `current`
/// is the open issue, `requested`/`step_pending` are in-flight fetches,
/// `visited`/`recent` feed the sidebar sections, `history`/`future` are
/// the Ctrl+O/Ctrl+I stacks, `pinned`/`pin_order` are user pins, and
/// `tab`/`focus`/`pane`/`scroll`/`hover` place the cursor.
pub(crate) struct Nav {
    pub(crate) current: Option<u64>,
    pub(crate) requested: Option<u64>,
    /// h/l neighbor-step fetch awaiting its result. While set, a 404 for
    /// that number toasts "no #N" instead of the raw error; any other
    /// error keeps the existing error path. Cleared on landing.
    pub(crate) step_pending: Option<u64>,
    /// Numbers the user explicitly asked for or clicked, in first-seen
    /// order. Merely viewing a recent row never lands here.
    pub(crate) visited: Vec<u64>,
    /// Background-fetched recent rows (window listing or GitHub search),
    /// in fetch order.
    pub(crate) recent: Vec<u64>,
    /// Back stack for Ctrl+O (older issues, top = most recent) and the
    /// forward stack Ctrl+I redoes. Recorded on every explicit visit
    /// through `ensure_visited`; cursor walks never touch them.
    pub(crate) history: Vec<u64>,
    pub(crate) future: Vec<u64>,
    pub(crate) pinned: HashSet<u64>,
    pub(crate) pin_order: Vec<u64>,
    pub(crate) tab: Tab,
    pub(crate) focus: Focus,
    pub(crate) pane: Pane,
    pub(crate) scroll: u16,
    /// Wheel-selected sidebar row: transient preview, never a visit and
    /// never the active highlight. Click/Enter/j/k/h/l promote it to
    /// `current` and clear it; the highlight always follows `current`
    /// through `sync_selection`. While it moves, `current`, body scroll,
    /// and the other panes stay untouched.
    pub(crate) hover: Option<u64>,
    /// History pane cursor: steps back from the current issue. Zero is
    /// the current row; `history_steps` clamps it to the rows drawn.
    pub(crate) history_cursor: usize,
}

impl Nav {
    pub(crate) fn new() -> Self {
        Self { current: None, requested: None, step_pending: None, visited: vec![],
            recent: vec![], history: vec![], future: vec![], pinned: HashSet::new(),
            pin_order: vec![], tab: Tab::Details, focus: Focus::Main, pane: Pane::Issues,
            scroll: 0, hover: None, history_cursor: 0 }
    }
}

impl App {
    /// File `number` under requested rows. Call this only for explicit
    /// user actions (typing, clicking, search commit): background fetches
    /// and cursor movement must not relocate rows between groups. Every
    /// explicit visit also records the issue we leave on the back stack
    /// for Ctrl+O and drops the forward chain, so history follows exactly
    /// the rows filed here.
    pub(crate) fn ensure_visited(&mut self, number: u64) {
        self.nav.history = core::history_push(&self.nav.history, self.nav.current, number);
        self.nav.future.clear();
        self.file_visited(number);
    }

    /// File `number` under requested rows without touching history.
    /// History restores (Ctrl+O / Ctrl+I) use this: re-recording the
    /// jump would corrupt both stacks.
    fn file_visited(&mut self, number: u64) {
        if !self.nav.visited.contains(&number) {
            self.nav.visited.push(number);
        }
        self.sync_selection();
    }

    /// Row the sidebar lists track: `current` while it names a visible
    /// row. `current` is the single active item; both sidebar selections
    /// derive from it. The wheel hover never outranks it — hover stays a
    /// transient preview (j/k continue from it, a commit promotes it) and
    /// can never present as the active highlight.
    pub(crate) fn effective(&self, rows: &[model::SideRow]) -> Option<u64> {
        self.nav.current
            .filter(|n| rows.iter().any(|r| r.number() == Some(*n)))
    }

    /// Position of `current` in display rows, +1 for the top spacer row.
    fn selected_index(rows: &[model::SideRow], current: Option<u64>) -> Option<usize> {
        current
            .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
            .map(|p| p + 1)
    }

    pub(super) fn sync_selection(&mut self) {
        let issue_rows = self.issue_rows();
        let pr_rows = self.pr_rows();
        match self.nav.current {
            Some(n) if issue_rows.iter().any(|r| r.number() == Some(n)) => {
                self.sidebar.issue_state.select(Self::selected_index(&issue_rows, self.nav.current));
                self.sidebar.pr_state.select(None);
            }
            Some(n) if pr_rows.iter().any(|r| r.number() == Some(n)) => {
                self.sidebar.issue_state.select(None);
                self.sidebar.pr_state.select(Self::selected_index(&pr_rows, self.nav.current));
            }
            _ => {}
        }
    }

    pub(crate) fn show_issue(&mut self, n: u64) {
        self.nav.requested = Some(n);
        if self.issues.partial.contains(&n) { self.load_issue(n, false); }
        self.nav.current = Some(n);
        if self.nav.step_pending == Some(n) {
            self.nav.step_pending = None;
        }
        self.nav.scroll = 0;
        self.diff.cursor = 0;
        self.diff.line = None;
        self.diff.touched = false;
        self.refs.highlight = None;
        self.nav.hover = None;
        self.sync_selection();
        self.load_diff(n, false);
        self.refresh_refs();
    }

    /// Step back to the previously viewed issue (Ctrl+O). The current
    /// issue moves to the forward stack so Ctrl+I can redo it. Restores
    /// without recording, keeping both stacks intact.
    pub(crate) fn go_back(&mut self) {
        self.go_back_n(1);
    }

    pub(crate) fn go_back_n(&mut self, steps: usize) {
        if steps > 0 && !self.guard_nav(NavIntent::Back(steps)) {
            self.go_back_inner(steps);
        }
    }

    pub(super) fn go_back_inner(&mut self, steps: usize) {
        let count = steps.min(self.nav.history.len());
        if count == 0 {
            self.notify("no older issue".to_string());
            return;
        }
        let mut current = self.nav.current;
        for _ in 0..count {
            let prev = self.nav.history.pop().expect("count is clamped");
            if let Some(cur) = current {
                if self.nav.future.last() != Some(&cur) {
                    self.nav.future.push(cur);
                }
            }
            current = Some(prev);
        }
        if let Some(prev) = current {
            self.file_visited(prev);
            self.load_issue(prev, false);
        }
    }

    /// Redo a step back (Ctrl+I). The current issue returns to the back
    /// stack through the normal deduped push; the rest of the forward
    /// chain stays redoable.
    pub(crate) fn go_forward(&mut self) {
        if !self.guard_nav(NavIntent::Forward) {
            self.go_forward_inner();
        }
    }

    pub(super) fn go_forward_inner(&mut self) {
        let Some(next) = self.nav.future.pop() else {
            self.notify("no newer issue".to_string());
            return;
        };
        self.nav.history = core::history_push(&self.nav.history, self.nav.current, next);
        self.file_visited(next);
        self.load_issue(next, false);
    }

    /// Step to the neighboring issue (`h`/`l` on the Main pane): the
    /// explicit-visit path (`ensure_visited` + `load_issue`), the same
    /// call sequence as board Enter and body-link clicks. Below #1 there
    /// is nothing to fetch, so it toasts immediately; past the top end
    /// the fetch 404s and `poll_issues` toasts instead. Either way
    /// `current` only moves on a real landing.
    pub(crate) fn step_issue(&mut self, delta: isize) {
        if !self.guard_nav(NavIntent::Step(delta)) {
            self.step_issue_inner(delta);
        }
    }

    pub(super) fn step_issue_inner(&mut self, delta: isize) {
        let Some(cur) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let next = cur.saturating_add_signed(delta as i64);
        if next < 1 {
            self.notify(format!("no #{next}"));
            return;
        }
        self.nav.step_pending = Some(next);
        self.ensure_visited(next);
        self.load_issue(next, false);
    }

    /// Select a refs row: highlight where it is mentioned in the body
    /// instead of opening it. Clicking the same row again clears it.
    /// Tab cycles Issues → PRs → Refs, so the pane keeps focus until then.
    pub(crate) fn select_ref(&mut self, n: u64) {
        self.nav.hover = None;
        if self.refs.highlight == Some(n) {
            self.refs.highlight = None;
            return;
        }
        self.refs.highlight = Some(n);
        // Scroll the body pane so the first mentioning row sits at the top.
        // `body_rows` is the visible slice taken at `body_base`.
        let repo = self.repo_key();
        if let Some(row) = self.hits.body_rows.iter().position(|line| {
            model::links_in_line(line, repo.as_deref())
                .iter()
                .any(|(_, t)| *t == model::LinkTarget::Issue(n))
        }) {
            self.nav.scroll = self.hits.body_base.saturating_add(row as u16);
        }
    }

    /// The right refs pane shows only when toggled on and non-empty.
    pub(crate) fn refs_visible(&self) -> bool {
        self.refs.show && !(self.refs.numbers.is_empty() && self.refs.paired.is_empty())
    }

    /// The Labels pane shows once the repo has labels or milestones, or
    /// while the first fetch is still pending.
    pub(crate) fn labels_visible(&self) -> bool {
        self.refs.show
            && (self.sidebar.meta_rx.is_some()
                || !self.sidebar.labels.is_empty()
                || !self.sidebar.milestones.is_empty())
    }

    /// The right column shows when toggled on and it has Refs, History or
    /// the Labels pane.
    pub(crate) fn right_visible(&self) -> bool {
        !self.layout.right_cut && self.right_wanted()
    }

    /// The right column's content rule, before the terminal width has a say.
    fn right_wanted(&self) -> bool {
        self.refs.show
            && (self.refs_visible() || !self.nav.history.is_empty() || self.labels_visible())
    }

    /// Fit the side columns to a terminal `total` columns wide. Focus leaves
    /// a pane the width removed, and a column that newly goes away toasts
    /// once. Runs before each draw.
    pub(crate) fn apply_width(&mut self, total: u16) {
        let (sidebar, right) = core::columns(total, self.sidebar.show, self.right_wanted());
        let sidebar_cut = self.sidebar.show && sidebar == 0;
        let right_cut = self.right_wanted() && right == 0;
        let notice = match (sidebar_cut && !self.layout.sidebar_cut, right_cut && !self.layout.right_cut) {
            (true, _) => Some("narrow terminal: sidebar hidden"),
            (_, true) => Some("narrow terminal: right column hidden"),
            _ => None,
        };
        self.layout = Columns { sidebar, right, sidebar_cut, right_cut };
        if sidebar_cut && matches!(self.nav.pane, Pane::Issues | Pane::PRs) {
            self.nav.pane = Pane::Main;
        }
        if right_cut && matches!(self.nav.pane, Pane::Refs | Pane::Labels | Pane::History) {
            self.nav.pane = Pane::Main;
        }
        if let Some(msg) = notice {
            self.notify(msg.to_string());
        }
    }

    /// The sidebar is on screen: toggled on and not hidden by width.
    pub(crate) fn sidebar_visible(&self) -> bool {
        self.sidebar.show && !self.layout.sidebar_cut
    }

    /// Flip the left sidebar (`E`). Focus leaves the hidden panes.
    pub(crate) fn toggle_sidebar(&mut self) {
        self.sidebar.show = !self.sidebar.show;
        if !self.sidebar.show {
            if matches!(self.nav.pane, Pane::Issues | Pane::PRs) {
                self.nav.pane = Pane::Main;
            }
            self.hits.side_issue_area = Rect::default();
            self.hits.side_pr_area = Rect::default();
        }
        self.persist_prefs();
        self.notify(if self.sidebar.show { "sidebar: shown" } else { "sidebar: hidden" }.to_string());
    }

    /// The History pane shows under Refs while there is a back stack.
    pub(crate) fn history_visible(&self) -> bool {
        self.right_visible() && self.nav.current.is_some() && !self.nav.history.is_empty()
    }

    /// Rows the History pane draws above the current issue.
    pub(crate) fn history_rows_back(&self) -> usize {
        self.nav.history.len().min(4)
    }

    /// History cursor in steps back, clamped to the drawn rows.
    pub(crate) fn history_steps(&self) -> usize {
        self.nav.history_cursor.min(self.history_rows_back())
    }

    /// `j`/`k` in History: `j` moves toward the current issue.
    pub(crate) fn history_step(&mut self, delta: isize) {
        self.nav.history_cursor = self.history_steps()
            .saturating_add_signed(-delta)
            .min(self.history_rows_back());
    }

    /// Enter or click on a History row: go back to it, like `Ctrl+O`
    /// repeated. The current row is a no-op.
    pub(crate) fn open_history_row(&mut self) {
        let steps = self.history_steps();
        self.go_back_n(steps);
        self.nav.history_cursor = 0;
    }

    /// Top layer plus the active pane beneath it.
    pub(crate) fn focus(&self) -> Focused {
        let layer = self.layer_stack().last().copied().unwrap_or(Layer::Base);
        Focused { layer, pane: self.nav.pane }
    }

    /// Whether `pane` draws as focused: it is active and no dialog or
    /// text input sits above the panes, so one focus shows per frame.
    pub(crate) fn pane_focused(&self, pane: Pane) -> bool {
        let focus = self.focus();
        focus.layer == Layer::Base && focus.pane == pane
    }

    /// Visible panes in screen order.
    pub(crate) fn visible_panes(&self) -> Vec<Pane> {
        core::visible_panes(&core::PaneVisibility {
            sidebar: self.sidebar_visible(),
            refs: self.refs_visible() && self.right_visible(),
            labels: self.labels_visible() && self.right_visible(),
            history: self.history_visible(),
        })
    }

    /// Tab (`delta` 1) and Shift-Tab (-1) step through the visible panes in
    /// screen order: Issues, PRs, Main, Refs, Filters, History.
    pub(crate) fn cycle_pane(&mut self, delta: isize) {
        self.nav.pane = core::step_pane(&self.visible_panes(), self.nav.pane, delta);
    }

    /// `Esc`: pop the top layer. Each layer's own exit lives here so the
    /// key handlers and mouse paths share one definition. At the base it
    /// clears the bar and the refs highlight.
    pub(crate) fn pop_layer(&mut self) {
        use crate::core::{Dialog, TextInput};
        use crate::app::{Slot, FormatEdit};
        match self.focus().layer {
            Layer::Dialog(Dialog::Shortcuts) => { self.close(Slot::Shortcuts); }
            Layer::Dialog(Dialog::Confirm) => self.confirm_cancel(),
            Layer::Dialog(Dialog::Compose) => self.request_discard_compose(),
            Layer::Dialog(Dialog::Menu) => { self.close(Slot::Menu); }
            Layer::Dialog(Dialog::Finder) => { self.close(Slot::Finder); }
            Layer::Dialog(Dialog::Format) => { self.close(Slot::Format); }
            Layer::Dialog(Dialog::Physics) => { self.close(Slot::Physics); }
            Layer::Dialog(Dialog::MapFilter) => { self.close(Slot::MapFilter); }
            Layer::Dialog(Dialog::BoardFilter) => {
                self.close(crate::app::Slot::Format);
                self.board.filter_focus = 0;
            }
            Layer::Dialog(Dialog::Board) => self.close_board(),
            Layer::Input(TextInput::Query) => {
                self.sidebar.query_draft = self.sidebar.query_active.clone();
                self.set_format_edit(crate::app::FormatEdit::None);
            }
            Layer::Input(TextInput::BoardQuery) => {
                self.sidebar.query_draft = self.board.filter.clone();
                self.set_format_edit(crate::app::FormatEdit::None);
            }
            Layer::Input(TextInput::Name) => self.set_format_edit(FormatEdit::None),
            Layer::Input(TextInput::Bar) => {
                self.nav.focus = Focus::Main;
                self.sidebar.searching = false;
                self.bar.clear();
                self.sidebar.search.clear();
                self.search.clear();
            }
            Layer::Base => {
                self.bar.clear();
                self.sidebar.searching = false;
                self.sidebar.search.clear();
                self.search.clear();
                self.refs.highlight = None;
            }
        }
    }

    pub(crate) fn visual_diff(&self) -> bool {
        self.nav.tab == Tab::Visual && self.nav.current.and_then(|n| self.issues.data.get(&n)).is_some_and(|d| d.is_pr)
    }

    /// The Map tab shows, with or without an open item.
    pub(crate) fn map_shown(&self) -> bool {
        self.nav.tab == Tab::Map
    }

    /// Switch tabs, resetting scroll. Selecting Diff on a PR starts its
    /// background fetch so the pane fills in without blocking; first visit
    /// fetches, later visits reuse the cache. The Map tab reads the
    /// local copy each frame (`map_tick`).
    pub(crate) fn set_tab(&mut self, tab: Tab) {
        self.request_tab(tab);
    }

    pub(super) fn set_tab_inner(&mut self, tab: Tab) {
        self.nav.tab = tab;
        self.nav.scroll = 0;
        self.diff.line = None;
        self.diff.touched = false;
        if self.visual_diff() {
            if let Some(n) = self.nav.current {
                self.load_diff(n, false);
            }
        }
    }

    pub(crate) fn open_number(&mut self, n: u64) {
        let url = if let Some(d) = self.issues.data.get(&n) {
            d.url.clone()
        } else if let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) {
            format!("https://github.com/{repo}/issues/{n}")
        } else {
            format!("https://github.com/issues/{n}")
        };
        open_in_browser(&url).ok();
        self.notify(format!("opened #{n} in browser"));
    }

    pub(crate) fn submit_bar(&mut self) {
        match core::parse_bar(&self.bar.clone()) {
            core::BarCommand::Numbers(numbers) => {
                for n in numbers {
                    self.open_issue(n);
                    if self.confirm_open() {
                        break;
                    }
                }
            }
            core::BarCommand::OpenCurrent => {
                if let Some(n) = self.nav.current {
                    self.open_number(n);
                } else {
                    self.notify("view an issue first".to_string());
                }
            }
            core::BarCommand::Open(n) => self.open_number(n),
            core::BarCommand::Quit => {}
            core::BarCommand::Dismiss => {
                self.nav.focus = Focus::Main;
            }
            core::BarCommand::Hint => self.message = "type an issue number, o, or q".to_string(),
        }
    }

    /// User-initiated issue preview through the pending guard: the view
    /// switches without filing the row, so cursor walks and first-press
    /// previews never relocate rows between groups.
    pub(crate) fn request_issue(&mut self, n: u64) {
        if self.nav.current == Some(n) {
            self.show_issue(n);
            return;
        }
        if !self.guard_nav(NavIntent::Issue(n)) {
            self.show_issue(n);
        }
    }

    /// User-initiated explicit open through the pending guard: files the
    /// row under requested before switching, like the bar, board Enter,
    /// and paired-refs opens always did.
    pub(crate) fn open_issue(&mut self, n: u64) {
        if self.nav.current == Some(n) {
            self.show_issue(n);
            // Already viewing still completes the submit tail: the bar
            // clears and focus returns to Main, or later keys land as
            // bar text. Matches the cached fast path in `load_issue`.
            self.bar.clear();
            self.nav.focus = Focus::Main;
            return;
        }
        if !self.guard_nav(NavIntent::Issue(n)) {
            self.open_issue_inner(n);
        }
    }

    pub(super) fn open_issue_inner(&mut self, n: u64) {
        self.ensure_visited(n);
        self.load_issue(n, false);
    }

    /// User-initiated tab switch through the pending guard.
    pub(crate) fn request_tab(&mut self, tab: Tab) {
        if self.nav.tab == tab {
            return;
        }
        if !self.guard_nav(NavIntent::Tab(tab)) {
            self.set_tab_inner(tab);
        }
    }
}
