//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::{App, Focus, Pane, Tab, board_move, board_open, board_selected_number};
use crate::core;

pub(crate) fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    if app.shortcuts_open {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            app.shortcuts_open = false;
        }
        return;
    }

    // Compose modal captures everything: typing edits the draft,
    // Ctrl+S sends, Esc discards.
    if app.compose_open() {
        handle_compose_key(app, key);
        return;
    }

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
        // History walks. Ctrl+I arrives as Tab on terminals without the
        // kitty protocol, so both Ctrl forms redo; plain Tab (no Ctrl)
        // keeps cycling panes below.
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => app.go_back(),
        KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::CONTROL) => app.go_forward(),
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::CONTROL) => app.go_forward(),
        KeyCode::Char('?') => app.shortcuts_open = true,
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('1') => app.set_tab(Tab::Details),
        KeyCode::Char('2') => app.set_tab(Tab::Visual),
        KeyCode::Char('t') => {
            app.notify(format!("theme: {}", crate::theme::cycle()));
        }
        KeyCode::Char('b') => app.open_board(),
        KeyCode::Char('c') => app.open_compose(),
        KeyCode::Char('f') => {
            app.filter_open = true;
            app.query_edit = false;
        }
        KeyCode::Char('j') => {
            if graph_focused(app) {
                app.graph_move(1);
            } else {
                pane_down(app, 1);
            }
        }
        KeyCode::Char('k') => {
            if graph_focused(app) {
                app.graph_move(-1);
            } else {
                pane_down(app, -1);
            }
        }
        KeyCode::Char('h') => {
            if graph_focused(app) {
                app.graph_move(-1);
            }
        }
        KeyCode::Char('l') => {
            if graph_focused(app) {
                app.graph_move(1);
            }
        }
        KeyCode::Enter => {
            if graph_focused(app) {
                app.open_graph_selection();
            } else if app.pane == Pane::Refs {
                app.open_selected_ref();
            }
        }
        KeyCode::Down => {
            if graph_focused(app) {
                app.graph_move(1);
            } else if app.visual_diff() {
                move_diff_cursor(app, 1);
            } else {
                app.scroll = app.scroll.saturating_add(3);
            }
        }
        KeyCode::Up => {
            if graph_focused(app) {
                app.graph_move(-1);
            } else if app.visual_diff() {
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
        KeyCode::Char('y') => app.yank(),
        KeyCode::Char('e') => app.toggle_refs(),
        // Sidebar-only: a cycles the row sort, A flips its direction, and
        // x evicts the highlighted row. Diff keeps s for split.
        KeyCode::Char('a') if matches!(app.pane, Pane::Issues | Pane::PRs) => app.cycle_sort(),
        KeyCode::Char('A') if matches!(app.pane, Pane::Issues | Pane::PRs) => app.toggle_sort_dir(),
        KeyCode::Char('x') if matches!(app.pane, Pane::Issues | Pane::PRs) => app.evict_current(),
        KeyCode::Char('s') if app.visual_diff() => app.toggle_diff_split(),
        KeyCode::Char(' ') if app.visual_diff() => app.toggle_diff_file(),

        KeyCode::Char(' ') => app.toggle_row_expansion(),
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

/// Compose modal keys: printable chars edit the draft, arrows move the
/// cursor, Ctrl+S submits, Esc discards. Ctrl+C never quits here — a
/// lost draft would be worse than an ignored key.
pub(crate) fn handle_compose_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('s') => app.submit_compose(),
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Esc => app.discard_compose(),
        KeyCode::Enter => app.compose_insert('\n'),
        KeyCode::Backspace => app.compose_backspace(),
        KeyCode::Left => app.compose_step(-1),
        KeyCode::Right => app.compose_step(1),
        KeyCode::Up => app.compose_line(-1),
        KeyCode::Down => app.compose_line(1),
        KeyCode::Char(c) => app.compose_insert(c),
        KeyCode::Tab => app.compose_insert('\t'),
        _ => {}
    }
}

/// Graph keys act only while the Graph tab has main focus: arrows and
/// hjkl move between nodes, Enter loads the selected issue. Anywhere
/// else those keys keep their existing meaning.
pub(crate) fn graph_focused(app: &App) -> bool {
    app.visual_graph() && app.pane == Pane::Main && app.focus == Focus::Main
}

/// j/k walks one sidebar list: issues or PRs, never crossing over.
/// Starts from the wheel hover when one names a row here, so keyboard
/// and wheel continue from the same place; the visit clears the hover.
fn move_in_list(app: &mut App, order: Vec<u64>, delta: isize) {
    let base = app.hover.filter(|n| order.contains(n)).or(app.current);
    if let Some(n) = core::next_in_order(&order, base, delta) {
        app.show_issue(n);
    }
}

