//! Frame tests for the board modal.

use crate::app::{App, board_move};
use crate::input::{handle_key, handle_mouse};
use crate::model::{
    BoardDetail, BoardItem, Label,
};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use super::test_support::{board_app, press_cell, TempPrefs, click, frame_text};
use crate::model::ItemRef;

#[test]
fn board_filter_form_clicks_toggle_fields_and_focus_text() {
    let mut app = board_app(2);
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let _ = frame_text(&mut app);
    let pills = app.board_pills().len();
    let before = app.board.fields.shown(0).unwrap_or(false);
    press_cell(&mut app, pills + 1, None);
    assert_eq!(app.board.fields.shown(0).unwrap_or(false), !before);
    assert_eq!(app.board.filter_focus, pills + 1);
    let _ = frame_text(&mut app);
    press_cell(&mut app, 0, None);
    assert!(app.query_edit());
    assert_eq!(app.board.filter_focus, 0);
    // Outside press and wheel leave the board and the form alone.
    let pos = (app.board.col, app.board.row);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), 0, 0));
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 0, 0));
    assert!(app.format_open());
    assert_eq!((app.board.col, app.board.row), pos);
    assert!(app.board.data.is_some());
}

#[test]
fn board_loading_shows_while_loading() {
    let mut app = App::new(Some("o/r".to_string()));
    app.board.loading = true;
    app.open(crate::app::Entry::Board);
    let text = frame_text(&mut app);
    assert!(text.contains("Fetching") && text.contains("Loading board…"), "loading body");
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
fn board_header_links_milestones_and_project() {
    let mut app = board_app(1);
    let _ = frame_text(&mut app);
    let urls: Vec<&str> = app.hits.board_links.iter().map(|(_, u)| u.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://github.com/o/r/milestones",
            "https://github.com/orgs/o/projects/1"
        ],
        "flag then project link: {urls:?}"
    );
    assert!(app.hits.board_links.iter().all(|(r, _)| r.width == 3 && r.height == 1));
}

#[test]
fn board_headers_show_card_counts() {
    let mut app = board_app(2);
    let board = app.board.data.as_mut().expect("board");
    board.order.push("Done".to_string());
    board.items.push(BoardItem {
        title: "Card 2".to_string(),
        number: Some(ItemRef::github(102)),
        status: Some("Done".to_string()),
        labels: vec![Label { name: "x".to_string() }],
        repo: Some("o/r".to_string()),
        milestone: None,
        content: serde_json::json!({ "type": "Issue" }),
    });
    let text = frame_text(&mut app);
    assert!(text.contains("Todo (2)"), "todo count missing: {text}");
    assert!(text.contains("Done (1)"), "done count missing: {text}");
}

#[test]
fn board_columns_fill_a_wide_modal() {
    let mut app = board_app(3);
    let board = app.board.data.as_mut().expect("board");
    board.order = vec!["Todo".to_string(), "Doing".to_string(), "Done".to_string()];
    board.items[1].status = Some("Doing".to_string());
    board.items[2].status = Some("Done".to_string());
    let backend = TestBackend::new(200, 50);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let mut cards = [Rect::new(0, 0, 0, 0); 3];
    for (rect, col, _) in &app.hits.board_cells {
        cards[*col] = *rect;
    }
    assert!(cards.iter().all(|r| r.width > 36), "columns stretch: {cards:?}");
    let (lo, hi) = (cards.iter().map(|r| r.width).min(), cards.iter().map(|r| r.width).max());
    assert!(hi.zip(lo).is_some_and(|(h, l)| h - l <= 1), "columns share the width evenly: {cards:?}");
    let popup = super::centered(Rect::new(0, 0, 200, 50), 92, 92);
    assert_eq!(
        cards[2].x + cards[2].width + 1,
        popup.x + popup.width - 1,
        "last column ends at the modal inner edge: {cards:?}"
    );
}

#[test]
fn board_empty_column_shows_empty_state() {
    let mut app = board_app(1);
    app.board.data.as_mut().expect("board").order.push("Done".to_string());
    let text = frame_text(&mut app);
    assert!(text.contains("Done (0)"), "zero count missing: {text}");
    assert!(text.contains("no cards"), "empty label missing: {text}");
}

