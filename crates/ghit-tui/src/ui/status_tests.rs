//! Frame tests for the status bar.

use crate::app::App;
use crate::input::handle_key;
use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{loaded_app, key, frame_text};
use ghit_core::model::ItemRef;

/// 80-cell bar text, so tests can assert the hint survives truncation.
fn frame_text_narrow(app: &mut App, width: u16) -> String {
    let backend = TestBackend::new(width, 40);
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

#[test]
fn bar_shows_spinner_while_loading_then_collapses() {
    let mut app = loaded_app();
    app.tick = 0;
    app.session.issues_test().pending.insert(ItemRef::github(7));
    let text = frame_text(&mut app);
    assert!(text.contains("⠋ issue"), "spinner plus label: {text}");
    // Determinate progress rides the label where known.
    app.session.issues_test().progress.insert(ItemRef::github(7), (2, 7));
    let text = frame_text(&mut app);
    assert!(text.contains("issue 2/7"), "determinate progress: {text}");
    // The spinner animates off the cheap frame tick.
    app.tick = 1;
    let text = frame_text(&mut app);
    assert!(text.contains("⠙ issue 2/7"), "next frame: {text}");
    // Done means collapsed, not blank space.
    app.session.issues_test().pending.clear();
    app.session.issues_test().progress.clear();
    let text = frame_text(&mut app);
    assert!(!text.contains("⠋") && !text.contains("⠙"), "spinner gone: {text}");
}

#[test]
fn bar_keeps_auth_badge_after_dismiss_and_clean_probe() {
    let mut app = loaded_app();
    *app.session.auth_test().dismissed = true;
    crate::app::auth_failure(
        &mut app,
        "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad credentials",
    );
    let text = frame_text(&mut app);
    assert!(text.contains("auth degraded"), "badge shows: {text}");
    assert!(text.contains("R re-auth"), "badge names the key: {text}");
    let (tx, rx) = std::sync::mpsc::channel();
    *app.session.auth_test().rx = Some(rx);
    tx.send(Ok(vec![])).unwrap();
    crate::app::poll_session(&mut app);
    let text = frame_text(&mut app);
    assert!(text.contains("auth degraded"), "a clean probe keeps it: {text}");
    *app.session.auth_test().degraded = false;
    let text = frame_text(&mut app);
    assert!(!text.contains("auth degraded"), "a successful re-auth clears it: {text}");
}

#[test]
fn bar_message_wins_over_spinner() {
    let mut app = loaded_app();
    app.session.issues_test().pending.insert(ItemRef::github(7));
    app.session.issues_test().progress.insert(ItemRef::github(7), (2, 7));
    app.notify("refreshed #7".to_string());
    let text = frame_text(&mut app);
    assert!(text.contains("refreshed #7"), "message shows: {text}");
    assert!(!text.contains("2/7"), "spinner yields: {text}");
}

#[test]
fn bar_loading_fits_the_80_cell_bar() {
    let mut app = loaded_app();
    app.session.issues_test().pending.insert(ItemRef::github(7));
    app.session.diff_test().pending.insert(ItemRef::github(9));
    app.session.board_test().detail_pending.insert(ItemRef::github(100));
    let (tx, rx) = std::sync::mpsc::channel();
    *app.session.sections_test().rx = Some(rx);
    let _ = tx.send(ghit_core::github::SectionEvent::Progress { done: 120, total: 500 });
    crate::app::poll_session(&mut app);
    // Narrow render must not panic, and the short hint always survives:
    // the loading label truncates first, never the hint.
    let text = frame_text_narrow(&mut app, 80);
    assert!(text.contains("? shortcuts · q quit"), "hint survives: {text}");
    assert!(!text.contains("tab next pane"), "context hints drop under load: {text}");
    assert!(text.contains("⠋"), "spinner shows: {text}");
}

#[test]
fn toast_marker_uses_warning_icon() {
    let mut app = loaded_app();
    app.notify("hello toast".to_string());
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f071}'), "{text}");
    assert!(text.contains("hello toast"));
}

#[test]
fn status_bar_shows_repo_project_and_pinned_hint() {
    let mut app = loaded_app();
    app.session.issues_test().statuses.insert(
        ItemRef::github(7),
        vec![("ghit demo".into(), Some("Todo".into()))],
    );
    let text = frame_text(&mut app);
    let line: String = text.chars().skip(120 * 39).take(120).collect();
    assert!(line.contains("o/r"), "{line}");
    assert!(line.contains("ghit demo (Todo)"), "{line}");
    assert!(line.contains("? shortcuts · q quit"), "{line}");
    assert!(line.ends_with("q quit"), "{line}");
}

#[test]
fn focused_input_replaces_status_and_positions_cursor() {
    let mut app = loaded_app();
    app.nav.focus = crate::app::Focus::Bar;
    app.bar = "123".into();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).unwrap();
    let line: String = (0..120).map(|x| terminal.backend().buffer()[(x, 39)].symbol()).collect();
    assert!(line.starts_with("❯ 123"), "{line}");
    assert!(!line.contains("? shortcuts"), "{line}");
    assert_eq!(terminal.get_cursor_position().unwrap(), (5, 39).into());
}

#[test]
fn visible_message_appears_only_in_status_bar() {
    let mut app = loaded_app();
    app.notify("hello status".into());
    let text = frame_text(&mut app);
    assert_eq!(text.matches("hello status").count(), 1);
    let line: String = text.chars().skip(120 * 39).take(120).collect();
    assert!(line.contains("hello status"), "{line}");
    assert!(line.contains("? shortcuts · q quit"), "{line}");
    assert!(line.ends_with("q quit"), "{line}");
}

#[test]
fn pending_g_shows_in_the_bar_until_the_next_key() {
    let mut app = loaded_app();
    assert!(!frame_text(&mut app).contains("g…"));
    handle_key(&mut app, key(KeyCode::Char('g')));
    assert!(frame_text(&mut app).contains("g…"), "the prefix waits in the bar");
    handle_key(&mut app, key(KeyCode::Char('g')));
    assert!(!frame_text(&mut app).contains("g…"), "gg ends the wait");
}

#[test]
fn bar_names_the_query_source_and_the_github_action_only_with_a_query() {
    let mut app = loaded_app();
    let text = frame_text(&mut app);
    assert!(!text.contains("search GitHub") && !text.contains(" local"), "no query, no source: {text}");
    app.sidebar.query_active = "label:bug".to_string();
    let text = frame_text(&mut app);
    assert!(text.contains("local") && text.contains("H search GitHub"), "{text}");
    app.sidebar.query_github = Some(vec![ItemRef::github(7)]);
    assert!(frame_text(&mut app).contains("GitHub ·"), "source flips to GitHub");
}

#[test]
fn h_key_searches_github_with_the_active_query() {
    let mut app = loaded_app();
    app.session.set_repo_slug("o/r");
    app.sidebar.query_active = "label:bug".to_string();
    handle_key(&mut app, key(KeyCode::Char('H')));
    assert!(app.session.sections_test().rx.is_some() && *app.session.sections_test().replace, "one GitHub search is in flight");
}
