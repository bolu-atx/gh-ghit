//! Frame tests for the shortcuts modal and help text.

use crate::app::App;
use crate::input::handle_key;
use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{loaded_app, key, frame_text};

/// Help section titles in drawn order, one per section line.
fn help_titles(app: &mut App) -> Vec<String> {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut titles = vec![];
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect();
        if let Some(at) = line.find("▸ ") {
            titles.push(line[at..].split('│').next().unwrap_or_default().trim_end().to_string());
        }
    }
    titles
}

#[test]
fn shortcuts_modal_opens_and_each_close_key_dismisses_it() {
    for close in [KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')] {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('?')));
        assert!(frame_text(&mut app).contains("Shortcuts"));
        assert!(frame_text(&mut app).contains("Back in history"));
        handle_key(&mut app, key(close));
        assert!(!app.shortcuts_open);
        assert!(!app.quit);
        assert!(!frame_text(&mut app).contains("Back in history"));
    }
}

#[test]
fn shortcuts_modal_shows_credit_footer() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('?')));
    assert!(app.shortcuts_open);
    let text = frame_text(&mut app);
    assert!(text.contains("Everywhere"), "sections render first: {text}");
    assert!(text.contains("esc close · j scroll down · k scroll up"), "closer survives: {text}");
    // The credit block ends the scrolling list.
    for _ in 0..40 {
        handle_key(&mut app, key(KeyCode::PageDown));
    }
    let text = frame_text(&mut app);
    // GHIT ASCII banner (4 lines), author, and repo URL.
    assert!(text.contains("___ _   _ ___"), "banner renders: {text}");
    assert!(text.contains("created by bolu-atx"), "author renders: {text}");
    assert!(
        text.contains("https://github.com/bolu-atx/gh-ghit"),
        "url renders: {text}"
    );
}

#[test]
fn generated_help_leads_with_the_issues_pane_context() {
    use crate::core::Pane;
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('?')));
    let text = frame_text(&mut app);
    assert!(text.contains("▸ Issues pane · here"), "{text}");
    assert!(text.contains("Evict row"), "pane keys come first: {text}");
    assert!(text.find("Evict row") < text.find("Everywhere"), "{text}");
    assert_eq!(help_titles(&mut app).first().map(String::as_str), Some("▸ Issues pane · here"));
    // The key column is amber, the label muted.
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let (row, label_x) = (0..buffer.area.height)
        .find_map(|y| {
            let line: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect();
            line.find("Evict row").map(|at| (y, line[..at].chars().count() as u16))
        })
        .expect("evict row drawn");
    let key_x = (0..label_x).rev().find(|x| buffer[(*x, row)].symbol() == "x").expect("x key cell");
    assert_eq!(buffer[(key_x, row)].fg, crate::theme::amber());
    assert_eq!(buffer[(label_x, row)].fg, crate::theme::muted());
}

#[test]
fn generated_help_leads_with_the_labels_pane_context() {
    use crate::core::Pane;
    let mut app = loaded_app();
    app.nav.pane = Pane::Labels;
    handle_key(&mut app, key(KeyCode::Char('?')));
    let text = frame_text(&mut app);
    assert!(text.contains("▸ Filters pane · here"), "{text}");
    assert!(text.contains("Clear all filters"), "{text}");
    assert!(!text.contains("Evict row"), "other panes stay off the first page: {text}");
    let titles = help_titles(&mut app);
    assert_eq!(titles.first().map(String::as_str), Some("▸ Filters pane · here"));
    assert_eq!(titles.get(1).map(String::as_str), Some("▸ Everywhere · here"));
}

#[test]
fn help_scrolls_and_clamps_at_both_ends() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('?')));
    let top = frame_text(&mut app);
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.help_scroll, 0);
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.help_scroll, 1);
    for _ in 0..40 {
        handle_key(&mut app, key(KeyCode::PageDown));
    }
    let bottom = frame_text(&mut app);
    assert_ne!(top, bottom);
    let clamped = app.help_scroll;
    handle_key(&mut app, key(KeyCode::Down));
    let _ = frame_text(&mut app);
    assert_eq!(app.help_scroll, clamped);
    assert!(bottom.contains("created by bolu-atx"), "{bottom}");
    // Reopening starts at the top.
    handle_key(&mut app, key(KeyCode::Esc));
    handle_key(&mut app, key(KeyCode::Char('?')));
    assert_eq!(app.help_scroll, 0);
}
