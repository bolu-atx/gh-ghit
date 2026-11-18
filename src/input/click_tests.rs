//! Tests for sidebar hit-testing and click routing.

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use super::click::sidebar_hit;
use super::test_support::{sample, wheel};
use crate::model::SideRow;

#[test]
fn expanded_row_detail_shifts_hits_below() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for n in [7, 9] {
        app.issues.data.insert(n, sample(n));
        app.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    let rows = app.issue_rows();
    assert_eq!(rows, vec![SideRow::Item(7), SideRow::Item(9)]);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).unwrap();
    let area = app.hits.side_issue_area;
    assert_eq!(app.hits.side_issue_heights, vec![3, 3]);
    assert_eq!(sidebar_hit(area, &app.hits.side_issue_rows, &app.hits.side_issue_heights, 5, area.y + 2), Some(7));

    app.diff.expanded_rows.insert(7);
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).unwrap();
    let heights = app.hits.side_issue_heights.clone();
    let rows = app.hits.side_issue_rows.clone();
    let detail = heights[0] - 3;
    assert!(detail > 3, "detail block is tall enough to span a row: {detail}");
    assert_eq!(heights[1], 3);
    // Every line of the detail block belongs to the expanded row.
    let detail_top = area.y + 4;
    for y in detail_top..detail_top + detail as u16 {
        assert_eq!(sidebar_hit(area, &rows, &heights, 5, y), Some(7), "detail line {y}");
    }
    // Row 9 starts only after the detail block plus the item's blank tail.
    let row9_top = area.y + 2 + 3 + detail as u16;
    for y in row9_top..row9_top + 3 {
        assert_eq!(sidebar_hit(area, &rows, &heights, 5, y), Some(9), "row 9 line {y}");
    }
}

#[test]
fn sidebar_click_focuses_recent_before_opening() {
    for (is_pr, pane) in [(false, Pane::Issues), (true, Pane::PRs)] {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut recent = sample(9);
        recent.is_pr = is_pr;
        app.issues.data.insert(7, sample(7));
        app.issues.data.insert(9, recent);
        app.issues.statuses.insert(7, vec![]);
        app.issues.statuses.insert(9, vec![]);
        app.ensure_visited(7);
        app.ensure_recent(9);
        app.show_issue(7);
        let area = Rect::new(0, 0, 30, 20);
        let rows = if is_pr { app.pr_rows() } else { app.issue_rows() };
        let row_index = rows.iter().position(|row| row.number() == Some(9)).unwrap();
        let y = area.y + 2 + rows[..row_index].iter()
            .map(|row| if row.number().is_some() { 3 } else { 1 }).sum::<u16>();
        if is_pr {
            app.hits.side_pr_area = area;
            app.hits.side_pr_heights = rows.iter().map(|row| if row.number().is_some() { 3 } else { 1 }).collect();
            app.hits.side_pr_rows = rows.clone();
        } else {
            app.hits.side_issue_area = area;
            app.hits.side_issue_heights = rows.iter().map(|row| if row.number().is_some() { 3 } else { 1 }).collect();
            app.hits.side_issue_rows = rows.clone();
        }
        let click = wheel(MouseEventKind::Down(MouseButton::Left), 5, y);

        handle_mouse(&mut app, click);
        assert_eq!(app.nav.pane, pane);
        assert_eq!(app.nav.current, Some(9));
        assert!(!app.nav.visited.contains(&9));
        assert_eq!(if is_pr { app.pr_rows() } else { app.issue_rows() }, rows);

        handle_mouse(&mut app, click);
        assert!(app.nav.visited.contains(&9));
        assert_eq!(app.nav.current, Some(9));
        assert_eq!(app.side_counts(is_pr), (0, 1 + usize::from(!is_pr), 0));
    }
}
