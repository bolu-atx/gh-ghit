//! Mouse on the dependency map: press a node to select and grab it,
//! drag it to move and pin it, drag empty space to pan, release on a
//! node to show its card and press it again to open it, click an edge
//! for its card, click the card's buttons, wheel to zoom, right-click
//! for the map menu, click a ranking strip row to pick it. Hit tests read
//! the label rects, edge cells, card parts and strip rows `ui` recorded
//! this frame; labels win over edges.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::hit::{hit, in_rect};
use super::menu;
use crate::app::{App, FilterRow, Focus, MenuKind};
use crate::core::Pane;
use crate::model::MapScope;

/// Route `m` to the map. True when the map took it.
pub(super) fn handle_map_mouse(app: &mut App, m: MouseEvent) -> bool {
    let area = app.hits.map_area;
    if !app.map_shown() {
        app.map.drag = None;
        return false;
    }
    if app.map.drag.is_none() && in_rect(app.hits.map_card, m.column, m.row) {
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            app.nav.focus = Focus::Main;
            app.nav.pane = Pane::Main;
            if let Some((_, part)) = hit(&app.hits.map_card_hits, m.column, m.row) {
                app.map_card_click(part);
            }
        }
        return true;
    }
    if app.map.drag.is_none() && in_rect(app.hits.map_strip, m.column, m.row) {
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            app.nav.focus = Focus::Main;
            app.nav.pane = Pane::Main;
            if let Some((_, row)) = hit(&app.hits.map_strip_rows, m.column, m.row) {
                app.map_strip_pick(row);
            }
        }
        return true;
    }
    if area.width == 0 {
        app.map.drag = None;
        return false;
    }
    let inside = in_rect(area, m.column, m.row);
    let (col, row) = (m.column.saturating_sub(area.x), m.row.saturating_sub(area.y));
    let node = hit(&app.hits.map_nodes, m.column, m.row).map(|(_, index)| index);
    // Edges drawn later sit on top, so the search runs from the end.
    let edges = &app.hits.map_edges;
    let edge = || edges.iter().rev().find(|(cell, _)| *cell == (m.column, m.row)).map(|&(_, e)| e);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if inside => {
            app.nav.focus = Focus::Main;
            app.nav.pane = Pane::Main;
            let edge = if node.is_none() { edge() } else { None };
            app.map_press(node, edge, col, row);
        }
        MouseEventKind::Drag(MouseButton::Left) if app.map.drag.is_some() => {
            let col = m.column.clamp(area.x, area.right() - 1) - area.x;
            let row = m.row.clamp(area.y, area.bottom() - 1) - area.y;
            app.map_drag_to(col, row);
        }
        MouseEventKind::Up(MouseButton::Left) if app.map.drag.is_some() => app.map_release(),
        MouseEventKind::Down(MouseButton::Right) if inside => {
            app.nav.focus = Focus::Main;
            app.nav.pane = Pane::Main;
            if let Some(index) = node {
                app.map.selected = Some(app.map.graph.nodes[index].id);
            }
            menu::open(app, MenuKind::Map, (m.column, m.row));
        }
        MouseEventKind::ScrollUp if inside => app.map_zoom(1),
        MouseEventKind::ScrollDown if inside => app.map_zoom(-1),
        _ => return false,
    }
    true
}

/// Mouse path while the physics dialog is open. A press on a `−` or `+`
/// steps that row, a press elsewhere on a row focuses it, the wheel over
/// a row raises or lowers it, and a press outside the popup closes it.
pub(super) fn handle_physics_mouse(app: &mut App, m: MouseEvent) {
    if menu::legend_click(app, &m) {
        return;
    }
    let cell = hit(&app.hits.form_cells, m.column, m.row);
    match m.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if !in_rect(app.hits.form_popup, m.column, m.row) {
                return;
            }
            if let Some((_, row, _)) = cell {
                app.set_physics_focus(row);
            }
            app.physics_nudge(if m.kind == MouseEventKind::ScrollUp { 1 } else { -1 });
        }
        MouseEventKind::Down(MouseButton::Left) => match cell {
            Some((_, row, option)) => {
                app.set_physics_focus(row);
                match option {
                    Some(0) => app.physics_nudge(-1),
                    Some(2) => app.physics_nudge(1),
                    _ => {}
                }
            }
            None if !in_rect(app.hits.form_popup, m.column, m.row) => {
                app.close(crate::app::Slot::Physics);
            }
            None => {}
        },
        _ => {}
    }
}

/// Mouse path while the map filter dialog is open. A press on a chip
/// focuses and flips it, a press on a scope option shows that scope, the
/// wheel over the popup moves the focus, and a press outside closes it.
pub(super) fn handle_map_filter_mouse(app: &mut App, m: MouseEvent) {
    if menu::legend_click(app, &m) {
        return;
    }
    let inside = in_rect(app.hits.form_popup, m.column, m.row);
    match m.kind {
        MouseEventKind::ScrollUp if inside => app.map_filter_step(-1),
        MouseEventKind::ScrollDown if inside => app.map_filter_step(1),
        MouseEventKind::Down(MouseButton::Left) => match hit(&app.hits.form_cells, m.column, m.row)
        {
            Some((_, row, option)) => {
                app.set_map_filter_focus(row);
                match (app.map_filter_rows().into_iter().nth(row), option) {
                    (Some(FilterRow::Scope), Some(0)) => app.map_filter_scope(MapScope::Component),
                    (Some(FilterRow::Scope), Some(_)) => app.map_filter_scope(MapScope::Repo),
                    (Some(row), _) => app.map_filter_flip(row),
                    (None, _) => {}
                }
            }
            None if !inside => {
                app.close(crate::app::Slot::MapFilter);
            }
            None => {}
        },
        _ => {}
    }
}
