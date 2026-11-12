//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::{App, Focus, Pane, Tab, board_move, board_open, board_selected_number};
use crate::core;

pub(crate) fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    // Auth prompt captures everything: y re-auths, n/Esc dismisses.
    if app.auth_prompt.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.reauth_requested = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                let cmd = crate::api::reauth_command();
                app.auth_prompt = None;
                app.auth_dismissed = true;
                // Back to Main so the hint renders: the bar keeps input
                // focus otherwise and swallows the toast.
                app.focus = crate::app::Focus::Main;
                app.notify(format!("auth dismissed — re-auth by hand: {cmd}"));
            }
            _ => {}
        }
        return;
    }
    if app.shortcuts_open {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            app.shortcuts_open = false;
        }
        return;
    }

    // Generic confirm modal captures everything: a listed choice key
    // picks it, Enter picks the first choice, Esc cancels. No other key
    // (including quit) escapes the dialog.
    if app.confirm_open() {
        handle_confirm_key(app, key);
        return;
    }

    // Compose modal captures everything: typing edits the draft,
    // Ctrl+S sends, Esc discards.
    if app.compose_open() {
        handle_compose_key(app, key);
        return;
    }

    // Filter dialog captures everything: over the board it filters cards,
    // over the main view it keeps its window/query/toggle keys.
    if app.filter_open {
        handle_filter_key(app, key);
        return;
    }

    // Board modal captures everything.
    if board_open(app) {
        match key.code {
            KeyCode::Char('q') => {
                app.board = None;
                app.board_loading = false;
            }
            KeyCode::Esc => {
                // Esc first clears an active filter, then closes the board.
                if app.board_filtered() {
                    app.clear_board_filter();
                } else {
                    app.board = None;
                    app.board_loading = false;
                }
            }
            KeyCode::Char('f') => app.open_board_filter(),
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
                    app.open_issue(n);
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
            } else {
                // The Main pane is mouse-gated (Tab never lands on it),
                // so neighbor-step works from every pane: nothing else
                // claims h/l outside Graph, and bar/board/filter return
                // before this match.
                app.step_issue(-1);
            }
        }
        KeyCode::Char('l') => {
            if graph_focused(app) {
                app.graph_move(1);
            } else {
                app.step_issue(1);
            }
        }
        KeyCode::Enter => {
            if graph_focused(app) {
                app.open_graph_selection();
            } else if app.pane == Pane::Refs {
                app.open_selected_ref();
            }
        }
        KeyCode::Char('J') if app.visual_diff() => app.jump_diff_file(1),
        KeyCode::Char('K') if app.visual_diff() => app.jump_diff_file(-1),
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
        KeyCode::Char('S') => app.submit_pending_review(),
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

/// Generic confirm dialog keys: a listed choice key picks it, Enter
/// picks the first choice, Esc cancels. Everything else is swallowed
/// so no navigation or quit escapes the dialog.
pub(crate) fn handle_confirm_key(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => app.confirm_cancel(),
        KeyCode::Enter => app.confirm_choose(0),
        KeyCode::Char(c) => {
            let lower = c.to_ascii_lowercase();
            if let Some(index) = app.confirm.as_ref()
                .and_then(|d| d.choices.iter().position(|ch| ch.key == lower))
            {
                app.confirm_choose(index);
            }
        }
        _ => {}
    }
}

