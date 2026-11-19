//! Mouse on the dependency map: press a node to select and grab it,
//! drag it to move and pin it, drag empty space to pan, press the
//! selected node again to open it, wheel to zoom, right-click for the
//! map menu. Hit tests read the label rects `ui` recorded this frame.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::hit::{hit, in_rect};
use super::menu;
use crate::app::{App, Focus, MenuKind};
use crate::core::Pane;

/// Route `m` to the map. True when the map took it.
pub(super) fn handle_map_mouse(app: &mut App, m: MouseEvent) -> bool {
    let area = app.hits.map_area;
    if !app.visual_graph() || area.width == 0 {
        app.map.drag = None;
        return false;
    }
    let inside = in_rect(area, m.column, m.row);
    let (col, row) = (m.column.saturating_sub(area.x), m.row.saturating_sub(area.y));
    let node = hit(&app.hits.map_nodes, m.column, m.row).map(|(_, index)| index);
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if inside => {
            app.nav.focus = Focus::Main;
            app.nav.pane = Pane::Main;
            app.map_press(node, col, row);
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
