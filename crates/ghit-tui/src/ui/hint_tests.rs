//! Frame tests for the hint line, dialog legends and empty states.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use super::hints;
use super::test_support::{board_app, click, frame_text_sized, key, loaded_app};
use crate::app::{App, Tab};
use ghit_core::core::{Dialog, Pane};
use crate::input::{handle_key, handle_mouse};
use crate::keymap::{Action, Context};
use ghit_core::model::ItemRef;

const HEIGHT: u16 = 50;

type Case = (&'static str, fn() -> App, &'static [&'static str]);

fn bar_row(text: &str, width: u16) -> String {
    text.chars().skip(width as usize * (HEIGHT as usize - 1)).take(width as usize).collect()
}

fn issues_app() -> App {
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    app
}

fn diff_app() -> App {
    let mut app = loaded_app();
    app.show_issue(ItemRef::github(9));
    app.set_tab(Tab::Visual);
    app.nav.pane = Pane::Main;
    app
}

fn filters_app() -> App {
    let mut app = loaded_app();
    app.refs.numbers = vec![ItemRef::github(9)];
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string()];
    app.nav.pane = Pane::Labels;
    app
}

/// Hint count on the bar row: separators plus one.
fn shown(row: &str) -> usize {
    row.matches(" · ").count()
}

#[test]
fn bar_hints_follow_the_focused_context_and_shed_when_narrow() {
    let cases: [Case; 3] = [
        ("issues", issues_app, &["enter focus content", "x evict row", "s cycle sort", "j select next", "p pin"]),
        ("diff", diff_app, &["J next file", "K previous file", "space expand or fold file", "s split view"]),
        ("filters", filters_app, &["space toggle filter", "X clear all filters"]),
    ];
    for (name, make, wanted) in cases {
        let mut counts = vec![];
        for width in [80u16, 120, 200] {
            let mut app = make();
            let text = frame_text_sized(&mut app, width, HEIGHT);
            let row = bar_row(&text, width);
            assert!(row.trim_end().ends_with("? shortcuts · q quit"), "{name} {width}: essentials last: {row}");
            counts.push(shown(&row));
            if width == 200 {
                for hint in wanted {
                    assert!(row.contains(hint), "{name} {width}: {hint}: {row}");
                }
            }
        }
        assert!(counts[0] <= counts[1] && counts[1] <= counts[2] && counts[0] < counts[2], "{name}: sheds when narrow: {counts:?}");
    }
}

#[test]
fn diff_and_issues_hint_lines_differ() {
    let mut issues = issues_app();
    let mut diff = diff_app();
    let a = bar_row(&frame_text_sized(&mut issues, 200, HEIGHT), 200);
    let b = bar_row(&frame_text_sized(&mut diff, 200, HEIGHT), 200);
    assert!(a.contains("x evict row") && !a.contains("next file"), "{a}");
    assert!(b.contains("J next file") && !b.contains("evict row"), "{b}");
}

#[test]
fn board_legend_sheds_by_width() {
    let mut last = 0;
    for width in [80u16, 120, 200] {
        let mut app = board_app(2);
        let text = frame_text_sized(&mut app, width, HEIGHT);
        assert!(text.contains("h previous column"), "{width}: first hint always shows: {text}");
        let count = text.matches("space expand card").count() + text.matches("f filter").count();
        assert!(count >= last, "{width}: wider shows at least as much");
        last = count;
        if width == 200 {
            assert!(text.contains("h previous column · l next column · space expand card · enter open card · f filter"), "{text}");
        }
    }
}

fn assert_legend(app: &mut App, context: Context) {
    let text = frame_text_sized(app, 220, HEIGHT);
    let hints = hints::collect(&[context], app);
    assert!(!hints.is_empty(), "{context:?} has a legend");
    for hint in hints {
        let want = format!("{} {}", hint.key, hint.label);
        assert!(text.contains(&want), "{context:?} legend lacks '{want}': {text}");
    }
}

#[test]
fn every_dialog_legend_comes_from_the_registry() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert_legend(&mut app, Context::Dialog(Dialog::Format));
    let text = frame_text_sized(&mut app, 220, HEIGHT);
    for want in ["o toggle open PRs", "m toggle merged PRs", "i toggle open issues", "c toggle closed issues",
        "g cycle grouping", "e edit query", "s save query", "x delete saved query", "1-4 window", "0 reset window"] {
        assert!(text.contains(want), "format legend lacks '{want}': {text}");
    }

    let mut app = board_app(2);
    assert_legend(&mut app, Context::Dialog(Dialog::Board));
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert_legend(&mut app, Context::Dialog(Dialog::BoardFilter));

    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert_legend(&mut app, Context::Dialog(Dialog::Compose));
    handle_key(&mut app, key(KeyCode::Char('x')));
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.confirm_open());
    assert_legend(&mut app, Context::Dialog(Dialog::Confirm));

    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('?')));
    assert_legend(&mut app, Context::Dialog(Dialog::Shortcuts));
}

#[test]
fn clicking_a_bar_hint_runs_its_action() {
    let mut app = issues_app();
    let _ = frame_text_sized(&mut app, 200, HEIGHT);
    let before = app.sort_label();
    let rect = app.hits.bar_hints.iter().find(|(_, a)| *a == Action::CycleSort).expect("sort hint").0;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x + 1, rect.y));
    assert_ne!(app.sort_label(), before, "click cycles the sort");

    let _ = frame_text_sized(&mut app, 200, HEIGHT);
    let rect = app.hits.bar_hints.iter().find(|(_, a)| *a == Action::ShowHelp).expect("help hint").0;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y));
    assert!(app.shortcuts_open(), "click on ? opens the help");
    let _ = frame_text_sized(&mut app, 200, HEIGHT);
    assert!(app.hits.bar_hints.is_empty(), "dialogs leave no bar hints to click");
}

#[test]
fn empty_states_name_the_key_that_helps() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let text = frame_text_sized(&mut app, 200, HEIGHT);
    assert!(text.contains("No issues. Press f to filter."), "{text}");
    assert!(text.contains("No pull requests. Press f to filter."), "{text}");
    assert!(text.contains("hit Enter (`:`)") || text.contains("hit Enter (:)"), "{text}");

    let mut app = loaded_app();
    let text = frame_text_sized(&mut app, 200, HEIGHT);
    assert!(text.contains("Press c to write one."), "comments: {text}");

    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('f')));
    let text = frame_text_sized(&mut app, 200, HEIGHT);
    assert!(text.contains("none (s saves the query)"), "saved queries: {text}");
}
