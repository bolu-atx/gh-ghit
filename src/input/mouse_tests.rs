//! Tests for the wheel router and wheel handling.

use crossterm::event::{KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use super::mouse::{WheelTarget, wheel_target};
use super::test_support::{sample, wheel};

fn wheel_app() -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let mut pr1 = sample(9);
    pr1.is_pr = true;
    let mut pr2 = sample(11);
    pr2.is_pr = true;
    for d in [sample(7), sample(8), pr1, pr2] {
        let n = d.number;
        app.issues.data.insert(n, d);
        app.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    app.hits.side_issue_area = Rect::new(0, 0, 30, 20);
    app.hits.side_pr_area = Rect::new(0, 20, 30, 20);
    app.hits.refs_area = Rect::new(90, 0, 30, 40);
    app.hits.body_area = Rect::new(30, 0, 60, 40);
    app.show_issue(7);
    // `show_issue` recomputes refs from the body, so seed the pane after.
    app.refs.numbers = vec![21, 22];
    app.nav.pane = crate::core::Pane::Main;
    app.nav.scroll = 0;
    app
}

#[test]
fn wheel_router_maps_each_area() {
    let app = wheel_app();
    assert_eq!(wheel_target(&app, 5, 5), WheelTarget::Issues);
    assert_eq!(wheel_target(&app, 5, 25), WheelTarget::PRs);
    assert_eq!(wheel_target(&app, 95, 5), WheelTarget::Refs);
    assert_eq!(wheel_target(&app, 40, 5), WheelTarget::Main);
}

#[test]
fn wheel_router_hidden_refs_falls_through_to_main() {
    let mut app = wheel_app();
    app.refs.show = false;
    assert!(!app.refs_visible());
    assert_eq!(wheel_target(&app, 95, 5), WheelTarget::Main);
}

#[test]
fn wheel_over_issues_hovers_without_visiting() {
    let mut app = wheel_app();
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.nav.hover, Some(8));
    assert_eq!(app.nav.current, Some(7), "wheel never visits");
    assert_eq!(app.nav.pane, crate::core::Pane::Main);
    assert_eq!(app.nav.scroll, 0, "body scroll untouched");
    // Wheeling back onto the current row dissolves the hover.
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 5));
    assert_eq!(app.nav.hover, None);
    assert_eq!(app.nav.current, Some(7));
}

#[test]
fn wheel_over_prs_hovers_pr_rows_only() {
    let mut app = wheel_app();
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
    assert_eq!(app.nav.hover, Some(9));
    assert_eq!(app.nav.current, Some(7), "main content untouched");
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
    assert_eq!(app.nav.hover, Some(11));
    assert_eq!(app.nav.current, Some(7));
    assert_eq!(app.nav.pane, crate::core::Pane::Main);
    assert_eq!(app.nav.scroll, 0);
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 25));
    assert_eq!(app.nav.hover, Some(9));
    assert_eq!(app.nav.current, Some(7));
}

#[test]
fn wheel_over_refs_moves_highlight_without_scrolling_body() {
    let mut app = wheel_app();
    // A mentioning line below the viewport: the old click-style jump
    // would move body scroll here; the wheel must not.
    app.hits.body_rows = vec!["unrelated".to_string(), "context".to_string(), "fixes #22".to_string()];
    app.hits.body_base = 5;
    app.nav.scroll = 9;
    assert_eq!(app.refs.highlight, None);
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
    assert_eq!(app.refs.highlight, Some(21));
    assert_eq!(app.nav.scroll, 9, "body stays where it was");
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
    assert_eq!(app.refs.highlight, Some(22));
    assert_eq!(app.nav.scroll, 9, "mention jump is click-only");
    assert_eq!(app.nav.pane, crate::core::Pane::Main);
    assert_eq!(app.nav.current, Some(7));
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 95, 5));
    assert_eq!(app.refs.highlight, Some(21));
    assert_eq!(app.nav.scroll, 9);
}

#[test]
fn jk_promotes_hover_to_current() {
    let mut app = wheel_app();
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
    assert_eq!(app.nav.hover, Some(8));
    app.nav.pane = crate::core::Pane::Issues;
    handle_key(&mut app, crossterm::event::KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()));
    assert_eq!(app.nav.current, Some(7), "k walks back from the hover");
    assert_eq!(app.nav.hover, None, "visiting clears the hover");
}

#[test]
fn wheel_over_body_scrolls_and_keeps_issue() {
    let mut app = wheel_app();
    app.nav.hover = Some(8);
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
    assert_eq!(app.nav.scroll, 3);
    assert_eq!(app.nav.current, Some(7));
    assert_eq!(app.nav.hover, Some(8), "body wheel leaves the hover alone");
    assert_eq!(app.nav.pane, crate::core::Pane::Main);
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 40, 5));
    assert_eq!(app.nav.scroll, 0);
}

#[test]
fn wheel_over_body_on_diff_tab_leaves_diff_line_alone() {
    let mut app = wheel_app();
    // Only PRs show a diff; issues show the graph tab instead.
    app.show_issue(9);
    app.nav.tab = crate::app::Tab::Visual;
    assert!(app.visual_diff());
    app.diff.line = Some(5);
    handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
    assert_eq!(app.nav.scroll, 3);
    assert_eq!(app.diff.line, Some(5));
}
