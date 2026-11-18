//! Mouse parity tests: every gesture goes through the rects `render`
//! recorded, the same way a real click does. Frame tests cover the menu.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::test_support::{board_app, click, frame_text_sized, key, loaded_app};
use crate::app::{App, Tab};
use crate::core::{Dialog, Layer, Pane, SideSort};
use crate::input::{handle_key, handle_mouse};
use crate::keymap::Action;

const W: u16 = 140;
const H: u16 = 40;

fn draw(app: &mut App) -> String {
    frame_text_sized(app, W, H)
}

fn press(app: &mut App, button: MouseButton, x: u16, y: u16) {
    handle_mouse(app, click(MouseEventKind::Down(button), x, y));
    handle_mouse(app, click(MouseEventKind::Up(button), x, y));
}

fn left(app: &mut App, x: u16, y: u16) {
    press(app, MouseButton::Left, x, y);
}

fn right(app: &mut App, x: u16, y: u16) {
    press(app, MouseButton::Right, x, y);
}

fn control(app: &App, action: Action) -> ratatui::layout::Rect {
    app.hits.controls.iter().find(|(_, a, _)| *a == action).map(|(rect, _, _)| *rect)
        .unwrap_or_else(|| panic!("no {action:?} control drawn"))
}

fn click_control(app: &mut App, action: Action) {
    let rect = control(app, action);
    left(app, rect.x, rect.y);
}

fn menu_labels(app: &App) -> Vec<&'static str> {
    app.menu().map(|m| m.entries.iter().map(|e| e.label).collect()).unwrap_or_default()
}

fn click_menu_row(app: &mut App, action: Action) {
    let menu = app.menu().expect("menu open");
    let index = menu.entries.iter().position(|e| e.action == action).expect("entry listed");
    let rect = app.hits.menu_rows.iter().find(|(_, i)| *i == index).map(|(r, _)| *r).expect("row drawn");
    left(app, rect.x + 2, rect.y);
}

fn first_issue_row(app: &App) -> (u16, u16) {
    (app.hits.side_issue_area.x + 5, app.hits.side_issue_area.y + 2)
}

#[test]
fn sort_title_click_cycles_and_its_arrow_reverses() {
    let mut app = loaded_app();
    draw(&mut app);
    click_control(&mut app, Action::CycleSort);
    assert_eq!(app.sidebar.sort, SideSort::Updated);
    assert_eq!(app.nav.pane, Pane::Issues, "a title click focuses its pane");
    draw(&mut app);
    let desc = app.sidebar.sort_desc;
    click_control(&mut app, Action::ReverseSort);
    assert_eq!(app.sidebar.sort, SideSort::Updated);
    assert_eq!(app.sidebar.sort_desc, !desc);
}

#[test]
fn list_state_and_footer_open_format() {
    for index in [0, 1] {
        let mut app = loaded_app();
        draw(&mut app);
        let rects: Vec<_> = app.hits.controls.iter().filter(|(_, a, _)| *a == Action::OpenFormat).collect();
        let (rect, _, _) = *rects[index];
        left(&mut app, rect.x, rect.y);
        assert!(app.format_open(), "control {index}");
    }
}

#[test]
fn filters_title_clears_every_filter() {
    let mut app = loaded_app();
    app.refs.numbers = vec![9];
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string()];
    app.sidebar.label_filter.insert("bug".to_string());
    let text = draw(&mut app);
    assert!(text.contains("clear all"));
    click_control(&mut app, Action::ClearLabels);
    assert!(app.sidebar.label_filter.is_empty());
    draw(&mut app);
    assert!(app.hits.controls.iter().all(|(_, a, _)| *a != Action::ClearLabels), "the control goes with the filters");
}

#[test]
fn diff_strip_toggles_split_and_steps_files() {
    let mut app = loaded_app();
    app.show_issue(9);
    app.set_tab(Tab::Visual);
    draw(&mut app);
    let split = app.diff.split;
    click_control(&mut app, Action::DiffSplit);
    assert_eq!(app.diff.split, !split);
    assert_eq!(app.nav.pane, Pane::Main);
    for action in [Action::DiffPrevFile, Action::DiffNextFile] {
        draw(&mut app);
        click_control(&mut app, action);
    }
}