/// j/k act on the focused side pane: each sidebar list walks only itself
/// and refs moves the source highlight. The body scrolls instead; the Diff
/// tab moves its line cursor with the arrow keys.
fn pane_down(app: &mut App, delta: isize) {
    match app.pane {
        Pane::Issues => move_in_list(app, app.issue_numbers(), delta),
        Pane::PRs => move_in_list(app, app.pr_numbers(), delta),
        Pane::Refs => {
            let next = core::next_in_order(&app.ref_order(), app.ref_highlight, delta);
            if let Some(n) = next.filter(|n| Some(*n) != app.ref_highlight) {
                app.select_ref(n);
            }
        }
        Pane::Main => {
            app.scroll = app.scroll.saturating_add_signed(delta as i16 * 3);
        }
    }
}

/// Up/Down in the Diff tab steps the active line one body row at a time so
/// wrapped rows just work. The cursor never leaves the viewport: at the
/// edge the view scrolls with it. Render resolves the owning file and
/// auto-expands a landed-on collapsed file from the fresh capture, so this
/// only moves the cursor and flags it touched.
fn move_diff_cursor(app: &mut App, delta: isize) {
    if !app.visual_diff() {
        return;
    }
    let height = app.body_area.height as usize;
    if height == 0 {
        return;
    }
    let base = app.body_base as usize;
    let scroll = app.scroll as usize;
    let mut abs = match app.diff_line {
        Some(a) => a.clamp(scroll, scroll + height.saturating_sub(1)),
        None => {
            // Anchor to the first header when no cursor yet; render fills
            // in the file without expanding it.
            if let Some(&h) = app.diff_headers.first() {
                base + h
            } else {
                return;
            }
        }
    };
    if delta > 0 {
        let vis = abs.saturating_sub(base);
        let rows = &app.body_rows;
        if vis + 1 < rows.len() {
            // A blank target with only blanks below is past the content.
            let target_blank = rows[vis + 1].trim().is_empty();
            let rest_blank = rows.get(vis + 2..).map(|r| r.iter().all(|l| l.trim().is_empty())).unwrap_or(true);
            if target_blank && rest_blank {
                return;
            }
            abs += 1;
        } else {
            app.scroll = app.scroll.saturating_add(1);
            abs += 1;
        }
    } else if delta < 0 {
        if abs > scroll {
            abs -= 1;
        } else if scroll > 0 {
            app.scroll -= 1;
            abs -= 1;
        } else {
            return;
        }
    } else {
        return;
    }
    app.diff_line = Some(abs);
    app.diff_touched = true;
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

/// Sidebar rows are 3 lines each (`Item`) with 1-line labeled dividers,
/// below a 1-line border plus a 1-line spacer. Dividers never hit. An
/// expanded row adds the wrapped detail block its render draws, measured
/// with the same inner width.
fn sidebar_hit(area: Rect, rows: &[crate::model::SideRow], app: &App, x: u16, y: u16) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    let inner_width = (area.width as usize).saturating_sub(2);
    let mut rel = (y - area.y - 2) as usize;
    for row in rows {
        let h = match row.number() {
            Some(n) => {
                let detail = match app.data.get(&n) {
                    Some(d) if app.expanded_rows.contains(&n) => {
                        crate::ui::expanded_row_height(d, inner_width)
                    }
                    _ => 0,
                };
                3 + detail
            }
            None => 1,
        };
        if rel < h {
            return row.number();
        }
        rel -= h;
    }
    None
}

/// Where a wheel event lands: the Issues/PRs side panes, the right
/// Refs pane while visible, or the main body everywhere else. A hidden
/// Refs pane falls through to Main.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WheelTarget {
    Issues,
    PRs,
    Refs,
    Main,
}

pub(crate) fn wheel_target(app: &App, x: u16, y: u16) -> WheelTarget {
    if in_rect(app.side_issue_area, x, y) {
        WheelTarget::Issues
    } else if in_rect(app.side_pr_area, x, y) {
        WheelTarget::PRs
    } else if app.refs_visible() && in_rect(app.refs_area, x, y) {
        WheelTarget::Refs
    } else {
        WheelTarget::Main
    }
}

