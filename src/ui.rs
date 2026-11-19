//! Look-and-feel: all rendering. Reads `App`, never fetches.
//!
//! Colors come only from `theme::` functions so `t` repaints everything.
//! Hit areas (`body_rows`, sidebar rects, board cells) are recorded here
//! during `render`; `input` consumes them on the next event.

mod board;
mod compose;
mod controls;
mod dialogs;
mod diff;
mod finder;
mod help;
mod hints;
mod main_pane;
mod map;
mod map_card;
mod map_strip;
mod search;
mod sidebar;
mod status;

#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod compose_tests;
#[cfg(test)]
mod dialogs_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod finder_tests;
#[cfg(test)]
mod help_tests;
#[cfg(test)]
mod hint_tests;
#[cfg(test)]
mod main_pane_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod mouse_tests;
#[cfg(test)]
mod render_tests;
#[cfg(test)]
mod sidebar_tests;
#[cfg(test)]
mod status_tests;
#[cfg(test)]
mod test_support;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;

use crate::app::App;
use crate::model::self;
use crate::theme;
use board::render_board;
use compose::render_compose;
use dialogs::{render_confirm, render_filter};
use diff::diff_header_offset;
use main_pane::render_main;
use sidebar::{render_sidebar, render_refs, render_labels, render_history};
use status::render_bar;

