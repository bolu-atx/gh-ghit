//! Frame tests for the whole-screen layout, focus and click feedback.

use crate::app::App;
use crate::input::{handle_key, handle_mouse};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{frame_text_sized, sample, loaded_app, key, click, draw_buffer, cells, frame_text, sidebar_text};
use ghit_core::model::ItemRef;

fn frame_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect())
        .collect()
}

/// Issue 7 with a Refs pane, a History stack and a toggled-on sidebar.
fn wide_content_app() -> App {
    let mut app = loaded_app();
    app.refs.numbers = vec![ItemRef::github(9)];
    app.nav.history = vec![ItemRef::github(9)];
    app
}

/// Foreground color of the first cell spelling `needle` on buffer row
/// `y`, if the row spells it.
fn title_fg(buf: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<ratatui::style::Color> {
    let want: Vec<char> = needle.chars().collect();
    let w = buf.area.width;
    for x in 0..w {
        let mut ok = !want.is_empty();
        for (i, ch) in want.iter().enumerate() {
            let xi = x as usize + i;
            if xi >= w as usize || buf[(xi as u16, y)].symbol() != ch.to_string() {
                ok = false;
                break;
            }
        }
        if ok {
            return Some(buf[(x, y)].fg);
        }
    }
    None
}

#[test]
fn history_alone_and_sidebar_toggle_resize_main() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    app.refs.numbers.clear();
    app.refs.paired.clear();
    frame_text_sized(&mut app, 160, 40);
    let full_width = app.hits.body_area.width;
    assert!(app.hits.history_rows.is_empty());
    app.nav.history = vec![ItemRef::github(9)];
    let text = frame_text_sized(&mut app, 160, 40);
    assert!(text.contains("History"));
    assert_eq!(app.hits.refs_area.width, 0);
    assert!(app.hits.body_area.width < full_width);
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('E')));
    assert_eq!(app.nav.pane, Pane::Main);
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.nav.pane, Pane::History, "Main and History are the only visible panes");
    let before = app.hits.body_area.width;
    let text = frame_text_sized(&mut app, 160, 40);
    assert!(!text.contains("Pull requests"));
    assert_eq!(app.hits.side_issue_area.width, 0);
    assert!(app.hits.body_area.width > before);
    handle_key(&mut app, key(KeyCode::Char('E')));
    assert!(app.sidebar.show);
}

#[test]
fn click_flash_appears_then_expires() {
    use ratatui::style::Modifier;
    use std::time::{Duration, Instant};
    let mut app = loaded_app();
    // First frame populates hit areas and the per-frame row cache.
    let _ = frame_text(&mut app);
    assert!(app.hits.last_click.is_none(), "no flash before input");
    let (x, y) = (app.hits.side_issue_area.x + 1, app.hits.side_issue_area.y + 2);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    let Some((rect, _)) = app.hits.last_click else {
        panic!("press records a flash");
    };
    let body_before = app.hits.body_rows.clone();
    // The next frame inverts the clicked rect, style-only.
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    assert_eq!(app.hits.body_rows, body_before, "flash paint keeps hit areas");
    let buf = terminal.backend().buffer();
    let flashed = (rect.y..rect.y + rect.height).any(|yy| {
        (rect.x..rect.x + rect.width)
            .any(|xx| buf[(xx, yy)].style().add_modifier.contains(Modifier::REVERSED))
    });
    assert!(flashed, "clicked row inverts while the flash lives");
    // Past the dwell the flash clears instead of painting.
    app.hits.last_click = Some((rect, Instant::now() - Duration::from_millis(1000)));
    let _ = frame_text(&mut app);
    assert!(app.hits.last_click.is_none(), "expired flash clears");
}

#[test]
fn drag_hovers_without_visiting_and_up_is_noop() {
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    let visited_before = app.nav.visited.clone();
    let (x, y) = (app.hits.side_pr_area.x + 1, app.hits.side_pr_area.y + 2);
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x, y));
    assert_eq!(app.nav.hover, Some(ItemRef::github(9)), "drag previews the row");
    assert_eq!(app.nav.current, Some(ItemRef::github(7)), "drag never visits");
    assert_eq!(app.nav.visited, visited_before, "drag files nothing");
    assert!(app.hits.last_click.is_some(), "drag flashes");
    let current = app.nav.current;
    let hover = app.nav.hover;
    let visited = app.nav.visited.clone();
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, current, "release changes nothing");
    assert_eq!(app.nav.hover, hover, "release keeps hover");
    assert_eq!(app.nav.visited, visited, "release files nothing");
}

#[test]
fn narrow_frames_keep_main_wide_and_drop_columns_in_order() {
    use ghit_core::core::Pane;
    use ghit_core::core::MIN_MAIN_WIDTH;
    // Main's border and padding take this many columns off the body.
    const MAIN_CHROME: u16 = 5;
    for (width, sidebar, right) in [(80u16, false, false), (100, true, false), (120, true, true)] {
        let mut app = wide_content_app();
        let rows = frame_rows(&mut app, width, 30);
        let text = rows.join("\n");
        assert_eq!(text.contains("Issues"), sidebar, "sidebar at {width}: {text}");
        assert_eq!(text.contains("Refs"), right, "right column at {width}: {text}");
        assert!(rows.iter().any(|r| r.contains("Fix the thing")), "title on one row at {width}: {text}");
        assert!(app.hits.body_area.width + MAIN_CHROME >= MIN_MAIN_WIDTH, "main width {:?} at {width}", app.hits.body_area);
        assert!(app.sidebar.show && app.refs.show, "saved toggles stay on at {width}");
        let mut seen = vec![app.nav.pane];
        for _ in 0..6 {
            handle_key(&mut app, key(KeyCode::Tab));
            seen.push(app.nav.pane);
        }
        assert_eq!(seen.contains(&Pane::Issues), sidebar, "Tab reaches sidebar at {width}: {seen:?}");
        assert_eq!(seen.contains(&Pane::Refs), right, "Tab reaches Refs at {width}: {seen:?}");
    }
}

