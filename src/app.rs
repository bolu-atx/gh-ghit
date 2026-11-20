//! App: state, transitions, event loop, background service wiring.
//!
//! `App` owns the data (issues, pins, board, channels) and coordinates
//! fetching through `github`. It never draws (`ui`) and never parses input
//! (`input`); pure rules live in `core`. Network work runs on one shared
//! tokio runtime and lands via channels polled once per frame, so the loop
//! never blocks.


mod backfill;
mod board;
mod compose;
mod confirm;
mod dialogs;
pub(crate) use dialogs::{Entry, FormatEdit, Slot};
mod diff;
mod fetch_queue;
mod finder;
mod format;
mod issue;
mod map;
mod map_filter;
mod map_physics;
mod map_rank;
pub(crate) use map::{CardHit, EdgeGrid, MapCard, MapView};
pub(crate) use map_filter::FilterRow;
#[cfg(test)]
pub(crate) use map::CardDetail;
mod menu;
pub(crate) use menu::{MenuKind, MenuState};
pub(crate) use finder::FinderRow;
mod nav;
mod poll;
mod search;
pub(crate) use search::{ContentMatch, ContentSearch};
mod sidebar;

#[cfg(test)]
mod sidebar_tests;
#[cfg(test)]
mod dialogs_tests;
#[cfg(test)]
mod nav_tests;
#[cfg(test)]
mod issue_tests;
#[cfg(test)]
mod confirm_tests;
#[cfg(test)]
mod poll_tests;
#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod test_support;

use std::io::Stdout;
use std::time::{Duration, Instant};

/// Blocking poll wait per frame: short enough that batched mouse/key
/// input feels instant, long enough to avoid a busy loop.
pub(crate) const POLL_TIMEOUT_MS: u64 = 15;
/// Click-flash dwell: the clicked rect inverts for ~150ms so the press
/// reads as acknowledged before its fetch lands.
pub(crate) const CLICK_FLASH_MS: u64 = 150;
/// Spinner frame dwell: while a fetch runs the loop repaints at this
/// cadence for the animation, not on every 15ms poll pass.
pub(crate) const SPIN_FRAME_MS: u64 = 100;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::ExecutableCommand;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;

use crate::core::{self, Layer, Pane};
use crate::{github, model};
use poll::poll_bootstrap;
pub(crate) use diff::{poll_review_submit, poll_diffs, poll_head_files, poll_sub_issues};
pub(crate) use board::{board_open, board_columns, board_move, board_selected_number, poll_board};
pub(crate) use compose::{poll_compose, run_compose_editor};
pub(crate) use confirm::{poll_auth, poll_set_state, auth_failure, run_reauth};
pub(crate) use poll::{poll_details, bootstrap, poll_repo_meta, poll_sections, poll_issues};
pub(crate) use confirm::{AuthState, ConfirmDialog, ConfirmKind, PendingUndo, UndoAction};
pub(crate) use board::BoardView;
pub(crate) use compose::ComposeSlot;
#[cfg(test)]
pub(crate) use compose::{ComposeState, ComposeTarget};
pub(crate) use diff::{DiffView, SubIssueLoads};
pub(crate) use sidebar::{FilterItem, LabelsPane, QueryAction, SectionFeed, Sidebar};
pub(crate) use issue::{IssueStore, RefsPane};
pub(crate) use nav::Nav;

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

/// Spawn a background task on the shared runtime. Unit tests never run
/// it: the task is dropped, so no test can reach the network.
fn spawn_bg<F: std::future::Future<Output = ()> + Send + 'static>(task: F) {
    if cfg!(test) {
        drop(task);
        return;
    }
    rt().spawn(task);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Details,
    /// A PR's diff.
    Visual,
    /// The dependency map. It needs no open item.
    Map,
}

/// Content tabs in order, each with its title and `1`/`2`/`3` shortcut.
pub(crate) const TABS: [(Tab, &str); 3] = [
    (Tab::Details, "\u{f15c}  1 Details"),
    (Tab::Visual, "\u{f121}  2 Diff"),
    (Tab::Map, "\u{f279}  3 Map"),
];