/// Compose modal keys: printable chars edit the draft, arrows move the
/// cursor, and every common bash/readline chord works like a terminal
/// prompt: `Ctrl+A/E` line ends, `Ctrl+B/F/P/N` steps, `Alt+B/F` word
/// jumps, `Ctrl+K/U/W` kills plus `Alt+D`, `Ctrl+Y` yanks, `Ctrl+D/T`
/// delete and transpose, `Home/End/Delete` included. `Ctrl+O` opens the
/// draft in `$EDITOR`, `Ctrl+S` submits, `Esc` discards. The modal
/// swallows every key — `Tab` (even as `Ctrl+I`) indents, `Ctrl+C`
/// never quits — so nothing behind it fires while it is up. A lost
/// draft would be worse than an ignored key, and control bytes must
/// never leak into the markdown.
pub(crate) fn handle_compose_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::{KeyCode, KeyModifiers};
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // Tab is Tab whatever the modifiers say: some terminals report it
    // as Ctrl+I, and the modal must indent, never walk history.
    if matches!(key.code, KeyCode::Tab) {
        app.compose_insert('\t');
        return;
    }
    if ctrl && !alt {
        match key.code {
            KeyCode::Char('s') | KeyCode::Char('S') => app.submit_compose(),
            KeyCode::Char('o') | KeyCode::Char('O') => app.request_compose_editor(),
            KeyCode::Char('a') | KeyCode::Char('A') => app.compose_home(),
            KeyCode::Char('e') | KeyCode::Char('E') => app.compose_end(),
            KeyCode::Char('b') | KeyCode::Char('B') => app.compose_step(-1),
            KeyCode::Char('f') | KeyCode::Char('F') => app.compose_step(1),
            KeyCode::Char('p') | KeyCode::Char('P') => app.compose_line(-1),
            KeyCode::Char('n') | KeyCode::Char('N') => app.compose_line(1),
            KeyCode::Char('h') | KeyCode::Char('H') => app.compose_backspace(),
            KeyCode::Char('d') | KeyCode::Char('D') => app.compose_delete_forward(),
            KeyCode::Char('k') | KeyCode::Char('K') => app.compose_kill_to_end(),
            KeyCode::Char('u') | KeyCode::Char('U') => app.compose_kill_to_start(),
            KeyCode::Char('w') | KeyCode::Char('W') => app.compose_delete_word_back(),
            KeyCode::Char('y') | KeyCode::Char('Y') => app.compose_yank(),
            KeyCode::Char('t') | KeyCode::Char('T') => app.compose_transpose(),
            KeyCode::Char('m') | KeyCode::Char('M') | KeyCode::Char('j') | KeyCode::Char('J') => {
                app.compose_insert('\n')
            }
            // Ctrl+I is Tab on terminals without the kitty protocol;
            // the global history path matches both forms, so this does.
            KeyCode::Char('i') | KeyCode::Char('I') => app.compose_insert('\t'),
            KeyCode::Enter => app.compose_insert('\n'),
            // Every other control chord is swallowed: Ctrl+C never
            // quits here and control bytes never reach the draft.
            _ => {}
        }
        return;
    }
    if alt && !ctrl {
        match key.code {
            KeyCode::Char('b') | KeyCode::Char('B') => app.compose_word_back(),
            KeyCode::Char('f') | KeyCode::Char('F') => app.compose_word_forward(),
            KeyCode::Char('d') | KeyCode::Char('D') => app.compose_delete_word_forward(),
            KeyCode::Backspace => app.compose_delete_word_back(),
            _ => {}
        }
        return;
    }
    if ctrl && alt {
        // Meta-control chords edit nothing: swallow them so they
        // neither type nor escape the modal.
        return;
    }
    match key.code {
        KeyCode::Esc => app.discard_compose(),
        KeyCode::Enter => app.compose_insert('\n'),
        KeyCode::Backspace => app.compose_backspace(),
        KeyCode::Delete => app.compose_delete_forward(),
        KeyCode::Home => app.compose_home(),
        KeyCode::End => app.compose_end(),
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
        app.request_issue(n);
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
        app.request_issue(n);
    }
}

