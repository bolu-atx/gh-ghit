//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

mod actions;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::{App, Focus, Tab, board_move, board_open, board_selected_number};
use crate::core::{self, Dialog, Layer, Pane, TextInput};
use crate::keymap;

pub(crate) fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    let chain = keymap::active_contexts(app);
    match keymap::lookup(&chain, key, app) {
        Some(binding) => actions::perform(app, binding.action),
        None => type_text(app, key),
    }
}

/// Printable keys no binding claimed. Text layers type them; every other
/// layer swallows them, so nothing behind a dialog fires.
fn type_text(app: &mut App, key: crossterm::event::KeyEvent) {
    let KeyCode::Char(c) = key.code else { return };
    let chord = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match app.focus().layer {
        Layer::Input(TextInput::Bar) if app.sidebar.searching => app.sidebar.search.push(c),
        Layer::Input(TextInput::Bar) => app.bar.push(c),
        Layer::Input(TextInput::Query | TextInput::BoardQuery) => app.sidebar.query_draft.push(c),
        Layer::Input(TextInput::Name) => app.sidebar.name_draft.push(c),
        // The editor never types a control chord into the draft.
        Layer::Dialog(Dialog::Compose) if !chord => app.compose_insert(c),
        Layer::Dialog(Dialog::Confirm) => {
            let lower = c.to_ascii_lowercase();
            if let Some(index) = app.confirm.as_ref()
                .and_then(|d| d.choices.iter().position(|ch| ch.key == lower))
            {
                app.confirm_choose(index);
            }
        }
        // Typing on the text row starts editing, like the old dialog.
        Layer::Dialog(Dialog::BoardFilter) if app.board.filter_focus == 0 => {
            app.sidebar.query_edit = true;
            app.sidebar.query_draft.push(c);
        }
        _ => {}
    }
}

/// j/k walks one sidebar list: issues or PRs, never crossing over.
/// Starts from the wheel hover when one names a row here, so keyboard
/// and wheel continue from the same place; the visit clears the hover.
fn move_in_list(app: &mut App, order: Vec<u64>, delta: isize) {
    let base = app.nav.hover.filter(|n| order.contains(n)).or(app.nav.current);
    if let Some(n) = core::next_in_order(&order, base, delta) {
        app.request_issue(n);
    }
}