#[test]
fn pane_title_row_focuses_refs_and_history() {
    let mut app = loaded_app();
    app.refs.numbers = vec![9];
    app.nav.history = vec![9];
    draw(&mut app);
    let refs = app.hits.refs_area;
    left(&mut app, refs.x + 2, refs.y);
    assert_eq!(app.nav.pane, Pane::Refs);
    let history = app.hits.history_area;
    left(&mut app, history.x + 2, history.y);
    assert_eq!(app.nav.pane, Pane::History);
}

#[test]
fn right_click_on_a_row_selects_it_and_lists_the_row_actions() {
    let mut app = loaded_app();
    draw(&mut app);
    let (x, y) = first_issue_row(&app);
    right(&mut app, x, y);
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Menu));
    assert_eq!(app.nav.pane, Pane::Issues);
    assert_eq!(app.nav.current, Some(7));
    let labels = menu_labels(&app);
    for wanted in ["Open row", "Pin", "Copy", "Open in browser", "Comment", "Evict row"] {
        assert!(labels.contains(&wanted), "{wanted} missing from {labels:?}");
    }
}

#[test]
fn menu_row_click_runs_the_action_and_closes_the_menu() {
    let mut app = loaded_app();
    draw(&mut app);
    let (x, y) = first_issue_row(&app);
    right(&mut app, x, y);
    draw(&mut app);
    click_menu_row(&mut app, Action::ToggleRow);
    assert!(app.menu().is_none());
    assert!(app.diff.expanded_rows.contains(&7));
    assert_eq!(app.focus().layer, Layer::Base);
}

#[test]
fn menu_keys_move_run_and_close() {
    let mut app = loaded_app();
    draw(&mut app);
    let (x, y) = first_issue_row(&app);
    right(&mut app, x, y);
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.menu().map(|m| m.cursor), Some(1));
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.menu().map(|m| m.cursor), Some(0));
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.menu().is_none());
    right(&mut app, x, y);
    handle_key(&mut app, key(KeyCode::Char('q')));
    assert!(app.menu().is_none());
    right(&mut app, x, y);
    for _ in 0..5 {
        handle_key(&mut app, key(KeyCode::Char('j')));
    }
    assert_eq!(menu_labels(&app)[5], "Expand row");
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.menu().is_none());
    assert!(app.diff.expanded_rows.contains(&7));
}

#[test]
fn m_opens_the_menu_of_the_focused_pane() {
    let mut app = loaded_app();
    app.nav.pane = Pane::Main;
    draw(&mut app);
    handle_key(&mut app, key(KeyCode::Char('m')));
    assert!(menu_labels(&app).contains(&"Comment"));
    handle_key(&mut app, key(KeyCode::Esc));
    app.nav.pane = Pane::PRs;
    handle_key(&mut app, key(KeyCode::Char('m')));
    assert!(menu_labels(&app).contains(&"Evict row"));
}

#[test]
fn click_outside_closes_the_menu_without_touching_the_selection() {
    let mut app = loaded_app();
    draw(&mut app);
    let (x, y) = first_issue_row(&app);
    right(&mut app, x, y);
    draw(&mut app);
    let current = app.nav.current;
    let body = app.hits.body_area;
    left(&mut app, body.x + 4, body.y + 4);
    assert!(app.menu().is_none());
    assert_eq!(app.nav.current, current);
    assert_eq!(app.focus().layer, Layer::Base);
}

#[test]
fn hover_never_changes_the_selection() {
    let mut app = loaded_app();
    draw(&mut app);
    let current = app.nav.current;
    let (x, y) = first_issue_row(&app);
    handle_mouse(&mut app, click(MouseEventKind::Moved, x, y + 3));
    assert_eq!(app.nav.current, current);
    assert!(app.nav.hover.is_none());
}

#[test]
fn right_click_on_refs_history_labels_and_body_opens_their_menus() {
    let mut app = loaded_app();
    app.refs.numbers = vec![9];
    app.nav.history = vec![9];
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string()];
    draw(&mut app);
    let cases = [
        (app.hits.refs_area, "Open reference", Pane::Refs),
        (app.hits.history_area, "Go to row", Pane::History),
        (app.hits.labels_area, "Toggle filter", Pane::Labels),
        (app.hits.body_area, "Comment", Pane::Main),
    ];
    for (area, wanted, pane) in cases {
        right(&mut app, area.x + 3, area.y + 1);
        assert!(menu_labels(&app).contains(&wanted), "{wanted}: {:?}", menu_labels(&app));
        assert_eq!(app.nav.pane, pane);
        handle_key(&mut app, key(KeyCode::Esc));
    }
}