/// Board filter form keys. While the text row edits, keys type into it
/// (`Esc` backs out to navigate mode, `Enter` applies); otherwise `j`/`k`
/// and the arrows move the active widget, `space` toggles the pill under
/// the cursor, typing on the text row starts editing it, `Enter` applies,
/// and `Esc` clears the whole filter.
fn handle_board_filter_key(app: &mut App, key: crossterm::event::KeyEvent) {
    if app.query_edit {
        match key.code {
            KeyCode::Esc => {
                // Revert the draft; the dialog stays open.
                app.query_draft = app.board_filter.clone();
                app.query_edit = false;
            }
            KeyCode::Enter => {
                app.query_edit = false;
                app.filter_open = false;
                app.filter_focus = 0;
                app.apply_board_filter();
            }
            KeyCode::Up => {
                app.query_edit = false;
                app.board_filter_step(-1);
            }
            KeyCode::Down => {
                app.query_edit = false;
                app.board_filter_step(1);
            }
            KeyCode::Char(c) => app.query_draft.push(c),
            KeyCode::Backspace => {
                app.query_draft.pop();
            }
            _ => {}
        }
        return;
    }
    match key.code {
        KeyCode::Esc => {
            app.filter_open = false;
            app.filter_focus = 0;
            app.clear_board_filter();
        }
        KeyCode::Enter => {
            app.filter_open = false;
            app.filter_focus = 0;
            app.apply_board_filter();
        }
        KeyCode::Char(' ') if app.filter_focus > 0 => toggle_board_pill(app),
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('l') | KeyCode::Right => {
            app.board_filter_step(1);
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('h') | KeyCode::Left => {
            app.board_filter_step(-1);
        }
        KeyCode::Backspace if app.filter_focus == 0 => {
            app.query_draft.pop();
        }
        KeyCode::Char(c) if app.filter_focus == 0 => {
            // Typing on the text row starts editing, like the old dialog.
            app.query_edit = true;
            app.query_draft.push(c);
        }
        _ => {}
    }
}

/// Toggle the milestone pill under the dialog cursor in the draft.
fn toggle_board_pill(app: &mut App) {
    let pills = app.board_pills();
    let Some(pill) = pills.get(app.filter_focus.saturating_sub(1)) else {
        return;
    };
    if !app.filter_milestone_draft.remove(pill) {
        app.filter_milestone_draft.insert(pill.clone());
    }
}

/// Filter dialog keys. Over the board the dialog is a form: the text row
/// plus one milestone pill per milestone on the board. `j`/`k` and the
/// arrows move the active widget, `space` toggles the pill under the
/// cursor, typing on the text row edits it, Enter applies, Esc clears the
/// filter. Everywhere else window presets refetch, toggles only hide
/// recent rows, and `e` edits the server-side GitHub query.
fn handle_filter_key(app: &mut App, key: crossterm::event::KeyEvent) {
    use crate::app::QueryAction;
    if board_open(app) {
        handle_board_filter_key(app, key);
        return;
    }
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

/// Visible height of one sidebar row: 3 lines per `Item` with 1-line
/// labeled dividers. An expanded row adds the wrapped detail block its
/// render draws, measured with the same inner width.
fn sidebar_row_height(
    row: &crate::model::SideRow,
    data: &std::collections::HashMap<u64, crate::model::IssueData>,
    expanded: &std::collections::HashSet<u64>,
    inner_width: usize,
) -> usize {
    match row.number() {
        Some(n) => {
            let detail = match data.get(&n) {
                Some(d) if expanded.contains(&n) => {
                    crate::ui::expanded_row_height(d, inner_width)
                }
                _ => 0,
            };
            3 + detail
        }
        None => 1,
    }
}

/// Sidebar rows are 3 lines each (`Item`) with 1-line labeled dividers,
/// below a 1-line border plus a 1-line spacer. Dividers never hit.
/// Takes the render-cached pane rows by borrow (no clone, no recompute):
/// the caller holds what `render` recorded for this frame.
fn sidebar_hit(
    area: Rect,
    rows: &[crate::model::SideRow],
    data: &std::collections::HashMap<u64, crate::model::IssueData>,
    expanded: &std::collections::HashSet<u64>,
    x: u16,
    y: u16,
) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    let inner_width = (area.width as usize).saturating_sub(2);
    let mut rel = (y - area.y - 2) as usize;
    for row in rows {
        let h = sidebar_row_height(row, data, expanded, inner_width);
        if rel < h {
            return row.number();
        }
        rel -= h;
    }
    None
}

/// Screen rect of the sidebar row naming `target`, for the click flash.
/// Walks the same geometry as `sidebar_hit` so the flash covers exactly
/// the row the press landed on.
fn sidebar_row_rect(
    area: Rect,
    rows: &[crate::model::SideRow],
    data: &std::collections::HashMap<u64, crate::model::IssueData>,
    expanded: &std::collections::HashSet<u64>,
    target: u64,
) -> Option<Rect> {
    if area.width == 0 {
        return None;
    }
    let inner_width = (area.width as usize).saturating_sub(2);
    let mut rel: u16 = 0;
    for row in rows {
        let h = sidebar_row_height(row, data, expanded, inner_width) as u16;
        if row.number() == Some(target) {
            return Some(Rect::new(area.x, area.y + 2 + rel, area.width, h));
        }
        rel += h;
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
    if app.auth_prompt.is_some() { return; }
    if app.confirm_open() { return; }
    if app.shortcuts_open { return; }
    // The compose modal steals all mouse focus: clicks and wheel never
    // reach the panes behind it, so the draft and selection survive.
    if app.compose_open() { return; }
    if board_open(app) {
        handle_board_mouse(app, m);
        return;
    }
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => handle_left(app, m.column, m.row, false),
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

/// Board mouse path: press opens (double-press), drag only trails the
/// cursor cell like a hover, scroll walks cards. Release never opens:
/// the press already committed, so release is a no-op.
fn handle_board_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Header icons first: they sit outside every card cell.
            if let Some((rect, url)) = app
                .board_links
                .iter()
                .find(|(rect, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, url)| (*rect, url.clone()))
            {
                app.note_click(rect);
                open_url(&url, app);
                return;
            }
            let hit = app
                .board_cells
                .iter()
                .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, c, r)| (*rect, *c, *r));
            if let Some((rect, c, r)) = hit {
                app.note_click(rect);
                // Clicking the URL line of an expanded card opens it.
                if app.board_expanded == Some((c, r)) && m.row - rect.y == 3 {
                    if let Some(board) = &app.board {
                        let cols =
                            crate::app::board_columns(board, &app.board_filter, &app.board_milestones);
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
                        app.open_issue(n);
                    }
                } else {
                    app.board_col = c;
                    app.board_row = r;
                    app.board_expanded = None;
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            let hit = app
                .board_cells
                .iter()
                .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, c, r)| (*rect, *c, *r));
            if let Some((rect, c, r)) = hit {
                app.board_col = c;
                app.board_row = r;
                app.note_click(rect);
            }
        }
        MouseEventKind::ScrollDown => board_move(app, 0, 1),
        MouseEventKind::ScrollUp => board_move(app, 0, -1),
        // Release never opens: the press already committed.
        MouseEventKind::Up(_) => {}
        _ => {}
    }
}

