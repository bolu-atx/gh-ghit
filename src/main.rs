//! ghit — terminal viewer for GitHub issues and PRs, backed by the `gh` CLI.
//!
//! Punch in an issue number, read the ticket. `o` opens it in GitHub.
//! Usage: `ghit [numbers...] [-R owner/repo]`

mod cache;
mod gh;
mod model;

use std::collections::{HashMap, HashSet};
use std::io::Stdout;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::ExecutableCommand;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
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

#[derive(Clone, Copy, PartialEq, Eq)]
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
    issue_state: ListState,
    pr_state: ListState,
    active_list: ActiveList,
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
            issue_state: ListState::default(),
            pr_state: ListState::default(),
            active_list: ActiveList::Issues,
            board: None,
            board_col: 0,
            board_row: 0,
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
            Err(e) => self.message = e,
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
            self.message = "view an issue first".to_string();
            return;
        };
        let Some(key) = self.repo_key() else {
            self.message = "pinning needs a repo: use -R or run inside one".to_string();
            return;
        };
        let mut pins = cache::load_pins(&key);
        if pins.contains(&n) {
            pins.retain(|x| *x != n);
            self.pinned.remove(&n);
            self.message = format!("unpinned #{n}");
        } else {
            pins.push(n);
            self.pinned.insert(n);
            self.message = format!("pinned #{n}");
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
            self.message = "view an issue first".to_string();
            return;
        };
        if let Some(owner) = board_owner(self.repo_flag.clone().or_else(|| self.repo_slug.clone())) {
            cache::cache_drop(&format!("board-{}", owner.to_lowercase()));
        }
        self.load_issue(n, true);
        self.message = format!("refreshed #{n}");
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
        self.message = format!("opened #{n} in browser");
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
                    self.message = "view an issue first".to_string();
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
        if !event::poll(Duration::from_millis(100)).map_err(|e| e.to_string())? {
            continue;
        }
        let Event::Key(key) = event::read().map_err(|e| e.to_string())? else {
            continue;
        };

        // Board modal captures everything.
        if app.board.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => app.board = None,
                KeyCode::Char('j') | KeyCode::Down => board_move(app, 0, 1),
                KeyCode::Char('k') | KeyCode::Up => board_move(app, 0, -1),
                KeyCode::Char('h') | KeyCode::Left => board_move(app, -1, 0),
                KeyCode::Char('l') | KeyCode::Right => board_move(app, 1, 0),
                KeyCode::Enter => {
                    if let Some(n) = board_selected_number(app) {
                        app.board = None;
                        app.load_issue(n, false);
                    }
                }
                _ => {}
            }
            continue;
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
                        return Ok(());
                    }
                }
                KeyCode::Char(c) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') {
                        return Ok(());
                    }
                    app.bar.push(c);
                }
                KeyCode::Backspace => {
                    app.bar.pop();
                }
                _ => {}
            }
            continue;
        }

        match key.code {
            KeyCode::Char('q') => return Ok(()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Char('1') => app.tab = Tab::Body,
            KeyCode::Char('2') => app.tab = Tab::Comments,
            KeyCode::Char('b') => open_board(app),
            KeyCode::Char('j') | KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT) || app.data.is_empty() {
                    move_selection(app, 1);
                } else {
                    app.scroll = app.scroll.saturating_add(3);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT) || app.data.is_empty() {
                    move_selection(app, -1);
                } else {
                    app.scroll = app.scroll.saturating_sub(3);
                }
            }
            KeyCode::Char(':') | KeyCode::Char('/') => app.focus = Focus::Bar,
            KeyCode::Char('o') => {
                if let Some(n) = app.current {
                    app.open_number(n);
                } else {
                    app.message = "view an issue first".to_string();
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
}

fn move_selection(app: &mut App, delta: isize) {
    let (list, state) = match app.active_list {
        ActiveList::Issues => (&app.issues, &mut app.issue_state),
        ActiveList::Prs => (&app.prs, &mut app.pr_state),
    };
    if list.is_empty() {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, list.len() as isize - 1) as usize;
    state.select(Some(next));
    let n = list[next];
    app.show_issue(n);
}

fn open_board(app: &mut App) {
    let owner = board_owner(app.repo_flag.clone().or_else(|| app.repo_slug.clone()));
    let Some(owner) = owner else {
        app.message = "board needs a repo: use -R or run inside one".to_string();
        return;
    };
    match gh::fetch_board(&owner) {
        Ok(data) => {
            app.board = Some(data);
            app.board_col = 0;
            app.board_row = 0;
        }
        Err(e) => app.message = e,
    }
}

fn board_move(app: &mut App, dx: isize, dy: isize) {
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
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(42), Constraint::Min(1)])
        .split(root[0]);

    render_sidebar(f, cols[0], app);
    render_main(f, cols[1], app);

    let hint = "1 body · 2 comments · b board · j/k scroll · : cmd · o open · r refresh · p pin · q quit";
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(app.message.clone(), Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().title(hint));
    f.render_widget(footer, root[1]);

    if app.board.is_some() {
        render_board(f, f.area(), app);
    }
}

