//! Frame and input tests for the Ctrl+P finder.

use crate::app::App;
use crate::core::{Dialog, Layer};
use crate::input::{handle_key, handle_mouse};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{click, key, loaded_app, sample};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn app() -> App {
    let mut app = loaded_app();
    for (n, title, pr) in [
        (241u64, "Fix flaky auth probe", false),
        (198, "Auth probe retries on 401", true),
        (77, "Probe for token scopes", false),
    ] {
        let mut data = sample(n);
        data.title = title.to_string();
        data.is_pr = pr;
        app.issues.data.insert(n, data);
        app.issues.statuses.insert(n, vec![]);
    }
    app
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        handle_key(app, key(KeyCode::Char(c)));
    }
}

/// The frame as lines, trailing blanks trimmed.
fn lines(app: &mut App) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

fn text(app: &mut App) -> String {
    lines(app).join("\n")
}

fn open(app: &mut App) {
    handle_key(app, ctrl('p'));
    assert_eq!(app.layer_stack().last(), Some(&Layer::Dialog(Dialog::Finder)));
}

#[test]
fn ctrl_p_opens_and_esc_closes() {
    let mut app = app();
    open(&mut app);
    let frame = text(&mut app);
    assert!(frame.contains("Go to issue or PR"), "{frame}");
    assert!(frame.contains("enter open · ↑ previous · ↓ next · esc close"), "{frame}");
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.finder().is_none());
    assert!(!text(&mut app).contains("Go to issue or PR"));
}

#[test]
fn empty_query_lists_everything_with_a_count() {
    let mut app = app();
    open(&mut app);
    let frame = text(&mut app);
    for needle in ["#241", "Fix flaky auth probe", "#198", "#77", "5 of 5"] {
        assert!(frame.contains(needle), "missing {needle}: {frame}");
    }
}

#[test]
fn typing_filters_and_highlights_matches_in_amber() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "flaky");
    let frame = text(&mut app);
    assert!(frame.contains("1 of 5") && frame.contains("Fix flaky auth probe"), "{frame}");
    assert!(!frame.contains("Auth probe retries"), "{frame}");
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer();
    let row = app.hits.finder_rows[0].0;
    let lit: String = (row.x..row.right())
        .filter(|x| buf[(*x, row.y)].fg == crate::theme::amber() && !matches!(buf[(*x, row.y)].symbol(), "█" | " "))
        .map(|x| buf[(x, row.y)].symbol())
        .collect();
    assert_eq!(lit, "flaky");
}

#[test]
fn query_row_places_the_real_cursor() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "ab");
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let pos = terminal.get_cursor_position().expect("cursor");
    let row: String = (0..120).map(|x| terminal.backend().buffer()[(x, pos.y)].symbol()).collect();
    assert!(row.contains("› ab"), "{row}");
    assert_eq!(terminal.backend().buffer()[(pos.x - 1, pos.y)].symbol(), "b");
}

#[test]
fn no_match_shows_a_muted_notice() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "zzq");
    let frame = text(&mut app);
    assert!(frame.contains("No issue or PR matches \"zzq\"."), "{frame}");
    assert!(frame.contains("0 of 5"), "{frame}");
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.finder().is_none(), "Enter on no results just closes");
}

#[test]
fn arrows_and_ctrl_chords_move_the_cursor() {
    let mut app = app();
    open(&mut app);
    let cursor = |app: &App| app.finder().map(|f| f.cursor);
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(cursor(&app), Some(1));
    handle_key(&mut app, ctrl('n'));
    assert_eq!(cursor(&app), Some(2));
    handle_key(&mut app, key(KeyCode::Up));
    handle_key(&mut app, ctrl('p'));
    assert_eq!(cursor(&app), Some(0));
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!(cursor(&app), Some(0), "stops at the top");
}

#[test]
fn backspace_edits_the_query() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "pro");
    handle_key(&mut app, key(KeyCode::Backspace));
    assert_eq!(app.finder().map(|f| f.query.as_str()), Some("pr"));
}

#[test]
fn enter_opens_the_item_and_closes() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "198");
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.finder().is_none());
    assert_eq!(app.nav.current, Some(198));
}

#[test]
fn click_on_a_row_opens_it() {
    let mut app = app();
    open(&mut app);
    text(&mut app);
    let (rect, _) = app.hits.finder_rows[2];
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x + 4, rect.y));
    assert!(app.finder().is_none());
    let opened = app.nav.current.expect("an item opened");
    assert_ne!(opened, 7);
}

#[test]
fn wheel_moves_the_cursor_and_outside_click_closes() {
    let mut app = app();
    open(&mut app);
    text(&mut app);
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 60, 10));
    assert_eq!(app.finder().map(|f| f.cursor), Some(1));
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 60, 10));
    assert_eq!(app.finder().map(|f| f.cursor), Some(0));
    let popup = app.hits.popup;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), popup.x + 1, popup.y + 1));
    assert!(app.finder().is_some(), "a click inside the dialog keeps it open");
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), 0, 0));
    assert!(app.finder().is_none());
    assert_eq!(app.nav.current, Some(7), "closing opens nothing");
}

#[test]
fn clicking_a_footer_hint_runs_it() {
    let mut app = app();
    open(&mut app);
    text(&mut app);
    let (rect, _) = *app.hits.legend.iter().find(|(_, a)| *a == crate::keymap::Action::Back).expect("esc hint");
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y));
    assert!(app.finder().is_none());
}

#[test]
fn typed_keys_never_leak_to_the_panes_behind() {
    let mut app = app();
    open(&mut app);
    typed(&mut app, "q?jpc");
    assert!(!app.quit && !app.shortcuts_open() && app.draft().is_none());
    assert_eq!(app.finder().map(|f| f.query.as_str()), Some("q?jpc"));
}