/// The tab a click `rel` columns into the tab line selects: each title
/// owns its cells plus the divider after it; the last takes the rest.
pub(crate) fn tab_at(rel: u16) -> Tab {
    let mut end = 0;
    for (tab, title) in TABS {
        end += title.chars().count() as u16 + 1;
        if rel < end {
            return tab;
        }
    }
    Tab::Map
}

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
    Back(usize),
    Forward,
    Evict,
}

/// Who owns input right now: the top open layer, plus the active pane
/// beneath it. `App::focus` is the single derivation input and render read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Focused {
    pub(crate) layer: Layer,
    pub(crate) pane: Pane,
}

/// Last-rendered hit areas for mouse clicks. The row maps hold the
/// display order per pane, `None` marking a divider slot.
#[derive(Default)]
pub(crate) struct HitAreas {
    pub(crate) body_area: Rect,
    pub(crate) body_rows: Vec<String>,
    /// Title/label block rendered above the content tabs. Clicks here
    /// resolve issue refs and URLs with the same mapping as the body.
    pub(crate) header_area: Rect,
    pub(crate) header_rows: Vec<String>,
    /// Scroll offset `body_rows` was captured at; selections add to this.
    pub(crate) body_base: u16,
    /// Rows of the whole body, wrapped to the pane width. `G` scrolls to it.
    pub(crate) body_total: usize,
    pub(crate) side_issue_area: Rect,
    pub(crate) side_pr_area: Rect,
    pub(crate) side_issue_rows: Vec<model::SideRow>,
    pub(crate) side_pr_rows: Vec<model::SideRow>,
    /// Drawn height of each row in the matching `*_rows` list, recorded by
    /// `render` (an expanded row includes its detail block).
    pub(crate) side_issue_heights: Vec<usize>,
    pub(crate) side_pr_heights: Vec<usize>,
    pub(crate) ref_heights: Vec<usize>,
    pub(crate) refs_area: Rect,
    pub(crate) labels_area: Rect,
    /// Labels pane rows as drawn: rect plus the `filter_items` index.
    pub(crate) labels_rows: Vec<(Rect, usize)>,
    pub(crate) history_rows: Vec<(Rect, usize)>,
    pub(crate) history_area: Rect,
    pub(crate) ref_rows: Vec<model::SideRow>,
    pub(crate) tabs_area: Rect,
    pub(crate) bar_area: Rect,
    pub(crate) board_cells: Vec<(Rect, usize, usize)>,
    /// Board header link hit areas (top-right icons): rect plus the URL
    /// to open. Rebuilt every frame in `render_board`, like `board_cells`.
    pub(crate) board_links: Vec<(Rect, String)>,
    /// Format dialog and board filter form: the popup rect (empty while
    /// closed) and one cell per focusable widget or choice option, as
    /// (rect, widget index, option index). Rebuilt every frame.
    pub(crate) form_popup: Rect,
    pub(crate) form_cells: Vec<(Rect, usize, Option<usize>)>,
    /// Confirm modal choice labels as drawn: rect plus choice index.
    /// Rebuilt every frame in `render_confirm`.
    pub(crate) confirm_choices: Vec<(Rect, usize)>,
    /// Status-bar rect of the undo toast while it shows; empty otherwise.
    pub(crate) undo_toast: Rect,
    /// Status-bar hints as drawn: rect plus the action a click runs.
    /// Rebuilt every frame in `render_bar`.
    pub(crate) bar_hints: Vec<(Rect, crate::keymap::Action)>,
    /// Dependency map view rect; empty while the map is not drawn.
    pub(crate) map_area: Rect,
    /// Map node labels as drawn, topmost first: rect plus layout id.
    pub(crate) map_nodes: Vec<(Rect, u64)>,
    /// Map edge cells as drawn, by layout ids of the ends. Node labels
    /// win over them.
    pub(crate) map_edges: crate::app::EdgeGrid,
    /// The map card's rect; empty while no card shows.
    pub(crate) map_card: Rect,
    /// Clickable parts of the map card.
    pub(crate) map_card_hits: Vec<(Rect, CardHit)>,
    /// The ranking strip's rect; empty while it is hidden.
    pub(crate) map_strip: Rect,
    /// Ranking strip rows as drawn: rect plus strip row.
    pub(crate) map_strip_rows: Vec<(Rect, usize)>,
    /// Click flash: the last clicked rect plus when it landed. Render
    /// inverts it for `CLICK_FLASH_MS`, then clears it. Theme-agnostic
    /// by construction (reversed style, no colors).
    pub(crate) last_click: Option<(Rect, Instant)>,
    /// Title and footer controls: rect, action, pane the click focuses.
    pub(crate) controls: Vec<(Rect, crate::keymap::Action, Pane)>,
    /// Dialog footer hints: rect plus the action a click runs.
    pub(crate) legend: Vec<(Rect, crate::keymap::Action)>,
    /// Text cells of the compose modal as drawn; empty while closed.
    pub(crate) compose_text: Rect,
    /// Rect of the top dialog; a click outside it dismisses the dialog.
    pub(crate) popup: Rect,
    /// Menu rows as drawn: rect plus entry index.
    pub(crate) menu_rows: Vec<(Rect, usize)>,
    /// Finder result rows, each with its index in the ranked results.
    pub(crate) finder_rows: Vec<(Rect, usize)>,
    /// The repo segment of the status bar.
    pub(crate) repo: Rect,
    /// The query source segment of the status bar.
    pub(crate) source: Rect,
}