fn section_list<'a>(title: String, numbers: &[u64], app: &App, selected: Option<usize>) -> (List<'a>, ListState) {
    let items: Vec<ListItem<'a>> = numbers
        .iter()
        .map(|n| {
            let (first, second) = match app.data.get(n) {
                Some(d) => {
                    let st = app.statuses.get(n).cloned().unwrap_or_default();
                    model::sidebar_lines(*n, &d.title, &d.labels, &st, 38, app.pinned.contains(n))
                }
                None => (format!("#{n}"), String::new()),
            };
            let pinned = app.pinned.contains(n);
            ListItem::new(vec![
                Line::from(Span::styled(
                    first,
                    Style::default().add_modifier(if pinned { Modifier::BOLD } else { Modifier::empty() }),
                )),
                Line::from(Span::styled(second, Style::default().fg(Color::DarkGray))),
                Line::from(""),
            ])
        })
        .collect();
    let mut state = ListState::default();
    state.select(selected);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    (list, state)
}

fn render_sidebar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let key = app.repo_key().unwrap_or_default();
    let (il, mut istate) = section_list(
        format!("ISSUES · {key}"),
        &app.issues.clone(),
        app,
        app.issue_state.selected(),
    );
    f.render_stateful_widget(il, rows[0], &mut istate);
    let (pl, mut pstate) = section_list(
        "PULL REQUESTS".to_string(),
        &app.prs.clone(),
        app,
        app.pr_state.selected(),
    );
    f.render_stateful_widget(pl, rows[1], &mut pstate);
}

fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let tabs = Tabs::new(vec!["Body (1)", "Comments (2)"])
        .select(match app.tab {
            Tab::Body => 0,
            Tab::Comments => 1,
        })
        .block(Block::default().borders(Borders::ALL).title("ghit"));
    f.render_widget(tabs, rows[0]);

    let body = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => {
            let st = app.statuses.get(&d.number).cloned().unwrap_or_default();
            match app.tab {
                Tab::Body => model::build_body_md(d, &st),
                Tab::Comments => model::build_comments_md(d),
            }
        }
        None => "No issue loaded.\n\nType a number in the command bar and hit Enter.\n(: or / focuses the bar)".to_string(),
    };
    let rendered = tui_markdown::from_str(&body);
    let pane = Paragraph::new(rendered).wrap(Wrap { trim: false }).scroll((app.scroll, 0));
    f.render_widget(pane, rows[1]);

    let bar_style = if app.focus == Focus::Bar {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let bar = Paragraph::new(format!(": {}", app.bar)).style(bar_style).block(
        Block::default()
            .borders(Borders::ALL)
            .title("#123 [#124 …] jump · o open · r refresh · p pin · q quit"),
    );
    f.render_widget(bar, rows[2]);
    if app.focus == Focus::Bar {
        f.set_cursor_position((rows[2].x + 2 + app.bar.len() as u16, rows[2].y + 1));
    }
}

fn render_board(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let Some(board) = &app.board else { return };
    let cols = board_columns(&board.items, &board.order);
    let popup = centered(area, 92, 92);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).title(board.title.clone());
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if cols.is_empty() {
        f.render_widget(Paragraph::new("empty board"), inner);
        return;
    }
    let widths: Vec<Constraint> = cols.iter().map(|_| Constraint::Length(34)).collect();
    let col_areas = Layout::default().direction(Direction::Horizontal).constraints(widths).split(inner);
    for (i, (name, cards)) in cols.iter().enumerate() {
        let Some(a) = col_areas.get(i) else { continue };
        let items: Vec<ListItem> = cards
            .iter()
            .enumerate()
            .map(|(ri, c)| {
                let sel = app.board_col == i && app.board_row == ri;
                let title = model::slug(&c.title, 26);
                let head = match c.number {
                    Some(n) => format!("#{n} {title}"),
                    None => format!("◇ {title}"),
                };
                let style = if sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(vec![Line::from(Span::styled(head, style))])
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(format!("{} ({})", name, cards.len())));
        f.render_widget(list, *a);
    }
}

#[cfg(test)]
mod smoke {
    #[test]
    fn markdown_renders_to_lines() {
        let text = tui_markdown::from_str("# hi\n\n- a\n- b\n");
        assert!(text.height() > 3);
        assert!(text.width() > 0);
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



