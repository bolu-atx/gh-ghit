//! Frame tests for the comment editor modal.

use crate::app::App;
use crate::input::{handle_key, handle_mouse};
use crate::keymap::Action;
use crate::theme;
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use super::test_support::{click, key, loaded_app};
use ghit_core::model::ItemRef;

/// One drawn frame: text rows, the buffer and the terminal cursor.
struct Shot {
    rows: Vec<String>,
    buf: ratatui::buffer::Buffer,
    cursor: ratatui::layout::Position,
}

impl Shot {
    fn text(&self) -> String {
        self.rows.join("\n")
    }
}

fn shot(app: &mut App, width: u16, height: u16) -> Shot {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let cursor = terminal.get_cursor_position().expect("cursor");
    let buf = terminal.backend().buffer().clone();
    let rows = (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
        .collect();
    Shot { rows, buf, cursor }
}

fn ctrl(c: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        handle_key(app, if ch == '\n' { key(KeyCode::Enter) } else { key(KeyCode::Char(ch)) });
    }
}

fn open() -> App {
    let mut app = loaded_app();
    app.show_issue(ItemRef::github(7));
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert!(app.compose_open());
    app
}

fn press(app: &mut App, x: u16, y: u16) {
    handle_mouse(app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(app, click(MouseEventKind::Up(MouseButton::Left), x, y));
}

fn press_switch(app: &mut App) {
    let rect = app.hits.legend.iter().find(|(_, a)| *a == Action::ComposeTogglePreview)
        .map(|(r, _)| *r).expect("switch rect");
    press(app, rect.x, rect.y);
}

fn word_fg(shot: &Shot, word: &str) -> ratatui::style::Color {
    let row = shot.rows.iter().position(|r| r.contains(word)).expect("word row");
    let col = shot.rows[row].find(word).expect("word col");
    let x = shot.rows[row][..col].chars().count() as u16;
    shot.buf[(x, row as u16)].fg
}

#[test]
fn empty_write_mode_shows_placeholder_counter_switch_and_real_cursor() {
    let mut app = open();
    let s = shot(&mut app, 120, 40);
    let text = s.text();
    assert!(text.contains("Comment on #7  Fix the thing"), "{text}");
    assert!(text.contains("Markdown works here. Ctrl+S sends."), "{text}");
    assert!(text.contains("0 chars · 0 lines"), "{text}");
    assert!(text.contains("ctrl-s send · ctrl-r preview · ctrl-o edit in $EDITOR · esc discard"), "{text}");
    assert!(!text.contains('▌'), "no glyph cursor: {text}");
    assert_eq!(word_fg(&s, "Write"), theme::amber());
    assert_eq!(word_fg(&s, "Preview"), theme::muted());
    let text_rect = app.hits.compose_text;
    assert_eq!((s.cursor.x, s.cursor.y), (text_rect.x, text_rect.y), "cursor at the first cell");
    assert!(text_rect.width <= 81, "text column is capped");
}

#[test]
fn popup_is_capped_on_a_huge_terminal() {
    let mut app = open();
    let _ = shot(&mut app, 240, 80);
    assert_eq!(app.hits.popup.width, 96);
    assert!(app.hits.popup.height < 80 * 7 / 10);
}

#[test]
fn write_mode_numbers_lines_wraps_and_places_the_cursor() {
    let mut app = open();
    let long = "word ".repeat(20);
    type_text(&mut app, &format!("first\n{long}end\nthird"));
    let s = shot(&mut app, 120, 40);
    let text = s.text();
    assert!(text.contains(" 1  first"), "{text}");
    assert!(text.contains(" 2  word word"), "{text}");
    assert!(text.contains(" 3  third"), "{text}");
    let popup = app.hits.popup;
    let gutters: Vec<String> = s.rows[popup.y as usize + 1..popup.bottom() as usize - 1].iter()
        .map(|r| r.chars().skip(popup.x as usize + 1).take(4).collect::<String>())
        .filter(|g| !g.trim().is_empty())
        .collect();
    assert_eq!(gutters, ["  1 ", "  2 ", "  3 "].map(String::from), "only a line's first row is numbered: {text}");
    let third = s.rows.iter().position(|r| r.contains(" 3  third")).unwrap() as u16;
    let text_rect = app.hits.compose_text;
    assert_eq!((s.cursor.x, s.cursor.y), (text_rect.x + 5, third), "cursor at the end of `third`");
    assert!(text.contains("115 chars · 3 lines"), "{text}");
    assert_eq!(s.buf[(1 + app.hits.popup.x, third)].bg, theme::line_hl(), "cursor row is tinted");
}

#[test]
fn preview_mode_renders_markdown_and_flips_the_switch() {
    let mut app = open();
    type_text(&mut app, "# Heading\n\n- item one");
    handle_key(&mut app, ctrl('r'));
    let s = shot(&mut app, 120, 40);
    let text = s.text();
    assert!(text.contains("Heading") && text.contains("item one"), "{text}");
    assert!(!text.contains("# Heading"), "markdown is rendered: {text}");
    assert_eq!(word_fg(&s, "Preview"), theme::amber());
    assert_eq!(word_fg(&s, "Write"), theme::muted());
    assert!(text.contains("21 chars · 3 lines"), "{text}");
    handle_key(&mut app, ctrl('r'));
    assert!(shot(&mut app, 120, 40).text().contains(" 1  # Heading"));
}

#[test]
fn the_switch_clicks_between_modes() {
    let mut app = open();
    type_text(&mut app, "hello");
    let _ = shot(&mut app, 120, 40);
    press_switch(&mut app);
    assert!(app.draft().unwrap().view.preview);
    let _ = shot(&mut app, 120, 40);
    press_switch(&mut app);
    assert!(!app.draft().unwrap().view.preview);
    assert!(shot(&mut app, 120, 40).text().contains(" 1  hello"));
}

#[test]
fn typing_in_preview_returns_to_write() {
    let mut app = open();
    type_text(&mut app, "ab");
    handle_key(&mut app, ctrl('r'));
    type_text(&mut app, "c");
    let draft = app.draft().unwrap();
    assert!(!draft.view.preview);
    assert_eq!(draft.body, "abc");
}

#[test]
fn sending_title_shows_the_spinner_and_hides_the_cursor() {
    let mut app = open();
    type_text(&mut app, "hi");
    app.draft_mut().unwrap().sending = true;
    let s = shot(&mut app, 120, 40);
    let text = s.text();
    assert!(text.contains("Sending… Comment on #7"), "{text}");
    assert!(text.contains("sending… esc closes"), "{text}");
}

#[test]
fn reply_and_line_titles_name_the_target() {
    let mut app = open();
    app.draft_mut().unwrap().target = crate::app::ComposeTarget::Reply { thread_id: "t".into() };
    assert!(shot(&mut app, 120, 40).text().contains("Reply on #7"));
    app.draft_mut().unwrap().target = crate::app::ComposeTarget::Line { path: "a.rs".into(), line: 3 };
    assert!(shot(&mut app, 120, 40).text().contains("Line comment  a.rs:3"));
}

#[test]
fn click_in_the_text_places_the_cursor_and_the_wheel_scrolls() {
    let mut app = open();
    type_text(&mut app, "alpha\nbeta\ngamma");
    let _ = shot(&mut app, 120, 40);
    let rect = app.hits.compose_text;
    press(&mut app, rect.x + 2, rect.y + 1);
    assert_eq!(app.draft().unwrap().cursor, 6 + 2, "row 2 col 2");
    press(&mut app, rect.x + 40, rect.y);
    assert_eq!(app.draft().unwrap().cursor, 5, "past the end of a line lands at its end");
    let gutter = app.hits.popup.x + 2;
    press(&mut app, gutter, rect.y);
    assert_eq!(app.draft().unwrap().cursor, 0, "a gutter click lands at the row start");
    let outside = app.hits.popup.bottom() + 1;
    press(&mut app, rect.x, outside);
    assert_eq!(app.draft().unwrap().cursor, 0, "a click outside the text keeps the cursor");

    let mut app = open();
    type_text(&mut app, &"line\n".repeat(60));
    let _ = shot(&mut app, 120, 40);
    let followed = app.draft().unwrap().view.scroll;
    assert!(followed > 0, "scroll follows the cursor to the end");
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 60, 20));
    let _ = shot(&mut app, 120, 40);
    assert_eq!(app.draft().unwrap().view.scroll, followed - 3, "wheel scrolls and render keeps it");
}

#[test]
fn up_and_down_move_by_wrapped_rows() {
    let mut app = open();
    type_text(&mut app, &"word ".repeat(40));
    let _ = shot(&mut app, 120, 40);
    let end = app.draft().unwrap().cursor;
    handle_key(&mut app, key(KeyCode::Up));
    assert!(app.draft().unwrap().cursor < end - 40, "up moves a whole visual row");
}

#[test]
fn toggle_has_a_key_a_mouse_gesture_and_a_hint() {
    let binding = crate::keymap::describe(Action::ComposeTogglePreview).expect("bound");
    assert_eq!(binding.key_name(), "ctrl-r");
    assert!(binding.hint.is_some());
}