/// Side column widths for the current terminal width, set once per frame
/// by `App::apply_width` before drawing. A wanted column the width removed
/// is `cut`. The saved toggles (`sidebar.show`, `refs.show`) stay untouched.
#[derive(Clone, Copy)]
pub(crate) struct Columns {
    pub(crate) sidebar: u16,
    pub(crate) right: u16,
    sidebar_cut: bool,
    right_cut: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Columns { sidebar: core::SIDEBAR_WIDTH, right: core::RIGHT_WIDTH, sidebar_cut: false, right_cut: false }
    }
}

pub(crate) struct App {
    pub(crate) issues: IssueStore,
    pub(crate) sections: SectionFeed,
    pub(crate) sidebar: Sidebar,
    pub(crate) nav: Nav,
    pub(crate) board: BoardView,
    pub(crate) diff: DiffView,
    pub(crate) map: MapView,
    pub(crate) subs: SubIssueLoads,
    pub(crate) refs: RefsPane,
    pub(crate) labels: LabelsPane,
    pub(crate) auth: AuthState,
    pub(crate) compose: ComposeSlot,
    pub(crate) hits: HitAreas,
    /// `/` search over the Main pane content.
    pub(crate) search: ContentSearch,
    pub(crate) layout: Columns,
    /// Open dialogs and text rows, bottom to top. See `dialogs.rs`.
    pub(crate) dialogs: Vec<Entry>,
    pub(crate) repo_flag: Option<String>,
    pub(crate) repo_slug: Option<String>,
    pub(crate) initial_numbers: Vec<u64>,
    /// True once the startup prefs load has applied. `persist_prefs`
    /// stays silent before that so pre-bootstrap input cannot write
    /// defaults over saved state.
    pub(crate) prefs_loaded: bool,
    pub(crate) bar: String,
    /// First visible line of the help screen. Render clamps it.
    pub(crate) help_scroll: u16,
    /// Help screen filter: rows whose keys, label or section contain it.
    pub(crate) help_filter: String,
    /// `/` on the help screen: typed keys edit `help_filter`.
    pub(crate) help_typing: bool,
    pub(crate) message: String,
    /// First key of a two-key sequence such as `gg`, while it waits.
    pub(crate) pending_prefix: Option<char>,
    pub(crate) message_at: Option<std::time::Instant>,
    pub(crate) quit: bool,
    /// Armed by `x` evict and saved-query delete; `u` consumes it.
    pub(crate) undo: Option<PendingUndo>,
    /// Frame counter, bumped once per event-loop pass. Drives the status
    /// bar spinner; cheap because the loop already redraws every pass.
    pub(crate) tick: u64,
    /// Queue and run-count for every background fetch; see `fetch_queue`.
    fetches: fetch_queue::Fetches,
}

