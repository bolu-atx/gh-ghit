//! Frame, key and mouse tests for the map filter dialog.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use crate::app::App;
use crate::core::Pane;
use crate::input::{handle_key, handle_mouse};
use crate::keymap::Action;
use crate::model::{LinkKind, MapLink, MapScope};
use super::map_tests::{map_app, map_rows, pin_layout, popup_rows};
use super::test_support::{click, draw_buffer, key};

/// The map of `map_tests`, with labels on two items, settled and pinned,
/// with the filter dialog open.
fn filter_app() -> App {
    let mut app = map_app();
    let mut items = app.map.items.clone();
    items[0].labels = vec!["perf".to_string()];
    items[1].labels = vec!["bug".to_string(), "perf".to_string()];
    let links = app.map.links.clone();
    app.set_map_data(items, links);
    pin_layout(&mut app);
    handle_key(&mut app, key(KeyCode::Char('f')));
    app
}

/// Click cell of row `row`, option `option`, as the last frame drew it.
fn cell(app: &App, row: usize, option: Option<usize>) -> (u16, u16) {
    let (rect, _, _) = app.hits.form_cells.iter().find(|(_, r, o)| *r == row && *o == option).copied()
        .unwrap_or_else(|| panic!("no cell for row {row} option {option:?}"));
    (rect.x, rect.y)
}

fn press(app: &mut App, (x, y): (u16, u16)) {
    handle_mouse(app, click(MouseEventKind::Down(MouseButton::Left), x, y));
}

#[test]
fn f_opens_the_map_filter_on_the_map_and_the_format_dialog_elsewhere() {
    let mut app = map_app();
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert_eq!(app.map_filter_focus(), Some(0), "f on the focused map opens its filter");
    assert!(!app.format_open());
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.map_filter_focus().is_none(), "Esc closes it");
    handle_key(&mut app, key(KeyCode::Char('f')));
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(app.map_filter_focus().is_none(), "f closes it again");
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(app.format_open(), "f on the sidebar keeps opening Format");
    assert!(app.map_filter_focus().is_none());
}

#[test]
fn map_filter_dialog_draws_scope_switches_and_chips() {
    let mut app = filter_app();
    let rows = popup_rows(&mut app);
    let text = rows.join("\n");
    assert!(rows[0].contains(" Filter map "), "{text}");
    // Each row: its label, then its chips in order. Closed items start off,
    // every other switch on, every chip off.
    let (on, off) = ("\u{f14a}", "\u{f096}");
    let expected: [(&str, Vec<String>); 5] = [
        ("│ scope       ▌● linked", vec!["○ whole repo".into()]),
        ("│ show ", vec![format!("{off} closed items"), format!("{on} PRs"), format!("{on} milestone hubs")]),
        ("│ links ", vec![format!("{on} blocks"), format!("{on} parent"), format!("{on} closes")]),
        ("│ milestones ", vec![format!("{off} Ship it")]),
        ("│ labels ", vec![format!("{off} bug"), format!("{off} perf")]),
    ];
    for (label, chips) in expected {
        let row = rows.iter().find(|row| row.starts_with(label)).unwrap_or_else(|| panic!("no `{label}` row in:\n{text}"));
        let mut from = 0;
        for chip in chips {
            let at = row[from..].find(&chip).unwrap_or_else(|| panic!("`{row}` lacks `{chip}` in order"));
            from += at + chip.len();
        }
    }
    for hint in ["esc close", "j next row", "space toggle row", "c toggle closed items", "r reset filter"] {
        assert!(text.contains(hint), "legend lacks `{hint}`:\n{text}");
    }
}

#[test]
fn filter_keys_apply_live_and_keep_surviving_nodes_in_place() {
    let mut app = filter_app();
    let at = |app: &App, id: u64| {
        let body = &app.map.layout.bodies()[app.map.graph.index_of(id).unwrap()];
        (body.x, body.y)
    };
    let before = at(&app, 8);
    assert!(app.map.graph.index_of(9).is_some());
    handle_key(&mut app, key(KeyCode::Char('p')));
    assert!(!app.map.filter.prs);
    app.map_tick();
    assert!(app.map.graph.index_of(9).is_none(), "PRs off drops #9 on the next frame");
    assert_eq!(at(&app, 8), before, "a surviving node keeps its place");
    assert!(app.map.moving(), "the change wakes the layout");
    let summary = map_rows(&mut app).join("\n");
    assert!(summary.contains("4 items · linked to #7 · filtered"), "{summary}");

    // j moves the focus; space flips the focused row: row 4 is blocks.
    for _ in 0..4 {
        handle_key(&mut app, key(KeyCode::Char('j')));
    }
    assert_eq!(app.map_filter_focus(), Some(4));
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.map.filter.blocks);
    handle_key(&mut app, key(KeyCode::Char('k')));
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(!app.map.filter.hubs, "Enter flips the focused hubs row");
    handle_key(&mut app, key(KeyCode::Char('m')));
    assert!(app.map.filter.hubs, "m flips hubs back");
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert!(app.map.filter.closed);

    handle_key(&mut app, key(KeyCode::Char('s')));
    assert_eq!(app.map.filter.scope, MapScope::Repo);
    app.map_tick();
    assert!(app.map.graph.index_of(30).is_some(), "the whole repo shows #30");
    handle_key(&mut app, key(KeyCode::Char('r')));
    assert_eq!(app.map.filter, crate::model::MapFilter { scope: MapScope::Repo, ..Default::default() },
        "r resets every switch but keeps the scope");
}