/// j/k act on the focused side pane: each sidebar list walks only itself
/// and refs moves the source highlight. The body scrolls instead; the Diff
/// tab moves its line cursor with the arrow keys.
fn pane_down(app: &mut App, delta: isize) {
    match app.nav.pane {
        Pane::Issues => move_in_list(app, app.issue_numbers(), delta),
        Pane::PRs => move_in_list(app, app.pr_numbers(), delta),
        Pane::Refs => {
            let next = core::next_in_order(&app.ref_order(), app.refs.highlight, delta);
            if let Some(n) = next.filter(|n| Some(*n) != app.refs.highlight) {
                app.select_ref(n);
            }
        }
        Pane::Labels => app.move_filter_cursor(delta),
        Pane::History => app.history_step(delta),
        Pane::Main => {
            app.nav.scroll = app.nav.scroll.saturating_add_signed(delta as i16 * 3);
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
    let height = app.hits.body_area.height as usize;
    if height == 0 {
        return;
    }
    let base = app.hits.body_base as usize;
    let scroll = app.nav.scroll as usize;
    let mut abs = match app.diff.line {
        Some(a) => a.clamp(scroll, scroll + height.saturating_sub(1)),
        None => {
            // Anchor to the first header when no cursor yet; render fills
            // in the file without expanding it.
            if let Some(&h) = app.diff.headers.first() {
                base + h
            } else {
                return;
            }
        }
    };
    if delta > 0 {
        let vis = abs.saturating_sub(base);
        let rows = &app.hits.body_rows;
        if vis + 1 < rows.len() {
            // A blank target with only blanks below is past the content.
            let target_blank = rows[vis + 1].trim().is_empty();
            let rest_blank = rows.get(vis + 2..).map(|r| r.iter().all(|l| l.trim().is_empty())).unwrap_or(true);
            if target_blank && rest_blank {
                return;
            }
            abs += 1;
        } else {
            app.nav.scroll = app.nav.scroll.saturating_add(1);
            abs += 1;
        }
    } else if delta < 0 {
        if abs > scroll {
            abs -= 1;
        } else if scroll > 0 {
            app.nav.scroll -= 1;
            abs -= 1;
        } else {
            return;
        }
    } else {
        return;
    }
    app.diff.line = Some(abs);
    app.diff.touched = true;
}

/// n/N jumps between `/` search hits, wrapping at the ends.
fn jump_search_hit(app: &mut App, delta: isize) {
    let hits = app.search_hits();
    if hits.is_empty() {
        app.notify("no search: press /".to_string());
        return;
    }
    if let Some(n) = core::next_in_order(&hits, app.nav.current, delta) {
        app.request_issue(n);
    }
}

/// Toggle the widget under the board dialog cursor: a milestone pill
/// flips in the draft (Enter applies it), a Card fields row flips live
/// and persists.
fn toggle_board_focused(app: &mut App) {
    let pills = app.board_pills();
    let focus = app.board.filter_focus;
    if focus == 0 {
        return;
    }
    let pill_index = focus - 1;
    if pill_index < pills.len() {
        let Some(pill) = pills.get(pill_index) else { return };
        if !app.board.milestone_draft.remove(pill) {
            app.board.milestone_draft.insert(pill.clone());
        }
    } else {
        app.toggle_board_card_field(pill_index - pills.len());
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
    Labels,
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
        WheelTarget::Main => {
            app.nav.scroll = app.nav.scroll.saturating_add_signed(delta as i16 * 3);
        }
    }
}

pub(crate) fn handle_mouse(app: &mut App, m: MouseEvent) {
    let stack = app.layer_stack();
    let open = |dialog: Dialog| stack.contains(&Layer::Dialog(dialog));
    if open(Dialog::Confirm) {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            let hit = app.hits.confirm_choices.iter()
                .find(|(r, _)| r.contains(ratatui::layout::Position::new(m.column, m.row)))
                .map(|(_, i)| *i);
            if let Some(i) = hit {
                app.confirm_choose(i);
            }
        }
        return;
    }
    if matches!(m.kind, MouseEventKind::Down(MouseButton::Left))
        && app.undo_live()
        && !open(Dialog::Shortcuts)
        && !open(Dialog::Compose)
        && app.hits.undo_toast.contains(ratatui::layout::Position::new(m.column, m.row))
    {
        app.undo_last();
        return;
    }
    // Shortcuts and compose steal all mouse focus: clicks and wheel never
    // reach the panes behind them, so the draft and selection survive.
    if open(Dialog::Shortcuts) || open(Dialog::Compose) {
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
        let at = ratatui::layout::Position::new(m.column, m.row);
        let hit = app.hits.bar_hints.iter().find(|(rect, _)| rect.contains(at)).map(|(rect, action)| (*rect, *action));
        if let Some((rect, action)) = hit {
            app.hits.last_click = Some((rect, std::time::Instant::now()));
            actions::perform(app, action);
            return;
        }
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

/// Mouse path while the Format dialog or board filter form is open. The
/// dialog owns every event: a press on a recorded cell focuses its widget
/// and runs the keyboard action, a press outside the popup closes the
/// sidebar dialog (as Esc does) and is ignored over the board, where Esc
/// would clear the filter. Wheel and drag never reach the panes behind.
fn handle_form_mouse(app: &mut App, m: MouseEvent) {
    if !matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
        return;
    }
    let on_board = board_open(app);
    let hit = app.hits.form_cells.iter()
        .find(|(rect, _, _)| in_rect(*rect, m.column, m.row)).copied();
    let Some((rect, index, option)) = hit else {
        if !on_board && !in_rect(app.hits.form_popup, m.column, m.row)
            && !app.sidebar.query_edit && !app.sidebar.name_edit
        {
            app.sidebar.filter_open = false;
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
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            // Header icons first: they sit outside every card cell.
            if let Some((rect, url)) = app.hits
                .board_links
                .iter()
                .find(|(rect, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, url)| (*rect, url.clone()))
            {
                app.note_click(rect);
                open_url(&url, app);
                return;
            }
            let hit = app.hits
                .board_cells
                .iter()
                .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, c, r)| (*rect, *c, *r));
            if let Some((rect, c, r)) = hit {
                app.note_click(rect);
                // Clicking the URL line of an expanded card opens it.
                if app.board.expanded == Some((c, r)) && m.row - rect.y == 3 {
                    if let Some(board) = &app.board.data {
                        let cols =
                            crate::app::board_columns(board, &app.board.filter, &app.board.milestones);
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
            let hit = app.hits
                .board_cells
                .iter()
                .find(|(rect, _, _)| in_rect(*rect, m.column, m.row))
                .map(|(rect, c, r)| (*rect, *c, *r));
            if let Some((rect, c, r)) = hit {
                app.board.col = c;
                app.board.row = r;
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
    if in_rect(app.hits.bar_area, x, y) {
        if !drag {
            app.nav.focus = Focus::Bar;
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    if !drag && in_rect(app.hits.history_area, x, y) {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::History;
        let row = app.hits.history_rows.iter().find(|(rect, _)| in_rect(*rect, x, y)).copied();
        if let Some((_, steps)) = row {
            app.nav.history_cursor = steps;
            app.open_history_row();
        }
        return;
    }
    if !drag && in_rect(app.hits.labels_area, x, y) {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Labels;
        let row = app.hits.labels_rows.iter().find(|(rect, _)| in_rect(*rect, x, y)).copied();
        if let Some((rect, index)) = row {
            app.toggle_filter_item(index);
            app.note_click(rect);
        }
        return;
    }
    // Hit-test against the render-cached pane rows by borrow: computed
    // once per frame in `render`, reused here with no clone.
    let issue_hit = sidebar_hit(
        app.hits.side_issue_area,
        &app.hits.side_issue_rows,
        &app.issues.data,
        &app.diff.expanded_rows,
        x,
        y,
    );
    if let Some(n) = issue_hit {
        if !drag && app.sidebar.sub_issue_tree {
            if let Some(&crate::model::SideRow::Tree { depth, has_children: true, folded, .. }) =
                app.hits.side_issue_rows.iter().find(|row| row.number() == Some(n))
            {
                let glyph_x = app.hits.side_issue_area.x + 3 + 2 * (depth as u16);
                let rect = sidebar_row_rect(app.hits.side_issue_area, &app.hits.side_issue_rows,
                    &app.issues.data, &app.diff.expanded_rows, n);
                if x == glyph_x && rect.is_some_and(|rect| y == rect.y) {
                    if folded {
                        app.sidebar.folded_issues.remove(&n);
                        app.load_sub_issues(n);
                    } else {
                        app.sidebar.folded_issues.insert(n);
                    }
                    app.nav.pane = Pane::Issues;
                    app.nav.focus = Focus::Main;
                    return;
                }
            }
        }
        let flash = sidebar_row_rect(
            app.hits.side_issue_area,
            &app.hits.side_issue_rows,
            &app.issues.data,
            &app.diff.expanded_rows,
            n,
        );
        // A press on the already-visible row files it under requested
        // (first press previews, second promotes); a drag only hovers.
        // The focus/pane guard reads pre-click state, as before.
        if !drag
            && app.nav.focus == Focus::Main
            && app.nav.pane == Pane::Issues
            && app.nav.current == Some(n)
            && app.effective(&app.hits.side_issue_rows) == Some(n)
        {
            app.ensure_visited(n);
        }
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Issues;
        if drag {
            app.nav.hover = if Some(n) == app.nav.current { None } else { Some(n) };
        } else {
            app.request_issue(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    let pr_hit = sidebar_hit(
        app.hits.side_pr_area,
        &app.hits.side_pr_rows,
        &app.issues.data,
        &app.diff.expanded_rows,
        x,
        y,
    );
    if let Some(n) = pr_hit {
        let flash = sidebar_row_rect(
            app.hits.side_pr_area,
            &app.hits.side_pr_rows,
            &app.issues.data,
            &app.diff.expanded_rows,
            n,
        );
        if !drag
            && app.nav.focus == Focus::Main
            && app.nav.pane == Pane::PRs
            && app.nav.current == Some(n)
            && app.effective(&app.hits.side_pr_rows) == Some(n)
        {
            app.ensure_visited(n);
        }
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::PRs;
        if drag {
            app.nav.hover = if Some(n) == app.nav.current { None } else { Some(n) };
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
        app.hits.refs_area,
        &app.hits.ref_rows,
        &app.issues.data,
        &app.diff.expanded_rows,
        x,
        y,
    );
    if let Some(n) = ref_hit {
        let flash = sidebar_row_rect(
            app.hits.refs_area,
            &app.hits.ref_rows,
            &app.issues.data,
            &app.diff.expanded_rows,
            n,
        );
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Refs;
        if drag {
            app.refs.highlight = Some(n);
        } else if app.refs.paired.contains(&n) {
            app.open_issue(n);
        } else {
            app.select_ref(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    if in_rect(app.hits.tabs_area, x, y) {
        if !drag {
            app.nav.pane = Pane::Main;
            let rel = x.saturating_sub(app.hits.tabs_area.x);
            app.set_tab(if rel < crate::app::TAB_SPLIT_X { Tab::Details } else { Tab::Visual });
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    // The meta/label block above the tabs resolves refs and URLs
    // exactly like body lines; only the body folds diff files.
    let in_body = in_rect(app.hits.body_area, x, y);
    let in_header = !in_body && in_rect(app.hits.header_area, x, y);
    if in_body || in_header {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Main;
        let area = if in_body { app.hits.body_area } else { app.hits.header_area };
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
            let abs = app.hits.body_base as usize + row;
            app.diff.line = Some(abs);
            app.diff.touched = true;
            let is_header = app.diff.headers.contains(&row);
            if is_header {
                if let Some(f) = app.diff.row_files.get(row).copied().flatten() {
                    app.diff.cursor = f;
                    let expanded = app.nav.current.and_then(|n| {
                        app.diff.files.get(&n).and_then(|fs| {
                            fs.get(f).map(|file| {
                                app.diff.expanded_files.contains(&(n, file.filename.clone()))
                            })
                        })
                    });
                    if expanded == Some(true) {
                        app.diff.touched = false;
                        app.toggle_diff_file();
                        return;
                    }
                }
                return;
            }
        }
        let line = {
            let rows = if in_body { &app.hits.body_rows } else { &app.hits.header_rows };
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
    app.nav.focus = Focus::Main;
}

/// Open a URL in the browser and say so in the bar.
fn open_url(url: &str, app: &mut App) {
    if crate::app::open_in_browser(url).is_ok() {
        app.notify(format!("opened {url}"));
    } else {
        app.notify(format!("couldn't open {url}"));
    }
}

#[cfg(test)]
mod parity;

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
            parent: None,
            sub_summary: None,
            is_pr: false,
        }
    }

    #[test]
    fn expanded_row_detail_shifts_hits_below() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7, 9] {
            app.issues.data.insert(n, sample(n));
            app.issues.statuses.insert(n, vec![]);
            app.nav.visited.push(n);
        }
        let rows = app.issue_rows();
        assert_eq!(rows, vec![SideRow::Item(7), SideRow::Item(9)]);

        let area = Rect::new(0, 0, 40, 40);
        let inner_width = (area.width as usize) - 2;
        let detail = crate::ui::expanded_row_height(app.issues.data.get(&7).unwrap(), inner_width);
        assert!(detail > 3, "detail block is tall enough to span a row: {detail}");

        // Collapsed geometry still applies: row 7 hits its own summary.
        assert_eq!(sidebar_hit(area, &rows, &app.issues.data, &app.diff.expanded_rows, 5, area.y + 2), Some(7));

        app.diff.expanded_rows.insert(7);
        // Every line of the detail block belongs to the expanded row.
        let detail_top = area.y + 4;
        for y in detail_top..detail_top + detail as u16 {
            assert_eq!(sidebar_hit(area, &rows, &app.issues.data, &app.diff.expanded_rows, 5, y), Some(7), "detail line {y}");
        }
        // Row 9 starts only after the detail block plus the item's blank tail.
        let row9_top = area.y + 2 + 3 + detail as u16;
        for y in row9_top..row9_top + 3 {
            assert_eq!(sidebar_hit(area, &rows, &app.issues.data, &app.diff.expanded_rows, 5, y), Some(9), "row 9 line {y}");
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
            app.issues.data.insert(n, d);
            app.issues.statuses.insert(n, vec![]);
            app.nav.visited.push(n);
        }
        app.hits.side_issue_area = Rect::new(0, 0, 30, 20);
        app.hits.side_pr_area = Rect::new(0, 20, 30, 20);
        app.hits.refs_area = Rect::new(90, 0, 30, 40);
        app.hits.body_area = Rect::new(30, 0, 60, 40);
        app.show_issue(7);
        // `show_issue` recomputes refs from the body, so seed the pane after.
        app.refs.numbers = vec![21, 22];
        app.nav.pane = crate::core::Pane::Main;
        app.nav.scroll = 0;
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
            app.issues.data.insert(7, sample(7));
            app.issues.data.insert(9, recent);
            app.issues.statuses.insert(7, vec![]);
            app.issues.statuses.insert(9, vec![]);
            app.ensure_visited(7);
            app.ensure_recent(9);
            app.show_issue(7);
            let area = Rect::new(0, 0, 30, 20);
            let rows = if is_pr { app.pr_rows() } else { app.issue_rows() };
            let row_index = rows.iter().position(|row| row.number() == Some(9)).unwrap();
            let y = area.y + 2 + rows[..row_index].iter()
                .map(|row| if row.number().is_some() { 3 } else { 1 }).sum::<u16>();
            if is_pr {
                app.hits.side_pr_area = area;
                app.hits.side_pr_rows = rows.clone();
            } else {
                app.hits.side_issue_area = area;
                app.hits.side_issue_rows = rows.clone();
            }
            let click = wheel(MouseEventKind::Down(MouseButton::Left), 5, y);

            handle_mouse(&mut app, click);
            assert_eq!(app.nav.pane, pane);
            assert_eq!(app.nav.current, Some(9));
            assert!(!app.nav.visited.contains(&9));
            assert_eq!(if is_pr { app.pr_rows() } else { app.issue_rows() }, rows);

            handle_mouse(&mut app, click);
            assert!(app.nav.visited.contains(&9));
            assert_eq!(app.nav.current, Some(9));
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
        app.refs.show = false;
        assert!(!app.refs_visible());
        assert_eq!(wheel_target(&app, 95, 5), WheelTarget::Main);
    }

    #[test]
    fn wheel_over_issues_hovers_without_visiting() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(app.nav.hover, Some(8));
        assert_eq!(app.nav.current, Some(7), "wheel never visits");
        assert_eq!(app.nav.pane, crate::core::Pane::Main);
        assert_eq!(app.nav.scroll, 0, "body scroll untouched");
        // Wheeling back onto the current row dissolves the hover.
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(app.nav.hover, None);
        assert_eq!(app.nav.current, Some(7));
    }

    #[test]
    fn wheel_over_prs_hovers_pr_rows_only() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
        assert_eq!(app.nav.hover, Some(9));
        assert_eq!(app.nav.current, Some(7), "main content untouched");
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 25));
        assert_eq!(app.nav.hover, Some(11));
        assert_eq!(app.nav.current, Some(7));
        assert_eq!(app.nav.pane, crate::core::Pane::Main);
        assert_eq!(app.nav.scroll, 0);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 5, 25));
        assert_eq!(app.nav.hover, Some(9));
        assert_eq!(app.nav.current, Some(7));
    }

    #[test]
    fn wheel_over_refs_moves_highlight_without_scrolling_body() {
        let mut app = wheel_app();
        // A mentioning line below the viewport: the old click-style jump
        // would move body scroll here; the wheel must not.
        app.hits.body_rows = vec!["unrelated".to_string(), "context".to_string(), "fixes #22".to_string()];
        app.hits.body_base = 5;
        app.nav.scroll = 9;
        assert_eq!(app.refs.highlight, None);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
        assert_eq!(app.refs.highlight, Some(21));
        assert_eq!(app.nav.scroll, 9, "body stays where it was");
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 95, 5));
        assert_eq!(app.refs.highlight, Some(22));
        assert_eq!(app.nav.scroll, 9, "mention jump is click-only");
        assert_eq!(app.nav.pane, crate::core::Pane::Main);
        assert_eq!(app.nav.current, Some(7));
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 95, 5));
        assert_eq!(app.refs.highlight, Some(21));
        assert_eq!(app.nav.scroll, 9);
    }

    #[test]
    fn jk_promotes_hover_to_current() {
        let mut app = wheel_app();
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(app.nav.hover, Some(8));
        app.nav.pane = crate::core::Pane::Issues;
        handle_key(&mut app, crossterm::event::KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()));
        assert_eq!(app.nav.current, Some(7), "k walks back from the hover");
        assert_eq!(app.nav.hover, None, "visiting clears the hover");
    }

    #[test]
    fn wheel_over_body_scrolls_and_keeps_issue() {
        let mut app = wheel_app();
        app.nav.hover = Some(8);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
        assert_eq!(app.nav.scroll, 3);
        assert_eq!(app.nav.current, Some(7));
        assert_eq!(app.nav.hover, Some(8), "body wheel leaves the hover alone");
        assert_eq!(app.nav.pane, crate::core::Pane::Main);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollUp, 40, 5));
        assert_eq!(app.nav.scroll, 0);
    }

    #[test]
    fn wheel_over_body_on_diff_tab_leaves_diff_line_alone() {
        let mut app = wheel_app();
        // Only PRs show a diff; issues show the graph tab instead.
        app.show_issue(9);
        app.nav.tab = crate::app::Tab::Visual;
        assert!(app.visual_diff());
        app.diff.line = Some(5);
        handle_mouse(&mut app, wheel(MouseEventKind::ScrollDown, 40, 5));
        assert_eq!(app.nav.scroll, 3);
        assert_eq!(app.diff.line, Some(5));
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
            app.issues.data.insert(n, sample(n));
            app.issues.statuses.insert(n, vec![]);
        }
        app.ensure_visited(7);
        app.load_issue(7, false);
        app.ensure_visited(9);
        app.load_issue(9, false);
        assert_eq!(app.nav.current, Some(9));

        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert_eq!(app.nav.current, Some(7));

        // Ctrl+I arrives as Tab on terminals without the kitty protocol.
        handle_key(&mut app, ctrl_key(KeyCode::Tab));
        assert_eq!(app.nav.current, Some(9));

        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
        assert_eq!(app.nav.current, Some(9));

        // Plain Tab (no Ctrl) still cycles panes instead of redoing.
        app.nav.pane = crate::core::Pane::Issues;
        handle_key(
            &mut app,
            crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
        );
        assert_eq!(app.nav.pane, crate::core::Pane::PRs);
        assert_eq!(app.nav.current, Some(9));
    }

    fn plain_key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
    }

    fn compose_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.issues.data.insert(7, sample(7));
        app.issues.statuses.insert(7, vec![]);
        app.nav.visited.push(7);
        app.show_issue(7);
        app
    }

    #[test]
    fn shift_r_reopens_reauth_only_while_degraded() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('R')));
        assert!(app.confirm.is_none(), "healthy key is a no-op");
        app.auth.dismissed = true;
        app.auth.degraded = true;
        handle_key(&mut app, plain_key(KeyCode::Char('R')));
        assert!(
            matches!(app.confirm.as_ref().map(|d| &d.kind), Some(crate::app::ConfirmKind::Reauth)),
            "degraded key reopens the dialog",
        );
    }

    #[test]
    fn c_opens_compose_and_esc_discards() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        assert!(app.compose_open());
        handle_key(&mut app, plain_key(KeyCode::Char('x')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "x");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(app.confirm_open(), "typed draft asks before discarding");
        assert!(app.compose_open());
        handle_key(&mut app, plain_key(KeyCode::Char('k')));
        assert!(app.compose_open() && !app.confirm_open(), "k keeps editing");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        handle_key(&mut app, plain_key(KeyCode::Char('d')));
        assert!(!app.compose_open(), "d discards");
    }

    #[test]
    fn esc_on_empty_draft_closes_silently() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.compose_open() && !app.confirm_open());
    }

    #[test]
    fn compose_ctrl_s_with_empty_draft_stays_open() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('s')));
        assert!(app.compose_open(), "empty draft never sends");
        assert!(!app.compose.draft.as_ref().unwrap().sending);
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
        app.compose.draft.as_ref().expect("compose").cursor
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
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "hello ");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "hello world");
        assert_eq!(compose_cursor(&app), 11);
        handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
        handle_key(&mut app, ctrl_key(KeyCode::Char('k')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "hello world");
    }

    #[test]
    fn compose_alt_d_deletes_the_word_ahead() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "foo bar");
        handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
        handle_key(&mut app, alt_key(KeyCode::Char('d')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, " bar");
        assert_eq!(compose_cursor(&app), 0, "cursor stays put");
        handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "foo bar");
    }

    #[test]
    fn compose_ctrl_u_kills_to_line_start() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "hello");
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, plain_key(KeyCode::Left));
        handle_key(&mut app, ctrl_key(KeyCode::Char('u')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "lo");
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
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "ac");
        assert_eq!(compose_cursor(&app), 1);
    }

    #[test]
    fn compose_ctrl_t_transposes_chars() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "ab");
        handle_key(&mut app, ctrl_key(KeyCode::Char('t')));
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "ba");
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
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "h");
    }

    #[test]
    fn compose_ctrl_i_inserts_tab_and_never_walks_history() {
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "a");
        handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
        type_text(&mut app, "b");
        assert_eq!(app.compose.draft.as_ref().unwrap().body, "a\tb");
        assert_eq!(app.nav.current, Some(7), "tab never leaves the modal");
        assert!(app.compose_open());
    }

    #[test]
    fn compose_ctrl_o_arms_editor_or_names_a_missing_editor() {
        // Serialized with every other env-mutating test process-wide.
        let _env = crate::cache::test_env_lock();
        let prior = std::env::var("EDITOR").ok();
        std::env::remove_var("EDITOR");
        let mut app = compose_app();
        handle_key(&mut app, plain_key(KeyCode::Char('c')));
        type_text(&mut app, "draft");
        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert!(!app.compose.editor_requested, "nothing to run without an editor");
        assert!(app.message.contains("EDITOR"), "toast names the fix: {}", app.message);
        assert!(app.compose_open(), "draft survives");

        std::env::set_var("EDITOR", "true");
        handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
        assert!(app.compose.editor_requested, "ctrl-o arms the editor run");
        assert!(app.compose_open(), "draft survives until the run lands");
        match prior {
            Some(v) => std::env::set_var("EDITOR", v),
            None => std::env::remove_var("EDITOR"),
        }
    }

    fn pending_dialog_app() -> App {
        let mut app = compose_app();
        app.issues.data.insert(9, sample(9));
        app.issues.statuses.insert(9, vec![]);
        app.nav.visited.push(9);
        let d = app.issues.data.get_mut(&7).unwrap();
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
        assert_eq!(app.nav.current, Some(9));
        assert!(app.compose.submitting.is_empty());
        // Submit navigates and submits.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Char('s')));
        assert!(!app.confirm_open());
        assert_eq!(app.nav.current, Some(9));
        assert!(app.compose.submitting.contains(&7));
        // Enter picks the first choice (Submit).
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Enter));
        assert!(!app.confirm_open());
        assert!(app.compose.submitting.contains(&7));
        // Esc cancels: no navigation, nothing sent.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.confirm_open());
        assert_eq!(app.nav.current, Some(7));
        assert!(app.compose.submitting.is_empty());
        // Quit never escapes the dialog.
        let mut app = pending_dialog_app();
        handle_key(&mut app, plain_key(KeyCode::Char('q')));
        assert!(!app.quit);
        assert!(app.confirm_open());
    }

    #[test]
    fn shift_s_submits_pending_review() {
        let mut app = compose_app();
        let d = app.issues.data.get_mut(&7).unwrap();
        d.is_pr = true;
        d.review_threads = vec![crate::model::ReviewThread {
            id: "t1".into(),
            path: "a.rs".into(),
            line: Some(1),
            comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
            ..Default::default()
        }];
        handle_key(&mut app, plain_key(KeyCode::Char('S')));
        assert!(app.compose.submitting.contains(&7), "S starts the submit");
    }

    fn step_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in [7u64, 8] {
            app.issues.data.insert(n, sample(n));
            app.issues.statuses.insert(n, vec![]);
            app.nav.visited.push(n);
        }
        app.show_issue(7);
        app.nav.pane = Pane::Main;
        app
    }

    /// Which handler owns keys, per open layer. Written against the flag
    /// storage and `handle_key` only, so it pins behavior across the
    /// focus-model migration.
    #[test]
    fn top_layer_owns_keys_and_esc_pops_exactly_one() {
        use crate::core::{Dialog, Layer, TextInput};
        let mut app = step_app();
        app.nav.pane = Pane::Issues;
        assert_eq!(app.focus().layer, Layer::Base);

        // Shortcuts swallow navigation; Esc closes.
        app.shortcuts_open = true;
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.nav.current, Some(7), "shortcuts swallow l");
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Shortcuts));
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.shortcuts_open);

        // Confirm swallows everything, quit included; Esc cancels.
        app.open_pending_nav(crate::app::NavIntent::Back(1));
        handle_key(&mut app, plain_key(KeyCode::Char('q')));
        assert!(!app.quit, "quit never escapes the confirm");
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Confirm));
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.confirm_open());

        // Compose takes typing; Esc on a typed draft asks before it
        // discards the whole modal.
        app.open_compose();
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Compose));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.nav.current, Some(7), "compose types, never steps");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Confirm));
        handle_key(&mut app, plain_key(KeyCode::Char('d')));
        assert!(!app.compose_open());

        // Format dialog: Esc closes from the dialog, but a text row pops
        // only itself and leaves the dialog open.
        app.open_sidebar_format();
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Format));
        app.start_query_edit();
        assert_eq!(app.focus().layer, Layer::Input(TextInput::Query));
        handle_key(&mut app, plain_key(KeyCode::Char('x')));
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(app.sidebar.filter_open && !app.sidebar.query_edit, "Esc pops the text row");
        assert_eq!(app.sidebar.query_draft, app.sidebar.query_active, "draft reverts");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.sidebar.filter_open);

        // Bar: Esc leaves it and clears the text.
        handle_key(&mut app, plain_key(KeyCode::Char(':')));
        assert_eq!(app.focus().layer, Layer::Input(TextInput::Bar));
        handle_key(&mut app, plain_key(KeyCode::Char('1')));
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert_eq!(app.focus().layer, Layer::Base);
        assert!(app.bar.is_empty());
    }

    #[test]
    fn board_esc_clears_filter_then_closes_and_filter_form_esc_clears_and_closes() {
        use crate::core::{Dialog, Layer};
        let mut app = step_app();
        app.board.loading = true;
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Board));
        app.board.filter = "bug".to_string();
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(board_open(&app), "first Esc only clears the filter");
        assert!(app.board.filter.is_empty());
        app.open_board_filter();
        assert_eq!(app.focus().layer, Layer::Dialog(Dialog::BoardFilter));
        app.board.filter = "bug".to_string();
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.sidebar.filter_open, "form Esc closes the form");
        assert!(app.board.filter.is_empty(), "and clears the filter (differs from Format)");
        assert!(board_open(&app), "the board stays up");
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!board_open(&app));
    }

    #[test]
    fn history_pane_keys_walk_and_go_back() {
        let mut app = step_app();
        app.nav.history = vec![5, 6, 8];
        app.refs.show = true;
        assert!(app.history_visible());
        app.nav.pane = Pane::History;
        // k moves up toward older rows, j back down; both clamp.
        handle_key(&mut app, plain_key(KeyCode::Char('j')));
        assert_eq!(app.history_steps(), 0);
        handle_key(&mut app, plain_key(KeyCode::Char('k')));
        handle_key(&mut app, plain_key(KeyCode::Char('k')));
        assert_eq!(app.history_steps(), 2);
        for _ in 0..9 {
            handle_key(&mut app, plain_key(KeyCode::Char('k')));
        }
        assert_eq!(app.history_steps(), 3, "clamped to the rows drawn");
        // Enter goes back that many steps, like clicking the row.
        handle_key(&mut app, plain_key(KeyCode::Enter));
        assert_eq!(app.nav.history, Vec::<u64>::new());
        assert_eq!(app.nav.history_cursor, 0);
    }

    #[test]
    fn hl_steps_between_neighbors_on_main() {
        let mut app = step_app();
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.nav.current, Some(8), "l opens the next issue");
        assert_eq!(app.sidebar.issue_state.selected(), Some(2));
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        assert_eq!(app.nav.current, Some(7), "h opens the previous issue");
        assert_eq!(app.sidebar.issue_state.selected(), Some(1));
        assert_eq!(app.nav.step_pending, None, "cached steps never linger");
    }

    #[test]
    fn hl_steps_from_every_pane() {
        // The neighbor step works from every pane: nothing else claims
        // h/l outside Graph.
        let mut app = step_app();
        for pane in [Pane::Main, Pane::Issues, Pane::PRs, Pane::Refs] {
            app.nav.pane = pane;
            app.message.clear();
            handle_key(&mut app, plain_key(KeyCode::Char('l')));
            assert_eq!(app.nav.current, Some(8), "l steps from {pane:?}");
            assert_eq!(app.nav.step_pending, None, "cached steps never linger");
            handle_key(&mut app, plain_key(KeyCode::Char('h')));
            assert_eq!(app.nav.current, Some(7), "h steps back from {pane:?}");
            assert!(app.message.is_empty(), "no toast on a live neighbor");
        }
    }

    #[test]
    fn hl_on_main_without_an_issue_toasts() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.nav.pane = Pane::Main;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        assert_eq!(app.message, "view an issue first");
        assert!(app.issues.pending.is_empty(), "no fetch without an issue");
    }

    #[test]
    fn bar_filter_and_board_keep_their_h_and_l() {
        let mut app = step_app();
        // The command bar types the keys as text.
        app.nav.focus = crate::app::Focus::Bar;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.bar, "hl");
        assert_eq!(app.nav.current, Some(7));
        // The filter dialog swallows them.
        app.nav.focus = crate::app::Focus::Main;
        app.sidebar.filter_open = true;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.nav.current, Some(7));
        assert_eq!(app.nav.step_pending, None);
        // The board modal moves columns instead of stepping issues.
        app.sidebar.filter_open = false;
        app.board.loading = true;
        handle_key(&mut app, plain_key(KeyCode::Char('h')));
        handle_key(&mut app, plain_key(KeyCode::Char('l')));
        assert_eq!(app.nav.current, Some(7), "board keeps h/l");
        assert_eq!(app.nav.step_pending, None);
        assert!(app.board.loading, "board stays open");
    }

    #[test]
    fn auth_prompt_y_requests_reauth_and_n_dismisses() {
        let mut app = step_app();
        app.open_reauth(
            "token lacks scopes ghit needs".to_string(),
            vec!["project".to_string()],
            true,
        );
        handle_key(&mut app, plain_key(KeyCode::Char('y')));
        assert!(app.auth.reauth_requested, "y arms the refresh");
        assert!(app.confirm.is_some(), "loop clears the modal after running");
        let mut app = step_app();
        app.open_reauth(
            "GitHub auth check failed: 401 Unauthorized".to_string(),
            vec![],
            false,
        );
        handle_key(&mut app, plain_key(KeyCode::Char('n')));
        assert!(!app.auth.reauth_requested, "n never refreshes");
        assert!(app.confirm.is_none(), "n dismisses");
        assert!(matches!(app.nav.focus, crate::app::Focus::Main), "hint must render");
        assert!(app.message.contains("gh auth refresh"), "hint: {}", app.message);
    }

    #[test]
    fn auth_esc_dismisses_like_n() {
        let mut app = step_app();
        app.open_reauth(
            "GitHub auth check failed: 401 Unauthorized".to_string(),
            vec![],
            false,
        );
        handle_key(&mut app, plain_key(KeyCode::Esc));
        assert!(!app.auth.reauth_requested, "esc never refreshes");
        assert!(app.confirm.is_none(), "esc dismisses");
        assert!(app.auth.dismissed, "esc counts as dismissed");
        assert!(app.message.contains("gh auth refresh"), "hint: {}", app.message);
    }
}
