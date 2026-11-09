//! ghit — terminal viewer for GitHub issues and PRs, backed by the `gh` CLI.
//!
//! Punch in an issue number, read the ticket. `o` opens it in GitHub.
//! Usage: `ghit [numbers...] [-R owner/repo]`

mod cache;
mod gh;
mod model;
mod theme;

use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::ExecutableCommand;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};

use model::{BoardData, IssueData, board_columns};

/// Terminal viewer for GitHub issues and PRs.
#[derive(Parser, Debug)]
#[command(name = "ghit", version)]
struct Args {
    /// Issue or PR numbers to open on startup.
    #[arg(value_name = "NUMBER")]
    numbers: Vec<String>,

    /// Repository in owner/repo form (default: `gh repo view` resolution).
    #[arg(short = 'R', long = "repo", value_name = "OWNER/REPO")]
    repo: Option<String>,

}

fn main() {
    let args = Args::parse();
    let numbers: Vec<u64> = args.numbers.iter().filter_map(|s| s.parse().ok()).collect();

    let mut app = App::new(args.repo.clone());

    if let Err(e) = run_tui(&mut app, &numbers) {
        eprintln!("ghit: {e}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Body,
    Comments,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Main,
    Bar,
}

struct App {
    repo_flag: Option<String>,
    repo_slug: Option<String>,
    data: HashMap<u64, IssueData>,
    statuses: HashMap<u64, Vec<(String, Option<String>)>>,
    issues: Vec<u64>,
    prs: Vec<u64>,
    pinned: HashSet<u64>,
    pin_order: Vec<u64>,
    current: Option<u64>,
    tab: Tab,
    focus: Focus,
    bar: String,
    scroll: u16,
    message: String,
    message_at: Option<std::time::Instant>,
    issue_state: ListState,
    pr_state: ListState,
    active_list: ActiveList,
    quit: bool,
    body_area: Rect,
    body_rows: Vec<String>,
    board_loading: bool,
    board_rx: Option<std::sync::mpsc::Receiver<Result<BoardData, String>>>,
    board_states: Vec<ListState>,
    board_expanded: Option<(usize, usize)>,
    // Last-rendered hit areas for mouse clicks.
    side_issue_area: Rect,
    side_pr_area: Rect,
    tabs_area: Rect,
    bar_area: Rect,
    board_cells: Vec<(Rect, usize, usize)>,
    // Board modal.
    board: Option<BoardData>,
    board_col: usize,
    board_row: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveList {
    Issues,
    Prs,
}

impl App {
    fn new(repo_flag: Option<String>) -> Self {
        Self {
            repo_flag,
            repo_slug: None,
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
    fn notify(&mut self, msg: String) {
        self.message = msg;
        self.message_at = Some(std::time::Instant::now());
    }

    fn message_visible(&self) -> bool {
        match (self.message.is_empty(), self.message_at) {
            (false, Some(at)) => at.elapsed() < std::time::Duration::from_secs(4),
            _ => false,
        }
    }

    fn repo_key(&self) -> Option<String> {
        let r = self
            .repo_flag
            .clone()
            .or_else(|| self.repo_slug.clone())?;
        Some(r.to_lowercase())
    }

    fn resolve_repo(&mut self) {
        if self.repo_flag.is_some() {
            self.repo_slug = self.repo_flag.clone();
            return;
        }
        if let Ok(out) = std::process::Command::new("gh")
            .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    self.repo_slug = Some(s);
                }
            }
        }
    }

    fn fetch_all(
        &mut self,
        number: u64,
        force: bool,
    ) -> Result<(IssueData, Vec<(String, Option<String>)>), String> {
        let repo = self
            .repo_flag
            .clone()
            .or_else(|| self.repo_slug.clone())
            .unwrap_or_default();
        let tag = repo.to_lowercase().replace('/', "__");
        let tag = if tag.is_empty() { "local".to_string() } else { tag };
        let iname = format!("issue-{tag}-{number}");
        let sname = format!("status-{tag}-{number}");

        let data: IssueData = if !force {
            match cache::cache_get::<IssueData>(&iname) {
                Some(d) => d,
                None => {
                    let d = gh::gh_issue_json(&repo_for_gh(&repo), number)?;
                    cache::cache_put(&iname, &d);
                    d
                }
            }
        } else {
            let d = gh::gh_issue_json(&repo_for_gh(&repo), number)?;
            cache::cache_put(&iname, &d);
            d
        };

        let statuses: Vec<(String, Option<String>)> = if !force {
            match cache::cache_get(&sname) {
                Some(s) => s,
                None => {
                    let s = gh::gh_project_statuses(&repo, number, data.is_pr);
                    cache::cache_put(&sname, &s);
                    s
                }
            }
        } else {
            let s = gh::gh_project_statuses(&repo, number, data.is_pr);
            cache::cache_put(&sname, &s);
            s
        };
        Ok((data, statuses))
    }

    fn load_issue(&mut self, number: u64, force: bool) {
        if !force && self.data.contains_key(&number) {
            self.show_issue(number);
            self.bar.clear();
            self.focus = Focus::Main;
            return;
        }
        match self.fetch_all(number, force) {
            Ok((data, statuses)) => {
                let n = data.number;
                let is_pr = data.is_pr;
                self.data.insert(n, data);
                self.statuses.insert(n, statuses);
                let list = if is_pr { &mut self.prs } else { &mut self.issues };
                if !list.contains(&n) {
                    list.push(n);
                }
                let key = if is_pr { "pr" } else { "issue" };
                self.sort_section(key);
                self.show_issue(n);
                self.bar.clear();
                self.focus = Focus::Main;
                self.message.clear();
            }
            Err(e) => self.notify(e),
        }
    }

    fn sort_section(&mut self, sec: &str) {
        let list = if sec == "pr" { &mut self.prs } else { &mut self.issues };
        let pins: Vec<u64> = self.pin_order.iter().filter(|n| list.contains(n)).copied().collect();
        let rest: Vec<u64> = list.iter().filter(|n| !self.pinned.contains(n)).copied().collect();
        *list = [pins, rest].concat();
    }

    fn show_issue(&mut self, n: u64) {
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

    fn toggle_pin(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let Some(key) = self.repo_key() else {
            self.notify("pinning needs a repo: use -R or run inside one".to_string());
            return;
        };
        let mut pins = cache::load_pins(&key);
        if pins.contains(&n) {
            pins.retain(|x| *x != n);
            self.pinned.remove(&n);
            self.notify(format!("unpinned #{n}"));
        } else {
            pins.push(n);
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

    fn refresh_current(&mut self) {
        let Some(n) = self.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if let Some(owner) = board_owner(self.repo_flag.clone().or_else(|| self.repo_slug.clone())) {
            cache::cache_drop(&format!("board-{}", owner.to_lowercase()));
        }
        self.load_issue(n, true);
        self.notify(format!("refreshed #{n}"));
    }

    fn open_number(&mut self, n: u64) {
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

    fn submit_bar(&mut self) {
        let text = self.bar.trim().to_string();
        let parts: Vec<&str> = text.split_whitespace().collect();
        if !parts.is_empty() && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
            for p in &parts {
                if let Ok(n) = p.parse::<u64>() {
                    self.load_issue(n, false);
                }
            }
            return;
        }
        match text.as_str() {
            "o" | "open" => {
                if let Some(n) = self.current {
                    self.open_number(n);
                } else {
                    self.notify("view an issue first".to_string());
                }
            }
            "q" | "quit" | "exit" => {}
            _ if parts.len() == 2 && (parts[0] == "o" || parts[0] == "open") && parts[1].chars().all(|c| c.is_ascii_digit()) => {
                if let Ok(n) = parts[1].parse::<u64>() {
                    self.open_number(n);
                }
            }
            "" => {
                self.focus = Focus::Main;
            }
            _ => self.message = "type an issue number, o, or q".to_string(),
        }
        if text == "q" || text == "quit" || text == "exit" {
            // handled by caller
        }
    }
}

fn repo_for_gh(repo: &str) -> String {
    repo.to_string()
}

fn board_owner(repo: Option<String>) -> Option<String> {
    let r = repo?;
    let (owner, _) = r.split_once('/')?;
    if owner.is_empty() { None } else { Some(owner.to_string()) }
}

// ---------------------------------------------------------------------------
// TUI runner
// ---------------------------------------------------------------------------

fn run_tui(app: &mut App, numbers: &[u64]) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen).map_err(|e| e.to_string())?;
    stdout.execute(EnableMouseCapture).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    app.resolve_repo();
    if let Some(key) = app.repo_key() {
        let mut pins = cache::load_pins(&key);
        pins.retain(|n| !numbers.contains(n));
        let mut initial: Vec<u64> = pins.clone();
        initial.extend(numbers.iter().copied());
        app.pinned = pins.iter().copied().collect();
        app.pin_order = pins;
        for n in initial {
            app.load_issue(n, false);
        }
    } else {
        for &n in numbers {
            app.load_issue(n, false);
        }
    }
    if app.current.is_none() {
        app.focus = Focus::Bar;
    }

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
        terminal.draw(|f| render(f, app)).map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) => handle_key(app, key),
                Event::Mouse(m) => handle_mouse(app, m),
                _ => {}
            };
        }
        poll_board(app);
        if app.quit {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    // Board modal captures everything.
    if board_open(app) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.board = None;
                app.board_loading = false;
            }
            KeyCode::Char('j') | KeyCode::Down => board_move(app, 0, 1),
            KeyCode::Char('k') | KeyCode::Up => board_move(app, 0, -1),
            KeyCode::PageDown => board_move(app, 0, 5),
            KeyCode::PageUp => board_move(app, 0, -5),
            KeyCode::Char('h') | KeyCode::Left => board_move(app, -1, 0),
            KeyCode::Char('l') | KeyCode::Right => board_move(app, 1, 0),
            KeyCode::Char('t') => {
                app.notify(format!("theme: {}", theme::cycle()));
            }
            KeyCode::Char(' ') => {
                let pos = (app.board_col, app.board_row);
                app.board_expanded = if app.board_expanded == Some(pos) { None } else { Some(pos) };
            }
            KeyCode::Enter => {
                if let Some(n) = board_selected_number(app) {
                    app.board = None;
                    app.load_issue(n, false);
                }
            }
            _ => {}
        }
        return;
    }

    if app.focus == Focus::Bar {
        match key.code {
            KeyCode::Esc => {
                app.focus = Focus::Main;
                app.bar.clear();
            }
            KeyCode::Enter => {
                let quit = matches!(app.bar.trim(), "q" | "quit" | "exit");
                app.submit_bar();
                if quit {
                    app.quit = true;
                }
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') {
                    app.quit = true;
                    return;
                }
                app.bar.push(c);
            }
            KeyCode::Backspace => {
                app.bar.pop();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('1') => app.tab = Tab::Body,
        KeyCode::Char('2') => app.tab = Tab::Comments,
        KeyCode::Char('t') => {
            app.notify(format!("theme: {}", theme::cycle()));
        }
        KeyCode::Char('b') => open_board(app),
        KeyCode::Char('j') => move_selection(app, 1),
        KeyCode::Char('k') => move_selection(app, -1),
        KeyCode::Down => {
            app.scroll = app.scroll.saturating_add(3);
        }
        KeyCode::Up => {
            app.scroll = app.scroll.saturating_sub(3);
        }
        KeyCode::PageDown => {
            app.scroll = app.scroll.saturating_add(12);
        }
        KeyCode::PageUp => {
            app.scroll = app.scroll.saturating_sub(12);
        }
        KeyCode::Char(':') | KeyCode::Char('/') => app.focus = Focus::Bar,
        KeyCode::Char('o') => {
            if let Some(n) = app.current {
                app.open_number(n);
            } else {
                app.notify("view an issue first".to_string());
            }
        }
        KeyCode::Char('r') => app.refresh_current(),
        KeyCode::Char('p') => app.toggle_pin(),
        KeyCode::Esc => app.bar.clear(),
        KeyCode::Tab => {
            app.tab = if app.tab == Tab::Body { Tab::Comments } else { Tab::Body };
        }
        _ => {}
    }
}

