//! Frame and mouse tests for the dependency map on the Graph tab.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use crate::app::{App, Tab};
use crate::core::Pane;
use crate::input::{handle_key, handle_mouse};
use crate::model::{LinkKind, MapItem, MapLink, MapScope};
use super::test_support::{click, draw_buffer, key, loaded_app};

fn item(number: u64, state: &str, title: &str, milestone: Option<&str>) -> MapItem {
    MapItem { number, is_pr: false, state: state.to_string(), title: title.to_string(), milestone: milestone.map(str::to_string) }
}

/// #7 (open) is blocked by #11 (open) and #12 (closed), blocks #8, and the
/// PR #9 closes it. #7 and #8 share a milestone; #30 is not linked.
fn map_app() -> App {
    let mut app = loaded_app();
    let mut pr = item(9, "OPEN", "Fix it", None);
    pr.is_pr = true;
    let items = vec![
        item(7, "OPEN", "Center issue", Some("Ship it")),
        item(8, "OPEN", "Waits on seven", Some("Ship it")),
        pr,
        item(11, "OPEN", "Open blocker", None),
        item(12, "CLOSED", "Done blocker", None),
        item(30, "OPEN", "Elsewhere", None),
    ];
    let link = |src: u64, dst: u64, kind: LinkKind| MapLink { src, dst, kind };
    let links = vec![
        link(11, 7, LinkKind::Blocks),
        link(12, 7, LinkKind::Blocks),
        link(7, 8, LinkKind::Blocks),
        link(9, 7, LinkKind::Closes),
    ];
    app.set_map_data(items, links);
    app.nav.pane = Pane::Main;
    app.set_tab(Tab::Visual);
    app
}

/// Pin every node at a hand-picked world point, so the frame does not
/// depend on the simulation.
fn pin_layout(app: &mut App) {
    app.settle_map();
    let hub = crate::model::hub_id("Ship it");
    let at = [(7, 0.0, 0.0), (8, 16.0, -4.0), (9, 0.0, 6.0), (11, -18.0, -4.0), (12, -18.0, 4.0), (hub, 16.0, 6.0)];
    for (id, x, y) in at {
        let index = app.map.graph.index_of(id).unwrap();
        app.map.layout.set_position(index, x, y);
        app.map.layout.pin(index);
    }
    app.map.follow = false;
    app.map.camera = crate::core::map::Camera::default();
}