#[test]
fn repo_segment_opens_the_global_menu_by_either_button() {
    for button in [MouseButton::Left, MouseButton::Right] {
        let mut app = loaded_app();
        draw(&mut app);
        let repo = app.hits.repo;
        assert!(repo.width > 0, "repo segment drawn");
        press(&mut app, button, repo.x + 1, repo.y);
        let labels = menu_labels(&app);
        for wanted in ["Refresh", "Cycle theme", "Open board", "Shortcuts", "Toggle sidebar", "Quit"] {
            assert!(labels.contains(&wanted), "{wanted} in {labels:?}");
        }
    }
}

#[test]
fn global_menu_runs_help_and_toggles_the_sidebar() {
    let mut app = loaded_app();
    draw(&mut app);
    let repo = app.hits.repo;
    left(&mut app, repo.x, repo.y);
    draw(&mut app);
    click_menu_row(&mut app, Action::ToggleSidebar);
    assert!(!app.sidebar.show);
    draw(&mut app);
    let repo = app.hits.repo;
    left(&mut app, repo.x, repo.y);
    draw(&mut app);
    click_menu_row(&mut app, Action::ShowHelp);
    assert!(app.shortcuts_open());
}

#[test]
fn board_right_click_selects_the_card_and_opens_the_board_menu() {
    let mut app = board_app(3);
    draw(&mut app);
    let (rect, col, row) = app.hits.board_cells[2];
    right(&mut app, rect.x + 1, rect.y);
    assert_eq!((app.board.col, app.board.row), (col, row));
    assert!(menu_labels(&app).contains(&"Open card"));
    draw(&mut app);
    click_menu_row(&mut app, Action::BoardExpand);
    assert_eq!(app.board.expanded, Some((col, row)));
    assert!(app.board.data.is_some(), "the board stays open");
}

#[test]
fn board_outside_click_closes_it() {
    let mut app = board_app(3);
    draw(&mut app);
    left(&mut app, 0, 0);
    assert!(app.board.data.is_none());
}

#[test]
fn help_wheel_scrolls_and_outside_click_closes() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('?')));
    draw(&mut app);
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 70, 20));
    assert_eq!(app.help_scroll, 3);
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 70, 20));
    assert_eq!(app.help_scroll, 0);
    left(&mut app, 0, 0);
    assert!(!app.shortcuts_open());
}

#[test]
fn help_shows_the_gesture_next_to_each_key() {
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('?')));
    let lines: Vec<String> = super::help::help_lines(&app).iter().map(|l| l.to_string()).collect();
    let find = |label: &str| lines.iter().find(|l| l.contains(label)).cloned().unwrap_or_default();
    assert!(find("Pin").contains("right-click menu"), "{}", find("Pin"));
    assert!(find("Cycle sort").contains("click title"), "{}", find("Cycle sort"));
    assert!(find("Open board").contains("click hint"), "{}", find("Open board"));
    assert!(find("Actions menu").contains("right-click"), "{}", find("Actions menu"));
    assert!(lines.iter().any(|l| l.contains("Close") && l.contains("click outside")));
}

#[test]
fn history_wheel_moves_the_history_cursor() {
    let mut app = loaded_app();
    app.nav.history = vec![9, 7];
    draw(&mut app);
    let (area, pane) = (app.hits.history_area, app.nav.pane);
    assert!(area.width > 0);
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, area.x + 3, area.y + 1));
    assert_eq!(app.nav.history_cursor, 1);
    assert_eq!(app.nav.pane, pane, "the wheel never moves focus");
}

#[test]
fn board_wheel_scrolls_the_column_under_the_pointer() {
    let mut app = board_app(4);
    draw(&mut app);
    let (rect, col, _) = app.hits.board_cells[0];
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, rect.x + 1, rect.y));
    assert_eq!((app.board.col, app.board.row), (col, 1));
}

#[test]
fn format_wheel_steps_the_focused_widget() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('f')));
    draw(&mut app);
    let before = app.sidebar.format_focus;
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 70, 20));
    assert_eq!(app.sidebar.format_focus, before + 1);
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 70, 20));
    assert_eq!(app.sidebar.format_focus, before);
}

#[test]
fn dialog_footer_hints_click_like_their_keys() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('f')));
    draw(&mut app);
    let (rect, _) = *app.hits.legend.iter().find(|(_, a)| *a == Action::FormatEditQuery).expect("e hint");
    left(&mut app, rect.x, rect.y);
    assert!(app.query_edit());

    let mut app = board_app(2);
    draw(&mut app);
    let (rect, _) = *app.hits.legend.iter().find(|(_, a)| *a == Action::BoardFilter).expect("f hint");
    left(&mut app, rect.x, rect.y);
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::BoardFilter));
}