impl App {
    /// Snapshot the visible body text so clicks can land on `#123` refs.
    fn capture_body_rows(&mut self, buf: &ratatui::buffer::Buffer) {
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

fn in_rect(area: Rect, x: u16, y: u16) -> bool {
    area.contains(ratatui::layout::Position::new(x, y))
}

/// Sidebar rows are 3 lines each below a 1-line border plus a 1-line spacer.
fn sidebar_hit(area: Rect, list: &[u64], x: u16, y: u16) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    list.get(((y - area.y - 2) / 3) as usize).copied()
}

fn handle_mouse(app: &mut App, m: MouseEvent) {
    if board_open(app) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = app
                    .board_cells
                    .iter()
                    .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                    .map(|(_, c, r)| (*c, *r));
                if let Some((c, r)) = hit {
                    if app.board_col == c && app.board_row == r {
                        if let Some(n) = board_selected_number(app) {
                            app.board = None;
                            app.board_expanded = None;
                            app.load_issue(n, false);
                        }
                    } else {
                        app.board_col = c;
                        app.board_row = r;
                        app.board_expanded = None;
                    }
                }
            }
            MouseEventKind::ScrollDown => board_move(app, 0, 1),
            MouseEventKind::ScrollUp => board_move(app, 0, -1),
            _ => {}
        }
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if in_rect(app.bar_area, m.column, m.row) {
                app.focus = Focus::Bar;
                return;
            }
            if let Some(n) = sidebar_hit(app.side_issue_area, &app.issues, m.column, m.row)
                .or_else(|| sidebar_hit(app.side_pr_area, &app.prs, m.column, m.row))
            {
                app.focus = Focus::Main;
                app.show_issue(n);
                return;
            }
            if in_rect(app.tabs_area, m.column, m.row) {
                // "Body" is 4 wide, then the divider: left half body, right half comments.
                app.tab = if m.column.saturating_sub(app.tabs_area.x) < 6 {
                    Tab::Body
                } else {
                    Tab::Comments
                };
                return;
            }
            if in_rect(app.body_area, m.column, m.row) {
                app.focus = Focus::Main;
                let row = (m.row - app.body_area.y) as usize;
                if let Some(line) = app.body_rows.get(row) {
                    let repo = app.repo_key();
                    if let Some(&n) = model::refs_in_line(line, repo.as_deref()).first() {
                        app.load_issue(n, false);
                    }
                }
                return;
            }
            app.focus = Focus::Main;
        }
        MouseEventKind::ScrollDown => {
            app.scroll = app.scroll.saturating_add(3);
        }
        MouseEventKind::ScrollUp => {
            app.scroll = app.scroll.saturating_sub(3);
        }
        _ => {}
    }
}