/// Non-blank rows of the map view, each as `row|text`.
fn snapshot(app: &mut App) -> String {
    map_rows(app).iter().enumerate()
        .filter(|(_, row)| !row.is_empty())
        .map(|(y, row)| format!("{y:02}|{row}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rows of the map view as text.
fn map_rows(app: &mut App) -> Vec<String> {
    let buf = draw_buffer(app);
    let area = app.hits.map_area;
    (area.y..area.bottom())
        .map(|y| (area.x..area.right()).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
        .collect()
}

#[test]
fn map_draws_edges_arrows_labels_and_hubs() {
    let mut app = map_app();
    pin_layout(&mut app);
    let expected = [
        "11|           #11 Open blocker                                                    #8 Waits on seven",
        "12|                        ─────────                                          ───────▸    ·",
        "13|                                 ─────────                         ────────            ·",
        "14|                                          ────────▸        ────────                    ·",
        "15|                                               ▌#7 Center issue                        ·",
        "16|                                          ────────▸    │  ·····                        ·",
        "17|                                 ─────────             │       ······                  ·",
        "18|                        ─────────                      │             ·····             ·",
        "19|           #12 Done blocker                            │                  ·····        ·",
        "20|                                                       │                       ······  ·",
        "21|                                                   #9 Fix it                      ╭ Ship it ╮",
        "30| 5 items · linked to #7 · 6 pinned",
    ];
    assert_eq!(snapshot(&mut app), expected.join("\n"));

    use crate::theme::{amber, green, muted, pr_ref, red};
    let buf = draw_buffer(&mut app);
    let color = |id: u64| {
        let index = app.map.graph.index_of(id).unwrap();
        let (rect, _) = app.hits.map_nodes.iter().find(|(_, i)| *i == index).unwrap();
        buf[(rect.x, rect.y)].fg
    };
    assert_eq!(color(7), red(), "blocked by the open #11");
    assert_eq!(color(11), green(), "ready: no open blocker");
    assert_eq!(color(12), muted(), "closed blocker reads dim");
    assert_eq!(color(9), pr_ref());
    assert_eq!(color(crate::model::hub_id("Ship it")), amber());
    let selected = app.hits.map_nodes.iter().find(|(_, i)| Some(*i) == app.map.selected_index()).unwrap().0;
    assert!(buf[(selected.x, selected.y)].modifier.contains(ratatui::style::Modifier::BOLD), "selection is bold");
    assert_eq!(buf[(selected.x - 1, selected.y)].symbol(), "▌", "selection carries the marker");
}

#[test]
fn settled_map_shows_every_linked_node_and_hides_the_rest() {
    let mut app = map_app();
    app.settle_map();
    let text = map_rows(&mut app).join("\n");
    for label in ["#7", "#8", "#9", "#11", "#12", "Ship it"] {
        assert!(text.contains(label), "{label} missing:\n{text}");
    }
    assert!(!text.contains("#30"), "#30 sits outside the component:\n{text}");
    assert!(text.contains("5 items · linked to #7"), "{text}");
}

#[test]
fn labels_shrink_as_the_map_zooms_out() {
    let mut app = map_app();
    pin_layout(&mut app);
    app.map.camera.zoom = 0.5;
    let text = map_rows(&mut app).join("\n");
    assert!(text.contains("#11") && !text.contains("Open blocker"), "{text}");
    assert!(text.contains("#7 Center issue"), "the selection keeps its title: {text}");
    app.map.camera.zoom = 0.2;
    let mut rows = map_rows(&mut app);
    rows.pop();
    let text = rows.join("\n");
    assert!(!text.contains("#11") && text.contains('•'), "{text}");
}

#[test]
fn an_unstored_issue_says_so() {
    let mut app = map_app();
    app.set_map_data(vec![], vec![]);
    app.settle_map();
    let text = map_rows(&mut app).join("\n");
    assert!(text.contains("#7 is not in the local copy yet"), "{text}");
}

/// Screen cell at the middle of node `id`'s label.
fn node_cell(app: &App, id: u64) -> (u16, u16) {
    let index = app.map.graph.index_of(id).unwrap();
    let (rect, _) = app.hits.map_nodes.iter().find(|(_, i)| *i == index).unwrap();
    (rect.x + rect.width / 2, rect.y)
}

#[test]
fn dragging_a_node_moves_and_pins_it() {
    let mut app = map_app();
    pin_layout(&mut app);
    let eight = app.map.graph.index_of(8).unwrap();
    app.map.layout.unpin(eight);
    draw_buffer(&mut app);
    let (x, y) = node_cell(&app, 8);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.map.selected, Some(8), "a press selects");
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x - 10, y + 3));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x - 10, y + 3));
    let body = &app.map.layout.bodies()[eight];
    assert!(body.pinned, "a drag pins");
    let area = app.hits.map_area;
    let cell = app.map.camera.cell(body.x, body.y, area.width, area.height);
    assert_eq!(cell, (i32::from(x - 10 - area.x), i32::from(y + 3 - area.y)), "the node lands under the pointer");
    assert_eq!(app.nav.current, Some(7), "a drag never opens");
    handle_key(&mut app, key(KeyCode::Char('x')));
    assert!(!app.map.layout.bodies()[eight].pinned, "x unpins");
}

#[test]
fn dragging_empty_space_pans_and_a_second_press_opens() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    let area = app.hits.map_area;
    let before = app.map.camera;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), area.x + 1, area.y + 1));
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), area.x + 5, area.y + 3));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), area.x + 5, area.y + 3));
    assert_eq!((app.map.camera.x, app.map.camera.y), (before.x - 2.0, before.y - 2.0), "the world follows the pointer");
    draw_buffer(&mut app);
    let (x, y) = node_cell(&app, 11);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(7), "the first press selects");
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
    assert_eq!(app.nav.requested.or(app.nav.current), Some(11), "the second press opens");
}

#[test]
fn wheel_zooms_and_right_click_opens_the_map_menu() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    let zoom = app.map.camera.zoom;
    let area = app.hits.map_area;
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, area.x + 2, area.y + 2));
    assert!(app.map.camera.zoom > zoom);
    draw_buffer(&mut app);
    let (x, y) = node_cell(&app, 8);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Right), x, y));
    assert_eq!(app.map.selected, Some(8));
    let menu = app.menu().expect("menu opens");
    assert!(menu.entries.iter().any(|e| e.action == crate::keymap::Action::GraphUnpin));
}