pub(crate) fn render(f: &mut ratatui::Frame, app: &mut App) {
    use crate::theme::ink;
    f.render_widget(Block::default().style(Style::new().bg(ink())), f.area());
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    if let Some((fi, delta)) = app.diff.pending_jump.take() {
        let offset = diff_header_offset(app, fi, delta);
        app.finish_diff_jump(delta, offset);
    }
    app.hits.controls.clear();
    app.hits.legend.clear();
    app.hits.popup = Rect::default();
    app.hits.menu_rows.clear();
    app.hits.finder_rows.clear();
    app.hits.history_rows.clear();
    app.hits.history_area = Rect::default();
    app.hits.refs_area = Rect::default();
    app.hits.labels_area = Rect::default();
    app.hits.labels_rows.clear();
    app.hits.side_issue_area = Rect::default();
    app.hits.side_pr_area = Rect::default();
    app.hits.map_area = Rect::default();
    app.hits.map_nodes.clear();
    app.hits.map_edges.clear();
    app.hits.map_card = Rect::default();
    app.hits.map_card_hits.clear();
    app.hits.map_strip = Rect::default();
    app.hits.map_strip_rows.clear();
    let (sidebar_w, right_w) = (app.layout.sidebar, app.layout.right);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(sidebar_w),
            Constraint::Min(1),
            Constraint::Length(right_w),
        ])
        .split(root[0]);
    if app.sidebar_visible() {
        render_sidebar(f, cols[0], app);
    }
    render_main(f, cols[1], app);
    if app.right_visible() {
        let count = if app.history_visible() { app.history_rows_back() + 1 } else { 0 };
        let height = if count > 0 { (count as u16 + 2).min(cols[2].height) } else { 0 };
        // Refs and Labels split what History leaves; Labels takes it all
        // when there are no refs.
        let avail = cols[2].height - height;
        let refs_height = match (app.refs_visible(), app.labels_visible()) {
            // Labels takes only what its rows need, up to half; Refs gets the rest.
            (true, true) => avail - (app.filter_items().len() as u16 + 4).min(avail / 2),
            (true, false) => avail,
            (false, _) => 0,
        };
        let panes = Layout::default().direction(Direction::Vertical)
            .constraints([
                Constraint::Length(refs_height),
                Constraint::Min(0),
                Constraint::Length(height),
            ]).split(cols[2]);
        if app.refs_visible() {
            render_refs(f, panes[0], app);
        }
        if app.labels_visible() {
            render_labels(f, panes[1], app);
        }
        if height > 0 {
            render_history(f, panes[2], app);
        }
    }
    app.capture_body_rows(f.buffer_mut());
    // Diff tab bookkeeping from the fresh capture. Only the Diff tab
    // renders `▸ `/`▾ ` markers. Headers match by filename (longest wins)
    // so wrapped rows and scrolled-off files resolve exactly; every other
    // row inherits the previous header's file, pre-header rows map to None.
    if app.visual_diff() {
        let header_rows: Vec<(usize, usize)> = {
            let empty: Vec<model::DiffFile> = vec![];
            let files = app.nav
                .current
                .and_then(|n| app.diff.files.get(&n))
                .map(|fs| fs.as_slice())
                .unwrap_or(&empty);
            app.hits.body_rows
                .iter()
                .enumerate()
                .filter_map(|(i, line)| model::header_file_index(line, files).map(|fi| (i, fi)))
                .collect()
        };
        app.diff.headers = header_rows.iter().map(|(i, _)| *i).collect();
        let mut row_files: Vec<Option<usize>> = vec![None; app.hits.body_rows.len()];
        let mut cur: Option<usize> = None;
        let mut hi = 0;
        for (i, slot) in row_files.iter_mut().enumerate() {
            while hi < header_rows.len() && header_rows[hi].0 == i {
                cur = Some(header_rows[hi].1);
                hi += 1;
            }
            *slot = cur;
        }
        app.diff.row_files = row_files;
        // Resolve the active line against the fresh capture: track the
        // owning file, and unfold a collapsed file landed on by input.
        // The lazy init anchor never expands: opening a Diff stays folded.
        match app.diff.line {
            None => {
                if let Some(&h) = app.diff.headers.first() {
                    app.diff.line = Some(app.hits.body_base as usize + h);
                }
            }
            Some(abs) => {
                let vis = abs.saturating_sub(app.hits.body_base as usize);
                let landed = app.diff.row_files.get(vis).copied().flatten();
                if let Some(f) = landed {
                    app.diff.cursor = f;
                    if app.diff.touched {
                        let key = app.nav.current.and_then(|n| {
                            app.diff.files
                                .get(&n)
                                .and_then(|fs| fs.get(f))
                                .map(|file| (n, file.filename.clone()))
                        });
                        if let Some(k) = key {
                            app.diff.expanded_files.insert(k);
                        }
                    }
                }
                if vis < app.diff.row_files.len() {
                    app.diff.touched = false;
                }
            }
        }
        // Active-line highlight, painted on the buffer after the paragraph
        // so wrapped rows light up exactly. Only the background tints, so
        // add/del colors stay readable on the active row.
        if let Some(abs) = app.diff.line {
            use crate::theme::line_hl;
            let vis = abs.saturating_sub(app.nav.scroll as usize);
            if vis < app.hits.body_area.height as usize {
                let y = app.hits.body_area.y + vis as u16;
                let buf = f.buffer_mut();
                for x in app.hits.body_area.x..app.hits.body_area.x + app.hits.body_area.width {
                    let cell = &mut buf[(x, y)];
                    cell.set_bg(line_hl());
                    // Text that matches the tint (muted on the system
                    // theme) would vanish; lift it to body text.
                    if cell.fg == line_hl() {
                        cell.set_fg(crate::theme::paper());
                    }
                }
            }
        }
    } else {
        app.diff.headers = vec![];
        app.diff.row_files = vec![];
    }

    app.hits.bar_area = root[1];
    render_bar(f, root[1], app);

    if crate::app::board_open(app) {
        dim_background(f, 0.35);
        render_board(f, f.area(), app);
    }
    if app.format_open() {
        dim_background(f, 0.35);
        render_filter(f, f.area(), app);
    }
    if app.physics_focus().is_some() {
        dim_background(f, 0.35);
        dialogs::render_physics(f, f.area(), app);
    }
    controls::render_menu(f, f.area(), app);
    if app.finder().is_some() {
        dim_background(f, 0.35);
        finder::render_finder(f, f.area(), app);
    }
    if app.shortcuts_open() {
        dim_background(f, 0.35);
        help::render_shortcuts(f, f.area(), app);
    }
    if app.compose_open() {
        dim_background(f, 0.35);
        render_compose(f, f.area(), app);
    }
    if app.confirm_open() {
        dim_background(f, 0.35);
        render_confirm(f, f.area(), app);
    }
    paint_click_flash(f, app);
}

