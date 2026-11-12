//! App: state, transitions, event loop, background service wiring.
//!
//! `App` owns the data (issues, pins, board, channels) and coordinates
//! fetching through `github`. It never draws (`ui`) and never parses input
//! (`input`); pure rules live in `core`. Network work runs on one shared
//! tokio runtime and lands via channels polled once per frame, so the loop
//! never blocks.

use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::time::{Duration, Instant};

const RECENT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const RECENT_REFRESH_MAX_INTERVAL: Duration = Duration::from_secs(60);

/// Blocking poll wait per frame: short enough that batched mouse/key
/// input feels instant, long enough to avoid a busy loop.
pub(crate) const POLL_TIMEOUT_MS: u64 = 15;
/// Click-flash dwell: the clicked rect inverts for ~150ms so the press
/// reads as acknowledged before its fetch lands.
pub(crate) const CLICK_FLASH_MS: u64 = 150;
/// Spinner frame dwell: while a fetch runs the loop repaints at this
/// cadence for the animation, not on every 15ms poll pass.
pub(crate) const SPIN_FRAME_MS: u64 = 100;

/// Auto-refresh cadence, overridable for deterministic runs: the e2e
/// harness stretches it past any scenario lifetime so each scenario
/// issues exactly one sections fetch per boot instead of hammering
/// the API every 5s. Zero or garbage falls back to the default.
fn auto_refresh_interval_default() -> Duration {
    std::env::var("GHIT_AUTO_REFRESH_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(RECENT_REFRESH_INTERVAL)
}

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::ExecutableCommand;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use crate::{cache, core, github, model};
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

/// Content tab titles with their `1`/`2` shortcuts. The mouse router
/// splits on the first title plus the divider; `TAB_SPLIT_X` stays in
/// sync by construction — see `tab_split_matches_first_title_width`.
pub(crate) fn tab_titles(visual_label: &str) -> [String; 2] {
    [format!("\u{f15c}  1 Details"), format!("\u{f121}  2 {visual_label}")]
}

/// Click columns before this select Details: first tab title plus divider.
pub(crate) const TAB_SPLIT_X: u16 = 14;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Main,
    Bar,
}

/// Deferred user navigation, stored when a confirm dialog interrupts
/// it. Variants recompute at execution time so cancelling leaves
/// history, selection, and caches untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NavIntent {
    Issue(u64),
    Tab(Tab),
    Step(isize),
    Back,
    Forward,
    Evict,
}

/// One button of a generic confirm dialog: the key that picks it plus
/// its label. `Esc` always cancels and never needs a listed choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmChoice {
    pub(crate) key: char,
    pub(crate) label: String,
}

/// What a generic confirm dialog decides. The renderer and key handling
/// only see title/body/choices; each variant owns its choice indexes
/// in `confirm_choose`. New uses add a variant plus its constructor
/// and one dispatch arm — no renderer or input changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmKind {
    /// Leaving a PR with unsubmitted review comments. Choice 0 submits
    /// then navigates, choice 1 navigates leaving them pending, choice
    /// 2 (and `Esc`) stays put.
    PendingNav { intent: NavIntent },
}

/// Generic confirm modal: title, message lines, and explicit choices.
/// Specializations build one with a constructor like
/// `open_pending_nav` and interpret choices in `confirm_choose`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfirmDialog {
    pub(crate) title: String,
    pub(crate) body: Vec<String>,
    pub(crate) choices: Vec<ConfirmChoice>,
    pub(crate) kind: ConfirmKind,
}

/// Pending auth-scope question: the sentence that triggered the prompt
/// plus the missing scopes. `scopes_known` is true only when a probe
/// read the granted scopes; fetch failures cannot name the gap, so a
/// missing list there means unknown, not clean.
#[derive(Clone, Debug)]
pub(crate) struct AuthPrompt {
    pub(crate) missing: Vec<String>,
    pub(crate) cause: String,
    pub(crate) scopes_known: bool,
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

/// Where a composed comment lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ComposeTarget {
    /// Plain issue/PR thread comment.
    Thread,
    /// Single-shot PR line comment: file plus new-side line.
    Line { path: String, line: u64 },
    /// Reply inside an existing review thread: the cursor sits in its
    /// rendered block on the Diff tab.
    Reply { thread_id: String },
}

impl ComposeTarget {
    pub(crate) fn title(&self, number: u64) -> String {
        match self {
            ComposeTarget::Thread => format!("Comment · thread #{number}"),
            ComposeTarget::Line { path, line } => format!("Comment · {path}:{line}"),
            ComposeTarget::Reply { .. } => format!("Reply · #{number}"),
        }
    }
}

/// The compose modal: a draft plus where it goes. `cursor` is a char
/// index into `body` where typing lands; `sending` locks editing while
/// the background submit is in flight. `kill` is the readline kill ring
/// (one slot): kills stash text here, `Ctrl+Y` pastes it back.
pub(crate) struct ComposeState {
    pub(crate) number: u64,
    pub(crate) target: ComposeTarget,
    pub(crate) body: String,
    pub(crate) cursor: usize,
    pub(crate) sending: bool,
    pub(crate) kill: String,
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
    pub(crate) section_rx: Option<std::sync::mpsc::Receiver<github::SectionEvent>>,
    last_sections_fetch: Instant,
    auto_refresh_interval: Duration,
    section_auto_refresh: bool,
    /// Section page progress: rows fetched vs the row cap, while pages
    /// remain. Cleared when the terminal result lands.
    pub(crate) section_progress: Option<(usize, usize)>,
    pub(crate) partial: HashSet<u64>,
    pub(crate) issue_tx: std::sync::mpsc::Sender<(u64, github::IssueEvent)>,
    pub(crate) issue_rx: std::sync::mpsc::Receiver<(u64, github::IssueEvent)>,
    pub(crate) issue_pending: HashSet<u64>,
    /// Comment-page progress per issue: pages done vs total. Cleared when
    /// the terminal result lands.
    pub(crate) issue_progress: HashMap<u64, (usize, usize)>,
    pub(crate) requested: Option<u64>,
    /// h/l neighbor-step fetch awaiting its result. While set, a 404 for
    /// that number toasts "no #N" instead of the raw error; any other
    /// error keeps the existing error path. Cleared on landing.
    pub(crate) step_pending: Option<u64>,
    pub(crate) data: HashMap<u64, IssueData>,
    pub(crate) statuses: HashMap<u64, Vec<(String, Option<String>)>>,
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
    /// Early auth probe in flight (`bootstrap` started it).
    pub(crate) auth_rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// Pending re-auth question: the token is dead or lacks scopes.
    /// A modal until the user answers; `y` runs `gh auth refresh`.
    pub(crate) auth_prompt: Option<AuthPrompt>,
    /// Generic confirm modal plus its deferred decision. While set, all
    /// keys and clicks route to the dialog until it resolves.
    pub(crate) confirm: Option<ConfirmDialog>,
    /// Set by input; the event loop runs the refresh outside the TUI.
    pub(crate) reauth_requested: bool,
    /// The user dismissed the prompt: later auth failures notify only,
    /// never re-modal. Cleared by a successful re-auth.
    pub(crate) auth_dismissed: bool,
    /// The repo-scoped access probe fired once the repo resolved.
    pub(crate) auth_repo_probed: bool,
    pub(crate) board_states: Vec<ListState>,
    pub(crate) board_expanded: Option<(usize, usize)>,
    /// Board text filter (`f` while the board is open): label, card title,
    /// or `#num`. Milestones filter through the pills below, never this
    /// text. Empty shows every card.
    pub(crate) board_filter: String,
    /// Board milestone pills: the selected milestones. Empty selects every
    /// card; `None` is the "(no milestone)" pill.
    pub(crate) board_milestones: HashSet<Option<String>>,
    /// Active widget in the board filter dialog: 0 is the text row, the
    /// rest are milestone pills in `model::board_milestones` order.
    pub(crate) filter_focus: usize,
    /// Milestone draft edited in the dialog; Enter applies it.
    pub(crate) filter_milestone_draft: HashSet<Option<String>>,
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
    /// Wheel-selected sidebar row: transient preview, never a visit and
    /// never the active highlight. Click/Enter/j/k/h/l promote it to
    /// `current` and clear it; the highlight always follows `current`
    /// through `sync_selection`. While it moves, `current`, body scroll,
    /// and the other panes stay untouched.
    pub(crate) hover: Option<u64>,
    pub(crate) refs_area: Rect,
    pub(crate) ref_rows: Vec<model::SideRow>,
    pub(crate) refs_state: ListState,
    pub(crate) tabs_area: Rect,
    pub(crate) bar_area: Rect,
    pub(crate) board_cells: Vec<(Rect, usize, usize)>,
    /// Board header link hit areas (top-right icons): rect plus the URL
    /// to open. Rebuilt every frame in `render_board`, like `board_cells`.
    pub(crate) board_links: Vec<(Rect, String)>,
    /// Click flash: the last clicked rect plus when it landed. Render
    /// inverts it for `CLICK_FLASH_MS`, then clears it. Theme-agnostic
    /// by construction (reversed style, no colors).
    pub(crate) last_click: Option<(Rect, Instant)>,
    // Compose modal (`c`) plus its background submit channel.
    pub(crate) compose: Option<ComposeState>,
    /// Set by input; the event loop suspends the TUI and runs `$EDITOR`
    /// on the draft outside it, then lands the edited text.
    pub(crate) editor_requested: bool,
    pub(crate) compose_tx: std::sync::mpsc::Sender<(u64, Result<(), String>)>,
    pub(crate) compose_rx: std::sync::mpsc::Receiver<(u64, Result<(), String>)>,
    // Pending review submit (`S`): PRs with pending line comments, plus
    // its background channel. One in-flight submit per PR.
    pub(crate) review_submit_tx: std::sync::mpsc::Sender<(u64, Result<(), String>)>,
    pub(crate) review_submit_rx: std::sync::mpsc::Receiver<(u64, Result<(), String>)>,
    pub(crate) review_submitting: HashSet<u64>,
    // Board modal.
    pub(crate) board: Option<BoardData>,
    /// Frame counter, bumped once per event-loop pass. Drives the status
    /// bar spinner; cheap because the loop already redraws every pass.
    pub(crate) tick: u64,
    pub(crate) board_col: usize,
    pub(crate) board_row: usize,
}