#[test]
fn map_keys_step_pan_zoom_fit_scope_and_move() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    handle_key(&mut app, key(KeyCode::Char('.')));
    assert_eq!(app.map.selected, Some(8), ". steps by number");
    handle_key(&mut app, key(KeyCode::Char(',')));
    handle_key(&mut app, key(KeyCode::Char(',')));
    assert_eq!(app.map.selected, Some(12), ", wraps back");
    let camera = app.map.camera;
    handle_key(&mut app, key(KeyCode::Char('l')));
    assert!(app.map.camera.x > camera.x, "l pans right");
    handle_key(&mut app, key(KeyCode::Char('+')));
    assert!(app.map.camera.zoom > camera.zoom);
    handle_key(&mut app, key(KeyCode::Char('0')));
    assert!(app.map.follow, "0 fits again");
    let twelve = app.map.graph.index_of(12).unwrap();
    app.map.layout.unpin(twelve);
    let before = app.map.layout.bodies()[twelve].x;
    handle_key(&mut app, key(KeyCode::Char('D')));
    assert!(app.map.layout.bodies()[twelve].x > before && app.map.layout.bodies()[twelve].pinned);
    assert!(!app.map.follow, "a nudge stops the fit, so the view holds still");
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert_eq!(app.map.scope, MapScope::Repo);
    app.settle_map();
    let text = map_rows(&mut app).join("\n");
    assert!(text.contains("#30") && text.contains("whole repo"), "{text}");
    assert!(app.map.layout.bodies()[app.map.graph.index_of(12).unwrap()].pinned, "rebuild keeps pins");
}

#[test]
fn a_store_reread_keeps_the_selection_and_a_new_center_takes_it() {
    let mut app = map_app();
    app.settle_map();
    app.map_step(1);
    assert_eq!(app.map.selected, Some(8));
    let (items, links) = (app.map.items.clone(), app.map.links.clone());
    app.set_map_data(items, links);
    app.settle_map();
    assert_eq!(app.map.selected, Some(8), "a reread keeps the selection");
    let eight = app.map.layout.bodies()[app.map.graph.index_of(8).unwrap()].clone();
    app.issues.data.insert(11, super::test_support::sample(11));
    app.show_issue(11);
    app.settle_map();
    assert_eq!(app.map.selected, Some(11), "the new center takes the selection");
    let moved = &app.map.layout.bodies()[app.map.graph.index_of(8).unwrap()];
    assert!((moved.x - eight.x).abs() < 5.0, "one component keeps its layout");
}

#[test]
fn an_unchanged_reread_keeps_the_map_still_and_a_change_wakes_it() {
    let mut app = map_app();
    app.settle_map();
    let before: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    let (items, links) = (app.map.items.clone(), app.map.links.clone());
    app.set_map_data(items.clone(), links.clone());
    assert!(!app.map_tick(), "equal data neither rebuilds nor steps");
    let after: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    assert_eq!(before, after, "nothing moved");
    let mut renamed = items;
    renamed[1].title = "Waits on seven, renamed".to_string();
    app.set_map_data(renamed, links);
    assert!(app.map_tick() && app.map.moving(), "a changed title rebuilds and wakes the layout");
    assert_eq!(app.map.graph.nodes[app.map.graph.index_of(8).unwrap()].title, "Waits on seven, renamed");
}

#[test]
fn a_drag_follows_its_node_through_a_rebuild_and_ends_when_it_vanishes() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    let (x, y) = node_cell(&app, 8);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    // A new lowest number shifts every node index by one.
    let (mut items, mut links) = (app.map.items.clone(), app.map.links.clone());
    items.push(item(1, "OPEN", "New blocker", None));
    links.push(MapLink { src: 1, dst: 7, kind: LinkKind::Blocks });
    app.set_map_data(items.clone(), links.clone());
    app.map_tick();
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x - 10, y + 3));
    let eight = app.map.graph.index_of(8).unwrap();
    let body = &app.map.layout.bodies()[eight];
    let area = app.hits.map_area;
    let cell = app.map.camera.cell(body.x, body.y, area.width, area.height);
    assert_eq!(cell, (i32::from(x - 10 - area.x), i32::from(y + 3 - area.y)), "#8 still follows the pointer");
    assert!(!app.map.layout.bodies()[app.map.graph.index_of(1).unwrap()].pinned, "the shifted index is not moved");
    items.retain(|item| item.number != 8);
    links.retain(|link| link.dst != 8);
    app.set_map_data(items, links);
    app.map_tick();
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x - 4, y + 1));
    assert_eq!(app.map.drag, None, "the drag ends once #8 is gone");
}

