//! App: state, transitions, event loop, background service wiring.
//!
//! `App` owns the data (issues, pins, board, channels) and coordinates
//! fetching through `github`. It never draws (`ui`) and never parses input
//! (`input`); pure rules live in `core`. Network work runs on threads and
//! lands via channels polled once per frame, so the loop never blocks.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Body,
    Comments,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Main,
    Bar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveList {
    Issues,
    Prs,
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
    pub(crate) issues: Vec<u64>,
    pub(crate) prs: Vec<u64>,
    pub(crate) pinned: HashSet<u64>,
    pub(crate) pin_order: Vec<u64>,
    pub(crate) current: Option<u64>,
    pub(crate) tab: Tab,
    pub(crate) focus: Focus,
    pub(crate) bar: String,
    pub(crate) scroll: u16,
    pub(crate) message: String,
    pub(crate) message_at: Option<std::time::Instant>,
    pub(crate) issue_state: ListState,
    pub(crate) pr_state: ListState,
    pub(crate) active_list: ActiveList,
    pub(crate) quit: bool,
    pub(crate) body_area: Rect,
    pub(crate) body_rows: Vec<String>,
    pub(crate) board_loading: bool,
    pub(crate) board_rx: Option<std::sync::mpsc::Receiver<Result<BoardData, String>>>,
    pub(crate) board_states: Vec<ListState>,
    pub(crate) board_expanded: Option<(usize, usize)>,
    pub(crate) merged_window_hours: u64,
    pub(crate) board_details: HashMap<u64, model::BoardDetail>,
    pub(crate) detail_pending: HashSet<u64>,
    pub(crate) detail_tx: std::sync::mpsc::Sender<(u64, Result<model::BoardDetail, String>)>,
    pub(crate) detail_rx: std::sync::mpsc::Receiver<(u64, Result<model::BoardDetail, String>)>,
    // Last-rendered hit areas for mouse clicks.
    pub(crate) side_issue_area: Rect,
    pub(crate) side_pr_area: Rect,
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
            issues: vec![],
            prs: vec![],
            pinned: HashSet::new(),
            pin_order: vec![],
            current: None,
            tab: Tab::Body,
            focus: Focus::Main,
            bar: String::new(),
            scroll: 0,
            message: String::new(),
            message_at: None,
            issue_state: ListState::default(),
            pr_state: ListState::default(),
            active_list: ActiveList::Issues,
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
            quit: false,
            body_area: Rect::default(),
            body_rows: Vec::new(),
            side_issue_area: Rect::default(),
            side_pr_area: Rect::default(),
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

    /// File `number` under its section. Clicked and fetched rows join the
    /// sidebar even when their data arrived earlier (warm cache, board
    /// detail prefetch) and the fetch path below is skipped.
    pub(crate) fn ensure_listed(&mut self, number: u64, is_pr: bool) {
        let key = if is_pr { "pr" } else { "issue" };
        let list = if is_pr { &mut self.prs } else { &mut self.issues };
        if !list.contains(&number) {
            list.push(number);
            self.sort_section(key);
        }
    }

    /// Pull open PRs plus PRs merged inside the window into the section.
    /// Offline or out of a repo this is a no-op: clicked rows still list.
    pub(crate) fn merge_pr_sections(&mut self) {
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return };
        let hours = self.merged_window_hours;
        let (tx, rx) = std::sync::mpsc::channel();
        self.section_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(github::fetch_pr_sections(&repo, hours));
        });
    }

    pub(crate) fn window_label(&self) -> String {
        core::window_label(self.merged_window_hours)
    }

    pub(crate) fn load_issue(&mut self, number: u64, force: bool) {
        self.requested = Some(number);
        if !force && self.data.contains_key(&number) && !self.partial.contains(&number) {
            let is_pr = self.data[&number].is_pr;
            self.ensure_listed(number, is_pr);
            self.show_issue(number);
            self.bar.clear();
            self.focus = Focus::Main;
            return;
        }
        if !self.issue_pending.insert(number) { return; }
        let tx = self.issue_tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        std::thread::spawn(move || {
            let _ = tx.send((number, github::fetch_number(&repo, number, force)));
        });
    }

    fn sort_section(&mut self, sec: &str) {
        let list = if sec == "pr" { &mut self.prs } else { &mut self.issues };
        *list = core::apply_pin_order(list, &self.pin_order, &self.pinned);
    }

    pub(crate) fn show_issue(&mut self, n: u64) {
        self.requested = Some(n);
        if self.partial.contains(&n) { self.load_issue(n, false); }
        self.current = Some(n);
        self.scroll = 0;
        if let Some(d) = self.data.get(&n) {
            let is_pr = d.is_pr;
            if is_pr {
                self.active_list = ActiveList::Prs;
                self.pr_state.select(self.prs.iter().position(|x| *x == n));
                self.issue_state.select(None);
            } else {
                self.active_list = ActiveList::Issues;
                self.issue_state.select(self.issues.iter().position(|x| *x == n));
                self.pr_state.select(None);
            }
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
        let sec = if self.data.get(&n).map(|d| d.is_pr).unwrap_or(false) {
            "pr"
        } else {
            "issue"
        };
        self.sort_section(sec);
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
        self.merge_pr_sections();
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

    pub(crate) fn submit_bar(&mut self) {
        match core::parse_bar(&self.bar.clone()) {
            core::BarCommand::Numbers(numbers) => {
                for n in numbers {
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
        std::thread::spawn(move || {
            let _ = tx.send(crate::gh::fetch_board(&owner));
        });
    }

    /// Snapshot the visible body text so clicks can land on `#123` refs.
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
    }
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

/// Fetch dates and comment counts for every card, in the background.
/// Cards fill in progressively; warm issue cache hits skip the network.
fn kick_off_details(app: &mut App) {
    let Some(board) = &app.board else { return };
    let tx = app.detail_tx.clone();
    for item in &board.items {
        let (Some(n), Some(repo)) = (item.number, item.repo.clone()) else {
            continue;
        };
        if app.board_details.contains_key(&n) || app.detail_pending.contains(&n) {
            continue;
        }
        if let Some(d) = app.data.get(&n) {
            app.board_details.insert(n, model::BoardDetail::from_issue(d));
            continue;
        }
        app.detail_pending.insert(n);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((n, github::fetch_board_detail(&repo, n)));
        });
    }
}

fn poll_details(app: &mut App) {
    let ready: Vec<(u64, Result<model::BoardDetail, String>)> = app.detail_rx.try_iter().collect();
    for (n, res) in ready {
        app.detail_pending.remove(&n);
        if let Ok(d) = res {
            app.board_details.insert(n, d);
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
    std::thread::spawn(move || {
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
        for n in pins.into_iter().chain(numbers) { app.load_issue(n, false); }
        app.merge_pr_sections();
    } else {
        for n in numbers { app.load_issue(n, false); }
    }
}

pub(crate) fn poll_sections(app: &mut App) {
    let Some(result) = app.section_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
    app.section_rx = None;
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
                    app.ensure_listed(n, true);
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

fn poll_issues(app: &mut App) {
    let ready: Vec<_> = app.issue_rx.try_iter().collect();
    for (number, result) in ready {
        app.issue_pending.remove(&number);
        match result {
            Ok((data, statuses)) => {
                let is_pr = data.is_pr;
                app.partial.remove(&number);
                app.data.insert(number, data);
                app.statuses.insert(number, statuses);
                app.ensure_listed(number, is_pr);
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
