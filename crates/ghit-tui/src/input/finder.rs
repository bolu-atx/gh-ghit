//! Mouse path for the Ctrl+P finder: a click on a result opens it, the
//! wheel moves the cursor, a click outside closes it.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::hit::{hit, in_rect};
use super::menu::legend_click;
use crate::app::{App, Slot};

/// The finder owns every mouse event while it is open.
pub(super) fn handle_finder_mouse(app: &mut App, m: MouseEvent) {
    if legend_click(app, &m) {
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => match hit(&app.hits.finder_rows, m.column, m.row) {
            Some((rect, index)) => {
                if let Some(finder) = app.finder_mut() {
                    finder.cursor = index;
                }
                app.note_click(rect);
                app.finder_open();
            }
            None if !in_rect(app.hits.popup, m.column, m.row) => { app.close(Slot::Finder); }
            None => {}
        },
        MouseEventKind::Down(_) if !in_rect(app.hits.popup, m.column, m.row) => { app.close(Slot::Finder); }
        MouseEventKind::ScrollDown => app.finder_step(1),
        MouseEventKind::ScrollUp => app.finder_step(-1),
        _ => {}
    }
}