#[test]
fn one_tick_steps_a_moving_layout_at_least_once() {
    let mut app = map_app();
    app.settle_map();
    draw_buffer(&mut app);
    // A nudge pins #8 somewhere new and wakes the layout.
    app.map_step(1);
    assert_eq!(app.map.selected, Some(8));
    app.map_nudge(4, 4);
    assert!(app.map.moving());
    let before: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    assert!(app.map_tick(), "a moving layout changes the screen");
    let after: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    assert_ne!(before, after, "the tick stepped the layout");
    assert_eq!(app.step_map(std::time::Duration::ZERO), 1, "a spent budget still takes one step");
}

/// Stored details for #7's card: status, labels, assignees and a body.
fn stored_detail(app: &mut App) {
    let mut issue = super::test_support::sample(7);
    issue.title = "Center issue".to_string();
    issue.labels = vec![crate::model::Label { name: "perf".to_string() }, crate::model::Label { name: "sync".to_string() }];
    issue.assignees = vec![crate::model::Author { login: "bo".to_string() }, crate::model::Author { login: "ana".to_string() }];
    let detail = crate::model::BoardDetail { created: "2026-01-02".to_string(), updated: "2026-01-05".to_string(), comments: 2 };
    let row = crate::github::Row {
        issue,
        statuses: vec![("Board".to_string(), Some("In progress".to_string()))],
        detail,
        complete: false,
        window_at: None,
    };
    let body = "Split the queue in two.\n\nThe current issue goes first; the rest wait their turn.".to_string();
    app.map.detail = Some(crate::app::CardDetail { number: 7, version: app.map.version, row: Some(row), body: Some(body) });
}

/// Rows of the card as text.
fn card_rows(app: &mut App) -> String {
    let buf = draw_buffer(app);
    let area = app.hits.map_card;
    (area.y..area.bottom())
        .map(|y| (area.x..area.right()).map(|x| buf[(x, y)].symbol()).collect::<String>().trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Screen cell of some drawn cell of the edge from `src` to `dst`.
fn edge_cell(app: &App, src: u64, dst: u64) -> (u16, u16) {
    let edge = app.map.graph.edge_between(src, dst).unwrap();
    app.hits.map_edges.iter().rev().find(|(_, e)| *e == edge).map(|(cell, _)| *cell).unwrap()
}

#[test]
fn node_card_shows_the_stored_item() {
    let mut app = map_app();
    pin_layout(&mut app);
    stored_detail(&mut app);
    app.map.card = Some(crate::app::MapCard::Node);
    let text = card_rows(&mut app);
    let rows: Vec<&str> = text.lines().collect();
    let expected = [
        "┌ #7 ───────────────────────────────── × ┐",
        "│ Center issue                           │",
        "│ ● issue · open · blocked               │",
        "│                                        │",
        "│ milestone Ship it                      │",
        &format!("│ status    {} In progress                │", crate::theme::column_icon("In progress")),
        "│ labels    ● perf ● sync                │",
        "│ assignees @bo @ana                     │",
        "│ updated   2 comments · 2026-01-05      │",
        "│                                        │",
        "│ Split the queue in two.                │",
        "│                                        │",
        "│ The current issue goes first; the rest │",
        "│ wait their turn.                       │",
    ];
    assert_eq!(rows[..expected.len()].join("\n"), expected.join("\n"));
    assert_eq!(rows[rows.len() - 2], "│ [open ↵] [next link e] [close esc]     │");
    let map = map_rows(&mut app).join("\n");
    assert!(map.contains("▌#7 Center issue"), "the selected node stays in view:\n{map}");
    assert!(app.hits.map_area.right() <= app.hits.map_card.x, "the map shrinks to the left of the card");

    app.map.detail = None;
    let text = card_rows(&mut app);
    assert!(text.contains("Not in the local copy yet.") && text.contains("milestone Ship it"), "{text}");
}

#[test]
fn edge_card_names_the_link_and_both_ends() {
    let mut app = map_app();
    pin_layout(&mut app);
    app.map.card = Some(crate::app::MapCard::Edge { src: 9, dst: 7 });
    let text = card_rows(&mut app);
    let rows: Vec<&str> = text.lines().collect();
    let expected = [
        "┌ link ─────────────────────────────── × ┐",
        "│ #9 closes #7                           │",
        "│                                        │",
        "│  #9 Fix it                             │",
        "│   PR · open                            │",
        "│                                        │",
        "│ ▌#7 Center issue                       │",
        "│   issue · open · blocked               │",
    ];
    assert_eq!(rows[..expected.len()].join("\n"), expected.join("\n"));
    assert_eq!(rows[rows.len() - 2], "│ [next link e] [close esc]              │");
    let buf = draw_buffer(&mut app);
    let (x, y) = edge_cell(&app, 9, 7);
    assert_eq!(buf[(x, y)].fg, crate::theme::teal(), "the card's edge is highlighted");
}

#[test]
fn edge_hits_skip_labels_and_a_click_on_an_edge_shows_its_card() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    for &((x, y), _) in &app.hits.map_edges {
        assert!(app.hits.map_nodes.iter().all(|(rect, _)| !rect.contains((x, y).into())), "labels win at {x},{y}");
    }
    let (x, y) = edge_cell(&app, 11, 7);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
    assert_eq!(app.map.card, Some(crate::app::MapCard::Edge { src: 11, dst: 7 }));
    let text = card_rows(&mut app);
    assert!(text.contains("#11 blocks #7"), "{text}");
    let (end, _) = app.hits.map_card_hits.iter().find(|(_, part)| *part == crate::app::CardHit::End(11)).unwrap();
    let (ex, ey) = (end.x + 2, end.y);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), ex, ey));
    assert_eq!((app.map.selected, app.map.card), (Some(11), Some(crate::app::MapCard::Node)), "an end card selects its node");
    draw_buffer(&mut app);
    let (x, y) = edge_cell(&app, 12, 7);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x + 3, y));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x + 3, y));
    assert_eq!(app.map.card, Some(crate::app::MapCard::Node), "dragging from an edge pans, it shows no card");
}

