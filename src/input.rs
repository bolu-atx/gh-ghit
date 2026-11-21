//! Input: key and mouse handling. Translates events into `App` calls.
//!
//! Reads hit areas recorded by `ui::render`; never draws. Selection and
//! cursor math delegates to `core` so the rules stay testable without a TUI.

mod actions;
mod click;
mod finder;
mod hit;
mod map;
mod menu;
mod mouse;
mod reach;
mod settings;

#[cfg(test)]
mod parity;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod click_tests;
#[cfg(test)]
mod mouse_tests;
#[cfg(test)]
mod tests;

use crossterm::event::{KeyCode, KeyModifiers};
use crate::app::App;
use crate::core::{self, Dialog, Layer, Pane, TextInput};
use crate::keymap;

pub(crate) use mouse::handle_mouse;

pub(crate) fn handle_key(app: &mut App, key: crossterm::event::KeyEvent) {
    let chain = keymap::active_contexts(app);
    // A waiting prefix takes the next key; any other key drops the
    // prefix and runs as usual.
    if let Some(prefix) = app.pending_prefix.take() {
        if let Some(binding) = keymap::lookup_after(&chain, prefix, key, app) {
            actions::perform(app, binding.action);
            return;
        }
    }
    match keymap::lookup(&chain, key, app) {
        Some(binding) => actions::perform(app, binding.action),
        None => match keymap::sequence_start(&chain, key, app) {
            Some(prefix) => app.pending_prefix = Some(prefix),
            None => type_text(app, key),
        },
    }
}

/// Printable keys no binding claimed. Text layers type them; every other
/// layer swallows them, so nothing behind a dialog fires.
fn type_text(app: &mut App, key: crossterm::event::KeyEvent) {
    let KeyCode::Char(c) = key.code else { return };
    let chord = key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    match app.focus().layer {
        Layer::Input(TextInput::Bar) if app.search.typing => app.content_search_push(c),
        Layer::Input(TextInput::Bar) if app.sidebar.searching => app.sidebar.search.push(c),
        Layer::Input(TextInput::Bar) => app.bar.push(c),
        Layer::Input(TextInput::Query | TextInput::BoardQuery) => app.sidebar.query_draft.push(c),
        Layer::Input(TextInput::Name) => app.sidebar.name_draft.push(c),
        Layer::Dialog(Dialog::Finder) if !chord => app.finder_type(c),
        Layer::Dialog(Dialog::Shortcuts) if app.help_typing && !chord => {
            app.help_filter.push(c);
            app.help_scroll = 0;
        }
        // The editor never types a control chord into the draft.
        Layer::Dialog(Dialog::Compose) if !chord => app.compose_insert(c),
        Layer::Dialog(Dialog::Confirm) => {
            let lower = c.to_ascii_lowercase();
            if let Some(index) = app.confirm()
                .and_then(|d| d.choices.iter().position(|ch| ch.key == lower))
            {
                app.confirm_choose(index);
            }
        }
        // Typing on the text row starts editing, like the old dialog.
        Layer::Dialog(Dialog::BoardFilter) if app.board.filter_focus == 0 && !chord => {
            app.set_format_edit(crate::app::FormatEdit::Query);
            app.sidebar.query_draft.push(c);
        }
        _ => {}
    }
}

/// j/k walks one sidebar list: issues or PRs, never crossing over.
/// Starts from the wheel hover when one names a row here, so keyboard
/// and wheel continue from the same place; the visit clears the hover.
pub(super) fn move_in_list(app: &mut App, order: Vec<u64>, delta: isize) {
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
pub(super) fn toggle_board_focused(app: &mut App) {
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