/// j/k walks issues then PRs as one flat list, crossing the section boundary.
fn move_selection(app: &mut App, delta: isize) {
    let order: Vec<u64> = app.issues.iter().chain(app.prs.iter()).copied().collect();
    if order.is_empty() {
        return;
    }
    let last = order.len() - 1;
    let next = match app
        .current
        .and_then(|n| order.iter().position(|x| *x == n))
    {
        Some(pos) => (pos as isize + delta).clamp(0, last as isize) as usize,
        None => {
            if delta > 0 {
                0
            } else {
                last
            }
        }
    };
    app.show_issue(order[next]);
}

/// Modal opens instantly with a skeleton; the network fetch lands in the background.
fn open_board(app: &mut App) {
    if board_open(app) {
        return;
    }
    let owner = board_owner(app.repo_flag.clone().or_else(|| app.repo_slug.clone()));
    let Some(owner) = owner else {
        app.notify("board needs a repo: use -R or run inside one".to_string());
        return;
    };
    app.board_loading = true;
    app.board_col = 0;
    app.board_row = 0;
    let (tx, rx) = std::sync::mpsc::channel();
    app.board_rx = Some(rx);
    std::thread::spawn(move || {
        let _ = tx.send(gh::fetch_board(&owner));
    });
}

/// The board modal is up, whether still fetching or fully loaded.
fn board_open(app: &App) -> bool {
    app.board.is_some() || app.board_loading
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
        }
        Err(e) => app.notify(e),
    }
}