#[test]
fn board_filter_narrows_columns_then_x_restores() {
    use crossterm::event::KeyEvent;
    // Enter/Esc persist the filter, so isolate the data dir.
    let _prefs = TempPrefs::new();
    let mut app = board_app(2);
    let board = app.board.data.as_mut().expect("board");
    board.order.push("Done".to_string());
    board.items.push(BoardItem {
        title: "Other card".to_string(),
        number: Some(ItemRef::github(200)),
        status: Some("Done".to_string()),
        labels: vec![Label { name: "chore".to_string() }],
        repo: Some("o/r".to_string()),
        milestone: None,
        content: serde_json::json!({ "type": "Issue" }),
    });
    board.items[0].milestone = Some("v0.3".to_string());
    let all = frame_text(&mut app);
    assert!(all.contains("Todo (2)") && all.contains("Done (1)"), "unfiltered counts: {all}");

    let key = |app: &mut App, code| handle_key(app, KeyEvent::new(code, KeyModifiers::empty()));
    key(&mut app, KeyCode::Char('f'));
    let dialog = frame_text(&mut app);
    assert!(dialog.contains("board cards"), "subtitle names the target: {dialog}");
    assert!(dialog.contains("v0.3"), "milestone pill lists the card milestone: {dialog}");
    // Down moves to the milestone pill, space toggles it, Enter applies.
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char(' '));
    key(&mut app, KeyCode::Enter);

    let narrowed = frame_text(&mut app);
    assert!(narrowed.contains("Todo (1)"), "filtered count: {narrowed}");
    assert!(narrowed.contains("Done (0)"), "empty lane count: {narrowed}");
    assert!(narrowed.contains("no matches · esc clears the filter"), "empty lane label: {narrowed}");
    assert!(narrowed.contains("Card 0") && !narrowed.contains("Card 1"), "hit stays, miss hides: {narrowed}");
    assert!(narrowed.contains("milestones v0.3"), "footer names the filter: {narrowed}");
    // Cursor math reads the narrowed grid too: the hit is selected and
    // the single row cannot scroll past itself.
    assert_eq!(
        crate::app::board_selected_number(&app),
        Some(ItemRef::github(100))
    );
    board_move(&mut app, 0, 1);
    assert_eq!((app.board.col, app.board.row), (0, 0), "cursor clamps to the filtered column");

    key(&mut app, KeyCode::Char('X'));
    let restored = frame_text(&mut app);
    assert!(restored.contains("Todo (2)") && restored.contains("Done (1)"), "clear restores: {restored}");
    assert!(!restored.contains("no matches"), "empty label clears: {restored}");
    assert!(!restored.contains("milestones"), "footer clears: {restored}");
}

#[test]
fn board_card_fields_render_when_on_and_hide_when_off() {
    let mut app = board_app(1);
    let board = app.board.data.as_mut().expect("board");
    board.items[0] = BoardItem {
        title: "Ship board".to_string(),
        number: Some(ItemRef::github(102)),
        status: Some("Done".to_string()),
        labels: vec![Label { name: "bug".to_string() }],
        repo: Some("o/r".to_string()),
        milestone: Some("v1".to_string()),
        content: serde_json::json!({
            "type": "PullRequest", "url": "https://github.com/o/r/pull/102",
            "body": "", "assignees": [{"login": "amy"}], "isDraft": true,
        }),
    };
    app.board.details.insert(ItemRef::github(102), BoardDetail {
        created: "2026-11-01".to_string(),
        updated: "2026-11-05".to_string(),
        comments: 2,
    });
    // Defaults reproduce the shipped card: labels and dates on, the
    // extra fields off.
    let text = frame_text(&mut app);
    assert!(text.contains("● bug"));
    assert!(text.contains("2 comments · 2026-11-05"));
    assert!(!text.contains("@amy"), "assignee stays off: {text}");
    assert!(!text.contains("draft"), "pr state stays off: {text}");
    // Labels off hides the chips; the dates stay.
    app.board.fields.labels = false;
    let text = frame_text(&mut app);
    assert!(!text.contains("● bug"), "labels hide: {text}");
    assert!(text.contains("2 comments · 2026-11-05"));
    // Milestone, status, assignee, and PR draft state join the meta
    // line. Issues show nothing for PR state.
    app.board.fields.labels = true;
    app.board.fields.milestone = true;
    app.board.fields.status = true;
    app.board.fields.assignee = true;
    app.board.fields.pr_state = true;
    let text = frame_text(&mut app);
    assert!(text.contains("v1 · Done · @amy · draft"), "meta line: {text}");
    app.board.data.as_mut().expect("board").items[0].content["type"] =
        serde_json::json!("Issue");
    let text = frame_text(&mut app);
    assert!(!text.contains("draft") && !text.contains("ready"), "issues skip pr state: {text}");
}

#[test]
fn board_footer_shows_loading_then_dates() {
    let mut app = board_app(1);
    let before = frame_text(&mut app);
    assert!(before.contains("● x"), "{before}");
    assert!(before.contains("…"), "date cell loads in");
    app.board.details.insert(ItemRef::github(100), BoardDetail {
        created: "2026-11-01".to_string(),
        updated: "2026-11-05".to_string(),
        comments: 2,
    });
    let after = frame_text(&mut app);
    assert!(after.contains("2 comments · 2026-11-05"), "{after}");
}

#[test]
fn board_expanded_card_shows_detail() {
    let mut app = board_app(3);
    app.board.expanded = Some((0, 0));
    let text = frame_text(&mut app);
    assert!(text.contains("Todo · ISSUE"));
    assert!(text.contains("https://github.com/o/r/issues"));
    assert!(text.contains("Why"));
}

#[test]
fn space_toggles_expansion_and_move_clears_it() {
    use crossterm::event::KeyEvent;
    let mut app = board_app(3);
    let space = || KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty());
    handle_key(&mut app, space());
    assert_eq!(app.board.expanded, Some((0, 0)));
    handle_key(&mut app, space());
    assert_eq!(app.board.expanded, None);
    handle_key(&mut app, space());
    board_move(&mut app, 0, 1);
    assert_eq!(app.board.expanded, None);
    assert_eq!((app.board.col, app.board.row), (0, 1));
}

#[test]
fn board_scrolls_selected_into_view() {
    let mut app = board_app(30);
    app.board.row = 29;
    let text = frame_text(&mut app);
    assert!(text.contains("Card 29"), "selected card visible");
    assert!(app.board.states[0].offset() > 0, "list scrolled");
}

#[test]
fn board_title_carries_card_count() {
    let mut app = board_app(2);
    assert!(frame_text(&mut app).contains("2 cards"), "board subtitle");
}

#[test]
fn board_title_shows_columns_icon() {
    let mut app = board_app(1);
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f0db}'), "{text}");
    assert!(text.contains("Roadmap"));
}
