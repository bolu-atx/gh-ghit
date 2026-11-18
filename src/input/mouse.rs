//! Mouse entry, wheel routing, and the form and board mouse paths.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::hit::{hit, in_rect};
use super::click::{handle_left, open_url};
use super::menu;
use super::toggle_board_focused;

use crate::app::{App, board_open, board_selected_number};
use crate::core::{self, Dialog, Layer};

/// Where a wheel event lands: the Issues/PRs side panes, the right
/// Refs pane while visible, or the main body everywhere else. A hidden
/// Refs pane falls through to Main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WheelTarget {
    Issues,
    PRs,
    Refs,
    Labels,
    History,
    Main,
}

pub(crate) fn wheel_target(app: &App, x: u16, y: u16) -> WheelTarget {
    if in_rect(app.hits.side_issue_area, x, y) {
        WheelTarget::Issues
    } else if in_rect(app.hits.side_pr_area, x, y) {
        WheelTarget::PRs
    } else if app.refs_visible() && in_rect(app.hits.refs_area, x, y) {
        WheelTarget::Refs
    } else if in_rect(app.hits.labels_area, x, y) {
        WheelTarget::Labels
    } else if in_rect(app.hits.history_area, x, y) {
        WheelTarget::History
    } else {
        WheelTarget::Main
    }
}

/// Wheel steps a highlight-only hover through one sidebar order. Unlike
/// j/k this never visits: `current`, body scroll, and the other panes
/// stay exactly where they were. Landing back on `current` clears the
/// hover since there is nothing left to distinguish.
fn wheel_hover(app: &mut App, order: Vec<u64>, delta: isize) {
    let base = app.nav.hover.filter(|n| order.contains(n)).or(app.nav.current);
    if let Some(n) = core::next_in_order(&order, base, delta) {
        app.nav.hover = if Some(n) == app.nav.current { None } else { Some(n) };
    }
}

/// Wheel scrolls only the hovered area: side panes move the hover so
/// `ListState` offsets follow, refs moves its highlight without the
/// click-style body jump, and Main keeps the body scroll. The focused
/// pane never changes; click/Enter/j/k promote the hover to a visit.
fn wheel_move(app: &mut App, target: WheelTarget, delta: isize) {
    match target {
        WheelTarget::Issues => wheel_hover(app, app.issue_numbers(), delta),
        WheelTarget::PRs => wheel_hover(app, app.pr_numbers(), delta),
        WheelTarget::Refs => {
            let next = core::next_in_order(&app.ref_order(), app.refs.highlight, delta);
            if let Some(n) = next.filter(|n| Some(*n) != app.refs.highlight) {
                app.refs.highlight = Some(n);
            }
        }
        WheelTarget::Labels => app.move_filter_cursor(delta),
        WheelTarget::History => app.history_step(delta),
        WheelTarget::Main => {
            app.nav.scroll = app.nav.scroll.saturating_add_signed(delta as i16 * 3);
        }
    }
}

pub(crate) fn handle_mouse(app: &mut App, m: MouseEvent) {
    if matches!(m.kind, MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
        app.pending_prefix = None;
    }
    let stack = app.layer_stack();
    let open = |dialog: Dialog| stack.contains(&Layer::Dialog(dialog));
    if open(Dialog::Confirm) {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some((_, i)) = hit(&app.hits.confirm_choices, m.column, m.row) {
                app.confirm_choose(i);
            } else if !in_rect(app.hits.popup, m.column, m.row) {
                app.confirm_cancel();
            }
        }
        return;
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
        && app.undo_live()
        && !open(Dialog::Shortcuts)
        && !open(Dialog::Compose)
        && in_rect(app.hits.undo_toast, m.column, m.row)
    {
        app.undo_last();
        return;
    }
    // Shortcuts and compose steal all mouse focus: clicks and wheel never
    // reach the panes behind them, so the draft and selection survive.
    if open(Dialog::Shortcuts) || open(Dialog::Compose) {
        menu::handle_modal_mouse(app, m, open(Dialog::Shortcuts));
        return;
    }
    if open(Dialog::Menu) {
        menu::handle_menu_mouse(app, m);
        return;
    }
    if open(Dialog::Format) || open(Dialog::BoardFilter) {
        handle_form_mouse(app, m);
        return;
    }
    if open(Dialog::Board) {
        handle_board_mouse(app, m);
        return;
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        let hint = hit(&app.hits.bar_hints, m.column, m.row);
        if menu::press(app, hint) {
            return;
        }
        if menu::repo_click(app, m.column, m.row) || menu::control_click(app, m.column, m.row) {
            return;
        }
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_left(app, m.column, m.row, false),
        MouseEventKind::Down(MouseButton::Right) => menu::right_click(app, m.column, m.row),
        // Drag keeps hover/scroll continuity while the button is held:
        // side panes preview the row under the cursor, Main keeps its
        // scroll, and nothing commits until a real press lands.
        MouseEventKind::Drag(MouseButton::Left) => handle_left(app, m.column, m.row, true),
        // Release is a no-op by design: selection commits on press, so a
        // release off-target never undoes it, and drag scroll never clicks.
        MouseEventKind::Up(_) => {}
        MouseEventKind::ScrollDown => {
            wheel_move(app, wheel_target(app, m.column, m.row), 1);
        }
        MouseEventKind::ScrollUp => {
            wheel_move(app, wheel_target(app, m.column, m.row), -1);
        }
        _ => {}
    }
}