impl App {
    pub(crate) fn new(repo_flag: Option<String>) -> Self {
        let (detail_tx, detail_rx) = std::sync::mpsc::channel();
        let (compose_tx, compose_rx) = std::sync::mpsc::channel();
        let (review_submit_tx, review_submit_rx) = std::sync::mpsc::channel();
        let (issue_tx, issue_rx) = std::sync::mpsc::channel();
        let (diff_tx, diff_rx) = std::sync::mpsc::channel();
        let (graph_tx, graph_rx) = std::sync::mpsc::channel();
        Self {
            repo_flag,
            repo_slug: None,
            bootstrap_rx: None,
            initial_numbers: vec![],
            section_rx: None,
            last_sections_fetch: Instant::now(),
            auto_refresh_interval: auto_refresh_interval_default(),
            section_auto_refresh: false,
            section_progress: None,
            partial: HashSet::new(),
            issue_tx, issue_rx, issue_pending: HashSet::new(), issue_progress: HashMap::new(), requested: None, step_pending: None,
            data: HashMap::new(),
            statuses: HashMap::new(),
            visited: vec![],
            recent: vec![],
            history: vec![],
            future: vec![],
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
            auth_rx: None,
            auth_prompt: None,
            confirm: None,
            reauth_requested: false,
            auth_dismissed: false,
            auth_repo_probed: false,
            compose: None,
            editor_requested: false,
            compose_tx,
            compose_rx,
            review_submit_tx,
            review_submit_rx,
            review_submitting: HashSet::new(),
            tick: 0,
            board_col: 0,
            board_row: 0,
            board_loading: false,
            board_rx: None,
            board_states: Vec::new(),
            board_expanded: None,
            board_filter: String::new(),
            board_milestones: HashSet::new(),
            filter_focus: 0,
            filter_milestone_draft: HashSet::new(),
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
            hover: None,
            refs_area: Rect::default(),
            ref_rows: vec![],
            refs_state: ListState::default(),
            tabs_area: Rect::default(),
            bar_area: Rect::default(),
            board_cells: Vec::new(),
            board_links: Vec::new(),
            last_click: None,
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

    /// Record a click flash: the pressed rect inverts on the next frames
    /// until `CLICK_FLASH_MS` elapses.
    pub(crate) fn note_click(&mut self, area: Rect) {
        self.last_click = Some((area, Instant::now()));
    }

    /// The flash still owes frames: visible now, or expired and owed one
    /// last draw to clear it.
    pub(crate) fn flash_live(&self) -> bool {
        self.last_click.is_some()
    }

    /// The flash currently paints: inside `CLICK_FLASH_MS`.
    pub(crate) fn flash_visible(&self) -> bool {
        matches!(self.last_click, Some((_, at)) if at.elapsed() < Duration::from_millis(CLICK_FLASH_MS))
    }

    /// Anything animates the status bar: a live fetch or a visible
    /// message. While true the loop repaints on the spinner cadence;
    /// while false an idle frame can skip the draw.
    pub(crate) fn spinner_active(&self) -> bool {
        self.message_visible() || !crate::core::loading(&self.fetch_state()).is_empty()
    }

    /// The spinner owes a repaint: active, and the last spinner paint
    /// is older than one frame dwell. Pure in its inputs so tests pin
    /// the pacing without a terminal.
    pub(crate) fn spin_owed(&self, last_paint: Instant) -> bool {
        self.spinner_active() && last_paint.elapsed() >= Duration::from_millis(SPIN_FRAME_MS)
    }

    /// Plain fetch snapshot for the status bar spinner. The pure `core`
    /// derivation decides what shows; this only gathers the counts.
    pub(crate) fn fetch_state(&self) -> core::FetchState {
        // Concurrent comment fetches report separately: sum what is known.
        let (done, total) = self.issue_progress.values().copied().fold((0, 0), |(d, t), (dd, tt)| (d + dd, t + tt));
        core::FetchState {
            issues: self.issue_pending.len(),
            diffs: self.diff_pending.len(),
            graphs: self.graph_pending.len(),
            details: self.detail_pending.len(),
            sections_live: self.section_rx.is_some(),
            bootstrap_live: self.bootstrap_rx.is_some(),
            board: self.board_loading,
            message_visible: self.message_visible(),
            issue_progress: if self.issue_progress.is_empty() { None } else { Some((done, total)) },
            section_progress: self.section_progress,
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
    /// and cursor movement must not relocate rows between groups. Every
    /// explicit visit also records the issue we leave on the back stack
    /// for Ctrl+O and drops the forward chain, so history follows exactly
    /// the rows filed here.
    pub(crate) fn ensure_visited(&mut self, number: u64) {
        self.history = core::history_push(&self.history, self.current, number);
        self.future.clear();
        self.file_visited(number);
    }

    /// File `number` under requested rows without touching history.
    /// History restores (Ctrl+O / Ctrl+I) use this: re-recording the
    /// jump would corrupt both stacks.
    fn file_visited(&mut self, number: u64) {
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

    /// Startup hydration: paint the last persisted sidebar instantly so a
    /// cold start shows previous state. The background refresh landing in
    /// `poll_sections` updates these rows in place, so hydration only
    /// changes when rows first appear. Missing or expired cache is a
    /// no-op and startup behaves as before. Startup-only: refreshes must
    /// never resurrect evicted rows, so nothing else calls this.
    fn hydrate_cached_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        let Some(rows) = github::cached_sections(&repo, self.merged_window_hours) else { return };
        for row in rows {
            let n = row.issue.number;
            if self.data.contains_key(&n) {
                continue;
            }
            if !row.complete {
                self.partial.insert(n);
            }
            self.data.insert(n, row.issue);
            self.statuses.insert(n, row.statuses);
            self.board_details.insert(n, row.detail);
            self.ensure_recent(n);
        }
    }

    /// Pull recent issues plus open/windowed-merged PRs into the sidebar.
    /// Offline or out of a repo this is a no-op: clicked rows still list.
    pub(crate) fn merge_recent_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        self.last_sections_fetch = Instant::now();
        self.section_auto_refresh = false;
        let hours = self.merged_window_hours;
        let (tx, rx) = std::sync::mpsc::channel();
        self.section_rx = Some(rx);
        rt().spawn(async move {
            let result = github::fetch_recent_sections(&repo, hours, Some(tx.clone())).await;
            let _ = tx.send(github::SectionEvent::Done(result));
        });
    }

    fn auto_refresh_recent(&mut self, now: Instant) {
        if now.duration_since(self.last_sections_fetch) < self.auto_refresh_interval
            || self.section_rx.is_some()
            || board_open(self)
            || self.compose.is_some()
            || self.confirm.is_some()
            || self.filter_open
            || self.shortcuts_open
            || self.focus == Focus::Bar
        {
            return;
        }
        self.merge_recent_sections();
        if self.section_rx.is_some() {
            self.section_auto_refresh = true;
        }
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
        self.last_sections_fetch = Instant::now();
        self.section_auto_refresh = false;
        self.section_replace = true;
        self.notify(format!("searching: {query}"));
        let (tx, rx) = std::sync::mpsc::channel();
        self.section_rx = Some(rx);
        rt().spawn(async move {
            // Server search is a single request: indeterminate, no progress.
            let _ = tx.send(github::SectionEvent::Done(github::search_repo(&repo, &query).await));
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
        self.issue_progress.remove(&n);
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
        if !self.guard_nav(NavIntent::Evict) {
            self.evict_current_inner();
        }
    }

    fn evict_current_inner(&mut self) {
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
            let result = github::fetch_number(&repo, number, force, Some(tx.clone())).await;
            let _ = tx.send((number, github::IssueEvent::Done(result)));
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
            self.open_issue(n);
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

    /// Filter context for the recent group: window label, or the active
    /// search query. Renders in the PR footer, never the title, so all
    /// pane titles share one shape.
    pub(crate) fn filter_context(&self) -> String {
        if self.query_active.trim().is_empty() {
            self.window_label()
        } else {
            format!("\"{}\"", self.query_active)
        }
    }

    /// Group counts for one sidebar pane, matching the displayed rows.
    pub(crate) fn side_counts(&self, want_pr: bool) -> (usize, usize, usize) {
        let (p, v, r) = self.pane_groups(want_pr, self.searching);
        (p.len(), v.len(), r.len())
    }

    /// Title subtitle for Issues/PRs: total rows plus the sort state.
    pub(crate) fn side_subtitle(&self, want_pr: bool) -> String {
        let (p, v, r) = self.side_counts(want_pr);
        format!("{} · {}", p + v + r, self.sort_label())
    }

    /// Footer for Issues/PRs: per-group counts. PRs append the filter
    /// context after the counts, so narrow panes clip context first.
    pub(crate) fn side_footer(&self, want_pr: bool) -> String {
        let (p, v, r) = self.side_counts(want_pr);
        let base = format!("pinned ({p}) · requested ({v}) · recent ({r})");
        if want_pr {
            format!("{} · {}", base, self.filter_context())
        } else {
            base
        }
    }

    /// Title subtitle for Refs: mention/pair counts (Refs has no sort).
    pub(crate) fn refs_subtitle(&self) -> String {
        if self.ref_paired.is_empty() {
            format!("{} mentioned", self.ref_numbers.len())
        } else {
            format!(
                "{} mentioned · {} paired",
                self.ref_numbers.len(),
                self.ref_paired.len()
            )
        }
    }

    /// Footer for Refs: per-group counts matching the subtitle.
    pub(crate) fn refs_footer(&self) -> String {
        format!(
            "mentioned ({}) · paired ({})",
            self.ref_numbers.len(),
            self.ref_paired.len()
        )
    }

    /// Row the sidebar lists track: `current` while it names a visible
    /// row. `current` is the single active item; both sidebar selections
    /// derive from it. The wheel hover never outranks it — hover stays a
    /// transient preview (j/k continue from it, a commit promotes it) and
    /// can never present as the active highlight.
    pub(crate) fn effective(&self, rows: &[model::SideRow]) -> Option<u64> {
        self.current
            .filter(|n| rows.iter().any(|r| r.number() == Some(*n)))
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
        if self.step_pending == Some(n) {
            self.step_pending = None;
        }
        self.scroll = 0;
        self.diff_cursor = 0;
        self.diff_line = None;
        self.diff_touched = false;
        self.ref_highlight = None;
        self.hover = None;
        self.graph_cursor = 0;
        self.sync_selection();
        self.load_diff(n, false);
        if self.visual_graph() {
            self.load_graph(n, false);
        }
        self.refresh_refs();
    }

    /// Step back to the previously viewed issue (Ctrl+O). The current
    /// issue moves to the forward stack so Ctrl+I can redo it. Restores
    /// without recording, keeping both stacks intact.
    pub(crate) fn go_back(&mut self) {
        if !self.guard_nav(NavIntent::Back) {
            self.go_back_inner();
        }
    }

    fn go_back_inner(&mut self) {
        let Some(prev) = self.history.pop() else {
            self.notify("no older issue".to_string());
            return;
        };
        if let Some(cur) = self.current {
            if self.future.last() != Some(&cur) {
                self.future.push(cur);
            }
        }
        self.file_visited(prev);
        self.load_issue(prev, false);
    }

    /// Redo a step back (Ctrl+I). The current issue returns to the back
    /// stack through the normal deduped push; the rest of the forward
    /// chain stays redoable.
    pub(crate) fn go_forward(&mut self) {
        if !self.guard_nav(NavIntent::Forward) {
            self.go_forward_inner();
        }
    }

    fn go_forward_inner(&mut self) {
        let Some(next) = self.future.pop() else {
            self.notify("no newer issue".to_string());
            return;
        };
        self.history = core::history_push(&self.history, self.current, next);
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

    fn step_issue_inner(&mut self, delta: isize) {
        let Some(cur) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let next = cur.saturating_add_signed(delta as i64);
        if next < 1 {
            self.notify(format!("no #{next}"));
            return;
        }
        self.step_pending = Some(next);
        self.ensure_visited(next);
        self.load_issue(next, false);
    }

    /// Select a refs row: highlight where it is mentioned in the body
    /// instead of opening it. Clicking the same row again clears it.
    /// Tab cycles Issues → PRs → Refs, so the pane keeps focus until then.
    pub(crate) fn select_ref(&mut self, n: u64) {
        self.hover = None;
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

    /// `J`/`K` jumps between diff file headers, landing the target header
    /// at the top of the viewport. The offset is measured with the same
    /// widget and width the body renders, so the landing matches exactly;
    /// past the scan cap it falls back to the next visible header, then to
    /// a page scroll. Clamped at the first and last file.
    pub(crate) fn jump_diff_file(&mut self, delta: isize) {
        if !self.visual_diff() {
            return;
        }
        let Some(n) = self.current else { return };
        let count = self.diffs.get(&n).map(|f| f.len().min(crate::ui::MAX_DIFF_FILES)).unwrap_or(0);
        if count == 0 {
            return;
        }
        let cur = self.diff_file_at_cursor().unwrap_or(self.diff_cursor).min(count - 1);
        let next = (cur as isize + delta).clamp(0, count as isize - 1) as usize;
        if next == cur {
            return;
        }
        // Update the file state first: the summary names the cursor file,
        // so the measured offsets must come from the post-jump text.
        self.diff_cursor = next;
        self.pane = Pane::Main;
        if let Some(files) = self.diffs.get(&n) {
            if let Some(f) = files.get(next) {
                self.expanded_files.insert((n, f.filename.clone()));
            }
        }
        if let Some(off) = crate::ui::diff_header_offset(self, next, delta) {
            self.scroll = off;
            self.diff_line = Some(usize::from(off));
            self.diff_touched = true;
            return;
        }
        // Fallback: the next visible header in this direction (headers run
        // in file order, so it owns the adjacent file), else a page scroll
        // toward it. The next render re-resolves the file from the line.
        let base = usize::from(self.body_base);
        let cur_vis = self.diff_line.map(|abs| abs.saturating_sub(base));
        let mut headers = self.diff_headers.clone();
        headers.sort_unstable();
        let target = if delta > 0 {
            cur_vis.and_then(|v| headers.iter().copied().find(|&h| h > v))
        } else {
            cur_vis.and_then(|v| headers.iter().copied().rev().find(|&h| h < v))
        };
        if let Some(h) = target {
            self.diff_line = Some(base + h);
            self.diff_touched = true;
        } else {
            let page = self.body_area.height;
            if delta > 0 {
                self.scroll = self.scroll.saturating_add(page);
            } else {
                self.scroll = self.scroll.saturating_sub(page);
            }
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
        self.open_issue(n);
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
                let result = github::fetch_number(&repo, m, false, Some(tx.clone())).await;
                let _ = tx.send((m, github::IssueEvent::Done(result)));
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
        self.request_tab(tab);
    }

    fn set_tab_inner(&mut self, tab: Tab) {
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
            self.open_issue(n);
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
                                model::strip_diff_gutter(line),
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
                    self.open_issue(n);
                    if self.confirm_open() {
                        break;
                    }
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
        self.board_filter.clear();
        self.board_milestones.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        self.board_rx = Some(rx);
        // `gh` subprocesses block: keep them off the async workers.
        rt().spawn_blocking(move || {
            let _ = tx.send(board_via_gh(&owner));
        });
    }

    /// Open the filter dialog against the board cards. It opens in
    /// navigate mode with the active text and pills prefilled; the board
    /// stays up behind it.
    pub(crate) fn open_board_filter(&mut self) {
        self.query_draft = self.board_filter.clone();
        self.filter_milestone_draft = self.board_milestones.clone();
        self.filter_focus = 0;
        self.query_edit = false;
        self.filter_open = true;
    }

    /// Apply the dialog drafts and keep the cursor inside the narrowed
    /// grid. Positions move under a new filter: never keep a stale
    /// expansion.
    pub(crate) fn apply_board_filter(&mut self) {
        self.board_filter = self.query_draft.trim().to_string();
        self.board_milestones = self.filter_milestone_draft.clone();
        self.board_expanded = None;
        self.clamp_board_cursor();
    }

    pub(crate) fn clear_board_filter(&mut self) {
        self.board_filter.clear();
        self.board_milestones.clear();
        self.board_expanded = None;
        self.clamp_board_cursor();
    }

    /// True when any board filter is active: text or milestone pills.
    pub(crate) fn board_filtered(&self) -> bool {
        !self.board_filter.trim().is_empty() || !self.board_milestones.is_empty()
    }

    /// Milestone pills for the dialog, in display order.
    pub(crate) fn board_pills(&self) -> Vec<Option<String>> {
        match &self.board {
            Some(board) => model::board_milestones(&board.items),
            None => vec![],
        }
    }

    /// Step the dialog focus across the text row plus pills, wrapping.
    pub(crate) fn board_filter_step(&mut self, delta: isize) {
        let n = self.board_pills().len() + 1;
        self.filter_focus = (self.filter_focus as isize + delta).rem_euclid(n as isize) as usize;
    }

    fn clamp_board_cursor(&mut self) {
        let Some(board) = &self.board else { return };
        let cols = board_columns(board, &self.board_filter, &self.board_milestones);
        let col = self.board_col.min(cols.len().saturating_sub(1));
        let rows = cols.get(col).map(|(_, cards)| cards.len()).unwrap_or(0);
        self.board_col = col;
        self.board_row = self.board_row.min(rows.saturating_sub(1));
    }

    /// The compose modal is up, whether drafting or sending.
    pub(crate) fn compose_open(&self) -> bool {
        self.compose.is_some()
    }

    /// Open the compose modal, deriving the target from context: on the
    /// Diff tab over a unified hunk row with a new-side line it drafts a
    /// line comment, everywhere else a thread comment. Split diffs fall
    /// back to a thread comment since paired rows carry no single line.
    pub(crate) fn open_compose(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if !self.data.contains_key(&n) {
            self.notify("still loading: wait for the issue first".to_string());
            return;
        }
        let target = self.compose_target().unwrap_or(ComposeTarget::Thread);
        self.compose = Some(ComposeState { number: n, target, body: String::new(), cursor: 0, sending: false, kill: String::new() });
    }

    /// Comment target for the active diff line, or `None` for a plain
    /// thread comment: not a PR diff, split layout, no file under the
    /// cursor, or no new-side line at the cursor. A cursor inside a
    /// rendered review-thread block answers a reply to that thread.
    fn compose_target(&self) -> Option<ComposeTarget> {
        if !self.visual_diff() || self.diff_split {
            return None;
        }
        let n = self.current?;
        let files = self.diffs.get(&n)?;
        let fi = self.diff_file_at_cursor()?;
        let file = files.get(fi)?;
        let abs = self.diff_line?;
        let vis = abs.checked_sub(self.body_base as usize)?;
        let header = self.body_rows.iter().enumerate()
            .find_map(|(i, line)| (model::header_file_index(line, files) == Some(fi)).then_some(i))?;
        if vis <= header {
            return None;
        }
        let section: Vec<String> = self.body_rows[header + 1..].to_vec();
        let cursor = vis - (header + 1);
        let d = self.data.get(&n)?;
        if let Some(thread_id) = model::thread_at_cursor(&section, cursor, file, &d.review_threads) {
            return Some(ComposeTarget::Reply { thread_id });
        }
        let line = model::line_target_at_cursor(&section, cursor, &file.patch)?;
        Some(ComposeTarget::Line { path: file.filename.clone(), line })
    }

    /// Discard the draft, whatever it holds. Quiet: Esc means gone.
    pub(crate) fn discard_compose(&mut self) {
        self.compose = None;
    }

    /// Byte offset of a char index, clamped to the end.
    fn compose_byte(body: &str, cursor: usize) -> usize {
        body.char_indices().nth(cursor).map(|(b, _)| b).unwrap_or(body.len())
    }

    /// Type a char at the cursor. Locked while sending so the in-flight
    /// body never changes under the request.
    pub(crate) fn compose_insert(&mut self, c: char) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        let cursor = s.cursor.min(s.body.chars().count());
        let byte = Self::compose_byte(&s.body, cursor);
        s.body.insert(byte, c);
        s.cursor = cursor + 1;
    }

    /// Delete the char before the cursor.
    pub(crate) fn compose_backspace(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending || s.cursor == 0 {
            return;
        }
        let cursor = s.cursor.min(s.body.chars().count());
        let end = Self::compose_byte(&s.body, cursor);
        let start = Self::compose_byte(&s.body, cursor - 1);
        s.body.drain(start..end);
        s.cursor = cursor - 1;
    }

    /// Step the cursor one char left or right.
    pub(crate) fn compose_step(&mut self, delta: isize) {
        let Some(s) = self.compose.as_mut() else { return };
        let len = s.body.chars().count();
        s.cursor = s.cursor.saturating_add_signed(delta).min(len);
    }

    /// Step the cursor one visual line up or down, keeping the column.
    pub(crate) fn compose_line(&mut self, delta: isize) {
        let Some(s) = self.compose.as_mut() else { return };
        let cursor = s.cursor.min(s.body.chars().count());
        let chars: Vec<char> = s.body.chars().collect();
        let head: String = chars[..cursor].iter().collect();
        let row = head.matches('\n').count() as isize;
        let col = head.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) as isize;
        let lines: Vec<&str> = s.body.split('\n').collect();
        let next_row = (row + delta).clamp(0, lines.len() as isize - 1) as usize;
        let next_col = col.min(lines[next_row].chars().count() as isize).max(0) as usize;
        let base: usize = lines[..next_row].iter().map(|l| l.chars().count() + 1).sum();
        s.cursor = base + next_col;
    }

    /// Line start of the cursor: the char after the previous `\n`, or 0.
    fn compose_line_start(body: &str, cursor: usize) -> usize {
        let head: String = body.chars().take(cursor.min(body.chars().count())).collect();
        head.rfind('\n').map(|b| head[..=b].chars().count()).unwrap_or(0)
    }

    /// Line end of the cursor: the next `\n`, or the body end.
    fn compose_line_end(body: &str, cursor: usize) -> usize {
        let cursor = cursor.min(body.chars().count());
        let mut end = cursor;
        for (i, ch) in body.chars().enumerate().skip(cursor) {
            if ch == '\n' {
                return i;
            }
            end = i + 1;
        }
        end
    }

    /// Word char for emacs word motions: letters, digits, `_`.
    fn compose_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Word start behind the cursor: skip whitespace, then word chars.
    fn compose_word_back_from(body: &str, cursor: usize) -> usize {
        let chars: Vec<char> = body.chars().collect();
        let mut i = cursor.min(chars.len());
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && Self::compose_word_char(chars[i - 1]) {
            i -= 1;
        }
        i
    }

    /// Word end ahead of the cursor: from mid-word to its end, else past
    /// the next word.
    fn compose_word_forward_from(body: &str, cursor: usize) -> usize {
        let chars: Vec<char> = body.chars().collect();
        let mut i = cursor.min(chars.len());
        if i < chars.len() && Self::compose_word_char(chars[i]) {
            while i < chars.len() && Self::compose_word_char(chars[i]) {
                i += 1;
            }
        } else {
            while i < chars.len() && !Self::compose_word_char(chars[i]) {
                i += 1;
            }
            while i < chars.len() && Self::compose_word_char(chars[i]) {
                i += 1;
            }
        }
        i
    }

    /// Drain `[start, end)` (char indices), stashing the cut text in the
    /// kill ring. Locked while sending so the in-flight body never changes.
    fn compose_cut(&mut self, start: usize, end: usize) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        let len = s.body.chars().count();
        let (start, end) = (start.min(len), end.min(len));
        if start >= end {
            return;
        }
        let killed: String = s.body.chars().skip(start).take(end - start).collect();
        let byte_start = Self::compose_byte(&s.body, start);
        let byte_end = Self::compose_byte(&s.body, end);
        s.body.drain(byte_start..byte_end);
        s.kill = killed;
        s.cursor = start;
    }

    /// Jump to the line start (`Ctrl+A`, `Home`).
    pub(crate) fn compose_home(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        s.cursor = Self::compose_line_start(&s.body, s.cursor);
    }

    /// Jump to the line end (`Ctrl+E`, `End`).
    pub(crate) fn compose_end(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        s.cursor = Self::compose_line_end(&s.body, s.cursor);
    }

    /// Jump one word back (`Alt+B`).
    pub(crate) fn compose_word_back(&mut self) {
        let (body, cursor, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_back_from(&body, cursor);
        if let Some(s) = self.compose.as_mut() {
            s.cursor = at;
        }
    }

    /// Jump one word forward (`Alt+F`).
    pub(crate) fn compose_word_forward(&mut self) {
        let (body, cursor, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_forward_from(&body, cursor);
        if let Some(s) = self.compose.as_mut() {
            s.cursor = at;
        }
    }

    /// Kill to the line end (`Ctrl+K`); at the end of a line the newline
    /// itself is the kill, joining the lines like bash does.
    pub(crate) fn compose_kill_to_end(&mut self) {
        let (body, cursor, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let len = body.chars().count();
        let cursor = cursor.min(len);
        let mut end = Self::compose_line_end(&body, cursor);
        if end == cursor && cursor < len {
            end = cursor + 1;
        }
        self.compose_cut(cursor, end);
    }

    /// Kill back to the line start (`Ctrl+U`).
    pub(crate) fn compose_kill_to_start(&mut self) {
        let (body, cursor, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let start = Self::compose_line_start(&body, cursor);
        self.compose_cut(start, cursor.min(body.chars().count()));
    }

    /// Delete the char ahead (`Ctrl+D`, `Delete`). A plain delete, not
    /// a kill: the kill ring keeps its text for yank. Never discards: an
    /// empty draft has nothing to delete.
    pub(crate) fn compose_delete_forward(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        let len = s.body.chars().count();
        let cursor = s.cursor.min(len);
        if cursor >= len {
            return;
        }
        let start = Self::compose_byte(&s.body, cursor);
        let end = Self::compose_byte(&s.body, cursor + 1);
        s.body.drain(start..end);
        s.cursor = cursor;
    }

    /// Delete the word behind (`Ctrl+W`).
    pub(crate) fn compose_delete_word_back(&mut self) {
        let (body, cursor, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_back_from(&body, cursor);
        self.compose_cut(at, cursor.min(body.chars().count()));
    }

    /// Delete the word ahead (`Alt+D`); the cursor stays put.
    pub(crate) fn compose_delete_word_forward(&mut self) {
        let (body, cursor, len, sending) = match self.compose.as_ref() {
            Some(s) => (s.body.clone(), s.cursor, s.body.chars().count(), s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let cursor = cursor.min(len);
        let end = Self::compose_word_forward_from(&body, cursor);
        if end == cursor {
            return;
        }
        let Some(s) = self.compose.as_mut() else { return };
        let killed: String = s.body.chars().skip(cursor).take(end - cursor).collect();
        let byte_start = Self::compose_byte(&s.body, cursor);
        let byte_end = Self::compose_byte(&s.body, end);
        s.body.drain(byte_start..byte_end);
        s.kill = killed;
        s.cursor = cursor;
    }

    /// Paste the kill ring back (`Ctrl+Y`).
    pub(crate) fn compose_yank(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending || s.kill.is_empty() {
            return;
        }
        let kill = s.kill.clone();
        let cursor = s.cursor.min(s.body.chars().count());
        let byte = Self::compose_byte(&s.body, cursor);
        s.body.insert_str(byte, &kill);
        s.cursor = cursor + kill.chars().count();
    }

    /// Swap the char before the cursor with the one under it (`Ctrl+T`);
    /// at the draft end that swaps the last two chars.
    pub(crate) fn compose_transpose(&mut self) {
        let Some(s) = self.compose.as_mut() else { return };
        if s.sending {
            return;
        }
        let chars: Vec<char> = s.body.chars().collect();
        let len = chars.len();
        if len < 2 {
            return;
        }
        let cursor = s.cursor.min(len);
        if cursor == 0 {
            return;
        }
        let (a, b) = if cursor >= len { (len - 2, len - 1) } else { (cursor - 1, cursor) };
        let mut next = chars;
        next.swap(a, b);
        s.body = next.into_iter().collect();
        s.cursor = if cursor >= len { len } else { cursor + 1 };
    }

    /// `$EDITOR` for the compose modal. Just the env var: no config
    /// plumbing, no fallback. `None` when unset or blank.
    pub(crate) fn compose_editor() -> Option<String> {
        std::env::var("EDITOR").ok().filter(|e| !e.trim().is_empty())
    }

    /// Arm the external editor run (`Ctrl+O` in the modal). With no
    /// `$EDITOR` there is nothing to run, so the draft stays and the
    /// toast names the fix.
    pub(crate) fn request_compose_editor(&mut self) {
        let sending = self.compose.as_ref().map(|c| c.sending).unwrap_or(true);
        if sending {
            return;
        }
        if Self::compose_editor().is_none() {
            self.notify("no $EDITOR set: export EDITOR to edit comments outside ghit".to_string());
            return;
        }
        self.editor_requested = true;
    }

    /// Land the external editor run: the file text replaces the draft
    /// verbatim and the cursor parks at the end.
    pub(crate) fn land_editor_text(&mut self, text: String, editor: &str) {
        let Some(c) = self.compose.as_mut() else { return };
        c.body = text;
        c.cursor = c.body.chars().count();
        self.notify(format!("edited in {editor}"));
    }

    /// Submit the draft on a background thread; the loop never blocks.
    /// Empty drafts stay open with a status-line error, never a send.
    pub(crate) fn submit_compose(&mut self) {
        let Some(c) = self.compose.as_mut() else { return };
        if c.sending {
            return;
        }
        if c.body.trim().is_empty() {
            self.notify("comment is empty: write something first".to_string());
            return;
        }
        c.sending = true;
        let (number, target, body) = (c.number, c.target.clone(), c.body.clone());
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        // A REST reply names a numeric comment id, not the GraphQL
        // thread id the cursor carries: resolve it from the loaded
        // thread before sending.
        let reply_comment_id = match &target {
            ComposeTarget::Reply { thread_id } => self
                .data
                .get(&number)
                .and_then(|d| model::thread_reply_comment_id(&d.review_threads, thread_id)),
            _ => None,
        };
        let tx = self.compose_tx.clone();
        rt().spawn(async move {
            let result = match &target {
                ComposeTarget::Thread => github::post_thread_comment(&repo, number, &body).await,
                ComposeTarget::Line { path, line } => github::post_line_comment(&repo, number, &body, path, *line).await,
                ComposeTarget::Reply { .. } => match reply_comment_id {
                    Some(comment_id) => github::post_thread_reply(&repo, number, comment_id, &body).await,
                    None => Err("reply needs a review comment: refresh the PR to reload its threads".to_string()),
                },
            };
            let _ = tx.send((number, result));
        });
        self.notify("sending comment…".to_string());
    }

    /// Pending review threads on the current PR, if any.
    pub(crate) fn pending_threads(&self) -> usize {
        let Some(n) = self.current else { return 0 };
        let Some(d) = self.data.get(&n) else { return 0 };
        d.review_threads.iter().filter(|t| t.has_pending()).count()
    }

    /// Submit the viewer's pending review so pending line comments go
    /// live. No pending threads means a status-line note, never a send.
    pub(crate) fn submit_pending_review(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if self.review_submitting.contains(&n) {
            return;
        }
        if self.pending_threads() == 0 {
            self.notify("no pending comments to submit".to_string());
            return;
        }
        self.review_submitting.insert(n);
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        let tx = self.review_submit_tx.clone();
        rt().spawn(async move {
            let result = github::submit_pending_review(&repo, n).await;
            let _ = tx.send((n, result));
        });
        self.notify("submitting pending review…".to_string());
    }

    /// A confirm modal is up, whatever it decides.
    pub(crate) fn confirm_open(&self) -> bool {
        self.confirm.is_some()
    }

    /// Pick choice `index` of the open dialog, then run what the
    /// specialization decides. Out-of-range indexes are ignored so a
    /// stale key never acts on the wrong button.
    pub(crate) fn confirm_choose(&mut self, index: usize) {
        let Some(dialog) = self.confirm.take() else { return };
        match dialog.kind {
            ConfirmKind::PendingNav { intent } => match index {
                0 => {
                    self.submit_pending_review();
                    self.run_nav(intent);
                }
                1 => self.run_nav(intent),
                _ => {}
            },
        }
    }

    /// Drop the open dialog: the deferred navigation never runs.
    pub(crate) fn confirm_cancel(&mut self) {
        self.confirm = None;
    }

    /// Run a deferred navigation after a confirm resolves. Never guards:
    /// the guard already ran when the intent was stored.
    fn run_nav(&mut self, intent: NavIntent) {
        match intent {
            NavIntent::Issue(n) => self.open_issue_inner(n),
            NavIntent::Tab(tab) => self.set_tab_inner(tab),
            NavIntent::Step(delta) => self.step_issue_inner(delta),
            NavIntent::Back => self.go_back_inner(),
            NavIntent::Forward => self.go_forward_inner(),
            NavIntent::Evict => self.evict_current_inner(),
        }
    }

    /// Specialization: leaving a PR with unsubmitted review comments.
    /// Choice order is Submit, Discard, Cancel; the renderer shows the
    /// keys. Discard only navigates — the comments stay pending on
    /// GitHub, nothing is deleted.
    pub(crate) fn open_pending_nav(&mut self, intent: NavIntent) {
        let Some(n) = self.current else { return };
        let count = self.pending_threads();
        self.confirm = Some(ConfirmDialog {
            title: "Unsubmitted comments".to_string(),
            body: vec![
                format!("#{n} has {count} unsubmitted comment{s}.", s = if count == 1 { "" } else { "s" }),
                "Discard leaves them pending on GitHub.".to_string(),
            ],
            choices: vec![
                ConfirmChoice { key: 's', label: "Submit them".to_string() },
                ConfirmChoice { key: 'd', label: "Discard".to_string() },
                ConfirmChoice { key: 'c', label: "Cancel".to_string() },
            ],
            kind: ConfirmKind::PendingNav { intent },
        });
    }

    /// Guard one navigation: with pending comments on the current issue
    /// the intent waits in a confirm dialog, otherwise it runs at once.
    fn guard_nav(&mut self, intent: NavIntent) -> bool {
        if self.pending_threads() == 0 {
            return false;
        }
        self.open_pending_nav(intent);
        true
    }

    /// User-initiated issue preview through the pending guard: the view
    /// switches without filing the row, so cursor walks and first-press
    /// previews never relocate rows between groups.
    pub(crate) fn request_issue(&mut self, n: u64) {
        if self.current == Some(n) {
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
        if self.current == Some(n) {
            self.show_issue(n);
            // Already viewing still completes the submit tail: the bar
            // clears and focus returns to Main, or later keys land as
            // bar text. Matches the cached fast path in `load_issue`.
            self.bar.clear();
            self.focus = Focus::Main;
            return;
        }
        if !self.guard_nav(NavIntent::Issue(n)) {
            self.open_issue_inner(n);
        }
    }

    fn open_issue_inner(&mut self, n: u64) {
        self.ensure_visited(n);
        self.load_issue(n, false);
    }

    /// User-initiated tab switch through the pending guard.
    pub(crate) fn request_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            return;
        }
        if !self.guard_nav(NavIntent::Tab(tab)) {
            self.set_tab_inner(tab);
        }
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

/// Board columns narrowed by the active filter: milestone pills select
/// (empty selects all) plus the text query over title, label, `#num`.
/// An empty filter returns every card.
pub(crate) fn board_columns<'a>(
    board: &'a BoardData,
    filter: &str,
    milestones: &HashSet<Option<String>>,
) -> Vec<(String, Vec<&'a model::BoardItem>)> {
    let cols = model::board_columns(&board.items, &board.order);
    if filter.trim().is_empty() && milestones.is_empty() {
        return cols;
    }
    cols.into_iter()
        .map(|(name, cards)| {
            let cards = cards
                .into_iter()
                .filter(|c| {
                    if !milestones.is_empty() && !milestones.contains(&c.milestone) {
                        return false;
                    }
                    let labels: Vec<&str> = c.labels.iter().map(|l| l.name.as_str()).collect();
                    core::matches_board_filter(c.number, &c.title, &labels, filter)
                })
                .collect();
            (name, cards)
        })
        .collect()
}

pub(crate) fn board_move(app: &mut App, dx: isize, dy: isize) {
    app.board_expanded = None;
    let Some(board) = &app.board else { return };
    let cols = board_columns(board, &app.board_filter, &app.board_milestones);
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
    let cols = board_columns(board, &app.board_filter, &app.board_milestones);
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

pub(crate) fn poll_details(app: &mut App) -> bool {
    let ready: Vec<(u64, Result<model::BoardDetail, String>)> = app.detail_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        app.detail_pending.remove(&n);
        match res {
            Ok(d) => { app.board_details.insert(n, d); }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

/// Land a pending-review submit: success refetches the issue so the
/// Diff tab shows submitted threads; failure just names the cause.
pub(crate) fn poll_review_submit(app: &mut App) -> bool {
    let ready: Vec<(u64, Result<(), String>)> = app.review_submit_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        app.review_submitting.remove(&n);
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

/// Land a compose submit: success closes the modal, confirms on the
/// status line, and refetches the issue so the Comments tab shows the
/// new comment; failure keeps the draft and names the cause.
pub(crate) fn poll_compose(app: &mut App) -> bool {
    let ready: Vec<(u64, Result<(), String>)> = app.compose_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (n, res) in ready {
        match res {
            Ok(()) => {
                let pending = matches!(app.compose.as_ref().map(|c| &c.target), Some(ComposeTarget::Line { .. } | ComposeTarget::Reply { .. }));
                app.compose = None;
                if pending {
                    app.notify(format!("line comment pending on #{n} — press S to submit"));
                } else {
                    app.notify(format!("comment posted to #{n}"));
                }
                app.load_issue(n, true);
            }
            Err(e) => {
                if let Some(c) = app.compose.as_mut() {
                    c.sending = false;
                }
                auth_failure(app, &e);
                app.notify(e);
            }
        }
    }
    landed
}

pub(crate) fn poll_board(app: &mut App) -> bool {
    if !app.board_loading {
        return false;
    }
    let res = app.board_rx.as_ref().and_then(|rx| rx.try_recv().ok());
    let Some(res) = res else { return false };
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
        Err(e) => { auth_failure(app, &e); app.notify(e); }
    }
    true
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
    app.bootstrap_rx = Some(rx);
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
    app.auth_rx = Some(arx);
    rt().spawn(async move {
        let _ = atx.send(github::check_auth().await);
    });
}

/// Land an auth probe: a dead token or missing scopes raises the
/// re-auth modal; a covered token stays silent.
pub(crate) fn poll_auth(app: &mut App) -> bool {
    let Some(result) = app.auth_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    app.auth_rx = None;
    match result {
        Ok(missing) if missing.is_empty() => {}
        Ok(missing) => {
            app.auth_prompt = Some(AuthPrompt {
                cause: "token lacks scopes ghit needs".to_string(),
                missing,
                scopes_known: true,
            });
        }
        // Only auth-class failures rise to the modal: re-auth can fix
        // those, while a rate limit or outage just needs a status line.
        Err(e) if crate::github::is_auth_error(&e) => {
            app.auth_prompt = Some(AuthPrompt { cause: e, missing: vec![], scopes_known: false });
        }
        Err(e) => app.notify(e),
    }
    true
}

/// Route an auth-class fetch failure at the re-auth modal: the early
/// probes cannot see every token kind or repo, so the first failing
/// fetch gets the same question. A dismissed prompt never re-rises;
/// the caller still notifies as before.
pub(crate) fn auth_failure(app: &mut App, e: &str) {
    if app.auth_dismissed || app.auth_prompt.is_some() {
        return;
    }
    if crate::github::is_auth_error(e) {
        app.auth_prompt = Some(AuthPrompt {
            cause: e.to_string(),
            missing: vec![],
            scopes_known: false,
        });
    }
}

/// Run `gh auth refresh` with the required scopes on the real terminal:
/// the TUI suspends (raw mode off, alternate screen left), the command
/// inherits stdio so it can prompt, then the TUI resumes and the probe
/// re-runs. Drive mode has no terminal to suspend; it just names the
/// command instead.
pub(crate) fn run_reauth(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    use crossterm::ExecutableCommand;
    let cmd = crate::github::reauth_command();
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    println!("ghit needs these scopes: {}", crate::github::REQUIRED_SCOPES.join(", "));
    let status = std::process::Command::new("gh")
        .args(["auth", "refresh"])
        .args(crate::github::REQUIRED_SCOPES.iter().flat_map(|s| ["-s", s]))
        .status();
    match status {
        Ok(s) if s.success() => {
            crate::github::forget_token();
            app.auth_dismissed = false;
            app.notify("re-auth ok: re-checking token…".to_string());
        }
        _ => app.notify(format!("re-auth failed — run by hand: {cmd}")),
    }
    let _ = enable_raw_mode();
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = terminal.clear();
    app.auth_prompt = None;
    app.reauth_requested = false;
    // Re-probe so the modal returns if the refresh did not cover it.
    let (atx, arx) = std::sync::mpsc::channel();
    app.auth_rx = Some(arx);
    rt().spawn(async move {
        let _ = atx.send(github::check_auth().await);
    });
}

/// Suspend the TUI, run `$EDITOR` on the draft, and land the result.
/// Mirrors `run_reauth`: the loop owns the terminal, so input only arms
/// `editor_requested` and the loop runs this with the terminal. The draft
/// stays put on every failure path, so no edit ever loses text.
pub(crate) fn run_compose_editor(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    use crossterm::ExecutableCommand;
    app.editor_requested = false;
    let Some(editor) = App::compose_editor() else {
        app.notify("no $EDITOR set: export EDITOR to edit comments outside ghit".to_string());
        return;
    };
    let drafting = app.compose.as_ref().map(|c| !c.sending).unwrap_or(false);
    if !drafting {
        return;
    }
    // Stage the draft in a temp file so the editor opens real markdown.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut path = std::env::temp_dir();
    path.push(format!("ghit-comment-{}-{stamp}.md", std::process::id()));
    if std::fs::write(&path, &app.compose.as_ref().expect("draft").body).is_err() {
        app.notify("editor needs a writable temp dir".to_string());
        return;
    }
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let status = std::process::Command::new(&editor).arg(&path).status();
    let _ = enable_raw_mode();
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = terminal.clear();
    let text = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    match (status, text) {
        (Ok(s), Some(text)) if s.success() => app.land_editor_text(text, &editor),
        (Ok(_), _) => app.notify(format!("{editor} failed: draft kept")),
        (Err(e), _) => app.notify(format!("{editor} failed to start ({e}): draft kept")),
    }
}

/// Every background lander, once per loop pass. The terminal and drive
/// loops share this so both pick up fetches the same way. True when any
/// lander changed visible state, so the loop knows a redraw is owed.
pub(crate) fn poll_all(app: &mut App) -> bool {
    let mut landed = false;
    landed |= poll_bootstrap(app);
    landed |= poll_auth(app);
    landed |= poll_sections(app);
    let had_fetch = app.section_rx.is_some();
    app.auto_refresh_recent(Instant::now());
    landed |= had_fetch != app.section_rx.is_some();
    landed |= poll_issues(app);
    landed |= poll_diffs(app);
    landed |= poll_graphs(app);
    landed |= poll_board(app);
    landed |= poll_details(app);
    landed |= poll_compose(app);
    landed |= poll_review_submit(app);
    landed
}

pub(crate) fn run(app: &mut App, numbers: &[u64]) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen).map_err(|e| e.to_string())?;
    stdout.execute(EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    bootstrap(app, numbers);

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
    // First paint is unconditional; after that a frame redraws only
    // when input, a poll landing, or an animation (spinner frame,
    // message flip, click flash) says something visible changed. The
    // tick bumps per paint, so the spinner keeps its old speed even
    // though passes now run faster than paints.
    let mut needs_draw = true;
    let mut spin_painted_at = Instant::now();
    let mut msg_shown = false;
    loop {
        // The spinner repaints on its own cadence, not every poll
        // pass; a message flip always owes one paint so its expiry
        // clears even when nothing else stirs.
        let spin_owed = app.spin_owed(spin_painted_at);
        let msg_owed = app.message_visible() != msg_shown;
        if needs_draw || app.flash_live() || spin_owed || msg_owed {
            // Cheap frame tick for the status bar spinner: one bump per
            // paint, no timers, no extra wakeups.
            app.tick = app.tick.wrapping_add(1);
            terminal.draw(|f| crate::ui::render(f, app)).map_err(|e| e.to_string())?;
            if spin_owed {
                spin_painted_at = Instant::now();
            }
            msg_shown = app.message_visible();
        }
        let mut input_dirty = false;
        if event::poll(Duration::from_millis(POLL_TIMEOUT_MS)).map_err(|e| e.to_string())? {
            // Drain the whole queue: one slow redraw must never strand a
            // second press behind it.
            loop {
                match event::read().map_err(|e| e.to_string())? {
                    Event::Key(key) => crate::input::handle_key(app, key),
                    Event::Mouse(m) => crate::input::handle_mouse(app, m),
                    _ => {}
                };
                input_dirty = true;
                if !event::poll(Duration::ZERO).map_err(|e| e.to_string())? {
                    break;
                }
            }
        }
        let poll_dirty = poll_all(app);
        if app.reauth_requested {
            run_reauth(app, terminal);
            input_dirty = true;
        }
        if app.editor_requested {
            run_compose_editor(app, terminal);
            input_dirty = true;
        }
        needs_draw = input_dirty || poll_dirty;
        if app.quit {
            return Ok(());
        }
    }
}

fn poll_bootstrap(app: &mut App) -> bool {
    let Some(result) = app.bootstrap_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    app.bootstrap_rx = None;
    app.repo_slug = result;
    // Repo-scoped access probe, once: the boot probe only checks the
    // token itself, while scopes need a repo to read against.
    if !app.auth_repo_probed {
        app.auth_repo_probed = true;
        if let Some(repo) = app.repo_flag.clone().or_else(|| app.repo_slug.clone()) {
            let (tx, rx) = std::sync::mpsc::channel();
            app.auth_rx = Some(rx);
            rt().spawn(async move {
                let _ = tx.send(github::check_repo_access(&repo).await);
            });
        }
    }
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
        app.hydrate_cached_sections();
        app.merge_recent_sections();
    } else {
        for n in numbers { app.load_issue(n, false); }
    }
    true
}

pub(crate) fn poll_sections(app: &mut App) -> bool {
    let Some(event) = app.section_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return false };
    match event {
        github::SectionEvent::Progress { done, total } => {
            app.section_progress = Some((done, total));
        }
        github::SectionEvent::Done(result) => {
            app.section_rx = None;
            app.section_progress = None;
            let auto = std::mem::take(&mut app.section_auto_refresh);
            if auto {
                app.auto_refresh_interval = if result.is_ok() {
                    auto_refresh_interval_default()
                } else {
                    (app.auto_refresh_interval * 2).min(RECENT_REFRESH_MAX_INTERVAL)
                };
            }
            let replace = std::mem::take(&mut app.section_replace);
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
                        if let Some(d) = app.data.get_mut(&n) {
                            // Window listings refresh display fields only.
                            // Body, comments, and review threads belong to
                            // the full fetch and stay. Search hits carry no
                            // statuses, so replace mode never touches known
                            // rows — dropped unvisited rows reinsert below.
                            if !replace {
                                d.title = row.issue.title;
                                d.state = row.issue.state;
                                d.labels = row.issue.labels;
                                app.statuses.insert(n, row.statuses);
                                app.board_details.insert(n, row.detail);
                            }
                        } else {
                            if !row.complete { app.partial.insert(n); }
                            app.data.insert(n, row.issue);
                            app.statuses.insert(n, row.statuses);
                            app.board_details.insert(n, row.detail);
                            app.ensure_recent(n);
                            last = Some(n);
                        }
                    }
                    if !auto && app.current.is_none() && app.requested.is_none() {
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
    let keep: HashSet<u64> = app.visited.iter().copied()
        .chain(app.pin_order.iter().copied())
        .chain(app.pinned.iter().copied())
        .chain(app.current)
        .chain(app.requested)
        .collect();
    let gone: HashSet<u64> = app.recent.iter().copied()
        .filter(|n| !fresh.contains(n) && !keep.contains(n))
        .collect();
    if gone.is_empty() { return; }
    app.recent.retain(|n| !gone.contains(n));
    for n in gone { app.drop_cached(n); }
}

pub(crate) fn poll_issues(app: &mut App) -> bool {
    let ready: Vec<_> = app.issue_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, event) in ready {
        match event {
            github::IssueEvent::Progress { done, total } => {
                app.issue_progress.insert(number, (done, total));
            }
            github::IssueEvent::Done(result) => {
                app.issue_progress.remove(&number);
                app.issue_pending.remove(&number);
                // An h/l step owns this number until it lands: remember
                // before clearing so a 404 still toasts below.
                let stepped = app.step_pending == Some(number);
                if stepped {
                    app.step_pending = None;
                }
                match result {
                    Ok((data, statuses)) => {
                        // Completing the full fetch never files the row:
                        // viewing a recent row must not relocate it to
                        // requested. Explicit actions file through
                        // `ensure_visited` at their own site.
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

pub(crate) fn poll_diffs(app: &mut App) -> bool {
    let ready: Vec<_> = app.diff_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, result) in ready {
        app.diff_pending.remove(&number);
        match result {
            Ok(files) => {
                app.diffs.insert(number, files);
            }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
}

pub(crate) fn poll_graphs(app: &mut App) -> bool {
    let ready: Vec<_> = app.graph_rx.try_iter().collect();
    let landed = !ready.is_empty();
    for (number, result) in ready {
        app.graph_pending.remove(&number);
        match result {
            Ok(data) => {
                app.graphs.insert(number, data);
                if app.current == Some(number) {
                    app.graph_cursor = app.graph_cursor.min(app.graph_order_numbers().len().saturating_sub(1));
                }
            }
            Err(e) => { auth_failure(app, &e); app.notify(e); }
        }
    }
    landed
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
            checks: None,
            milestone: None,
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

    /// On-disk name of the o/r sections cache at the default window.
    /// Mirrors `github::sections_cache_name("o/r", 24)`; update together.
    const SECTIONS_CACHE: &str = "sections-o__r-24";

    fn cached_row(number: u64, complete: bool) -> crate::github::Row {
        crate::github::Row {
            issue: issue(number, "2026-01-05T00:00:00Z"),
            statuses: vec![],
            detail: crate::model::BoardDetail {
                created: "2026-01-01".to_string(),
                updated: "2026-01-05".to_string(),
                comments: 0,
            },
            complete,
            window_at: None,
        }
    }

    fn temp_xdg() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir, Option<String>) {
        let guard = crate::cache::test_env_lock().lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let saved = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        (guard, dir, saved)
    }

    fn restore_xdg(saved: Option<String>) {
        match saved {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn startup_hydrates_sidebar_from_sections_cache() {
        let (_guard, _dir, saved) = temp_xdg();
        cache::cache_put(SECTIONS_CACHE, &vec![cached_row(7, false), cached_row(9, true)]);
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.hydrate_cached_sections();
        assert_eq!(app.recent, vec![7, 9], "cached order becomes sidebar order");
        assert!(app.data.contains_key(&7) && app.data.contains_key(&9));
        assert!(app.partial.contains(&7), "slim rows stay partial so open runs the full fetch");
        assert!(!app.partial.contains(&9));
        assert!(app.statuses.contains_key(&7) && app.board_details.contains_key(&7));
        restore_xdg(saved);
    }

    #[test]
    fn startup_hydration_leaves_present_rows_alone() {
        let (_guard, _dir, saved) = temp_xdg();
        cache::cache_put(SECTIONS_CACHE, &vec![cached_row(7, false), cached_row(9, false)]);
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut kept = issue(7, "2026-01-05T00:00:00Z");
        kept.title = "kept".to_string();
        app.data.insert(7, kept);
        app.hydrate_cached_sections();
        assert_eq!(app.data[&7].title, "kept", "CLI-opened rows win over the cache");
        assert!(app.data.contains_key(&9));
        assert_eq!(app.recent, vec![9], "present rows are not re-added to recent");
        restore_xdg(saved);
    }

    #[test]
    fn startup_hydration_ignores_missing_and_expired_cache() {
        let (_guard, dir, saved) = temp_xdg();
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.hydrate_cached_sections();
        assert!(app.recent.is_empty() && app.data.is_empty(), "no cache means no rows");
        cache::cache_put(SECTIONS_CACHE, &vec![cached_row(7, false)]);
        let path = dir.path().join("ghit").join("cache").join(format!("{SECTIONS_CACHE}.json"));
        let mut payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        payload["fetched_at"] = serde_json::json!(now - crate::cache::CACHE_TTL_SECS - 1);
        std::fs::write(&path, serde_json::to_string(&payload).unwrap()).unwrap();
        app.hydrate_cached_sections();
        assert!(app.recent.is_empty() && app.data.is_empty(), "expired cache reads as a miss");
        restore_xdg(saved);
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
    fn issue_progress_rides_the_issue_channel() {
        let mut app = sidebar_app(&[7]);
        app.issue_pending.insert(9);
        app.issue_tx.send((9, github::IssueEvent::Progress { done: 2, total: 7 })).unwrap();
        poll_issues(&mut app);
        assert_eq!(app.issue_progress.get(&9), Some(&(2, 7)));
        assert!(app.issue_pending.contains(&9), "progress is not completion");
        let fresh = issue(9, "2026-01-06T00:00:00Z");
        app.issue_tx.send((9, github::IssueEvent::Done(Ok((fresh, vec![]))))).unwrap();
        poll_issues(&mut app);
        assert!(!app.issue_progress.contains_key(&9), "terminal result clears progress");
        assert!(!app.issue_pending.contains(&9));
        assert!(app.data.contains_key(&9));
    }

    #[test]
    fn auto_refresh_interval_env_override() {
        let _guard = crate::cache::test_env_lock().lock().unwrap();
        std::env::remove_var("GHIT_AUTO_REFRESH_SECS");
        assert_eq!(auto_refresh_interval_default(), RECENT_REFRESH_INTERVAL);
        std::env::set_var("GHIT_AUTO_REFRESH_SECS", "3600");
        assert_eq!(auto_refresh_interval_default(), Duration::from_secs(3600));
        for bad in ["0", "nope", ""] {
            std::env::set_var("GHIT_AUTO_REFRESH_SECS", bad);
            assert_eq!(auto_refresh_interval_default(), RECENT_REFRESH_INTERVAL, "reject {bad}");
        }
        std::env::remove_var("GHIT_AUTO_REFRESH_SECS");
    }

    #[test]
    fn auto_refresh_fires_at_interval() {
        let mut app = sidebar_app(&[7]);
        let start = app.last_sections_fetch;
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL - Duration::from_millis(1));
        assert!(app.section_rx.is_none());
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
        assert!(app.section_rx.is_some());
        assert!(app.section_auto_refresh);
    }

    #[test]
    fn spin_owed_paces_spinner_repaints() {
        let mut app = sidebar_app(&[7]);
        assert!(!app.spin_owed(Instant::now()), "idle owes nothing");
        app.notify("loading".to_string());
        assert!(!app.spin_owed(Instant::now()), "fresh paint owes nothing");
        let stale = Instant::now() - Duration::from_millis(SPIN_FRAME_MS + 50);
        assert!(app.spin_owed(stale), "stale frame owes a repaint");
        app.message_at = Some(Instant::now() - Duration::from_secs(5));
        assert!(!app.spin_owed(stale), "expired message owes nothing");
    }

    #[test]
    fn tab_split_matches_first_title_width() {
        let first = tab_titles("Diff View")[0].chars().count() as u16;
        assert_eq!(TAB_SPLIT_X, first + 2, "click split follows the first title plus divider");
    }

    #[test]
    fn auto_refresh_guards_each_active_view() {
        for guard in 0..7 {
            let mut app = sidebar_app(&[7]);
            let (_tx, rx) = std::sync::mpsc::channel();
            match guard {
                0 => app.section_rx = Some(rx),
                1 => app.board_loading = true,
                2 => app.board = Some(BoardData { title: String::new(), order: vec![], items: vec![], url: String::new() }),
                3 => app.compose = Some(ComposeState { number: 7, target: ComposeTarget::Thread, body: String::new(), cursor: 0, sending: false, kill: String::new() }),
                4 => app.filter_open = true,
                5 => app.shortcuts_open = true,
                6 => app.focus = Focus::Bar,
                _ => unreachable!(),
            }
            let start = app.last_sections_fetch;
            app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
            assert_eq!(app.last_sections_fetch, start, "guard {guard}");
            assert!(!app.section_auto_refresh, "guard {guard}");
        }
    }

    #[test]
    fn bootstrap_starts_in_main_so_auto_refresh_runs() {
        let mut app = App::new(None);
        bootstrap(&mut app, &[]);
        assert!(matches!(app.focus, Focus::Main), "boot must not sit in the command bar");
        // A Main boot leaves the refresh gate open; the Bar guard above
        // still holds once the user opens `:` or `/` on purpose.
        let start = app.last_sections_fetch;
        app.repo_slug = Some("o/r".to_string());
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
        assert!(app.section_rx.is_some(), "startup focus must not block the refresh");
        app.bootstrap_rx = None;
        app.auth_rx = None;
        app.section_rx = None;
    }

    #[test]
    fn bar_submit_of_current_issue_clears_bar_and_returns_focus() {
        // Boot auto-shows the last number, so `:7 Enter` re-opens the
        // current issue through the already-current branch. The submit
        // tail must still run: without it focus strands in the bar and
        // later keys (`y`, `2`) land as bar text (e2e pr/diff/refs yank).
        let mut app = sidebar_app(&[7]);
        app.focus = Focus::Bar;
        app.bar = "7".to_string();
        app.submit_bar();
        assert!(matches!(app.focus, Focus::Main), "submit leaves the bar");
        assert!(app.bar.is_empty(), "submit clears its text");
    }

    #[test]
    fn auto_refresh_failure_backs_off_and_success_resets() {
        let mut app = sidebar_app(&[7]);
        for expected in [10, 20, 40, 60, 60] {
            let (tx, rx) = std::sync::mpsc::channel();
            app.section_rx = Some(rx);
            app.section_auto_refresh = true;
            tx.send(github::SectionEvent::Done(Err("offline".into()))).unwrap();
            poll_sections(&mut app);
            assert_eq!(app.auto_refresh_interval, Duration::from_secs(expected));
        }
        let start = app.last_sections_fetch;
        app.auto_refresh_recent(start + Duration::from_secs(59));
        assert!(app.section_rx.is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        app.section_auto_refresh = true;
        tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
        poll_sections(&mut app);
        assert_eq!(app.auto_refresh_interval, RECENT_REFRESH_INTERVAL);
    }

    #[test]
    fn auto_refresh_landing_preserves_view() {
        for current in [Some(7), None] {
            let mut app = sidebar_app(&[7]);
            app.current = current;
            app.scroll = 12;
            app.hover = Some(7);
            let (tx, rx) = std::sync::mpsc::channel();
            app.section_rx = Some(rx);
            app.section_auto_refresh = true;
            let row = crate::github::Row { issue: issue(9, "2026-01-06T00:00:00Z"), statuses: vec![], detail: model::BoardDetail::default(), complete: true, window_at: None };
            tx.send(github::SectionEvent::Done(Ok(vec![row]))).unwrap();
            poll_sections(&mut app);
            assert!(app.recent.contains(&9));
            assert_eq!(app.current, current);
            assert_eq!(app.scroll, 12);
            assert_eq!(app.hover, Some(7));
        }
    }

    fn section_row(issue: IssueData) -> crate::github::Row {
        crate::github::Row {
            issue,
            statuses: vec![],
            detail: model::BoardDetail::default(),
            complete: false,
            window_at: None,
        }
    }

    fn land_sections(app: &mut App, rows: Vec<crate::github::Row>) {
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        tx.send(github::SectionEvent::Done(Ok(rows))).unwrap();
        poll_sections(app);
    }

    #[test]
    fn section_landing_refreshes_known_row_display_fields() {
        let mut app = sidebar_app(&[7]);
        {
            let d = app.data.get_mut(&7).unwrap();
            d.title = "stale title".to_string();
            d.state = "OPEN".to_string();
            d.labels = vec![model::Label { name: "stale".to_string() }];
        }
        let mut fresh = issue(7, "2026-01-06T00:00:00Z");
        fresh.title = "fresh title".to_string();
        fresh.state = "CLOSED".to_string();
        fresh.labels = vec![model::Label { name: "bug".to_string() }];
        let mut row = section_row(fresh);
        row.statuses = vec![("Roadmap".to_string(), Some("Done".to_string()))];
        row.detail = model::BoardDetail {
            created: "2025-12-01".to_string(),
            updated: "2026-01-06".to_string(),
            comments: 4,
        };
        land_sections(&mut app, vec![row]);

        let d = app.data.get(&7).unwrap();
        assert_eq!(d.title, "fresh title", "stale title refreshes");
        assert_eq!(d.state, "CLOSED", "stale state refreshes");
        assert_eq!(d.labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), vec!["bug"]);
        assert_eq!(app.statuses.get(&7), Some(&vec![("Roadmap".to_string(), Some("Done".to_string()))]));
        assert_eq!(app.board_details.get(&7).map(|d| d.comments), Some(4));
        assert!(app.visited.contains(&7), "group membership is untouched");
        assert!(!app.recent.contains(&7), "known rows never re-file as recent");
    }

    #[test]
    fn board_columns_gate_on_milestone_pills_and_text() {
        use crate::model::BoardItem;
        let card = |number: u64, milestone: Option<&str>| BoardItem {
            title: format!("card {number}"),
            number: Some(number),
            status: Some("Todo".to_string()),
            labels: vec![],
            repo: None,
            milestone: milestone.map(str::to_string),
            content: serde_json::Value::Null,
        };
        let board = BoardData {
            title: "b".to_string(),
            order: vec!["Todo".to_string()],
            items: vec![card(1, Some("v1")), card(2, None)],
            url: String::new(),
        };
        let numbers = |cols: Vec<(String, Vec<&BoardItem>)>| {
            cols.into_iter().flat_map(|(_, cards)| cards.into_iter().map(|c| c.number)).collect::<Vec<_>>()
        };
        assert_eq!(numbers(board_columns(&board, "", &HashSet::new())), vec![Some(1), Some(2)]);
        assert_eq!(
            numbers(board_columns(&board, "", &HashSet::from([Some("v1".to_string())]))),
            vec![Some(1)]
        );
        assert_eq!(
            numbers(board_columns(&board, "", &HashSet::from([None]))),
            vec![Some(2)]
        );
        // Text still narrows within the pill selection; milestones never
        // match the text.
        assert_eq!(
            numbers(board_columns(&board, "card 2", &HashSet::from([Some("v1".to_string())]))),
            Vec::<Option<u64>>::new()
        );
        assert_eq!(
            numbers(board_columns(&board, "v1", &HashSet::new())),
            Vec::<Option<u64>>::new()
        );
    }

    #[test]
    fn section_landing_keeps_visited_body_comments_and_threads() {
        let mut app = sidebar_app(&[7]);
        {
            let d = app.data.get_mut(&7).unwrap();
            d.body = "full body".to_string();
            d.comments = vec![model::Comment {
                author: Author { login: "reviewer".to_string() },
                body: "full comment".to_string(),
                created_at: "2026-01-02T00:00:00Z".to_string(),
            }];
            d.review_threads = vec![model::ReviewThread {
                id: "t1".to_string(),
                path: "a.rs".to_string(),
                line: Some(2),
                ..Default::default()
            }];
        }
        let mut fresh = issue(7, "2026-01-06T00:00:00Z");
        fresh.title = "renamed".to_string();
        fresh.body = "listing body".to_string();
        fresh.review_threads = vec![];
        land_sections(&mut app, vec![section_row(fresh)]);

        let d = app.data.get(&7).unwrap();
        assert_eq!(d.title, "renamed", "display fields still refresh");
        assert_eq!(d.body, "full body", "full fetch body wins");
        assert_eq!(d.comments.len(), 1);
        assert_eq!(d.comments[0].body, "full comment");
        assert_eq!(d.review_threads.len(), 1);
        assert_eq!(d.review_threads[0].id, "t1");
    }

    #[test]
    fn section_landing_evicts_unvisited_rows_missing_from_the_window() {
        let mut app = sidebar_app(&[7]);
        app.data.insert(9, issue(9, "2026-01-05T00:00:00Z"));
        app.statuses.insert(9, vec![]);
        app.board_details.insert(9, model::BoardDetail::default());
        app.partial.insert(9);
        app.recent.push(9);
        // The fresh window carries a new row and the visited row; 9 is gone.
        land_sections(&mut app, vec![section_row(issue(11, "2026-01-06T00:00:00Z"))]);

        assert!(!app.data.contains_key(&9), "vanished row data dropped");
        assert!(!app.statuses.contains_key(&9));
        assert!(!app.board_details.contains_key(&9));
        assert!(!app.partial.contains(&9));
        assert!(!app.recent.contains(&9));
        assert!(app.data.contains_key(&7), "visited row survives its absence");
        assert!(app.visited.contains(&7));
        assert!(app.recent.contains(&11), "new window rows list as recent");
        assert_eq!(app.issue_numbers(), vec![7, 11]);
    }

    #[test]
    fn search_landing_keeps_replace_semantics() {
        let mut app = sidebar_app(&[7]);
        app.data.get_mut(&7).unwrap().body = "full body".to_string();
        let mut stale = issue(9, "2026-01-05T00:00:00Z");
        stale.body = "stale body".to_string();
        app.data.insert(9, stale);
        app.statuses.insert(9, vec![]);
        app.partial.insert(9);
        app.recent.push(9);
        app.section_replace = true;

        let mut hit = issue(9, "2026-01-06T00:00:00Z");
        hit.title = "search hit".to_string();
        hit.body = "search body".to_string();
        land_sections(&mut app, vec![section_row(hit)]);

        assert!(!app.section_replace, "replace flag is consumed");
        assert_eq!(
            app.data.get(&9).unwrap().body, "search body",
            "search rows replace cached data instead of refreshing display fields"
        );
        assert_eq!(app.recent.iter().filter(|n| **n == 9).count(), 1, "dedup holds");
        let visited = app.data.get(&7).expect("visited row survives an absent search result");
        assert_eq!(visited.body, "full body", "visited body is never clobbered");
    }

    #[test]
    fn section_landing_keeps_current_and_scroll_when_rows_vanish() {
        let mut app = sidebar_app(&[7]);
        // The open row was reached by cursor walk, so it is current but
        // never filed as visited.
        app.current = Some(9);
        app.scroll = 12;
        app.hover = Some(9);
        app.data.insert(9, issue(9, "2026-01-05T00:00:00Z"));
        app.statuses.insert(9, vec![]);
        app.partial.insert(9);
        app.recent.push(9);
        land_sections(&mut app, vec![section_row(issue(7, "2026-01-06T00:00:00Z"))]);

        assert_eq!(app.current, Some(9), "current survives leaving the window");
        assert!(app.data.contains_key(&9), "open row keeps its data");
        assert!(app.recent.contains(&9));
        assert_eq!(app.scroll, 12);
        assert_eq!(app.hover, Some(9));
    }

    #[test]
    fn section_progress_rides_the_section_channel() {
        let mut app = sidebar_app(&[7]);
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        tx.send(github::SectionEvent::Progress { done: 120, total: 500 }).unwrap();
        poll_sections(&mut app);
        assert_eq!(app.section_progress, Some((120, 500)));
        assert!(app.section_rx.is_some(), "progress keeps the receiver live");
        tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
        poll_sections(&mut app);
        assert_eq!(app.section_progress, None);
        assert!(app.section_rx.is_none(), "terminal result releases the receiver");
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
        app.issue_tx.send((7, github::IssueEvent::Done(Ok((fresh, vec![]))))).unwrap();
        poll_issues(&mut app);
        assert!(app.data.contains_key(&7));
    }

    #[test]
    fn slim_row_completes_on_open_with_refs_recomputed() {
        // Slim sections rows land partial with no body; opening one runs
        // the full fetch, which completes the row and recomputes refs.
        let mut app = sidebar_app(&[7]);
        let mut slim = issue(9, "2026-01-06T00:00:00Z");
        slim.body = String::new();
        land_sections(&mut app, vec![section_row(slim)]);
        assert!(app.partial.contains(&9), "slim row lands partial");
        assert!(app.data.get(&9).unwrap().body.is_empty());

        app.show_issue(9);
        assert!(app.issue_pending.contains(&9), "open runs the full fetch");
        assert!(app.ref_numbers.is_empty(), "no refs from an empty slim body");

        let mut full = issue(9, "2026-01-06T00:00:00Z");
        full.body = "see #7".to_string();
        full.paired = vec![model::PairedRef { number: 12, state: "OPEN".to_string() }];
        app.issue_tx.send((9, github::IssueEvent::Done(Ok((full, vec![]))))).unwrap();
        poll_issues(&mut app);
        assert!(!app.partial.contains(&9), "full fetch completes the row");
        assert_eq!(app.data.get(&9).unwrap().body, "see #7");
        assert_eq!(app.ref_numbers, vec![7], "refs recompute from the full body");
        assert_eq!(app.ref_paired, vec![12]);
    }

    #[test]
    fn history_back_and_forward_walk_explicit_visits() {
        let mut app = sidebar_app(&[7, 9, 11]);

        app.ensure_visited(7);
        app.load_issue(7, false);
        app.ensure_visited(9);
        app.load_issue(9, false);
        app.ensure_visited(11);
        app.load_issue(11, false);
        assert_eq!(app.current, Some(11));
        assert_eq!(app.history, vec![7, 9]);

        app.go_back();
        assert_eq!(app.current, Some(9));
        assert_eq!(app.history, vec![7]);
        assert_eq!(app.future, vec![11]);

        app.go_back();
        assert_eq!(app.current, Some(7));
        assert_eq!(app.history, Vec::<u64>::new());
        assert_eq!(app.future, vec![11, 9]);

        // An empty back stack toasts instead of moving.
        app.go_back();
        assert_eq!(app.current, Some(7));
        assert_eq!(app.message, "no older issue");

        app.go_forward();
        assert_eq!(app.current, Some(9));
        assert_eq!(app.history, vec![7]);
        assert_eq!(app.future, vec![11]);

        // A new jump drops the forward chain.
        app.ensure_visited(11);
        app.load_issue(11, false);
        assert_eq!(app.current, Some(11));
        assert_eq!(app.history, vec![7, 9]);
        assert_eq!(app.future, Vec::<u64>::new());

        // An empty forward stack toasts instead of moving.
        app.go_forward();
        assert_eq!(app.current, Some(11));
        assert_eq!(app.message, "no newer issue");
    }

    #[test]
    fn compose_needs_a_loaded_issue() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.open_compose();
        assert!(app.compose.is_none());
        assert_eq!(app.message, "view an issue first");

        let mut app = sidebar_app(&[7]);
        app.open_compose();
        let c = app.compose.as_ref().expect("compose");
        assert_eq!((c.number, c.body.as_str(), c.cursor), (7, "", 0));
        assert_eq!(c.target, ComposeTarget::Thread);
    }

    /// PR with one file, one thread on its second line, and the cursor
    /// parked by absolute body row. Rows mirror `ui` rendering: header,
    /// dented patch rows, then the thread block.
    fn thread_pr_app(diff_line: usize) -> App {
        let mut app = sidebar_app(&[9]);
        let d = app.data.get_mut(&9).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(2),
            ..Default::default()
        }];
        app.diffs.insert(
            9,
            vec![crate::model::DiffFile {
                filename: "a.rs".into(),
                patch: "@@ -1,2 +1,2 @@\n a\n+b".into(),
                ..Default::default()
            }],
        );
        app.tab = Tab::Visual;
        app.body_rows = vec![
            "▸ a.rs",
            "  @@ -1,2 +1,2 @@",
            "   a",
            "  +b",
            "      \u{f075} line 2 · unresolved",
            "      reviewer",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
        app.body_base = 0;
        app.diff_line = Some(diff_line);
        app.diff_row_files = vec![None, None, None, Some(0), Some(0), Some(0)];
        app
    }

    #[test]
    fn compose_on_a_thread_row_targets_the_reply() {
        let mut app = thread_pr_app(4);
        app.open_compose();
        let c = app.compose.as_ref().expect("compose");
        assert_eq!(c.target, ComposeTarget::Reply { thread_id: "t1".into() });
    }

    #[test]
    fn compose_on_a_code_row_still_targets_the_line() {
        let mut app = thread_pr_app(3);
        app.open_compose();
        let c = app.compose.as_ref().expect("compose");
        assert_eq!(c.target, ComposeTarget::Line { path: "a.rs".into(), line: 2 });
    }

    #[test]
    fn compose_edits_at_the_cursor() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        for ch in "hi\nok".chars() {
            app.compose_insert(ch);
        }
        assert_eq!(app.compose.as_ref().unwrap().body, "hi\nok");
        app.compose_step(-3);
        app.compose_insert('!');
        assert_eq!(app.compose.as_ref().unwrap().body, "hi!\nok");
        app.compose_backspace();
        assert_eq!(app.compose.as_ref().unwrap().body, "hi\nok");
        // Down keeps the column across lines.
        app.compose_line(1);
        app.compose_insert('?');
        assert_eq!(app.compose.as_ref().unwrap().body, "hi\nok?");
    }

    #[test]
    fn compose_kills_stop_at_newlines() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        for ch in "one\ntwo".chars() {
            app.compose_insert(ch);
        }
        app.compose_step(-3);
        assert_eq!(app.compose.as_ref().unwrap().cursor, 4);
        app.compose_kill_to_end();
        assert_eq!(app.compose.as_ref().unwrap().body, "one\n");
        // On the newline itself the kill eats it, joining the lines.
        app.compose_step(-1);
        app.compose_kill_to_end();
        assert_eq!(app.compose.as_ref().unwrap().body, "one");
        assert_eq!(app.compose.as_ref().unwrap().kill, "\n");
        app.compose_yank();
        assert_eq!(app.compose.as_ref().unwrap().body, "one\n");
    }

    #[test]
    fn compose_kill_to_start_keeps_the_tail() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        for ch in "ab\ncd".chars() {
            app.compose_insert(ch);
        }
        app.compose_step(-1);
        app.compose_kill_to_start();
        assert_eq!(app.compose.as_ref().unwrap().body, "ab\nd");
        assert_eq!(app.compose.as_ref().unwrap().kill, "c");
    }

    #[test]
    fn compose_editor_run_roundtrips_through_a_real_subprocess() {
        // cargo test owns no TTY: raw-mode and alt-screen calls fail
        // and are ignored, so this runs the whole suspend-spawn-land
        // path headless. `true` edits nothing and exits 0, so the draft
        // lands byte-identical through the temp file.
        let _env = crate::cache::test_env_lock().lock().unwrap();
        let prior = std::env::var("EDITOR").ok();
        std::env::set_var("EDITOR", "true");
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        for ch in "draft ✓".chars() {
            app.compose_insert(ch);
        }
        app.editor_requested = true;
        let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
        let mut terminal = ratatui::Terminal::new(backend).expect("headless terminal");
        run_compose_editor(&mut app, &mut terminal);
        assert!(!app.editor_requested, "the run disarms itself");
        assert_eq!(app.compose.as_ref().unwrap().body, "draft ✓");
        assert!(app.message.contains("edited in true"), "toast: {}", app.message);
        // A failing editor keeps the draft.
        std::env::set_var("EDITOR", "false");
        app.editor_requested = true;
        run_compose_editor(&mut app, &mut terminal);
        assert_eq!(app.compose.as_ref().unwrap().body, "draft ✓");
        assert!(app.message.contains("draft kept"), "toast: {}", app.message);
        // A missing binary names the failure and keeps the draft.
        std::env::set_var("EDITOR", "/nonexistent-ghit-editor-xyz");
        app.editor_requested = true;
        run_compose_editor(&mut app, &mut terminal);
        assert_eq!(app.compose.as_ref().unwrap().body, "draft ✓");
        assert!(app.message.contains("failed to start"), "toast: {}", app.message);
        match prior {
            Some(v) => std::env::set_var("EDITOR", v),
            None => std::env::remove_var("EDITOR"),
        }
    }

    #[test]
    fn compose_lands_editor_text_at_the_end() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        app.land_editor_text("edited\ntext".to_string(), "true");
        let c = app.compose.as_ref().unwrap();
        assert_eq!((c.body.as_str(), c.cursor), ("edited\ntext", 11));
        assert!(app.message.contains("true"), "toast names the editor");
    }

    #[test]
    fn compose_empty_submit_stays_open() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        app.submit_compose();
        let c = app.compose.as_ref().expect("draft kept");
        assert!(!c.sending);
        assert!(app.message.contains("empty"), "status names the problem");
    }

    #[test]
    fn compose_poll_success_closes_and_refreshes() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        app.compose.as_mut().unwrap().body = "looks good".to_string();
        app.compose.as_mut().unwrap().sending = true;
        app.compose_tx.send((7, Ok(()))).unwrap();
        poll_compose(&mut app);
        assert!(app.compose.is_none());
        assert_eq!(app.message, "comment posted to #7");
        assert!(app.issue_pending.contains(&7), "refetch shows the new comment");
    }

    #[test]
    fn compose_poll_failure_keeps_the_draft() {
        let mut app = sidebar_app(&[7]);
        app.open_compose();
        app.compose.as_mut().unwrap().body = "looks good".to_string();
        app.compose.as_mut().unwrap().sending = true;
        app.compose_tx.send((7, Err("token scopes deny comments".to_string()))).unwrap();
        poll_compose(&mut app);
        let c = app.compose.as_ref().expect("draft kept");
        assert!(!c.sending);
        assert_eq!(c.body, "looks good");
        assert_eq!(app.message, "token scopes deny comments");
    }

    fn pending_nav_app() -> App {
        let mut app = sidebar_app(&[7, 9]);
        app.show_issue(9);
        let d = app.data.get_mut(&9).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(2),
            comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
            ..Default::default()
        }];
        app
    }

    #[test]
    fn pending_nav_guard_trips_and_cancel_stays() {
        let mut app = pending_nav_app();
        app.request_issue(7);
        let dialog = app.confirm.as_ref().expect("guard trips");
        assert_eq!(dialog.title, "Unsubmitted comments");
        assert!(dialog.body.iter().any(|l| l.contains("#9")), "names the issue: {:?}", dialog.body);
        assert_eq!(dialog.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Submit them", "Discard", "Cancel"]);
        assert_eq!(app.current, Some(9), "no navigation while the dialog is up");
        app.confirm_cancel();
        assert!(app.confirm.is_none());
        assert_eq!(app.current, Some(9), "cancel stays put");
    }

    #[test]
    fn pending_nav_discard_navigates_without_submitting() {
        let mut app = pending_nav_app();
        app.request_issue(7);
        app.confirm_choose(1);
        assert!(app.confirm.is_none());
        assert_eq!(app.current, Some(7));
        assert!(app.review_submitting.is_empty(), "discard never submits");
    }

    #[test]
    fn pending_nav_submit_navigates_and_submits() {
        let mut app = pending_nav_app();
        app.request_issue(7);
        app.confirm_choose(0);
        assert!(app.confirm.is_none());
        assert_eq!(app.current, Some(7), "submit still navigates");
        assert!(app.review_submitting.contains(&9), "submit starts the review submit");
    }

    #[test]
    fn pending_nav_tab_switch_guards_and_same_target_passes() {
        let mut app = pending_nav_app();
        app.request_tab(crate::app::Tab::Visual);
        assert!(app.confirm.is_some(), "tab switch trips the guard");
        app.confirm_choose(1);
        assert_eq!(app.tab, crate::app::Tab::Visual);
        // Same-target requests never open the dialog.
        let mut plain = sidebar_app(&[7, 9]);
        plain.show_issue(9);
        plain.request_issue(9);
        assert!(plain.confirm.is_none());
        plain.request_tab(plain.tab);
        assert!(plain.confirm.is_none());
        // Without pending threads everything navigates directly.
        plain.request_issue(7);
        assert!(plain.confirm.is_none());
        assert_eq!(plain.current, Some(7));
    }

    #[test]
    fn pending_submit_counts_threads_and_lands() {
        let mut app = sidebar_app(&[9]);
        let d = app.data.get_mut(&9).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(2),
            comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
            ..Default::default()
        }];
        app.current = Some(9);
        assert_eq!(app.pending_threads(), 1);
        // Landing a submit refetches so the Diff tab shows submitted threads.
        app.review_submitting.insert(9);
        app.review_submit_tx.send((9, Ok(()))).unwrap();
        assert!(poll_review_submit(&mut app));
        assert!(!app.review_submitting.contains(&9));
        assert_eq!(app.message, "pending review submitted on #9");
        assert!(app.issue_pending.contains(&9));
        // No pending threads means a note, never a send.
        let d = app.data.get_mut(&9).unwrap();
        d.review_threads.clear();
        assert_eq!(app.pending_threads(), 0);
        app.submit_pending_review();
        assert!(app.review_submitting.is_empty());
        assert_eq!(app.message, "no pending comments to submit");
    }

    #[test]
    fn selection_tracks_current_not_hover() {
        let mut app = sidebar_app(&[7, 9, 11]);
        // Click/Enter/j/k/h/l all funnel through `show_issue`.
        app.show_issue(9);
        assert_eq!(app.current, Some(9));
        assert_eq!(app.issue_state.selected(), Some(2));
        assert_eq!(app.pr_state.selected(), None);
        // A wheel hover never moves the tracked row.
        app.hover = Some(11);
        assert_eq!(app.effective(&app.issue_rows()), Some(9));
        // Committing from the hover lands on it and clears it.
        app.ensure_visited(11);
        app.load_issue(11, false);
        assert_eq!(app.current, Some(11));
        assert_eq!(app.hover, None, "a commit clears the hover");
        assert_eq!(app.issue_state.selected(), Some(3));
        assert_eq!(app.pr_state.selected(), None);
    }

    #[test]
    fn step_issue_walks_cached_neighbors() {
        let mut app = sidebar_app(&[589, 590, 591]);
        app.show_issue(590);
        app.step_issue(1);
        assert_eq!(app.current, Some(591), "l opens #591");
        assert_eq!(app.hover, None);
        assert_eq!(app.step_pending, None, "cached landing clears the step");
        assert!(app.visited.contains(&591), "explicit visit files the row");
        assert_eq!(app.issue_state.selected(), Some(3));
        app.step_issue(-1);
        assert_eq!(app.current, Some(590), "h opens #590");
        assert_eq!(app.issue_state.selected(), Some(2));
        app.step_issue(-1);
        assert_eq!(app.current, Some(589), "h opens #589");
        assert_eq!(app.issue_state.selected(), Some(1));
    }

    #[test]
    fn step_issue_without_current_toasts() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.step_issue(1);
        assert_eq!(app.message, "view an issue first");
        assert!(app.issue_pending.is_empty(), "no fetch without an issue");
        assert_eq!(app.step_pending, None);
    }

    #[test]
    fn step_issue_below_one_toasts_without_fetch() {
        let mut app = sidebar_app(&[1, 2]);
        app.show_issue(1);
        app.step_issue(-1);
        assert_eq!(app.message, "no #0");
        assert_eq!(app.current, Some(1), "nothing moves");
        assert!(app.issue_pending.is_empty(), "h on #1 issues no fetch");
        assert_eq!(app.requested, Some(1));
        assert_eq!(app.step_pending, None);
    }

    #[test]
    fn step_issue_records_pending_until_the_fetch_lands() {
        let mut app = sidebar_app(&[590]);
        app.show_issue(590);
        app.step_issue(1);
        assert_eq!(app.current, Some(590), "current waits for the fetch");
        assert_eq!(app.requested, Some(591));
        assert_eq!(app.step_pending, Some(591));
        assert!(app.issue_pending.contains(&591), "neighbor fetch in flight");
        assert!(app.visited.contains(&591), "explicit visit files the row");
        // The landing clears the step and moves the highlight.
        let fresh = issue(591, "2026-01-06T00:00:00Z");
        app.issue_tx.send((591, github::IssueEvent::Done(Ok((fresh, vec![]))))).unwrap();
        poll_issues(&mut app);
        assert_eq!(app.current, Some(591));
        assert_eq!(app.step_pending, None);
        assert_eq!(app.issue_state.selected(), Some(2));
    }

    #[test]
    fn step_404_toasts_no_number_and_stays_put() {
        let mut app = sidebar_app(&[590]);
        app.show_issue(590);
        app.step_pending = Some(589);
        app.requested = Some(589);
        app.issue_tx
            .send((589, github::IssueEvent::Done(Err(
                "GitHub request failed: HTTP status client error (404 Not Found) for url (https://api.github.com/repos/o/r/issues/589)".to_string(),
            ))))
            .unwrap();
        poll_issues(&mut app);
        assert_eq!(app.message, "no #589");
        assert_eq!(app.current, Some(590), "nothing moves past the end");
        assert_eq!(app.step_pending, None);
    }

    #[test]
    fn auto_refresh_failures_notify_instead_of_going_silent() {
        let mut app = App::new(Some("o/r".to_string()));
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        app.section_auto_refresh = true;
        tx.send(github::SectionEvent::Done(Err("GitHub request failed: 403".to_string()))).unwrap();
        poll_sections(&mut app);
        assert!(app.section_rx.is_none(), "terminal result clears the channel");
        assert!(app.message.contains("403"), "auto error surfaces: {}", app.message);
    }

    #[test]
    fn auth_probe_landing_drives_the_prompt() {
        // Missing scopes raise the modal; full coverage stays silent;
        // a dead token names its cause with no scopes to list.
        let mut app = App::new(Some("o/r".to_string()));
        for (result, expect_prompt) in [
            (Ok(vec!["project".to_string()]), true),
            (Ok(vec![]), false),
            (Err("GitHub auth check failed: 401 Unauthorized".to_string()), true),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            app.auth_rx = Some(rx);
            tx.send(result).unwrap();
            poll_auth(&mut app);
            assert!(app.auth_rx.is_none(), "probe lands once");
            assert_eq!(app.auth_prompt.is_some(), expect_prompt);
            app.auth_prompt = None;
        }
    }

    #[test]
    fn auth_failure_prompts_once_then_stays_quiet() {
        let mut app = App::new(Some("o/r".to_string()));
        auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
        let prompt = app.auth_prompt.as_ref().expect("first auth failure prompts");
        assert!(!prompt.scopes_known, "fetch failures cannot name the gap");
        app.auth_prompt = None;
        app.auth_dismissed = true;
        auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
        assert!(app.auth_prompt.is_none(), "dismissed stays dismissed");
        app.auth_dismissed = false;
        auth_failure(&mut app, "bad search payload");
        assert!(app.auth_prompt.is_none(), "non-auth never prompts");
    }

    #[test]
    fn auth_probe_non_auth_failure_notifies_without_modal() {
        // A rate limit is not a scope problem: status line, never the
        // re-auth modal.
        let mut app = App::new(Some("o/r".to_string()));
        let (tx, rx) = std::sync::mpsc::channel();
        app.auth_rx = Some(rx);
        tx.send(Err("GitHub auth check failed: 403 Forbidden for https://api.github.com/user: rate limit".to_string())).unwrap();
        poll_auth(&mut app);
        assert!(app.auth_prompt.is_none(), "no modal for quota errors");
        assert!(app.message.contains("403"), "still surfaces: {}", app.message);
    }

    #[test]
    fn step_other_error_keeps_the_existing_error_path() {
        let mut app = sidebar_app(&[590]);
        app.show_issue(590);
        app.step_pending = Some(591);
        app.requested = Some(591);
        let raw = "GitHub request failed: error sending request for url (https://api.github.com/repos/o/r/issues/591)".to_string();
        app.issue_tx.send((591, github::IssueEvent::Done(Err(raw.clone())))).unwrap();
        poll_issues(&mut app);
        assert_eq!(app.message, raw, "network errors pass through unchanged");
        assert_eq!(app.current, Some(590), "nothing moves on error");
        assert_eq!(app.step_pending, None);
    }
}
