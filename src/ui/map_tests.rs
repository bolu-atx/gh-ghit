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