#[test]
fn a_press_shows_the_card_and_the_open_button_or_a_second_press_opens() {
    let mut app = map_app();
    pin_layout(&mut app);
    draw_buffer(&mut app);
    let (x, y) = node_cell(&app, 8);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
    assert_eq!((app.map.selected, app.map.card), (Some(8), Some(crate::app::MapCard::Node)), "the first press shows the card");
    assert_eq!(app.nav.current, Some(7));
    draw_buffer(&mut app);
    let (open, _) = *app.hits.map_card_hits.iter().find(|(_, part)| *part == crate::app::CardHit::Open).unwrap();
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), open.x, open.y));
    assert_eq!(app.nav.requested.or(app.nav.current), Some(8), "the open button opens");
}

#[test]
fn card_keys_toggle_step_links_and_close() {
    let mut app = map_app();
    pin_layout(&mut app);
    handle_key(&mut app, key(KeyCode::Char('i')));
    assert_eq!(app.map.card, Some(crate::app::MapCard::Node), "i shows the card");
    handle_key(&mut app, key(KeyCode::Char('i')));
    assert_eq!(app.map.card, None, "i again hides it");
    let seven = app.map.graph.index_of(7).unwrap();
    let links: Vec<String> = app.map.graph.edges_of(seven).into_iter().map(|e| app.map.graph.relation(e, &app.map.links)).collect();
    let mut seen = vec![];
    for _ in 0..links.len() {
        handle_key(&mut app, key(KeyCode::Char('>')));
        seen.push(app.map.graph.relation(app.map.card_edge().unwrap(), &app.map.links));
    }
    assert_eq!(seen, links, "> steps through every link of the node");
    handle_key(&mut app, key(KeyCode::Char('>')));
    assert_eq!(app.map.graph.relation(app.map.card_edge().unwrap(), &app.map.links), links[0], "and wraps");
    handle_key(&mut app, key(KeyCode::Char('<')));
    assert_eq!(app.map.graph.relation(app.map.card_edge().unwrap(), &app.map.links), links[links.len() - 1]);
    handle_key(&mut app, key(KeyCode::Esc));
    assert_eq!(app.map.card, None, "esc closes the card");
    assert_eq!(app.nav.tab, Tab::Visual, "and nothing else");
}

#[test]
fn a_milestone_card_lists_its_items_and_a_click_selects_one() {
    let mut app = map_app();
    pin_layout(&mut app);
    app.map.selected = Some(crate::model::hub_id("Ship it"));
    handle_key(&mut app, key(KeyCode::Char('i')));
    let text = card_rows(&mut app);
    for line in ["┌ milestone ", "◆ milestone · 2 items on the map", "#7 Center issue", "#8 Waits on seven"] {
        assert!(text.contains(line), "{line} missing:\n{text}");
    }
    assert!(!text.contains("[open"), "a milestone has nothing to open:\n{text}");
    let (row, _) = *app.hits.map_card_hits.iter().find(|(_, part)| *part == crate::app::CardHit::End(8)).unwrap();
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), row.x, row.y));
    assert_eq!((app.map.selected, app.map.card), (Some(8), Some(crate::app::MapCard::Node)));
}