fn board_move(app: &mut App, dx: isize, dy: isize) {
    app.board_expanded = None;
    let Some(board) = &app.board else { return };
    let cols = board_columns(&board.items, &board.order);
    if cols.is_empty() {
        return;
    }
    let col = (app.board_col as isize + dx).clamp(0, cols.len() as isize - 1) as usize;
    let rows = cols[col].1.len();
    let row = if rows == 0 {
        0
    } else {
        (app.board_row as isize + dy).clamp(0, rows as isize - 1) as usize
    };
    app.board_col = col;
    app.board_row = row;
}

fn board_selected_number(app: &App) -> Option<u64> {
    let board = app.board.as_ref()?;
    let cols = board_columns(&board.items, &board.order);
    cols.get(app.board_col)?.1.get(app.board_row)?.number
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(f: &mut ratatui::Frame, app: &mut App) {
    use theme::{amber, ink, muted};
    f.render_widget(
        Block::default().style(Style::new().bg(ink())),
        f.area(),
    );
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(44), Constraint::Min(1)])
        .split(root[0]);

    render_sidebar(f, cols[0], app);
    render_main(f, cols[1], app);
    app.capture_body_rows(f.buffer_mut());

    let footer_line = if app.message_visible() {
        Line::from(vec![
            Span::styled("! ", Style::new().fg(amber()).bold()),
            Span::styled(app.message.clone(), Style::new().fg(amber())),
        ])
    } else {
        Line::from(Span::styled(
            "j/k issue · ↑↓ scroll · 1/2 tab · b board · : cmd · o open · r refresh · p pin · t theme · click #n · q quit",
            Style::new().fg(muted()),
        ))
    };
    f.render_widget(Paragraph::new(footer_line), root[1]);

    if board_open(app) {
        dim_background(f, 0.35);
        render_board(f, f.area(), app);
    }
}

/// Darken every rendered cell toward the background so the modal sits proud.
fn dim_background(f: &mut ratatui::Frame, amount: f32) {
    let Color::Rgb(ir, ig, ib) = theme::ink() else {
        return;
    };
    for cell in f.buffer_mut().content.iter_mut() {
        match cell.fg {
            Color::Rgb(r, g, b) => {
                let mix = |c: u8, i: u8| (i as f32 + (c as f32 - i as f32) * amount) as u8;
                cell.set_fg(Color::Rgb(mix(r, ir), mix(g, ig), mix(b, ib)));
            }
            Color::Reset => {}
            _ => {
                cell.set_fg(Color::Rgb(70, 76, 88));
            }
        }
    }
}