impl App {
    pub(crate) fn new(repo_flag: Option<String>) -> Self {
        let (detail_tx, detail_rx) = std::sync::mpsc::channel();
        let (compose_tx, compose_rx) = std::sync::mpsc::channel();
        let (review_submit_tx, review_submit_rx) = std::sync::mpsc::channel();
        let (issue_tx, issue_rx) = std::sync::mpsc::channel();
        let (diff_tx, diff_rx) = std::sync::mpsc::channel();
        Self {
            repo_flag,
            repo_slug: None,
            prefs_loaded: false,
            auth: AuthState::default(),
            initial_numbers: vec![],
            sections: SectionFeed::new(),
            issues: IssueStore::new(issue_tx, issue_rx),
            nav: Nav::new(),
            bar: String::new(),
            sidebar: Sidebar::new(),
            help_scroll: 0,
            help_filter: String::new(),
            help_typing: false,
            message: String::new(),
            pending_prefix: None,
            message_at: None,
            undo: None,
            compose: ComposeSlot::new(compose_tx, compose_rx, review_submit_tx, review_submit_rx),
            tick: 0,
            fetches: fetch_queue::Fetches::default(),
            board: BoardView::new(detail_tx, detail_rx),
            diff: DiffView::new(diff_tx, diff_rx),
            map: MapView::default(),
            subs: SubIssueLoads::new(),
            quit: false,
            hits: HitAreas::default(),
            search: ContentSearch::default(),
            layout: Columns::default(),
            dialogs: Vec::new(),
            refs: RefsPane::new(),
            labels: LabelsPane::default(),
        }
    }

    /// Status-line message that expires after a few seconds so the
    /// shortcut hints always come back.
    pub(crate) fn notify(&mut self, msg: String) {
        self.message = msg;
        self.message_at = Some(std::time::Instant::now());
    }

    /// Queue a background fetch and start whatever fits. Tiers decide the
    /// start order (`core::urgency`); `fetch_queue` caps what runs at once.
    fn enqueue(
        &mut self,
        kind: core::Kind,
        number: Option<u64>,
        task: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        self.fetches.push(kind, number, Box::pin(task));
        self.pump_fetches();
    }

    /// Start queued fetches while slots are free. While the API quota is
    /// spent only the current issue's fetches start.
    pub(crate) fn pump_fetches(&mut self) {
        let view = fetch_queue::View {
            current: self.nav.requested.or(self.nav.current),
            tab_shown: self.nav.tab == Tab::Visual,
        };
        self.fetches.start(&view, github::rate_limited());
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
        self.hits.last_click = Some((area, Instant::now()));
    }

    /// The flash still owes frames: visible now, or expired and owed one
    /// last draw to clear it.
    pub(crate) fn flash_live(&self) -> bool {
        self.hits.last_click.is_some()
    }

    /// The flash currently paints: inside `CLICK_FLASH_MS`.
    pub(crate) fn flash_visible(&self) -> bool {
        matches!(self.hits.last_click, Some((_, at)) if at.elapsed() < Duration::from_millis(CLICK_FLASH_MS))
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
        let (done, total) = self.issues.progress.values().copied().fold((0, 0), |(d, t), (dd, tt)| (d + dd, t + tt));
        core::FetchState {
            issues: self.issues.pending.len(),
            diffs: self.diff.pending.len(),
            details: self.board.detail_pending.len(),
            sections_live: self.sections.rx.is_some(),
            bootstrap_live: self.auth.bootstrap_rx.is_some(),
            board: self.board.loading,
            message_visible: self.message_visible(),
            issue_progress: if self.issues.progress.is_empty() { None } else { Some((done, total)) },
            section_progress: self.sections.progress,
            backfill: self.backfill_state().0,
            backfill_progress: self.backfill_state().1,
        }
    }