#[test]
fn focus_on_a_width_hidden_pane_falls_to_main_and_toasts_once() {
    use ghit_core::core::Pane;
    let mut app = wide_content_app();
    frame_rows(&mut app, 120, 30);
    app.nav.pane = Pane::Refs;
    frame_rows(&mut app, 90, 30);
    assert_eq!(app.nav.pane, Pane::Main);
    assert_eq!(app.message, "narrow terminal: right column hidden");
    app.message.clear();
    frame_rows(&mut app, 90, 30);
    assert!(app.message.is_empty(), "no repeat toast while the width holds");
    frame_rows(&mut app, 120, 30);
    assert!(app.right_visible(), "column returns when the width does");
}

#[test]
fn jk_stays_in_the_focused_pane() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    // A second issue to walk between.
    let mut extra = sample(ItemRef::github(5));
    extra.body = String::new();
    app.issues.data.insert(ItemRef::github(5), extra);
    app.issues.statuses.insert(ItemRef::github(5), vec![]);
    app.nav.visited.push(ItemRef::github(5));
    app.show_issue(ItemRef::github(7));
    assert_eq!(app.nav.pane, Pane::Issues);
    // j never crosses into PRs: it walks issues only.
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.nav.current, Some(ItemRef::github(5)));
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.nav.current, Some(ItemRef::github(5)));
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    // The PRs pane walks PRs only, starting from the first PR even
    // when the selection sits on an issue.
    app.nav.pane = Pane::PRs;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
}

#[test]
fn pane_focus_drives_border_title_and_marker() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    assert_eq!(app.nav.pane, Pane::Issues);
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    // Focused pane: thick double border, amber title, row marker.
    assert_eq!(buf[(40, 0)].symbol(), "═", "active pane has the thick border");
    assert_eq!(title_fg(&buf, 0, "Issues"), Some(crate::theme::amber()));
    let pr_top = app.hits.side_pr_area.y;
    assert_eq!(title_fg(&buf, pr_top, "Pull requests"), Some(crate::theme::muted()));
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("█"), "focused pane marks its row");
    let marker = buf.content().iter().find(|c| c.symbol() == "█").expect("marker cell");
    assert_eq!(marker.bg, crate::theme::code_bg(), "marked row carries the shade");
    // Tab away: border thins and title quiets, the marker stays on
    // the active row, cursor stays.
    handle_key(&mut app, key(KeyCode::Tab));
    assert_eq!(app.nav.pane, Pane::PRs);
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert_eq!(app.sidebar.issue_state.selected(), Some(1), "cursor kept while blurred");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    assert_eq!(buf[(40, 0)].symbol(), "─", "inactive pane thins its border");
    assert_eq!(title_fg(&buf, 0, "Issues"), Some(crate::theme::muted()));
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("█"), "blurred pane keeps the active marker");
    // Shift-Tab back: everything returns on the same row.
    handle_key(&mut app, key(KeyCode::BackTab));
    assert_eq!(app.nav.pane, Pane::Issues);
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    assert_eq!(buf[(40, 0)].symbol(), "═");
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("█"), "marker returns with focus");
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn every_pane_takes_the_double_amber_frame_and_one_focus_shows() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    app.nav.history = vec![ItemRef::github(9)];
    // Issues, History: boxed panes draw a double top-left corner.
    for pane in [Pane::Issues, Pane::History] {
        app.nav.pane = pane;
        let buf = draw_buffer(&mut app);
        let corners = cells(&buf, "╔");
        assert_eq!(corners.len(), 1, "{pane:?}: exactly one double frame");
        assert_eq!(corners[0].fg, crate::theme::amber());
    }
    let area = app.hits.history_area;
    assert_eq!(draw_buffer(&mut app)[(area.x, area.y)].symbol(), "╔");
    // Main has no box: its tab rule turns double and amber.
    app.nav.pane = Pane::Main;
    let buf = draw_buffer(&mut app);
    assert!(cells(&buf, "╔").is_empty());
    let rule = app.hits.tabs_area;
    let cell = &buf[(rule.x + 1, rule.y + rule.height - 1)];
    assert_eq!((cell.symbol(), cell.fg), ("═", crate::theme::amber()));
    // A dialog on top takes the only focus: panes go quiet.
    app.nav.pane = Pane::Issues;
    app.open(crate::app::Entry::Shortcuts);
    assert!(cells(&draw_buffer(&mut app), "╔").is_empty());
}

#[test]
fn main_focus_keeps_sidebar_marker_for_current() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    // Clicking the Diff/Visual tab moves focus to Main; the sidebar
    // marker must stay coupled to what the main pane displays.
    app.nav.pane = Pane::Main;
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    let text = sidebar_text(&mut app);
    assert!(text.contains("█"), "main focus keeps the active marker");
    // Same for a PR in the lower pane.
    app.show_issue(ItemRef::github(9));
    app.nav.pane = Pane::Main;
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let area = app.hits.side_pr_area;
    let buf = terminal.backend().buffer().clone();
    let mut pr_text = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            pr_text.push_str(buf[(x, y)].symbol());
        }
        pr_text.push('\n');
    }
    assert!(pr_text.contains("█"), "PR marker survives main focus");
}