fn section_list<'a>(
    title: Line<'a>,
    numbers: &[u64],
    app: &App,
    selected: Option<usize>,
) -> (List<'a>, ListState) {
    use theme::{amber, ink, muted, paper, state_color};
    let mut items: Vec<ListItem<'a>> = vec![ListItem::new(Line::from(""))];
    items.extend(numbers.iter().enumerate().map(|(idx, n)| {
            let sel = selected == Some(idx);
            let marker = if sel { "▌ " } else { "  " };
            let marker_style = if sel {
                Style::new().fg(amber()).bold()
            } else {
                Style::new().fg(muted())
            };
            let (title_text, second) = match app.data.get(n) {
                Some(d) => {
                    let st = app.statuses.get(n).cloned().unwrap_or_default();
                    (
                        model::slug(&d.title, 30),
                        model::sidebar_lines(*n, &d.title, &d.labels, &st, 38, app.pinned.contains(n)).1,
                    )
                }
                None => (String::new(), String::new()),
            };
            let pin = if app.pinned.contains(n) { "◆ " } else { "" };
            let data = app.data.get(n);
            let num_style = Style::new()
                .fg(data.map(|d| state_color(&d.state)).unwrap_or(muted()))
                .bold();
            let title_style = if sel {
                Style::new().fg(paper()).bold()
            } else {
                Style::new().fg(paper())
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(marker, marker_style),
                    Span::styled(pin, Style::new().fg(amber())),
                    Span::styled(format!("#{n} "), num_style),
                    Span::styled(title_text, title_style),
                ]),
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled(second, Style::new().fg(muted())),
                ]),
                Line::from(""),
            ])
        }));
    let mut state = ListState::default();
    // +1 for the breathing-room spacer row at the top.
    state.select(selected.map(|s| s + 1));
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .title(title)
                .style(Style::new().bg(ink())),
        )
        .highlight_style(Style::new().bold());
    (list, state)
}