    /// Per-pane loading flags for the pane spinners and loading bodies.
    pub(crate) fn pane_loading(&self) -> core::PaneLoading {
        let shown = self.nav.current.or(self.nav.requested);
        let refs_pending = self
            .refs
            .numbers
            .iter()
            .chain(self.refs.paired.iter())
            .any(|n| self.issues.pending.contains(n));
        core::pane_loading(&core::PaneInputs {
            sections_live: self.sections.rx.is_some(),
            bootstrap_live: self.auth.bootstrap_rx.is_some(),
            requested_pending: self.nav.requested.is_some_and(|n| self.issues.pending.contains(&n)),
            shown_cached: shown.is_some_and(|n| self.issues.data.contains_key(&n)),
            refs_pending,
        })
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

    /// Snapshot the visible body text so clicks can land on `#123` refs.
    /// `body_base` records the scroll offset the snapshot was taken at, so
    /// selections map visible rows back without accumulating across clicks.
    pub(crate) fn capture_body_rows(&mut self, buf: &ratatui::buffer::Buffer) {
        let area = self.hits.body_area;
        self.hits.body_rows = (0..area.height)
            .map(|r| {
                let y = area.y + r;
                (area.x..area.x + area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        self.hits.body_base = self.nav.scroll;
    }
}

/// The board modal is up, whether still fetching or fully loaded.
/// Draw one frame: fit the columns to the terminal width and the map camera
/// to the graph, then render.
pub(crate) fn draw_frame(f: &mut ratatui::Frame, app: &mut App) {
    app.apply_width(f.area().width);
    app.map_before_draw();
    crate::ui::render(f, app);
    // The map's area is known only once it drew. A new area or a pending
    // reveal moves the camera here, and the frame draws again, so no frame
    // shows a stale view.
    if app.map_after_draw() {
        f.buffer_mut().reset();
        crate::ui::render(f, app);
    }
}

/// Every background lander, once per loop pass. The terminal and drive
/// loops share this so both pick up fetches the same way. True when any
/// lander changed visible state, so the loop knows a redraw is owed.
pub(crate) fn poll_all(app: &mut App) -> bool {
    let mut landed = false;
    app.pump_fetches();
    landed |= poll_bootstrap(app);
    landed |= poll_auth(app);
    landed |= poll_sections(app);
    landed |= poll_repo_meta(app);
    landed |= app.watch_store(Instant::now());
    let had_fetch = app.sections.rx.is_some();
    app.auto_refresh_recent(Instant::now());
    landed |= had_fetch != app.sections.rx.is_some();
    landed |= app.drive_backfill(Instant::now());
    landed |= poll_issues(app);
    landed |= poll_diffs(app);
    landed |= poll_head_files(app);
    landed |= app.sync_diff();
    landed |= poll_sub_issues(app);
    landed |= poll_board(app);
    landed |= poll_details(app);
    landed |= poll_compose(app);
    landed |= poll_review_submit(app);
    landed |= poll_set_state(app);
    landed |= app.map_tick();
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
            terminal.draw(|f| draw_frame(f, app)).map_err(|e| e.to_string())?;
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
        if app.auth.reauth_requested {
            run_reauth(app, terminal);
            input_dirty = true;
        }
        if app.compose.editor_requested {
            run_compose_editor(app, terminal);
            input_dirty = true;
        }
        needs_draw = input_dirty || poll_dirty;
        if app.quit {
            return Ok(());
        }
    }
}

/// The one place ghit launches the system browser. Tests get a stub so a
/// suite never opens a real tab.
#[cfg(not(test))]
pub(crate) fn open_in_browser(url: &str) -> std::io::Result<()> {
    open::that(url)
}

#[cfg(test)]
use open as _;


#[cfg(test)]
pub(crate) fn open_in_browser(_url: &str) -> std::io::Result<()> {
    Ok(())
}
