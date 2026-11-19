//! Mouse paths for the actions menu and the muted controls: right-click
//! menus, title and footer controls, dialog-footer hints, click outside
//! a dialog, and the wheel inside dialogs. Input never draws; it reads
//! the rects `ui` recorded in `app.hits`.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use super::hit::{hit, in_rect};
use super::click::{handle_left, sidebar_hit};
use super::actions;
use crate::app::{App, Focus, MenuKind, MenuState};
use crate::core::Pane;
use crate::keymap::Action;

pub(super) fn open(app: &mut App, kind: MenuKind, anchor: (u16, u16)) {
    let menu = MenuState::new(kind, app, anchor);
    if !menu.entries.is_empty() {
        app.open(crate::app::Entry::Menu(menu));
    }
}

/// `m`: the actions menu of the focused pane, near its top-left corner.
/// Over the board it is the board menu.
pub(super) fn open_for_focus(app: &mut App) {
    let (kind, area) = if crate::app::board_open(app) {
        (MenuKind::Board, app.hits.popup)
    } else {
        match app.nav.pane {
            Pane::Issues => (MenuKind::Issue, app.hits.side_issue_area),
            Pane::PRs => (MenuKind::Issue, app.hits.side_pr_area),
            Pane::Refs => (MenuKind::Ref, app.hits.refs_area),
            Pane::Labels => (MenuKind::Labels, app.hits.labels_area),
            Pane::History => (MenuKind::History, app.hits.history_area),
            Pane::Main if app.visual_graph() => (MenuKind::Map, app.hits.map_area),
            Pane::Main => (MenuKind::Body, app.hits.body_area),
        }
    };
    open(app, kind, (area.x + 2, area.y + 2));
}

/// Enter or a click on the cursor row: close the menu, run its action.
pub(super) fn run(app: &mut App) {
    let Some(menu) = app.take_menu() else { return };
    if let Some(entry) = menu.entries.get(menu.cursor) {
        actions::perform(app, entry.action);
    }
}

pub(super) fn step(app: &mut App, delta: isize) {
    if let Some(menu) = app.menu_mut() {
        let last = menu.entries.len().saturating_sub(1) as isize;
        menu.cursor = (menu.cursor as isize + delta).clamp(0, last) as usize;
    }
}

/// Mouse path while the menu is open. It owns every event: a click on a
/// row runs it, a click outside closes the menu, the wheel moves the cursor.
pub(super) fn handle_menu_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            match hit(&app.hits.menu_rows, m.column, m.row) {
                Some((rect, index)) => {
                    if let Some(menu) = app.menu_mut() {
                        menu.cursor = index;
                    }
                    app.note_click(rect);
                    run(app);
                }
                None if !in_rect(app.hits.popup, m.column, m.row) => { app.close(crate::app::Slot::Menu); }
                None => {}
            }
        }
        MouseEventKind::Down(_) if !in_rect(app.hits.popup, m.column, m.row) => { app.close(crate::app::Slot::Menu); }
        MouseEventKind::ScrollDown => step(app, 1),
        MouseEventKind::ScrollUp => step(app, -1),
        _ => {}
    }
}

/// Flash the clicked hint and run its action. `false` when nothing was hit.
pub(super) fn press(app: &mut App, hint: Option<(Rect, Action)>) -> bool {
    let Some((rect, action)) = hint else { return false };
    app.note_click(rect);
    actions::perform(app, action);
    true
}

/// A press on a dialog-footer hint runs the hint's action.
pub(super) fn legend_click(app: &mut App, m: &MouseEvent) -> bool {
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return false;
    }
    let hint = hit(&app.hits.legend, m.column, m.row);
    press(app, hint)
}

/// Mouse path while the help screen or the comment editor is open. Both
/// keep the panes behind them out of reach. Their footer hints click, the
/// help screen scrolls with the wheel and closes on an outside click.
pub(super) fn handle_modal_mouse(app: &mut App, m: MouseEvent, help: bool) {
    if legend_click(app, &m) {
        return;
    }
    if !help {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => app.compose_click(m.column, m.row),
            MouseEventKind::ScrollDown => app.compose_scroll(3),
            MouseEventKind::ScrollUp => app.compose_scroll(-3),
            _ => {}
        }
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) if !in_rect(app.hits.popup, m.column, m.row) => {
            app.close(crate::app::Slot::Shortcuts);
        }
        MouseEventKind::ScrollDown => app.help_scroll = app.help_scroll.saturating_add(3),
        MouseEventKind::ScrollUp => app.help_scroll = app.help_scroll.saturating_sub(3),
        _ => {}
    }
}

/// A press on a pane-title or footer control: focus its pane, run it.
pub(super) fn control_click(app: &mut App, x: u16, y: u16) -> bool {
    let Some((rect, action, pane)) = hit(&app.hits.controls, x, y) else { return false };
    app.nav.focus = Focus::Main;
    app.nav.pane = pane;
    app.note_click(rect);
    actions::perform(app, action);
    true
}