/// Styled header for the body pane: title plus the color-coded meta line.
/// Each segment carries its semantic role; colors stay consistent by construction.
fn header_lines<'a>(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<Line<'a>> {
    use model::MetaKind;
    use theme::{amber, ink, muted, paper, teal};
    let title = Line::from(vec![
        Span::styled("■ ", Style::new().bg(ink()).fg(muted())),
        Span::styled(format!("#{} ", d.number), Style::new().bg(ink()).fg(muted())),
        Span::styled(model::slug(&d.title, 60), Style::new().bg(ink()).fg(paper()).bold()),
    ]);
    let mut lines = vec![title];
    let mut current: Vec<Span<'static>> = Vec::new();
    for (kind, text) in model::meta_spans(d, statuses) {
        let style = match kind {
            MetaKind::Dim => Style::new().bg(ink()).fg(muted()),
            MetaKind::State => Style::new().bg(ink()).fg(theme::state_color(&d.state)).bold(),
            MetaKind::Label => Style::new().bg(ink()).fg(amber()),
            MetaKind::Normal => Style::new().bg(ink()).fg(paper()),
            MetaKind::Project => Style::new().bg(ink()).fg(teal()).bold(),
            MetaKind::Url => Style::new().bg(ink()).fg(teal()).underlined(),
        };
        let mut chunks = text.split('\n');
        if let Some(first) = chunks.next() {
            if !first.is_empty() {
                current.push(Span::styled(first.to_string(), style));
            }
        }
        for chunk in chunks {
            lines.push(Line::from(std::mem::take(&mut current)));
            if !chunk.is_empty() {
                current.push(Span::styled(chunk.to_string(), style));
            }
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

fn section_title<'a>(name: &str, detail: &str) -> Line<'a> {
    use theme::{amber, muted};
    let name_span = Span::styled(format!("{name} "), Style::new().fg(amber()).bold());
    let detail_span = Span::styled(detail.to_string(), Style::new().fg(muted()));
    Line::from(vec![name_span, detail_span])
}

fn render_sidebar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let key = app.repo_key().unwrap_or_default();
    app.side_issue_area = rows[0];
    app.side_pr_area = rows[1];
    let (il, mut istate) = section_list(
        section_title("Issues", &key),
        &app.issues.clone(),
        app,
        app.issue_state.selected(),
    );
    f.render_stateful_widget(il, rows[0], &mut istate);
    let (pl, mut pstate) = section_list(
        section_title("Pull requests", ""),
        &app.prs.clone(),
        app,
        app.pr_state.selected(),
    );
    f.render_stateful_widget(pl, rows[1], &mut pstate);
}

fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use theme::{amber, ink, muted, paper, base};
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let repo = app.repo_key().unwrap_or_default();
    // rows[0] masthead, rows[1] breathing room, rows[2] tabs, rows[3] pane, rows[4] bar.
    app.tabs_area = rows[2];
    app.bar_area = rows[4];
    let mast = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => Line::from(vec![
            Span::styled("ghit ", Style::new().fg(amber()).bold()),
            Span::styled(format!("{repo}  "), Style::new().fg(muted())),
            Span::styled(format!("#{} ", d.number), Style::new().fg(muted())),
            Span::styled(model::slug(&d.title, 60), Style::new().fg(paper()).bold()),
        ]),
        None => Line::from(vec![
            Span::styled("ghit ", Style::new().fg(amber()).bold()),
            Span::styled(repo, Style::new().fg(muted())),
        ]),
    };
    f.render_widget(Paragraph::new(mast).style(base()), rows[0]);

    let tabs = Tabs::new(vec!["Body", "Comments"])
        .select(match app.tab {
            Tab::Body => 0,
            Tab::Comments => 1,
        })
        .style(Style::new().bg(ink()).fg(muted()))
        .highlight_style(Style::new().bg(ink()).fg(amber()).bold().underlined())
        .divider("  ")
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .style(Style::new().bg(ink())),
        );
    f.render_widget(tabs, rows[2]);

    let owned_body: String;
    let owned_comments: String;
    let text = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => {
            let st = app.statuses.get(&d.number).cloned().unwrap_or_default();
            match app.tab {
                Tab::Body => {
                    let mut lines = header_lines(d, &st);
                    lines.push(Line::from(""));
                    owned_body = model::body_md(d);
                    lines.extend(theme::render_markdown(&owned_body));
                    ratatui::text::Text::from(lines)
                }
                Tab::Comments => {
                    owned_comments = model::build_comments_md(d);
                    theme::render_markdown(&owned_comments)
                }
            }
        }
        None => theme::render_markdown(
            "No issue loaded.\n\nType a number in the command bar and hit Enter.\n(: or / focuses the bar)",
        ),
    };
    let pane = Paragraph::new(text)
        .style(base())
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    // Breathing room: text never touches the column edges.
    let gutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[3]);
    app.body_area = gutter[1];
    f.render_widget(pane, gutter[1]);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(base())
        .border_style(
            Style::new().bg(ink()).fg(if app.focus == Focus::Bar { amber() } else { muted() }),
        );
    let inner = bar_block.inner(rows[4]);
    f.render_widget(bar_block, rows[4]);
    let mut spans = vec![Span::styled("❯ ", Style::new().bg(ink()).fg(amber()).bold())];
    if app.bar.is_empty() {
        spans.push(Span::styled(
            "type #123, o, q …",
            Style::new().bg(ink()).fg(muted()).italic(),
        ));
    } else {
        spans.push(Span::styled(app.bar.clone(), Style::new().bg(ink()).fg(paper())));
    }
    // Theme pill pinned right, like a status chip.
    let pill = format!(" {} ", theme::name());
    let used = 2 + app.bar.chars().count().max(19) + pill.chars().count() + 2;
    let fill = (inner.width as usize).saturating_sub(used);
    if fill >= 1 {
        spans.push(Span::raw(" ".repeat(fill)));
        spans.push(Span::styled(
            pill,
            Style::new().bg(amber()).fg(ink()).bold(),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(base()), inner);
    if app.focus == Focus::Bar && !app.bar.is_empty() {
        f.set_cursor_position((inner.x + 2 + app.bar.len() as u16, inner.y));
    } else if app.focus == Focus::Bar {
        f.set_cursor_position((inner.x + 2, inner.y));
    }
}

/// Collapsed card: title, labels, spacer. Expanded adds status, url, excerpt.
const BASE_CARD_H: u16 = 3;
const EXPANDED_CARD_H: u16 = 6;

fn card_height(expanded: bool) -> u16 {
    if expanded {
        EXPANDED_CARD_H
    } else {
        BASE_CARD_H
    }
}

/// Column areas with a 1-cell gutter so borders never touch.
fn column_layout(area: Rect, n_cols: usize, col_width: u16) -> Vec<Rect> {
    let mut constraints = Vec::with_capacity(n_cols * 2);
    for i in 0..n_cols {
        constraints.push(Constraint::Length(col_width));
        if i + 1 < n_cols {
            constraints.push(Constraint::Length(1));
        }
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    chunks.iter().step_by(2).copied().collect()
}

fn render_board(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use theme::{amber, ink, muted, base};
    let popup = centered(area, 92, 92);
    f.render_widget(Clear, popup);
    let title = match &app.board {
        Some(b) => format!(" {} ", b.title),
        None => " Fetching board … ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Span::styled(title, Style::new().bg(ink()).fg(amber()).bold()));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (inner, hint_area) = (chunks[0], chunks[1]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "h/l columns · j/k cards · space expand · enter open · esc close",
            Style::new().bg(ink()).fg(muted()),
        ))),
        hint_area,
    );

    let Some(board) = &app.board else {
        render_skeleton(f, inner);
        return;
    };
    let cols = board_columns(&board.items, &board.order);
    if cols.is_empty() {
        f.render_widget(Paragraph::new("empty board").style(base()), inner);
        return;
    }
    while app.board_states.len() < cols.len() {
        app.board_states.push(ListState::default());
    }
    let col_areas = column_layout(inner, cols.len(), 36);
    app.board_cells.clear();
    for (i, (name, cards)) in cols.iter().enumerate() {
        let Some(a) = col_areas.get(i) else { continue };
        let items: Vec<ListItem> = cards
            .iter()
            .enumerate()
            .map(|(ri, c)| {
                let sel = app.board_col == i && app.board_row == ri;
                board_card(c, sel, app.board_expanded == Some((i, ri)))
            })
            .collect();
        let active = app.board_col == i;
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(if active { amber() } else { muted() }))
                .style(base())
                .title(Line::from(vec![
                    Span::styled(format!("{} ", name), Style::new().bg(ink()).fg(amber()).bold()),
                    Span::styled(format!("({})", cards.len()), Style::new().bg(ink()).fg(muted())),
                ])),
        );
        let state = &mut app.board_states[i];
        state.select(if active { Some(app.board_row) } else { None });
        f.render_stateful_widget(list, *a, state);
        // Map visible cards back to click areas using the widget's scroll
        // offset. Cards have variable height, so walk line offsets.
        let offset = app.board_states[i].offset();
        let col_inner = Block::default().borders(Borders::ALL).inner(*a);
        let mut y = col_inner.y;
        for (ri, _) in cards.iter().enumerate().skip(offset) {
            let h = card_height(app.board_expanded == Some((i, ri)));
            if y + h > col_inner.y + col_inner.height {
                break;
            }
            app.board_cells.push((Rect::new(col_inner.x, y, col_inner.width, h), i, ri));
            y += h;
        }
    }
}