/// Click flash: invert the last-clicked rect for `CLICK_FLASH_MS` so the
/// press reads instantly, even before its fetch lands. Reversed style
/// only, no colors, so every theme flashes. Style-only and painted after
/// the frame capture above, so `body_rows` hit areas stay intact;
/// an expired flash clears here, queuing nothing.
fn paint_click_flash(f: &mut ratatui::Frame, app: &mut App) {
    if !app.flash_visible() {
        app.hits.last_click = None;
        return;
    }
    let Some((rect, _)) = app.hits.last_click else { return };
    let area = f.area();
    let x0 = rect.x.max(area.x);
    let y0 = rect.y.max(area.y);
    let x1 = (rect.x + rect.width).min(area.x + area.width);
    let y1 = (rect.y + rect.height).min(area.y + area.height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let buf = f.buffer_mut();
    for y in y0..y1 {
        for x in x0..x1 {
            buf[(x, y)].set_style(Style::new().reversed());
        }
    }
}

/// Darken every rendered cell toward the background so the modal sits proud.
fn dim_background(f: &mut ratatui::Frame, amount: f32) {
    let Color::Rgb(ir, ig, ib) = crate::theme::ink() else {
        return;
    };
    for cell in f.buffer_mut().content.iter_mut() {
        match cell.fg {
            Color::Rgb(r, g, b) => {
                let mix = |c: u8, i: u8| (i as f32 + (c as f32 - i as f32) * amount) as u8;
                cell.set_fg(Color::Rgb(mix(r, ir), mix(g, ig), mix(b, ib)));
            }
            Color::Reset => {}
            _ => {
                cell.set_fg(crate::theme::dim_fg());
            }
        }
    }
}

fn section_title<'a>(name: &str, detail: &str, icon: &str, active: bool) -> Line<'a> {
    use crate::theme::{amber, muted};
    // Inactive panes go quiet: the colored name drops to the detail grey
    // so the active pane is obvious at a glance.
    let (icon_fg, name_style) = if active {
        (muted(), Style::new().fg(amber()).bold())
    } else {
        (muted(), Style::new().fg(muted()))
    };
    let icon_span = Span::styled(format!(" {icon} "), Style::new().fg(icon_fg));
    let name_span = Span::styled(format!("{name} "), name_style);
    let detail_span = Span::styled(detail.to_string(), Style::new().fg(muted()));
    Line::from(vec![icon_span, name_span, detail_span])
}

/// Braille frame for the current tick; the bar and pane titles share it.
fn spinner_frame(tick: u64) -> &'static str {
    SPINNER[(tick as usize) % SPINNER.len()]
}

/// Append the spinner to a pane title while that pane's data is in flight.
fn spinning(mut title: Line<'static>, loading: bool, tick: u64) -> Line<'static> {
    if loading {
        title.spans.push(Span::styled(
            format!(" {}", spinner_frame(tick)),
            Style::new().fg(theme::amber()),
        ));
    }
    title
}

/// Body spinner frames: a heavier braille ring than the title spinner.
const BODY_SPINNER: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// Body spinner glyph for the current tick (same tick as the title).
fn body_spinner_frame(tick: u64) -> &'static str {
    BODY_SPINNER[(tick as usize) % BODY_SPINNER.len()]
}

/// Loading placeholder for a pane body: one centered `<spinner> <label>`
/// line about a third of the way down `height` rows.
fn loading_body(label: &str, tick: u64, height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(""); height.saturating_sub(1) / 3];
    lines.push(
        Line::from(vec![
            Span::styled(body_spinner_frame(tick), Style::new().fg(theme::amber())),
            Span::styled(format!(" {label}"), Style::new().fg(theme::muted())),
        ])
        .centered(),
    );
    lines
}

/// Bottom border label for a list pane: per-group counts in the same
/// muted voice as the title subtitle. Overlong footers clip at the
/// border; callers order counts before context so counts survive.
fn footer_line(text: &str) -> Line<'static> {
    use crate::theme::muted;
    Line::from(Span::styled(text.to_string(), Style::new().fg(muted())))
}

/// Status bar spinner frames, stepped by the event-loop tick. Braille
/// dots read as motion at a glance and stay one cell wide.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Popup rect sized to `content` rows plus borders, clamped to the
/// area and centered. Short terminals get a short popup whose content
/// scrolls instead of a clipped one.
fn sized_popup(area: Rect, pct_x: u16, content: usize) -> Rect {
    let height = (content.saturating_add(2) as u16).min(area.height.max(1));
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v)[1]
}

/// Paragraph scroll offset that keeps `focus_line` visible inside
/// `inner`. The text row edits from row 0, so editing pins the top.
fn scroll_to(inner: Rect, focus_line: usize, editing: bool) -> u16 {
    if editing {
        return 0;
    }
    focus_line.saturating_sub(inner.height.saturating_sub(2) as usize) as u16
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v)[1]
}