#[test]
fn board_filter_outside_click_applies_and_closes_the_form() {
    let mut app = board_app(2);
    handle_key(&mut app, key(KeyCode::Char('f')));
    draw(&mut app);
    left(&mut app, 0, 0);
    assert!(!app.format_open());
    assert!(app.board.data.is_some());
}

#[test]
fn the_menu_frame_shows_actions_with_keys_and_one_focus() {
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    let backend = TestBackend::new(W, H);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let (x, y) = first_issue_row(&app);
    right(&mut app, x, y);
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer();
    let popup = app.hits.popup;
    let row_text = |y: u16| -> String { (popup.x..popup.right()).map(|x| buf[(x, y)].symbol()).collect() };
    let rows: Vec<String> = (popup.y..popup.bottom()).map(row_text).collect();
    let joined = rows.join("\n");
    assert!(rows.iter().any(|r| r.contains("█ Open row") && r.contains("enter")), "{joined}");
    assert!(rows.iter().any(|r| r.contains("Pin") && r.contains(" p ")), "{joined}");
    assert!(rows.iter().any(|r| r.contains("Evict row") && r.contains(" x ")), "{joined}");
    let doubles = buf.content().iter().filter(|c| c.symbol() == "╔").count();
    assert_eq!(doubles, 1, "only the menu wears the focus frame while it owns input");
    assert_eq!(buf[(popup.x, popup.y)].symbol(), "╔", "the menu is the focused layer");
}

fn tab_line(text: &str) -> String {
    text.lines().find(|l| l.contains("1 Details")).expect("tab line").to_string()
}

fn toolbar_actions(app: &App) -> Vec<Action> {
    let toolbar = [
        Action::DiffPrevFile, Action::DiffNextFile, Action::DiffSplit, Action::DiffExpandAll, Action::Comment,
        Action::CloseItem,
    ];
    let mut found: Vec<_> = app.hits.controls.iter().filter(|(_, a, _)| toolbar.contains(a)).collect();
    found.sort_by_key(|(rect, _, _)| rect.x);
    found.into_iter().map(|(_, a, _)| *a).collect()
}

fn open_item(number: u64, tab: Tab) -> App {
    let mut app = loaded_app();
    app.show_issue(number);
    app.set_tab(tab);
    app
}

#[test]
fn toolbar_on_an_issue_has_comment_and_close() {
    let mut app = open_item(7, Tab::Details);
    let line = tab_line(&draw(&mut app));
    assert!(line.contains("comment") && line.contains("close") && !line.contains("split"), "{line}");
    assert!(line.contains(crate::theme::TOOL_CLOSE));
    assert_eq!(toolbar_actions(&app), vec![Action::Comment, Action::CloseItem]);
}

#[test]
fn toolbar_on_a_closed_issue_reads_reopen() {
    let mut app = open_item(7, Tab::Details);
    app.issues.data.get_mut(&7).unwrap().state = "CLOSED".to_string();
    let line = tab_line(&draw(&mut app));
    assert!(line.contains("reopen") && !line.contains("close"), "{line}");
}

#[test]
fn toolbar_split_shows_on_the_pr_diff_view_only() {
    let mut app = open_item(9, Tab::Details);
    assert!(!tab_line(&draw(&mut app)).contains("split"));
    app.set_tab(Tab::Visual);
    let line = tab_line(&draw(&mut app));
    assert!(line.contains("‹ ›") && line.contains("split") && line.contains("comment"), "{line}");
    assert_eq!(
        toolbar_actions(&app),
        vec![
            Action::DiffPrevFile, Action::DiffNextFile, Action::DiffSplit, Action::DiffExpandAll,
            Action::Comment, Action::CloseItem,
        ]
    );
}