/// What kind of item this is: issue, PR, or draft.
fn card_kind(c: &crate::model::BoardItem) -> String {
    match c.content.get("type").and_then(|t| t.as_str()) {
        Some("PullRequest") => "PR".to_string(),
        Some("DraftIssue") => "DRAFT".to_string(),
        Some("Issue") => "ISSUE".to_string(),
        Some(other) => other.to_string(),
        None => "ITEM".to_string(),
    }
}

fn card_url(c: &crate::model::BoardItem) -> Option<String> {
    c.content.get("url").and_then(|u| u.as_str()).map(|s| s.to_string())
}

fn card_excerpt(c: &crate::model::BoardItem) -> String {
    let body = c.content.get("body").and_then(|b| b.as_str()).unwrap_or("");
    match body.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(first) => model::slug(first, 28),
        None => "–".to_string(),
    }
}

fn board_card(c: &crate::model::BoardItem, sel: bool, expanded: bool) -> ListItem<'static> {
    use theme::{amber, ink, muted, paper};
    let title = model::slug(&c.title, 26);
    let head = match c.number {
        Some(n) => format!("#{n} {title}"),
        None => format!("◇ {title}"),
    };
    // Labels own the second line now that the swimlane carries the status.
    let mut chips: Vec<String> = c.labels.iter().take(4).map(|l| format!("● {}", l.name)).collect();
    if c.labels.len() > 4 {
        chips.push(format!("+{}", c.labels.len() - 4));
    }
    let chips_line = if chips.is_empty() { "–".to_string() } else { chips.join("  ") };
    let first = if sel {
        Line::from(vec![
            Span::styled("▌ ", Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled(head, Style::new().bg(ink()).fg(paper()).bold()),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::new().bg(ink())),
            Span::styled(head, Style::new().bg(ink()).fg(paper())),
        ])
    };
    let mut lines = vec![
        first,
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(chips_line, Style::new().bg(ink()).fg(muted())),
        ]),
    ];
    if expanded {
        let status = c.status.clone().unwrap_or_else(|| "No status".to_string());
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(
                format!("{} · {}", status, card_kind(c)),
                Style::new().bg(ink()).fg(amber()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(
                card_url(c).unwrap_or_else(|| "–".to_string()),
                Style::new().bg(ink()).fg(muted()),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(card_excerpt(c), Style::new().bg(ink()).fg(paper())),
        ]));
    }
    lines.push(Line::from(Span::styled("", Style::new().bg(ink()))));
    ListItem::new(lines)
}

/// Placeholder columns shown while the network fetch is in flight.
fn render_skeleton(f: &mut ratatui::Frame, area: Rect) {
    use theme::{ink, muted, base};
    let col_areas = column_layout(area, 3, 36);
    for a in col_areas.iter() {
        let ghost: Vec<ListItem> = (0..4)
            .map(|_| {
                ListItem::new(vec![
                    Line::from(Span::styled("  ▓▓▓▓▓▓▓▓▓▓▓▓", Style::new().bg(ink()).fg(muted()))),
                    Line::from(Span::styled("    ▓▓▓▓▓▓▓▓", Style::new().bg(ink()).fg(muted()))),
                    Line::from(""),
                ])
            })
            .collect();
        let list = List::new(ghost).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .style(base())
                .title(Span::styled(" ━━━ ", Style::new().bg(ink()).fg(muted()))),
        );
        f.render_widget(list, *a);
    }
}

#[cfg(test)]
mod render_smoke {
    use super::*;
    use crate::model::{Author, BoardData, BoardItem, IssueData, Label};
    use ratatui::backend::TestBackend;