/// Mouse path while the Format dialog or board filter form is open. The
/// dialog owns every event: a press on a recorded cell focuses its widget
/// and runs the keyboard action, a press outside the popup closes the
/// sidebar dialog (as Esc does) and is ignored over the board, where Esc
/// would clear the filter. Wheel and drag never reach the panes behind.
fn handle_form_mouse(app: &mut App, m: MouseEvent) {
    if menu::legend_click(app, &m) {
        return;
    }
    match m.kind {
        MouseEventKind::ScrollDown => menu::form_wheel(app, 1),
        MouseEventKind::ScrollUp => menu::form_wheel(app, -1),
        _ => {}
    }
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    let on_board = board_open(app);
    let Some((rect, index, option)) = hit(&app.hits.form_cells, m.column, m.row) else {
        if !on_board && !in_rect(app.hits.form_popup, m.column, m.row)
            && !app.sidebar.query_edit && !app.sidebar.name_edit
        {
            app.sidebar.filter_open = false;
        } else if on_board && !in_rect(app.hits.form_popup, m.column, m.row) && !app.sidebar.query_edit {
            menu::apply_board_filter(app);
        }
        return;
    };
    if on_board {
        app.sidebar.query_edit = false;
        app.board.filter_focus = index;
        if index == 0 {
            app.sidebar.query_edit = true;
        } else {
            toggle_board_focused(app);
        }
    } else {
        if app.sidebar.query_edit || app.sidebar.name_edit {
            return;
        }
        app.sidebar.format_focus = index;
        match option {
            Some(option) => app.select_format_option(option),
            None => app.toggle_format_widget(),
        }
    }
    app.note_click(rect);
}

/// Board mouse path: press opens (double-press), drag only trails the
/// cursor cell like a hover, scroll walks cards. Release never opens:
/// the press already committed, so release is a no-op.
fn handle_board_mouse(app: &mut App, m: MouseEvent) {
    if menu::legend_click(app, &m) {
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Right) => {
            menu::board_right_click(app, m.column, m.row);
            return;
        }
        MouseEventKind::Down(MouseButton::Left)
            if !in_rect(app.hits.popup, m.column, m.row) =>
        {
            app.pop_layer();
            return;
        }
        _ => {}
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Header icons first: they sit outside every card cell.
            if let Some((rect, url)) = hit(&app.hits.board_links, m.column, m.row) {
                app.note_click(rect);
                open_url(&url, app);
                return;
            }
            if let Some((rect, c, r)) = hit(&app.hits.board_cells, m.column, m.row) {
                app.note_click(rect);
                // Clicking the URL line of an expanded card opens it.
                if app.board.expanded == Some((c, r)) && m.row - rect.y == 3 {
                    if let Some(board) = &app.board.data {
                        let cols =
                            crate::app::board_columns(board, &app.board.filter, &app.board.milestones);
                        if let Some(url) = cols
                            .get(c)
                            .and_then(|(_, cards)| cards.get(r))
                            .and_then(|card| card.url())
                        {
                            open_url(&url, app);
                            return;
                        }
                    }
                }
                if app.board.col == c && app.board.row == r {
                    if let Some(n) = board_selected_number(app) {
                        app.board.data = None;
                        app.board.expanded = None;
                        app.open_issue(n);
                    }
                } else {
                    app.board.col = c;
                    app.board.row = r;
                    app.board.expanded = None;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((rect, c, r)) = hit(&app.hits.board_cells, m.column, m.row) {
                app.board.col = c;
                app.board.row = r;
                app.note_click(rect);
            }
        }
        MouseEventKind::ScrollDown => menu::board_wheel(app, m.column, 1),
        MouseEventKind::ScrollUp => menu::board_wheel(app, m.column, -1),
        // Release never opens: the press already committed.
        MouseEventKind::Up(_) => {}
        _ => {}
    }
}