#[test]
fn toolbar_buttons_show_their_keys_and_keep_them_when_labels_drop() {
    let (comment, close, split) = (crate::theme::TOOL_COMMENT, crate::theme::TOOL_CLOSE, crate::theme::TOOL_SPLIT);
    let mut wide = open_item(9, Tab::Visual);
    let line = tab_line(&frame_text_sized(&mut wide, 140, H));
    for want in [format!("s {split} split"), format!("c {comment} comment"), format!("C {close} close")] {
        assert!(line.contains(&want), "{want}: {line}");
    }
    let narrow = (60..140).rev().find_map(|w| {
        let mut app = open_item(9, Tab::Visual);
        let line = tab_line(&frame_text_sized(&mut app, w, H));
        (!line.contains("comment") && line.contains(comment)).then_some(line)
    });
    let line = narrow.expect("an icons-and-keys tier exists");
    for want in [format!("s {split}"), format!("c {comment}"), format!("C {close}")] {
        assert!(line.contains(&want), "{want}: {line}");
    }
}

#[test]
fn toolbar_drops_labels_then_hides_as_the_pane_narrows() {
    let widths = |w: u16| {
        let mut app = open_item(9, Tab::Visual);
        let line = tab_line(&frame_text_sized(&mut app, w, H));
        (line.contains("comment"), line.contains(crate::theme::TOOL_COMMENT), toolbar_actions(&app).len())
    };
    let (labels, icon, _) = widths(140);
    assert!(labels && icon);
    let mut seen_icons_only = false;
    let mut seen_hidden = false;
    for w in (60..140).rev() {
        let (labels, icon, controls) = widths(w);
        seen_icons_only |= !labels && icon;
        seen_hidden |= !icon && controls == 0;
    }
    assert!(seen_icons_only, "icon-only tier never shows");
    assert!(seen_hidden, "toolbar never hides");
}

#[test]
fn toolbar_close_click_asks_before_writing() {
    let mut app = open_item(7, Tab::Details);
    draw(&mut app);
    click_control(&mut app, Action::CloseItem);
    let dialog = app.confirm().expect("confirm opens");
    assert_eq!(dialog.title, "Close #7?");
    app.confirm_cancel();
    assert!(app.confirm().is_none());
    assert_eq!(app.issues.data[&7].state, "OPEN", "cancel changes nothing");
}

#[test]
fn toolbar_close_confirm_starts_the_write_and_reopen_asks_too() {
    let mut app = open_item(7, Tab::Details);
    app.request_set_state();
    app.confirm_choose(0);
    assert!(app.message.contains("closing #7"), "{}", app.message);
    app.issues.data.get_mut(&7).unwrap().state = "CLOSED".to_string();
    app.request_set_state();
    assert_eq!(app.confirm().unwrap().title, "Reopen #7?");
}

#[test]
fn toolbar_comment_click_opens_compose() {
    let mut app = open_item(7, Tab::Details);
    draw(&mut app);
    click_control(&mut app, Action::Comment);
    assert!(app.compose_open());
}

#[test]
fn close_key_asks_first() {
    let mut app = open_item(7, Tab::Details);
    handle_key(&mut app, key(KeyCode::Char('C')));
    assert_eq!(app.confirm().map(|d| d.title.as_str()), Some("Close #7?"));
}

#[test]
fn main_menu_lists_close_or_reopen() {
    let mut app = open_item(7, Tab::Details);
    draw(&mut app);
    let body = app.hits.body_area;
    right(&mut app, body.x + 2, body.y + 1);
    assert!(menu_labels(&app).contains(&"Close or reopen"), "{:?}", menu_labels(&app));
}

#[test]
fn toolbar_expand_button_shows_on_diff_only_and_click_runs_it() {
    let (expand, fold) = (crate::theme::TOOL_EXPAND, crate::theme::TOOL_FOLD);
    for (number, tab) in [(9, Tab::Details), (7, Tab::Visual)] {
        let mut app = open_item(number, tab);
        let line = tab_line(&draw(&mut app));
        assert!(!line.contains("expand") && !line.contains(expand), "{line}");
    }
    let mut issue = open_item(7, Tab::Details);
    assert!(!tab_line(&draw(&mut issue)).contains("expand"));
    let mut app = open_item(9, Tab::Visual);
    app.diff.files.insert(9, vec![crate::model::DiffFile {
        filename: "a.rs".into(), additions: 1, patch: "@@ -1 +1 @@\n+new".into(), ..Default::default()
    }]);
    let line = tab_line(&frame_text_sized(&mut app, 140, H));
    assert!(line.contains(&format!("a {expand} expand")), "{line}");
    app.diff.line = None;
    click_control(&mut app, Action::DiffExpandAll);
    assert!(app.diff.expanded_files.contains(&(9, "a.rs".to_string())));
    let line = tab_line(&frame_text_sized(&mut app, 140, H));
    assert!(line.contains(&format!("a {fold} fold")), "{line}");
}