/// Wheel steps a highlight-only hover through one sidebar order. Unlike
/// j/k this never visits: `current`, body scroll, and the other panes
/// stay exactly where they were. Landing back on `current` clears the
/// hover since there is nothing left to distinguish.
fn wheel_hover(app: &mut App, order: Vec<u64>, delta: isize) {
    let base = app.hover.filter(|n| order.contains(n)).or(app.current);
    if let Some(n) = core::next_in_order(&order, base, delta) {
        app.hover = if Some(n) == app.current { None } else { Some(n) };
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
            let next = core::next_in_order(&app.ref_order(), app.ref_highlight, delta);
            if let Some(n) = next.filter(|n| Some(*n) != app.ref_highlight) {
                app.ref_highlight = Some(n);
            }
        }
        WheelTarget::Main => {
            app.scroll = app.scroll.saturating_add_signed(delta as i16 * 3);
        }
    }
}

pub(crate) fn handle_mouse(app: &mut App, m: MouseEvent) {
    if app.shortcuts_open { return; }
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
            if let Some(n) = sidebar_hit(app.side_issue_area, &issue_rows, app, m.column, m.row) {
                if app.focus == Focus::Main && app.pane == Pane::Issues
                    && app.current == Some(n) && app.effective(&issue_rows) == Some(n)
                {
                    app.ensure_visited(n);
                }
                app.focus = Focus::Main;
                app.pane = Pane::Issues;
                app.show_issue(n);
                return;
            }
            if let Some(n) = sidebar_hit(app.side_pr_area, &pr_rows, app, m.column, m.row) {
                if app.focus == Focus::Main && app.pane == Pane::PRs
                    && app.current == Some(n) && app.effective(&pr_rows) == Some(n)
                {
                    app.ensure_visited(n);
                }
                app.focus = Focus::Main;
                app.pane = Pane::PRs;
                app.show_issue(n);
                return;
            }
            // Mention rows only highlight their source line; paired rows
            // open their counterpart like sidebar rows.
            let ref_rows = app.ref_rows.clone();
            if let Some(n) = sidebar_hit(app.refs_area, &ref_rows, app, m.column, m.row) {
                app.focus = Focus::Main;
                app.pane = Pane::Refs;
                if app.ref_paired.contains(&n) {
                    app.ensure_visited(n);
                    app.show_issue(n);
                } else {
                    app.select_ref(n);
                }
                return;
            }
            if in_rect(app.tabs_area, m.column, m.row) {
                app.pane = Pane::Main;
                let rel = m.column.saturating_sub(app.tabs_area.x);
                app.set_tab(if rel < 12 { Tab::Details } else { Tab::Visual });
                return;
            }
            // The meta/label block above the tabs resolves refs and URLs
            // exactly like body lines; only the body folds diff files.
            let in_body = in_rect(app.body_area, m.column, m.row);
            let in_header = !in_body && in_rect(app.header_area, m.column, m.row);
            if in_body || in_header {
                app.focus = Focus::Main;
                app.pane = Pane::Main;
                let area = if in_body { app.body_area } else { app.header_area };
                let row = (m.row - area.y) as usize;
                if in_body && app.visual_diff() {
                    // Clicking parks the active line there; render resolves
                    // the file and expands a collapsed header. Clicking an
                    // expanded header folds it back instead. Other rows fall
                    // through to link resolution below so refs still open.
                    let abs = app.body_base as usize + row;
                    app.diff_line = Some(abs);
                    app.diff_touched = true;
                    let is_header = app.diff_headers.iter().any(|&r| r == row);
                    if is_header {
                        if let Some(f) = app.diff_row_files.get(row).copied().flatten() {
                            app.diff_cursor = f;
                            let expanded = app.current.and_then(|n| {
                                app.diffs.get(&n).and_then(|fs| {
                                    fs.get(f).map(|file| {
                                        app.expanded_files.contains(&(n, file.filename.clone()))
                                    })
                                })
                            });
                            if expanded == Some(true) {
                                app.diff_touched = false;
                                app.toggle_diff_file();
                                return;
                            }
                        }
                        return;
                    }
                }
                let rows = if in_body { app.body_rows.clone() } else { app.header_rows.clone() };
                if let Some(line) = rows.get(row) {
                    let repo = app.repo_key();
                    let rel_x = (m.column - area.x) as usize;
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
            wheel_move(app, wheel_target(app, m.column, m.row), 1);
        }
        MouseEventKind::ScrollUp => {
            wheel_move(app, wheel_target(app, m.column, m.row), -1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Author, IssueData, SideRow};

    fn sample(number: u64) -> IssueData {
        IssueData {
            number,
            title: "Fix the thing".to_string(),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![],
            assignees: vec![],
            created_at: "2026-01-02T00:00:00Z".to_string(),
            updated_at: "2026-01-05T00:00:00Z".to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            body: String::new(),
            comments: vec![],
            review_threads: vec![],
            paired: vec![],
            checks: None,
            is_pr: false,
        }
    }

    #[test]
    fn expanded_row_detail_shifts_hits_below() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7, 9] {
            app.data.insert(n, sample(n));
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        let rows = app.issue_rows();
        assert_eq!(rows, vec![SideRow::Item(7), SideRow::Item(9)]);

        let area = Rect::new(0, 0, 40, 40);
        let inner_width = (area.width as usize) - 2;
        let detail = crate::ui::expanded_row_height(app.data.get(&7).unwrap(), inner_width);
        assert!(detail > 3, "detail block is tall enough to span a row: {detail}");

        // Collapsed geometry still applies: row 7 hits its own summary.
        assert_eq!(sidebar_hit(area, &rows, &app, 5, area.y + 2), Some(7));

        app.expanded_rows.insert(7);
        // Every line of the detail block belongs to the expanded row.
        let detail_top = area.y + 4;
        for y in detail_top..detail_top + detail as u16 {
            assert_eq!(sidebar_hit(area, &rows, &app, 5, y), Some(7), "detail line {y}");
        }
        // Row 9 starts only after the detail block plus the item's blank tail.
        let row9_top = area.y + 2 + 3 + detail as u16;
        for y in row9_top..row9_top + 3 {
            assert_eq!(sidebar_hit(area, &rows, &app, 5, y), Some(9), "row 9 line {y}");
        }
    }

    fn wheel_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut pr1 = sample(9);
        pr1.is_pr = true;
        let mut pr2 = sample(11);
        pr2.is_pr = true;
        for d in [sample(7), sample(8), pr1, pr2] {
            let n = d.number;
            app.data.insert(n, d);
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.side_issue_area = Rect::new(0, 0, 30, 20);
        app.side_pr_area = Rect::new(0, 20, 30, 20);
        app.refs_area = Rect::new(90, 0, 30, 40);
        app.body_area = Rect::new(30, 0, 60, 40);
        app.show_issue(7);
        // `show_issue` recomputes refs from the body, so seed the pane after.
        app.ref_numbers = vec![21, 22];
        app.pane = crate::app::Pane::Main;
        app.scroll = 0;
        app
    }

    fn wheel(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::empty() }
    }

    #[test]
    fn sidebar_click_focuses_recent_before_opening() {
        for (is_pr, pane) in [(false, Pane::Issues), (true, Pane::PRs)] {
            let mut app = App::new(Some("o/r".to_string()));
            app.repo_slug = Some("o/r".to_string());
            let mut recent = sample(9);
            recent.is_pr = is_pr;
            app.data.insert(7, sample(7));
            app.data.insert(9, recent);
            app.statuses.insert(7, vec![]);
            app.statuses.insert(9, vec![]);
            app.ensure_visited(7);
            app.ensure_recent(9);
            app.show_issue(7);
            let area = Rect::new(0, 0, 30, 20);
            let rows = if is_pr { app.pr_rows() } else { app.issue_rows() };
            let row_index = rows.iter().position(|row| row.number() == Some(9)).unwrap();
            let y = area.y + 2 + rows[..row_index].iter()
                .map(|row| if row.number().is_some() { 3 } else { 1 }).sum::<u16>();
            if is_pr {
                app.side_pr_area = area;
                app.side_pr_rows = rows.clone();
            } else {
                app.side_issue_area = area;
                app.side_issue_rows = rows.clone();
            }
            let click = wheel(MouseEventKind::Down(MouseButton::Left), 5, y);

            handle_mouse(&mut app, click);
            assert_eq!(app.pane, pane);
            assert_eq!(app.current, Some(9));
            assert!(!app.visited.contains(&9));
            assert_eq!(if is_pr { app.pr_rows() } else { app.issue_rows() }, rows);

            handle_mouse(&mut app, click);
            assert!(app.visited.contains(&9));
            assert_eq!(app.current, Some(9));
            assert_eq!(app.side_counts(is_pr), (0, 1 + usize::from(!is_pr), 0));
        }
    }

    #[test]
    fn wheel_router_maps_each_area() {
        let app = wheel_app();
        assert_eq!(wheel_target(&app, 5, 5), WheelTarget::Issues);
        assert_eq!(wheel_target(&app, 5, 25), WheelTarget::PRs);
        assert_eq!(wheel_target(&app, 95, 5), WheelTarget::Refs);
        assert_eq!(wheel_target(&app, 40, 5), WheelTarget::Main);
    }

    #[test]
    fn wheel_router_hidden_refs_falls_through_to_main() {
        let mut app = wheel_app();
        app.show_refs = false;
        assert!(!app.refs_visible());
        assert_eq!(wheel_target(&app, 95, 5), WheelTarget::Main);
    }

    #[test]
    fn wheel_over_issues_hovers_without_visiting() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(app.hover, Some(8));
        assert_eq!(app.current, Some(7), "wheel never visits");
        assert_eq!(app.pane, crate::app::Pane::Main);
        assert_eq!(app.scroll, 0, "body scroll untouched");
        // Wheeling back onto the current row dissolves the hover.
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(app.hover, None);
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn wheel_over_prs_hovers_pr_rows_only() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
        assert_eq!(app.hover, Some(9));
        assert_eq!(app.current, Some(7), "main content untouched");
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
        assert_eq!(app.hover, Some(11));
        assert_eq!(app.current, Some(7));
        assert_eq!(app.pane, crate::app::Pane::Main);
        assert_eq!(app.scroll, 0);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 25));
        assert_eq!(app.hover, Some(9));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn wheel_over_refs_moves_highlight_without_scrolling_body() {
        let mut app = wheel_app();
        // A mentioning line below the viewport: the old click-style jump
        // would move body scroll here; the wheel must not.
        app.body_rows = vec!["unrelated".to_string(), "context".to_string(), "fixes #22".to_string()];
        app.body_base = 5;
        app.scroll = 9;
        assert_eq!(app.ref_highlight, None);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
        assert_eq!(app.ref_highlight, Some(21));
        assert_eq!(app.scroll, 9, "body stays where it was");
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
        assert_eq!(app.ref_highlight, Some(22));
        assert_eq!(app.scroll, 9, "mention jump is click-only");
        assert_eq!(app.pane, crate::app::Pane::Main);
        assert_eq!(app.current, Some(7));
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 95, 5));
        assert_eq!(app.ref_highlight, Some(21));
        assert_eq!(app.scroll, 9);
    }

    #[test]
    fn jk_promotes_hover_to_current() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(app.hover, Some(8));
        app.pane = crate::app::Pane::Issues;
        handle_key(&mut app, crossterm::event::KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()));
        assert_eq!(app.current, Some(7), "k walks back from the hover");
        assert_eq!(app.hover, None, "visiting clears the hover");
    }

    #[test]
    fn wheel_over_body_scrolls_and_keeps_issue() {
        let mut app = wheel_app();
        app.hover = Some(8);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.current, Some(7));
        assert_eq!(app.hover, Some(8), "body wheel leaves the hover alone");
        assert_eq!(app.pane, crate::app::Pane::Main);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 40, 5));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn wheel_over_body_on_diff_tab_leaves_diff_line_alone() {
        let mut app = wheel_app();
        // Only PRs show a diff; issues show the graph tab instead.
        app.show_issue(9);
        app.tab = crate::app::Tab::Visual;
        assert!(app.visual_diff());
        app.diff_line = Some(5);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.diff_line, Some(5));
    }

    fn ctrl_key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_o_and_ctrl_i_walk_history() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7, 9] {
            app.data.insert(n, sample(n));
            app.statuses.insert(n, vec![]);
        }
        app.ensure_visited(7);
        app.load_issue(7, false);
        app.ensure_visited(9);
        app.load_issue(9, false);
        assert_eq!(app.current, Some(9));

        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert_eq!(app.current, Some(7));

        // Ctrl+I arrives as Tab on terminals without the kitty protocol.
        handle_key(&mut app, ctrl_key(KeyCode::Tab));
        assert_eq!(app.current, Some(9));

        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
        assert_eq!(app.current, Some(9));

        // Plain Tab (no Ctrl) still cycles panes instead of redoing.
        app.pane = crate::app::Pane::Issues;
        handle_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
        );
        assert_eq!(app.pane, crate::app::Pane::PRs);
        assert_eq!(app.current, Some(9));
    }

    fn plain_key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
    }

    fn compose_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.data.insert(7, sample(7));
        app.statuses.insert(7, vec![]);
        app.visited.push(7);
        app.show_issue(7);
        app
    }

    #[test]
    fn c_opens_compose_and_esc_discards() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        assert!(app.compose_open());
        handle_key(&mut app, plain_key(KeyCode::Char('x')));
        assert_eq!(app.compose.as_ref().unwrap().body, "x");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.compose_open());
    }

    #[test]
    fn compose_ctrl_s_with_empty_draft_stays_open() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('s')));
        assert!(app.compose_open(), "empty draft never sends");
        assert!(!app.compose.as_ref().unwrap().sending);
        assert!(app.message.contains("empty"));
    }

    #[test]
    fn compose_ctrl_c_never_quits() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('c')));
        assert!(!app.quit);
        assert!(app.compose_open(), "draft survives");
    }
}
