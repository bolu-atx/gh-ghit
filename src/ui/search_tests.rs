//! Frame tests for content search on the Main pane.

use crate::app::{App, Focus, Tab};
use crate::core::{Layer, Pane, TextInput};
use crate::input::{handle_key, handle_mouse};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use super::test_support::{click, diff_pr, draw_buffer, frame_text, key, loaded_app, sample};

fn main_app(body: &str) -> App {
    let mut app = loaded_app();
    let mut d = sample(7);
    d.body = body.to_string();
    app.issues.data.insert(7, d);
    app.nav.pane = Pane::Main;
    app.nav.focus = Focus::Main;
    app
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        handle_key(app, key(KeyCode::Char(c)));
    }
}

fn search(app: &mut App, query: &str) {
    handle_key(app, key(KeyCode::Char('/')));
    type_str(app, query);
    frame_text(app);
}

fn long_body() -> String {
    let mut body: String = (0..60).map(|i| format!("filler line {i}\n\n")).collect();
    body.push_str("the needle sits here\n");
    body
}

/// Count of cells wearing the match and current-match backgrounds.
fn painted(app: &mut App) -> (usize, usize) {
    let buf = draw_buffer(app);
    let near = |a: Option<ratatui::style::Color>, b: Option<ratatui::style::Color>| a == b;
    let hit = crate::theme::search_match().bg;
    let current = crate::theme::search_current().bg;
    let count = |want| buf.content().iter().filter(|c| near(Some(c.bg), want)).count();
    (count(hit), count(current))
}

#[test]
fn typing_highlights_every_match_and_the_current_one_stronger() {
    let mut app = main_app("alpha beta alpha gamma");
    search(&mut app, "alpha");
    assert_eq!(app.focus().layer, Layer::Input(TextInput::Bar));
    assert_eq!(app.search.matches.len(), 2);
    assert_eq!(painted(&mut app), (5, 5), "one current match, one other");
    let text = frame_text(&mut app);
    assert!(text.contains("/ alpha") && text.contains("1/2"), "{text}");
}

#[test]
fn enter_keeps_the_highlights_and_shows_the_counter_in_the_toolbar() {
    let mut app = main_app("alpha beta alpha gamma");
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.focus().layer, Layer::Base);
    let text = frame_text(&mut app);
    assert!(text.contains("‹ 1/2 ›"), "{text}");
    assert_eq!(painted(&mut app), (5, 5));
}

#[test]
fn n_and_capital_n_cycle_with_wrap() {
    let mut app = main_app("alpha beta alpha gamma alpha");
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Enter));
    let mut seen = vec![];
    for k in ['n', 'n', 'n', 'N'] {
        handle_key(&mut app, key(KeyCode::Char(k)));
        frame_text(&mut app);
        seen.push(app.search.counter());
    }
    assert_eq!(seen, ["2/3", "3/3", "1/3", "3/3"]);
}

#[test]
fn typing_scrolls_to_the_first_match_and_n_scrolls_between_matches() {
    let mut body = long_body();
    body.extend((0..60).map(|i| format!("\n\nmore filler {i}")));
    body.push_str("\n\nanother needle at the end\n");
    let mut app = main_app(&body);
    assert_eq!(app.nav.scroll, 0);
    search(&mut app, "needle");
    let first = app.nav.scroll;
    assert!(first > 0, "view scrolled to the first match");
    assert!(frame_text(&mut app).contains("the needle sits here"));
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(&mut app, key(KeyCode::Char('n')));
    frame_text(&mut app);
    assert!(app.nav.scroll > first, "n scrolled to the second match");
    handle_key(&mut app, key(KeyCode::Char('n')));
    frame_text(&mut app);
    assert_eq!(app.nav.scroll, first, "wrap back to the first match");
}

