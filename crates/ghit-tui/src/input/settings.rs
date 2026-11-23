//! Mouse on the settings dialog. A press on an option picks it, a press
//! on a stepper's `−` or `+` steps it, a press elsewhere on a row focuses
//! it, the wheel over a row steps its value, and a press outside the
//! popup closes the dialog.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::hit::{hit, in_rect};
use super::menu;
use crate::app::{App, Slot};

pub(super) fn handle_settings_mouse(app: &mut App, m: MouseEvent) {
    if menu::legend_click(app, &m) {
        return;
    }
    let cell = hit(&app.hits.form_cells, m.column, m.row);
    match m.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if let Some((_, row, _)) = cell {
                app.set_settings_focus(row);
                app.settings_nudge(if m.kind == MouseEventKind::ScrollUp { -1 } else { 1 });
            }
        }
        MouseEventKind::Down(MouseButton::Left) => match cell {
            Some((_, row, option)) => {
                app.set_settings_focus(row);
                if let Some(option) = option {
                    app.settings_select(row, option);
                }
            }
            None if !in_rect(app.hits.form_popup, m.column, m.row) => {
                app.close(Slot::Settings);
            }
            None => {}
        },
        _ => {}
    }
}