/// Shared Left-press path. `drag` is a held-button move: side panes
/// preview the row under the cursor (transient hover, never a visit),
/// the refs pane previews the highlight without opening pairs, and
/// Main only takes focus — Scroll events keep owning the scroll, and
/// links/diff lines commit on a real press only. Both press and drag
/// record a click flash; release stays a no-op at the call site.
#[allow(clippy::too_many_lines)]
fn handle_left(app: &mut App, x: u16, y: u16, drag: bool) {
    if in_rect(app.bar_area, x, y) {
        if !drag {
            app.focus = Focus::Bar;
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    // Hit-test against the render-cached pane rows by borrow: computed
    // once per frame in `render`, reused here with no clone.
    let issue_hit = sidebar_hit(
        app.side_issue_area,
        &app.side_issue_rows,
        &app.data,
        &app.expanded_rows,
        x,
        y,
    );
    if let Some(n) = issue_hit {
        let flash = sidebar_row_rect(
            app.side_issue_area,
            &app.side_issue_rows,
            &app.data,
            &app.expanded_rows,
            n,
        );
        // A press on the already-visible row files it under requested
        // (first press previews, second promotes); a drag only hovers.
        // The focus/pane guard reads pre-click state, as before.
        if !drag
            && app.focus == Focus::Main
            && app.pane == Pane::Issues
            && app.current == Some(n)
            && app.effective(&app.side_issue_rows) == Some(n)
        {
            app.ensure_visited(n);
        }
        app.focus = Focus::Main;
        app.pane = Pane::Issues;
        if drag {
            app.hover = if Some(n) == app.current { None } else { Some(n) };
        } else {
            app.request_issue(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    let pr_hit = sidebar_hit(
        app.side_pr_area,
        &app.side_pr_rows,
        &app.data,
        &app.expanded_rows,
        x,
        y,
    );
    if let Some(n) = pr_hit {
        let flash = sidebar_row_rect(
            app.side_pr_area,
            &app.side_pr_rows,
            &app.data,
            &app.expanded_rows,
            n,
        );
        if !drag
            && app.focus == Focus::Main
            && app.pane == Pane::PRs
            && app.current == Some(n)
            && app.effective(&app.side_pr_rows) == Some(n)
        {
            app.ensure_visited(n);
        }
        app.focus = Focus::Main;
        app.pane = Pane::PRs;
        if drag {
            app.hover = if Some(n) == app.current { None } else { Some(n) };
        } else {
            app.request_issue(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    // Mention rows only highlight their source line; paired rows
    // open their counterpart like sidebar rows.
    let ref_hit = sidebar_hit(
        app.refs_area,
        &app.ref_rows,
        &app.data,
        &app.expanded_rows,
        x,
        y,
    );
    if let Some(n) = ref_hit {
        let flash = sidebar_row_rect(
            app.refs_area,
            &app.ref_rows,
            &app.data,
            &app.expanded_rows,
            n,
        );
        app.focus = Focus::Main;
        app.pane = Pane::Refs;
        if drag {
            app.ref_highlight = Some(n);
        } else if app.ref_paired.contains(&n) {
            app.open_issue(n);
        } else {
            app.select_ref(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    if in_rect(app.tabs_area, x, y) {
        if !drag {
            app.pane = Pane::Main;
            let rel = x.saturating_sub(app.tabs_area.x);
            app.set_tab(if rel < crate::app::TAB_SPLIT_X { Tab::Details } else { Tab::Visual });
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    // The meta/label block above the tabs resolves refs and URLs
    // exactly like body lines; only the body folds diff files.
    let in_body = in_rect(app.body_area, x, y);
    let in_header = !in_body && in_rect(app.header_area, x, y);
    if in_body || in_header {
        app.focus = Focus::Main;
        app.pane = Pane::Main;
        let area = if in_body { app.body_area } else { app.header_area };
        let row = (y - area.y) as usize;
        app.note_click(Rect::new(area.x, y, area.width, 1));
        if drag {
            return;
        }
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
        let line = {
            let rows = if in_body { &app.body_rows } else { &app.header_rows };
            rows.get(row).cloned()
        };
        if let Some(line) = line {
            let repo = app.repo_key();
            let rel_x = (x - area.x) as usize;
            let links = crate::model::links_in_line(&line, repo.as_deref());
            match crate::model::nearest_link(&line, &links, rel_x) {
                Some(crate::model::LinkTarget::Issue(n)) => {
                    app.open_issue(n);
                }
                Some(crate::model::LinkTarget::Url(url)) => open_url(&url, app),
                None => {}
            }
        }
        return;
    }
    app.focus = Focus::Main;
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
            milestone: None,
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
        assert_eq!(sidebar_hit(area, &rows, &app.data, &app.expanded_rows, 5, area.y + 2), Some(7));

        app.expanded_rows.insert(7);
        // Every line of the detail block belongs to the expanded row.
        let detail_top = area.y + 4;
        for y in detail_top..detail_top + detail as u16 {
            assert_eq!(sidebar_hit(area, &rows, &app.data, &app.expanded_rows, 5, y), Some(7), "detail line {y}");
        }
        // Row 9 starts only after the detail block plus the item's blank tail.
        let row9_top = area.y + 2 + 3 + detail as u16;
        for y in row9_top..row9_top + 3 {
            assert_eq!(sidebar_hit(area, &rows, &app.data, &app.expanded_rows, 5, y), Some(9), "row 9 line {y}");
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

    fn alt_key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::ALT)
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

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            handle_key(app, plain_key(KeyCode::Char(ch)));
        }
    }

    fn compose_cursor(app: &App) -> usize {
        app.compose.as_ref().expect("compose").cursor
    }

    #[test]
    fn compose_emacs_home_end_and_char_steps() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "hello");
        handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
        assert_eq!(compose_cursor(&app), 0, "ctrl-a jumps to line start");
        handle_key(&mut app, ctrl_key(KeyCode::Char('f')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('f')));
        assert_eq!(compose_cursor(&app), 2, "ctrl-f steps forward");
        handle_key(&mut app, ctrl_key(KeyCode::Char('e')));
        assert_eq!(compose_cursor(&app), 5, "ctrl-e jumps to line end");
        handle_key(&mut app, ctrl_key(KeyCode::Char('b')));
        assert_eq!(compose_cursor(&app), 4, "ctrl-b steps back");
        handle_key(&mut app, ctrl_key(KeyCode::Char('p')));
        assert_eq!(compose_cursor(&app), 4, "ctrl-p on one line stays put");
    }

    #[test]
    fn compose_word_jumps() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "foo bar");
        handle_key(&mut app, alt_key(KeyCode::Char('b')));
        assert_eq!(compose_cursor(&app), 4, "alt-b jumps to the word start");
        handle_key(&mut app, alt_key(KeyCode::Char('b')));
        assert_eq!(compose_cursor(&app), 0, "alt-b again jumps to the first word");
        handle_key(&mut app, alt_key(KeyCode::Char('f')));
        assert_eq!(compose_cursor(&app), 3, "alt-f jumps to the word end");
        handle_key(&mut app, alt_key(KeyCode::Char('f')));
        assert_eq!(compose_cursor(&app), 7, "alt-f again jumps past the last word");
    }

    #[test]
    fn compose_kill_ring_kills_and_yanks() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "hello world");
        handle_key(&mut app, ctrl_key(KeyCode::Char('w')));
        assert_eq!(app.compose.as_ref().unwrap().body, "hello ");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.as_ref().unwrap().body, "hello world");
        assert_eq!(compose_cursor(&app), 11);
        handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.compose.as_ref().unwrap().body, "");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.as_ref().unwrap().body, "hello world");
    }

    #[test]
    fn compose_alt_d_deletes_the_word_ahead() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "foo bar");
        handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
        handle_key(&mut app, alt_key(KeyCode::Char('d')));
        assert_eq!(app.compose.as_ref().unwrap().body, " bar");
        assert_eq!(compose_cursor(&app), 0, "cursor stays put");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.as_ref().unwrap().body, "foo bar");
    }

    #[test]
    fn compose_ctrl_u_kills_to_line_start() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "hello");
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, ctrl_key(KeyCode::Char('u')));
        assert_eq!(app.compose.as_ref().unwrap().body, "lo");
        assert_eq!(compose_cursor(&app), 0);
    }

    #[test]
    fn compose_ctrl_d_deletes_the_char_ahead() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "abc");
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, ctrl_key(KeyCode::Char('d')));
        assert_eq!(app.compose.as_ref().unwrap().body, "ac");
        assert_eq!(compose_cursor(&app), 1);
    }

    #[test]
    fn compose_ctrl_t_transposes_chars() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "ab");
        handle_key(&mut app, ctrl_key(KeyCode::Char('t')));
        assert_eq!(app.compose.as_ref().unwrap().body, "ba");
    }

    #[test]
    fn compose_home_end_and_delete_keys() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "hi");
        handle_key(&mut app, plain_key(KeyCode::Home));
        assert_eq!(compose_cursor(&app), 0, "home jumps to line start");
        handle_key(&mut app, plain_key(KeyCode::End));
        assert_eq!(compose_cursor(&app), 2, "end jumps to line end");
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, plain_key(KeyCode::Delete));
        assert_eq!(app.compose.as_ref().unwrap().body, "h");
    }

    #[test]
    fn compose_ctrl_i_inserts_tab_and_never_walks_history() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "a");
        handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
        type_text(&mut app, "b");
        assert_eq!(app.compose.as_ref().unwrap().body, "a\tb");
        assert_eq!(app.current, Some(7), "tab never leaves the modal");
        assert!(app.compose_open());
    }

    #[test]
    fn compose_ctrl_o_arms_editor_or_names_a_missing_editor() {
        // Serialized with every other env-mutating test process-wide.
        let _env = crate::cache::test_env_lock().lock().unwrap();
        let prior = std::env::var("EDITOR").ok();
        std::env::remove_var("EDITOR");
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "draft");
        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert!(!app.editor_requested, "nothing to run without an editor");
        assert!(app.message.contains("EDITOR"), "toast names the fix: {}", app.message);
        assert!(app.compose_open(), "draft survives");

        std::env::set_var("EDITOR", "true");
        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert!(app.editor_requested, "ctrl-o arms the editor run");
        assert!(app.compose_open(), "draft survives until the run lands");
        match prior {
            Some(v) => std::env::set_var("EDITOR", v),
            None => std::env::remove_var("EDITOR"),
        }
    }

    fn pending_dialog_app() -> App {
        let mut app = compose_app();
        app.data.insert(9, sample(9));
        app.statuses.insert(9, vec![]);
        app.visited.push(9);
        let d = app.data.get_mut(&7).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(1),
            comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
            ..Default::default()
        }];
        app.request_issue(9);
        assert!(app.confirm_open(), "guard trips");
        app
    }

    #[test]
    fn confirm_dialog_keys_submit_discard_and_cancel() {
        // Discard navigates without submitting.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Char('d')));
        assert!(!app.confirm_open());
        assert_eq!(app.current, Some(9));
        assert!(app.review_submitting.is_empty());
        // Submit navigates and submits.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Char('s')));
        assert!(!app.confirm_open());
        assert_eq!(app.current, Some(9));
        assert!(app.review_submitting.contains(&7));
        // Enter picks the first choice (Submit).
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Enter));
        assert!(!app.confirm_open());
        assert!(app.review_submitting.contains(&7));
        // Esc cancels: no navigation, nothing sent.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.confirm_open());
        assert_eq!(app.current, Some(7));
        assert!(app.review_submitting.is_empty());
        // Quit never escapes the dialog.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Char('q')));
        assert!(!app.quit);
        assert!(app.confirm_open());
    }

    #[test]
    fn shift_s_submits_pending_review() {
        let mut app = compose_app();
        let d = app.data.get_mut(&7).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(1),
            comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
            ..Default::default()
        }];
        handle_key(&mut app, plain_key(KeyCode::Char('S')));
        assert!(app.review_submitting.contains(&7), "S starts the submit");
    }

    fn step_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7u64, 8] {
            app.data.insert(n, sample(n));
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.show_issue(7);
        app.pane = Pane::Main;
        app
    }

    #[test]
    fn hl_steps_between_neighbors_on_main() {
        let mut app = step_app();
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.current, Some(8), "l opens the next issue");
        assert_eq!(app.issue_state.selected(), Some(2));
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        assert_eq!(app.current, Some(7), "h opens the previous issue");
        assert_eq!(app.issue_state.selected(), Some(1));
        assert_eq!(app.step_pending, None, "cached steps never linger");
    }

    #[test]
    fn hl_steps_from_every_pane() {
        // The Main pane is mouse-gated (Tab never lands on it), so the
        // neighbor step works from every pane: nothing else claims h/l
        // outside Graph.
        let mut app = step_app();
        for pane in [Pane::Main, Pane::Issues, Pane::PRs, Pane::Refs] {
            app.pane = pane;
            app.message.clear();
            handle_key(&mut app, plain_key(KeyCode::Char('l')));
            assert_eq!(app.current, Some(8), "l steps from {pane:?}");
            assert_eq!(app.step_pending, None, "cached steps never linger");
            handle_key(&mut app, plain_key(KeyCode::Char('h')));
            assert_eq!(app.current, Some(7), "h steps back from {pane:?}");
            assert!(app.message.is_empty(), "no toast on a live neighbor");
        }
    }

    #[test]
    fn hl_on_main_without_an_issue_toasts() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.pane = Pane::Main;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        assert_eq!(app.message, "view an issue first");
        assert!(app.issue_pending.is_empty(), "no fetch without an issue");
    }

    #[test]
    fn bar_filter_and_board_keep_their_h_and_l() {
        let mut app = step_app();
        // The command bar types the keys as text.
        app.focus = crate::app::Focus::Bar;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.bar, "hl");
        assert_eq!(app.current, Some(7));
        // The filter dialog swallows them.
        app.focus = crate::app::Focus::Main;
        app.filter_open = true;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.current, Some(7));
        assert_eq!(app.step_pending, None);
        // The board modal moves columns instead of stepping issues.
        app.filter_open = false;
        app.board_loading = true;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.current, Some(7), "board keeps h/l");
        assert_eq!(app.step_pending, None);
        assert!(app.board_loading, "board stays open");
    }

    #[test]
    fn auth_prompt_y_requests_reauth_and_n_dismisses() {
        let mut app = step_app();
        app.auth_prompt = Some(crate::app::AuthPrompt {
            missing: vec!["project".to_string()],
            cause: "token lacks scopes ghit needs".to_string(),
            scopes_known: true,
        });
        handle_key(&mut app, plain_key(KeyCode::Char('y')));
        assert!(app.reauth_requested, "y arms the refresh");
        assert!(app.auth_prompt.is_some(), "loop clears the modal after running");
        let mut app = step_app();
        app.auth_prompt = Some(crate::app::AuthPrompt {
            missing: vec![],
            cause: "GitHub auth check failed: 401 Unauthorized".to_string(),
            scopes_known: false,
        });
        handle_key(&mut app, plain_key(KeyCode::Char('n')));
        assert!(!app.reauth_requested, "n never refreshes");
        assert!(app.auth_prompt.is_none(), "n dismisses");
        assert!(matches!(app.focus, crate::app::Focus::Main), "hint must render");
        assert!(app.message.contains("gh auth refresh"), "hint: {}", app.message);
    }
}