#[test]
fn smart_case_applies_to_the_live_query() {
    let mut app = main_app("Alpha alpha ALPHA");
    search(&mut app, "alpha");
    assert_eq!(app.search.matches.len(), 3);
    handle_key(&mut app, key(KeyCode::Esc));
    search(&mut app, "Alpha");
    assert_eq!(app.search.matches.len(), 1);
}

#[test]
fn esc_cancels_in_the_bar_and_clears_at_base() {
    let mut app = main_app("alpha beta alpha");
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(!app.search.active() && app.focus().layer == Layer::Base);
    assert_eq!(painted(&mut app), (0, 0));
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.search.active());
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(!app.search.active());
    assert_eq!(painted(&mut app), (0, 0));
}

#[test]
fn n_without_a_search_says_so() {
    let mut app = main_app("alpha");
    handle_key(&mut app, key(KeyCode::Char('n')));
    assert!(app.message.contains("press /"), "{}", app.message);
}

#[test]
fn hints_show_next_and_prev_only_while_a_search_is_active() {
    let mut app = main_app("alpha beta alpha");
    assert!(!frame_text(&mut app).contains("next match"));
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Enter));
    let text = frame_text(&mut app);
    assert!(text.contains("n next match") && text.contains("N previous match"), "{text}");
}

#[test]
fn clicking_the_counter_arrows_steps_the_match() {
    let mut app = main_app("alpha beta alpha gamma alpha");
    search(&mut app, "alpha");
    handle_key(&mut app, key(KeyCode::Enter));
    frame_text(&mut app);
    let rect_of = |app: &App, action| app.hits.controls.iter().find(|(_, a, _)| *a == action).map(|(r, _, _)| *r).expect("control");
    let next = rect_of(&app, crate::keymap::Action::ContentNext);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), next.x + 1, next.y));
    frame_text(&mut app);
    assert_eq!(app.search.counter(), "2/3");
    let prev = rect_of(&app, crate::keymap::Action::ContentPrev);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), prev.x, prev.y));
    frame_text(&mut app);
    assert_eq!(app.search.counter(), "1/3");
}

#[test]
fn sidebar_focus_keeps_the_row_search() {
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('/')));
    assert!(app.sidebar.searching && !app.search.typing);
    type_str(&mut app, "fix");
    assert_eq!(app.sidebar.search, "fix");
    assert!(app.search.query.is_empty());
}

fn diff_app() -> App {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    let file = |name: &str, patch: &str| DiffFile {
        filename: name.into(), status: "modified".into(), additions: 1, deletions: 0, patch: patch.into(),
    };
    app.diff.files.insert(9, vec![
        file("src/a.rs", "@@ -1 +1 @@\n-old\n+new needle"),
        file("src/b.rs", "@@ -1 +1 @@\n-x\n+hidden needle here"),
    ]);
    app.diff.expanded_files.insert((9, "src/a.rs".to_string()));
    app.set_tab(Tab::Visual);
    app.nav.pane = Pane::Main;
    app.nav.focus = Focus::Main;
    app
}

#[test]
fn a_match_in_a_folded_file_unfolds_it_only_when_jumped_to() {
    let mut app = diff_app();
    search(&mut app, "needle");
    handle_key(&mut app, key(KeyCode::Enter));
    frame_text(&mut app);
    assert_eq!(app.search.matches.len(), 2);
    assert_eq!(app.search.matches[1].hidden_file, Some(1));
    assert!(!app.diff.expanded_files.contains(&(9, "src/b.rs".to_string())), "typing never unfolds");
    handle_key(&mut app, key(KeyCode::Char('n')));
    assert!(app.diff.expanded_files.contains(&(9, "src/b.rs".to_string())));
    let text = frame_text(&mut app);
    assert!(text.contains("hidden needle here") && text.contains("2/2"), "{text}");
    assert_eq!(app.search.matches[1].hidden_file, None);
}

#[test]
fn diff_search_skips_the_line_number_gutter() {
    let mut app = diff_app();
    search(&mut app, "│");
    assert!(app.search.matches.is_empty());
}
