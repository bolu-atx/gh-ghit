//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::{App, Focus, Pane, Tab, board_move, board_open, board_selected_number};
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
                    app.ensure_visited(n);
                    app.load_issue(n, false);
                }
            }
            _ => {}
        }
        return;
    }

    // Filter dialog captures everything except the command bar path below.
    if app.filter_open {
        handle_filter_key(app, key);
        return;
    }

    if app.focus == Focus::Bar {
        match key.code {
            KeyCode::Esc => {
                app.focus = Focus::Main;
                app.searching = false;
                app.bar.clear();
                app.search.clear();
            }
            KeyCode::Enter => {
                if app.searching {
                    app.commit_search();
                    return;
                }
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
                if app.searching {
                    app.search.push(c);
                } else {
                    app.bar.push(c);
                }
            }
            KeyCode::Backspace => {
                if app.searching {
                    app.search.pop();
                } else {
                    app.bar.pop();
                }
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('1') => app.set_tab(Tab::Body),
        KeyCode::Char('2') => app.set_tab(Tab::Comments),
        KeyCode::Char('3') => app.set_tab(Tab::Diff),
        KeyCode::Char('t') => {
            app.notify(format!("theme: {}", crate::theme::cycle()));
        }
        KeyCode::Char('b') => app.open_board(),
        KeyCode::Char('f') => {
            app.filter_open = true;
            app.query_edit = false;
        }
        KeyCode::Char('j') => pane_down(app, 1),
        KeyCode::Char('k') => pane_down(app, -1),
        KeyCode::Down => {
            if app.tab == Tab::Diff {
                move_diff_cursor(app, 1);
            } else {
                app.scroll = app.scroll.saturating_add(3);
            }
        }
        KeyCode::Up => {
            if app.tab == Tab::Diff {
                move_diff_cursor(app, -1);
            } else {
                app.scroll = app.scroll.saturating_sub(3);
            }
        }
        KeyCode::PageDown => {
            app.scroll = app.scroll.saturating_add(12);
        }
        KeyCode::PageUp => {
            app.scroll = app.scroll.saturating_sub(12);
        }
        KeyCode::Char(':') => {
            app.searching = false;
            app.focus = Focus::Bar;
        }
        KeyCode::Char('/') => {
            app.searching = true;
            app.search.clear();
            app.focus = Focus::Bar;
        }
        KeyCode::Char('n') => jump_search_hit(app, 1),
        KeyCode::Char('N') => jump_search_hit(app, -1),
        KeyCode::Char('o') => {
            if let Some(n) = app.current {
                app.open_number(n);
            } else {
                app.notify("view an issue first".to_string());
            }
        }
        KeyCode::Char('r') => app.refresh_current(),
        KeyCode::Char('p') => app.toggle_pin(),
        KeyCode::Char('e') => app.toggle_refs(),
        KeyCode::Char('s') if app.tab == Tab::Diff => app.toggle_diff_split(),
        KeyCode::Char(' ') if app.tab == Tab::Diff => app.toggle_diff_file(),
        KeyCode::Char('w') => {
            app.merged_window_hours = core::cycle_window(app.merged_window_hours);
            app.refetch_recent();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Esc => {
            app.bar.clear();
            app.searching = false;
            app.search.clear();
            app.ref_highlight = None;
        }
        KeyCode::Tab => {
            app.cycle_pane();
        }
        _ => {}
    }
}

/// j/k walks the issues pane then the PRs pane as one flat list.
fn move_selection(app: &mut App, delta: isize) {
    let order = app.flat_numbers();
    if let Some(n) = core::next_in_order(&order, app.current, delta) {
        app.show_issue(n);
    }
}

/// j/k act on the focused side pane: the sidebar walks selection and refs
/// moves the source highlight. The body scrolls instead; the Diff tab moves
/// its file cursor with the arrow keys.
fn pane_down(app: &mut App, delta: isize) {
    if app.pane == Pane::Refs {
        let next = core::next_in_order(&app.ref_numbers, app.ref_highlight, delta);
        if let Some(n) = next.filter(|n| Some(*n) != app.ref_highlight) {
            app.select_ref(n);
        }
        return;
    }
    match app.pane {
        Pane::Left => move_selection(app, delta),
        _ => {
            app.scroll = app.scroll.saturating_add_signed(delta as i16 * 3);
        }
    }
}

/// Up/Down in the Diff tab walks folded files and scrolls the cursor into view.
fn move_diff_cursor(app: &mut App, delta: isize) {
    let count = app
        .current
        .and_then(|n| app.diffs.get(&n))
        .map(|files| files.len().min(crate::ui::MAX_DIFF_FILES))
        .unwrap_or(0);
    if count == 0 {
        return;
    }
    let next = (app.diff_cursor as isize + delta).clamp(0, count as isize - 1) as usize;
    app.diff_cursor = next;
    // `diff_headers` is the visible slice taken at `body_base`.
    if let Some(row) = app.diff_headers.get(next) {
        app.scroll = app.body_base.saturating_add(*row as u16);
    }
}

/// n/N jumps between `/` search hits, wrapping at the ends.
fn jump_search_hit(app: &mut App, delta: isize) {
    let hits = app.search_hits();
    if hits.is_empty() {
        app.notify("no search: press /".to_string());
        return;
    }
    if let Some(n) = core::next_in_order(&hits, app.current, delta) {
        app.show_issue(n);
    }
}

/// Filter dialog keys: window presets refetch the window listing, toggles
/// only hide recent rows here, `e` edits the GitHub query.
fn handle_filter_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crate::app::QueryAction;
    if app.query_edit {
        match key.code {
            KeyCode::Esc => {
                // Revert the draft; the dialog stays open.
                app.query_draft = app.query_active.clone();
                app.query_edit = false;
            }
            KeyCode::Enter => match app.commit_query_draft() {
                QueryAction::Run(q) => app.run_gh_search(q),
                QueryAction::Refetch => {
                    app.refetch_recent();
                    app.notify("back to recent".to_string());
                }
            },
            KeyCode::Char(c) => app.query_draft.push(c),
            KeyCode::Backspace => {
                app.query_draft.pop();
            }
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Char('f') | KeyCode::Esc | KeyCode::Char('q') => {
            app.filter_open = false;
        }
        KeyCode::Char('e') => app.start_query_edit(),
        KeyCode::Char('1') => {
            app.merged_window_hours = 1;
            app.refetch_recent();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Char('2') => {
            app.merged_window_hours = 24;
            app.refetch_recent();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Char('3') => {
            app.merged_window_hours = 168;
            app.refetch_recent();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Char('w') => {
            app.merged_window_hours = core::cycle_window(app.merged_window_hours);
            app.refetch_recent();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        KeyCode::Char('o') => app.show_open_prs = !app.show_open_prs,
        KeyCode::Char('m') => app.show_merged_prs = !app.show_merged_prs,
        KeyCode::Char('i') => app.show_open_issues = !app.show_open_issues,
        KeyCode::Char('c') => app.show_closed_issues = !app.show_closed_issues,
        _ => {}
    }
}

fn in_rect(area: Rect, x: u16, y: u16) -> bool {
    area.contains(ratatui::layout::Position::new(x, y))
}

/// Sidebar rows are 3 lines each (`Some`) with 1-line dividers (`None`),
/// below a 1-line border plus a 1-line spacer. Dividers never hit.
fn sidebar_hit(area: Rect, rows: &[Option<u64>], x: u16, y: u16) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    let mut rel = (y - area.y - 2) as usize;
    for row in rows {
        let h = if row.is_some() { 3 } else { 1 };
        if rel < h {
            return *row;
        }
        rel -= h;
    }
    None
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
                            app.ensure_visited(n);
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
            let issue_rows = app.side_issue_rows.clone();
            let pr_rows = app.side_pr_rows.clone();
            if let Some(n) = sidebar_hit(app.side_issue_area, &issue_rows, m.column, m.row)
                .or_else(|| sidebar_hit(app.side_pr_area, &pr_rows, m.column, m.row))
            {
                app.focus = Focus::Main;
                app.pane = Pane::Left;
                app.ensure_visited(n);
                app.show_issue(n);
                return;
            }
            // Refs rows never navigate: selecting one highlights its source
            // line in the body instead.
            let ref_rows = app.ref_rows.clone();
            if let Some(n) = sidebar_hit(app.refs_area, &ref_rows, m.column, m.row) {
                app.focus = Focus::Main;
                app.pane = Pane::Refs;
                app.select_ref(n);
                return;
            }
            if in_rect(app.tabs_area, m.column, m.row) {
                // Tabs are icon + "Body  Comments  Diff": Body ends at 7,
                // Comments ends at 20, the rest is Diff.
                app.pane = Pane::Main;
                let rel = m.column.saturating_sub(app.tabs_area.x);
                app.set_tab(if rel < 9 {
                    Tab::Body
                } else if rel < 22 {
                    Tab::Comments
                } else {
                    Tab::Diff
                });
                return;
            }
            if in_rect(app.body_area, m.column, m.row) {
                app.focus = Focus::Main;
                app.pane = Pane::Main;
                let row = (m.row - app.body_area.y) as usize;
                if app.tab == Tab::Diff {
                    if let Some(fi) = app.diff_headers.iter().position(|&r| r == row) {
                        app.diff_cursor = fi;
                        app.toggle_diff_file();
                        return;
                    }
                }
                if let Some(line) = app.body_rows.get(row) {
                    let repo = app.repo_key();
                    let rel_x = (m.column - app.body_area.x) as usize;
                    let links = crate::model::links_in_line(line, repo.as_deref());
                    match crate::model::nearest_link(line, &links, rel_x) {
                        Some(crate::model::LinkTarget::Issue(n)) => {
                            app.ensure_visited(n);
                            app.load_issue(n, false);
                        }
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
