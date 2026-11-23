//! Helpers shared by the frame tests.

use crate::app::App;
use crate::input::handle_mouse;
use ghit_core::model::{
    Author, BoardData, BoardItem, IssueData, Label,
};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ghit_core::model::ItemRef;

pub(super) fn board_app(n_cards: usize) -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    let items: Vec<BoardItem> = (0..n_cards)
        .map(|i| BoardItem {
            title: format!("Card {i}"),
            number: Some(ItemRef::github(100 + i as u64)),
            status: Some("Todo".to_string()),
            labels: vec![Label { name: "x".to_string() }],
            repo: Some("o/r".to_string()),
            milestone: None,
            content: serde_json::json!({
                "type": "Issue",
                "url": format!("https://github.com/o/r/issues/{}", 100 + i),
                "body": "## Why\n\nSome details here",
            }),
        })
        .collect();
    app.board.data = Some(BoardData {
        title: "Roadmap".to_string(),
        order: vec!["Todo".to_string()],
        items,
        url: "https://github.com/orgs/o/projects/1".to_string(),
    });
    app.open(crate::app::Entry::Board);
    app
}

pub(super) fn press_cell(app: &mut App, widget: usize, option: Option<usize>) {
    let rect = app.hits.form_cells.iter()
        .find(|(_, w, o)| *w == widget && *o == option)
        .map(|(r, _, _)| *r)
        .unwrap_or_else(|| panic!("no cell for widget {widget} option {option:?}"));
    handle_mouse(app, click(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y));
}

/// Sized frame text, for short-terminal dialog fits.
pub(super) fn frame_text_sized(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

pub(super) fn sample(number: ItemRef) -> IssueData {
    IssueData {
        number,
        title: "Fix the thing".to_string(),
        state: "OPEN".to_string(),
        author: Author { login: "bo".to_string() },
        labels: vec![],
        assignees: vec![],
        created_at: "2026-01-02T00:00:00Z".to_string(),
        updated_at: "2026-01-05T00:00:00Z".to_string(),
        url: format!(
            "https://github.com/o/r/issues/{number}",
            number = number.number
        ),
        body: "# hello\n\n- a\n- b\n".to_string(),
        comments: vec![],
        review_threads: vec![],
        paired: vec![],
        checks: None,
        milestone: None,
        parent: None,
        sub_summary: None,
            links: Default::default(),
        is_pr: false,
    }
}

pub(super) fn loaded_app() -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    let mut pr = sample(ItemRef::github(9));
    pr.is_pr = true;
    for d in [sample(ItemRef::github(7)), pr] {
        let n = d.number;
        app.session.issues.data.insert(n, d);
        app.session.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    app.show_issue(ItemRef::github(7));
    app
}

pub(super) fn key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
}

/// Point XDG at a temp dir so prefs persistence in key-driven tests
/// never touches real data. Holds the env lock so parallel cache
/// tests cannot interleave, and restores the var on drop.
pub(super) struct TempPrefs {
    saved: Option<String>,
    _dir: tempfile::TempDir,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl TempPrefs {
    pub(super) fn new() -> Self {
        let guard = ghit_core::cache::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        Self { saved, _dir: dir, _guard: guard }
    }
}

impl Drop for TempPrefs {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }
}

pub(super) fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    }
}

pub(super) fn draw_buffer(app: &mut App) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    terminal.backend().buffer().clone()
}

pub(super) fn cells<'a>(buf: &'a ratatui::buffer::Buffer, symbol: &str) -> Vec<&'a ratatui::buffer::Cell> {
    buf.content().iter().filter(|c| c.symbol() == symbol).collect()
}

pub(super) fn check_pr(number: ItemRef, checks: Option<ghit_core::model::CheckState>) -> IssueData {
    let mut d = sample(number);
    d.is_pr = true;
    d.checks = checks;
    d
}

pub(super) fn check_app(number: ItemRef, checks: Option<ghit_core::model::CheckState>) -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    app.session.issues.data.insert(number, check_pr(number, checks));
    app.session.issues.statuses.insert(number, vec![]);
    app.nav.visited.push(number);
    app.show_issue(number);
    app
}

pub(super) fn frame_text(app: &mut App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

/// Text of the Issues sidebar only, so main-pane text cannot satisfy
/// or break row-render assertions.
pub(super) fn sidebar_text(app: &mut App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let area = app.hits.side_issue_area;
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

pub(super) fn diff_pr(number: ItemRef) -> IssueData {
    let mut d = sample(number);
    d.is_pr = true;
    d
}