#[test]
fn the_map_s_key_and_the_dialog_share_one_scope() {
    let mut app = map_app();
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert_eq!(app.map.filter.scope, MapScope::Repo, "s on the map flips the filter's scope");
    handle_key(&mut app, key(KeyCode::Char('f')));
    let text = popup_rows(&mut app).join("\n");
    assert!(text.contains("○ linked  ▌● whole repo"), "{text}");
}

#[test]
fn map_filter_mouse_flips_chips_picks_scope_scrolls_and_dismisses() {
    let mut app = filter_app();
    popup_rows(&mut app);
    let at = cell(&app, 2, None);
    press(&mut app, at);
    assert!(!app.map.filter.prs, "a click flips the PRs chip");
    assert_eq!(app.map_filter_focus(), Some(2), "and focuses it");
    popup_rows(&mut app);
    let repo = cell(&app, 0, Some(1));
    press(&mut app, repo);
    assert_eq!(app.map.filter.scope, MapScope::Repo, "a click on whole repo shows it");
    popup_rows(&mut app);
    let repo = cell(&app, 0, Some(1));
    press(&mut app, repo);
    assert_eq!(app.map.filter.scope, MapScope::Repo, "a second click on the shown scope keeps it");
    popup_rows(&mut app);
    let labels = app.map_filter_rows().iter().position(|row| *row == crate::app::FilterRow::Label("bug".into())).unwrap();
    let at = cell(&app, labels, None);
    press(&mut app, at);
    assert!(app.map.filter.labels.contains("bug"), "a click turns a label chip on");
    popup_rows(&mut app);
    let (x, y) = cell(&app, 0, Some(0));
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, x, y));
    assert_eq!(app.map_filter_focus(), Some(labels + 1), "the wheel moves the focus");
    popup_rows(&mut app);
    let (rect, _) = app.hits.legend.iter().find(|(_, action)| *action == Action::MapFilterReset).copied().unwrap();
    press(&mut app, (rect.x, rect.y));
    assert!(!app.map.filter.changed(), "the legend resets the filter");
    press(&mut app, (0, 0));
    assert!(app.map_filter_focus().is_none(), "a click outside closes the dialog");
}

#[test]
fn the_toolbar_menu_and_hint_bar_reach_the_map_filter() {
    let mut app = map_app();
    draw_buffer(&mut app);
    // At 120 columns the map's own hints fill the bar, so the f hint sheds;
    // the toolbar button shows the key instead.
    let (bar, _) = super::hints::bar_hints(&app);
    assert!(bar.iter().any(|hint| hint.action == Action::OpenFormat && hint.key == "f"), "the map hints f");
    let (rect, _, _) = app.hits.controls.iter()
        .find(|(_, action, pane)| *action == Action::OpenFormat && *pane == Pane::Main).copied()
        .expect("a filter button on the map toolbar");
    let tools = super::map_tests::tab_line(&mut app);
    assert!(tools.contains("f \u{f0b0} filter"), "{tools}");
    press(&mut app, (rect.x, rect.y));
    assert!(app.map_filter_focus().is_some(), "the toolbar button opens the map filter");
    handle_key(&mut app, key(KeyCode::Esc));
    let map = app.hits.map_area;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Right), map.x + 1, map.y + 1));
    let entry = app.menu().expect("map menu").entries.iter().position(|entry| entry.action == Action::OpenFormat)
        .expect("the map menu lists the filter");
    app.menu_mut().unwrap().cursor = entry;
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.map_filter_focus().is_some(), "the map menu entry opens the map filter");
    handle_key(&mut app, key(KeyCode::Esc));
    app.nav.pane = Pane::Issues;
    let (bar, _) = super::hints::bar_hints(&app);
    assert!(bar.iter().all(|hint| hint.action != Action::OpenFormat), "the sidebar shows no f hint");
}

#[test]
fn an_edge_card_names_every_kind_the_edge_carries() {
    let mut app = map_app();
    let mut links = app.map.links.clone();
    links.push(MapLink { src: 7, dst: 8, kind: LinkKind::Parent });
    let items = app.map.items.clone();
    app.set_map_data(items, links);
    pin_layout(&mut app);
    let edge = app.map.graph.edge_between(7, 8).unwrap();
    assert_eq!(app.map.graph.relation(edge), "#7 blocks and is the parent of #8");
    app.map.selected = Some(7);
    while app.map.card_edge() != Some(edge) {
        handle_key(&mut app, key(KeyCode::Char('>')));
    }
    let frame = draw_buffer(&mut app);
    let card = app.hits.map_card;
    let head: String = (card.x..card.right()).map(|x| frame[(x, card.y + 1)].symbol()).collect();
    assert!(head.contains("#7 blocks and is the parent"), "{head}");
    handle_key(&mut app, key(KeyCode::Char('f')));
    handle_key(&mut app, key(KeyCode::Char('p')));
    for _ in 0..5 {
        handle_key(&mut app, key(KeyCode::Char('j')));
    }
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.map.filter.parents);
    app.map_tick();
    let edge = app.map.graph.edge_between(7, 8).unwrap();
    assert_eq!(app.map.graph.relation(edge), "#7 blocks #8", "a hidden kind leaves the edge's words");
}