/// A press on the repo segment of the status bar opens the global menu.
pub(super) fn repo_click(app: &mut App, x: u16, y: u16) -> bool {
    if !in_rect(app.hits.repo, x, y) {
        return false;
    }
    let rect = app.hits.repo;
    app.note_click(rect);
    open(app, MenuKind::Global, (rect.x, rect.y));
    if let Some(menu) = app.menu_mut() {
        menu.anchor.1 = rect.y.saturating_sub(menu.entries.len() as u16 + 2);
    }
    true
}

/// A press on the query source segment of the status bar searches GitHub.
pub(super) fn source_click(app: &mut App, x: u16, y: u16) -> bool {
    if !in_rect(app.hits.source, x, y) {
        return false;
    }
    let rect = app.hits.source;
    app.note_click(rect);
    app.search_github();
    true
}

/// Which pane's title row a press landed on: it focuses that pane.
pub(super) fn title_row_pane(app: &App, x: u16, y: u16) -> Option<Pane> {
    [
        (app.hits.side_issue_area, Pane::Issues),
        (app.hits.side_pr_area, Pane::PRs),
        (app.hits.refs_area, Pane::Refs),
        (app.hits.labels_area, Pane::Labels),
        (app.hits.history_area, Pane::History),
    ]
    .into_iter()
    .find(|(area, _)| area.width > 0 && y == area.y && in_rect(*area, x, y))
    .map(|(_, pane)| pane)
}

/// Right press in the base layer: select what is under the pointer, then
/// open the menu for it. Selecting reuses the left-press path for the
/// sidebar rows; refs, history, labels and the body only take focus.
pub(super) fn right_click(app: &mut App, x: u16, y: u16) {
    if repo_click(app, x, y) {
        return;
    }
    let h = &app.hits;
    let kind = if sidebar_hit(h.side_issue_area, &h.side_issue_rows, &h.side_issue_heights, x, y).is_some()
        || sidebar_hit(h.side_pr_area, &h.side_pr_rows, &h.side_pr_heights, x, y).is_some()
    {
        handle_left(app, x, y, false);
        MenuKind::Issue
    } else if in_rect(h.side_issue_area, x, y) || in_rect(h.side_pr_area, x, y) {
        app.nav.focus = Focus::Main;
        app.nav.pane = if in_rect(h.side_pr_area, x, y) { Pane::PRs } else { Pane::Issues };
        MenuKind::Issue
    } else if in_rect(h.refs_area, x, y) {
        let row = sidebar_hit(h.refs_area, &h.ref_rows, &h.ref_heights, x, y);
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Refs;
        if row.is_some() {
            app.refs.highlight = row;
        }
        MenuKind::Ref
    } else if in_rect(h.history_area, x, y) {
        let row = hit(&h.history_rows, x, y).map(|(_, steps)| steps);
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::History;
        if let Some(steps) = row {
            app.nav.history_cursor = steps;
        }
        MenuKind::History
    } else if in_rect(h.labels_area, x, y) {
        let row = hit(&h.labels_rows, x, y).map(|(_, index)| index);
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Labels;
        if let Some(index) = row {
            app.labels.cursor = index;
        }
        MenuKind::Labels
    } else {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Main;
        MenuKind::Body
    };
    open(app, kind, (x, y));
}

/// Right press over the board: the card under the pointer takes the
/// cursor, then the board menu opens.
pub(super) fn board_right_click(app: &mut App, x: u16, y: u16) {
    if let Some((rect, col, row)) = hit(&app.hits.board_cells, x, y) {
        app.board.col = col;
        app.board.row = row;
        app.board.expanded = None;
        app.note_click(rect);
    }
    open(app, MenuKind::Board, (x, y));
}

/// Wheel over a board column moves that column's card cursor.
pub(super) fn board_wheel(app: &mut App, x: u16, dy: isize) {
    let over = app.hits.board_cells.iter()
        .find(|(rect, _, _)| x >= rect.x && x < rect.right())
        .map(|(_, col, _)| *col);
    if let Some(col) = over.filter(|col| *col != app.board.col) {
        let delta = col as isize - app.board.col as isize;
        crate::app::board_move(app, delta, 0);
    }
    crate::app::board_move(app, 0, dy);
}

/// Wheel over the Format dialog or the board filter form steps its focus.
pub(super) fn form_wheel(app: &mut App, delta: isize) {
    if app.query_edit() || app.name_edit() {
        return;
    }
    if crate::app::board_open(app) {
        app.board_filter_step(delta);
    } else {
        app.format_step(delta);
    }
}

/// Outside press on the board filter form applies it, like Enter does.
pub(super) fn apply_board_filter(app: &mut App) {
    actions::perform(app, Action::BoardApply);
}
