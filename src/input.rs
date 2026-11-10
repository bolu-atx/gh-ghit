//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::{App, Focus, Tab, board_move, board_open, board_selected_number};
use crate::core;

pub(crate) fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    // Board modal captures everything.
    if board_open(app) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.board = None;
                app.board_loading = false;
            }
            KeyCode::Char('j') | KeyCode::Down => board_move(app, 0, 1),
            KeyCode::Char('k') | KeyCode::Up => board_move(app, 0, -1),
            KeyCode::PageDown => board_move(app, 0, 5),
            KeyCode::PageUp => board_move(app, 0, -5),
            KeyCode::Char('h') | KeyCode::Left => board_move(app, -1, 0),
            KeyCode::Char('l') | KeyCode::Right => board_move(app, 1, 0),
            KeyCode::Char('t') => {
                app.notify(format!("theme: {}", crate::theme::cycle()));
            }
            KeyCode::Char(' ') => {
                let pos = (app.board_col, app.board_row);
                app.board_expanded = if app.board_expanded == Some(pos) { None } else { Some(pos) };
            }
            KeyCode::Enter => {
                if let Some(n) = board_selected_number(app) {
                    app.board = None;
                    app.load_issue(n, false);
                }
            }
            _ => {}
        }
        return;
    }

    if app.focus == Focus::Bar {
        match key.code {
            KeyCode::Esc => {
                app.focus = Focus::Main;
                app.bar.clear();
            }
            KeyCode::Enter => {
                let quit = matches!(app.bar.trim(), "q" | "quit" | "exit");
                app.submit_bar();
                if quit {
                    app.quit = true;
                }
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && (c == 'c' || c == 'd') {
                    app.quit = true;
                    return;
                }
                app.bar.push(c);
            }
            KeyCode::Backspace => {
                app.bar.pop();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('1') => app.tab = Tab::Body,
        KeyCode::Char('2') => app.tab = Tab::Comments,
        KeyCode::Char('t') => {
            app.notify(format!("theme: {}", crate::theme::cycle()));
        }
        KeyCode::Char('b') => app.open_board(),
        KeyCode::Char('j') => move_selection(app, 1),
        KeyCode::Char('k') => move_selection(app, -1),
        KeyCode::Down => {
            app.scroll = app.scroll.saturating_add(3);
        }
        KeyCode::Up => {
            app.scroll = app.scroll.saturating_sub(3);
        }
        KeyCode::PageDown => {
            app.scroll = app.scroll.saturating_add(12);
        }
        KeyCode::PageUp => {
            app.scroll = app.scroll.saturating_sub(12);
        }
        KeyCode::Char(':') | KeyCode::Char('/') => app.focus = Focus::Bar,
        KeyCode::Char('o') => {
            if let Some(n) = app.current {
                app.open_number(n);
            } else {
                app.notify("view an issue first".to_string());
            }
        }
        KeyCode::Char('r') => app.refresh_current(),
        KeyCode::Char('p') => app.toggle_pin(),
        KeyCode::Char('w') => {
            app.merged_window_hours = core::cycle_window(app.merged_window_hours);
            app.merge_pr_sections();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Esc => app.bar.clear(),
        KeyCode::Tab => {
            app.tab = if app.tab == Tab::Body { Tab::Comments } else { Tab::Body };
        }
        _ => {}
    }
}

/// j/k walks issues then PRs as one flat list, crossing the section boundary.
fn move_selection(app: &mut App, delta: isize) {
    let order: Vec<u64> = app.issues.iter().chain(app.prs.iter()).copied().collect();
    if let Some(n) = core::next_in_order(&order, app.current, delta) {
        app.show_issue(n);
    }
}

fn in_rect(area: Rect, x: u16, y: u16) -> bool {
    area.contains(ratatui::layout::Position::new(x, y))
}

/// Sidebar rows are 3 lines each below a 1-line border plus a 1-line spacer.
fn sidebar_hit(area: Rect, list: &[u64], x: u16, y: u16) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    list.get(((y - area.y - 2) / 3) as usize).copied()
}

pub(crate) fn handle_mouse(app: &mut App, m: MouseEvent) {
    if board_open(app) {
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = app
                    .board_cells
                    .iter()
                    .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                    .map(|(rect, c, r)| (*rect, *c, *r));
                if let Some((rect, c, r)) = hit {
                    // Clicking the URL line of an expanded card opens it.
                    if app.board_expanded == Some((c, r)) && m.row - rect.y == 3 {
                        if let Some(board) = &app.board {
                            let cols = crate::model::board_columns(&board.items, &board.order);
                            if let Some(url) = cols
                                .get(c)
                                .and_then(|(_, cards)| cards.get(r))
                                .and_then(|card| crate::ui::card_url(card))
                            {
                                open_url(&url, app);
                                return;
                            }
                        }
                    }
                    if app.board_col == c && app.board_row == r {
                        if let Some(n) = board_selected_number(app) {
                            app.board = None;
                            app.board_expanded = None;
                            app.load_issue(n, false);
                        }
                    } else {
                        app.board_col = c;
                        app.board_row = r;
                        app.board_expanded = None;
                    }
                }
            }
            MouseEventKind::ScrollDown => board_move(app, 0, 1),
            MouseEventKind::ScrollUp => board_move(app, 0, -1),
            _ => {}
        }
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if in_rect(app.bar_area, m.column, m.row) {
                app.focus = Focus::Bar;
                return;
            }
            if let Some(n) = sidebar_hit(app.side_issue_area, &app.issues, m.column, m.row)
                .or_else(|| sidebar_hit(app.side_pr_area, &app.prs, m.column, m.row))
            {
                app.focus = Focus::Main;
                app.show_issue(n);
                return;
            }
            if in_rect(app.tabs_area, m.column, m.row) {
                // "Body" is 4 wide, then the divider: left half body, right half comments.
                app.tab = if m.column.saturating_sub(app.tabs_area.x) < 6 {
                    Tab::Body
                } else {
                    Tab::Comments
                };
                return;
            }
            if in_rect(app.body_area, m.column, m.row) {
                app.focus = Focus::Main;
                let row = (m.row - app.body_area.y) as usize;
                if let Some(line) = app.body_rows.get(row) {
                    let repo = app.repo_key();
                    let rel_x = (m.column - app.body_area.x) as usize;
                    let links = crate::model::links_in_line(line, repo.as_deref());
                    match crate::model::nearest_link(line, &links, rel_x) {
                        Some(crate::model::LinkTarget::Issue(n)) => app.load_issue(n, false),
                        Some(crate::model::LinkTarget::Url(url)) => open_url(&url, app),
                        None => {}
                    }
                }
                return;
            }
            app.focus = Focus::Main;
        }
        MouseEventKind::ScrollDown => {
            app.scroll = app.scroll.saturating_add(3);
        }
        MouseEventKind::ScrollUp => {
            app.scroll = app.scroll.saturating_sub(3);
        }
        _ => {}
    }
}

/// Open a URL in the browser. Stubbed in tests so suites never launch one.
#[cfg(not(test))]
fn open_url(url: &str, app: &mut App) {
    if open::that(url).is_ok() {
        app.notify(format!("opened {url}"));
    } else {
        app.notify(format!("couldn't open {url}"));
    }
}

#[cfg(test)]
fn open_url(url: &str, app: &mut App) {
    app.notify(format!("opened {url}"));
}