    fn board_app(n_cards: usize) -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let items: Vec<BoardItem> = (0..n_cards)
            .map(|i| BoardItem {
                id: format!("c{i}"),
                title: format!("Card {i}"),
                number: Some(100 + i as u64),
                status: Some("Todo".to_string()),
                labels: vec![Label { name: "x".to_string() }],
                content: serde_json::json!({
                    "type": "Issue",
                    "url": format!("https://github.com/o/r/issues/{}", 100 + i),
                    "body": "## Why\n\nSome details here",
                }),
            })
            .collect();
        app.board = Some(BoardData {
            number: 1,
            title: "Roadmap".to_string(),
            order: vec!["Todo".to_string()],
            items,
        });
        app
    }

    #[test]
    fn board_skeleton_shows_while_loading() {
        let mut app = App::new(Some("o/r".to_string()));
        app.board_loading = true;
        let text = frame_text(&mut app);
        assert!(text.contains("Fetching"), "skeleton title");
    }

    #[test]
    fn board_cards_show_labels_not_status() {
        let mut app = board_app(3);
        let text = frame_text(&mut app);
        assert!(text.contains("Card 0"));
        assert!(text.contains("● x"));
        // The swimlane carries the status; collapsed cards don't repeat it.
        // (The column header "Todo (n)" is not indented like a card line.)
        assert!(!text.contains("    Todo"), "status leaks into collapsed card");
    }

    #[test]
    fn board_expanded_card_shows_detail() {
        let mut app = board_app(3);
        app.board_expanded = Some((0, 0));
        let text = frame_text(&mut app);
        assert!(text.contains("Todo · ISSUE"));
        assert!(text.contains("https://github.com/o/r/issues"));
        assert!(text.contains("Why"));
    }

    #[test]
    fn space_toggles_expansion_and_move_clears_it() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = board_app(3);
        let space = || KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty());
        handle_key(&mut app, space());
        assert_eq!(app.board_expanded, Some((0, 0)));
        handle_key(&mut app, space());
        assert_eq!(app.board_expanded, None);
        handle_key(&mut app, space());
        board_move(&mut app, 0, 1);
        assert_eq!(app.board_expanded, None);
        assert_eq!((app.board_col, app.board_row), (0, 1));
    }

    #[test]
    fn board_scrolls_selected_into_view() {
        let mut app = board_app(30);
        app.board_row = 29;
        let text = frame_text(&mut app);
        assert!(text.contains("Card 29"), "selected card visible");
        assert!(app.board_states[0].offset() > 0, "list scrolled");
    }

    fn sample(number: u64) -> IssueData {
        IssueData {
            number,
            title: "Fix the thing".to_string(),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![],
            assignees: vec![],
            created_at: "2026-01-02T00:00:00Z".to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            body: "# hello\n\n- a\n- b\n".to_string(),
            comments: vec![],
            is_pr: false,
        }
    }

    fn loaded_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut pr = sample(9);
        pr.is_pr = true;
        for d in [sample(7), pr] {
            let n = d.number;
            let is_pr = d.is_pr;
            app.data.insert(n, d);
            app.statuses.insert(n, vec![]);
            if is_pr {
                app.prs.push(n);
            } else {
                app.issues.push(n);
            }
        }
        app.show_issue(7);
        app
    }

    fn key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
    }

    fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn jk_walks_issues_then_prs() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn arrows_scroll_instead() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.current, Some(7));
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn mouse_click_selects_sidebar_row() {
        let mut app = loaded_app();
        app.show_issue(9);
        let _ = frame_text(&mut app); // records hit areas
        // +2 skips the border and the breathing-room spacer row.
        let (x, y) = (app.side_issue_area.x + 2, app.side_issue_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn mouse_click_switches_tabs() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 8, y),
        );
        assert_eq!(app.tab, Tab::Comments);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 1, y),
        );
        assert_eq!(app.tab, Tab::Body);
    }

    #[test]
    fn mouse_click_on_ref_opens_issue() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "blocked by #9, see details".to_string();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("#9"))
            .expect("ref visible");
        let (x, y) = (app.body_area.x + 2, app.body_area.y + row as u16);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(9));
    }

    #[test]
    fn mouse_click_on_plain_text_keeps_issue() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("hello"))
            .expect("body visible");
        let (x, y) = (app.body_area.x + 2, app.body_area.y + row as u16);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn mouse_wheel_scrolls() {
        let mut app = loaded_app();
        handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 60, 20));
        assert_eq!(app.scroll, 3);
        handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 60, 20));
        assert_eq!(app.scroll, 0);
    }

    fn frame_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn desk_renders_issue() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let d = sample(7);
        app.data.insert(7, d);
        app.statuses.insert(7, vec![]);
        app.issues.push(7);
        app.show_issue(7);
        let text = frame_text(&mut app);
        assert!(text.contains("ghit"), "masthead wordmark");
        assert!(text.contains("#7"), "issue number");
        assert!(text.contains("Body"), "tab label");
        assert!(text.contains("hello"), "markdown body");
    }

    #[test]
    fn desk_renders_empty() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let text = frame_text(&mut app);
        assert!(text.contains("ghit"));
        assert!(text.contains("No issue loaded"));
    }
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v)[1]
}



