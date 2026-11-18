//! Clickable controls drawn outside rows: sidebar title and footer
//! controls, the Filters "clear all", the diff strip and the actions menu.
//! Each records its rect in `app.hits` while it draws.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::App;
use crate::core::Pane;
use crate::keymap::Action;
use crate::theme::{self, amber, ink, muted, paper};

/// Rect of `width` cells at `x` on `y`, cut at the pane's right border.
fn clipped(area: Rect, x: u16, y: u16, width: u16) -> Option<Rect> {
    let right = area.right().saturating_sub(1);
    (x < right && width > 0).then(|| Rect::new(x, y, width.min(right - x), 1))
}

/// Records the controls of a sidebar pane: the list state opens Format,
/// the sort word cycles the sort, its arrow reverses it, and the footer
/// counts open Format. `title` is the title line before any spinner.
pub(super) fn side_controls(app: &mut App, area: Rect, title: &Line<'static>, want_pr: bool, pane: Pane) {
    let (state, sort) = app.side_subtitle_parts(want_pr);
    let detail = title.spans.last().map_or(0, Span::width);
    let x = area.x + 1 + (title.width() - detail) as u16;
    let state_w = state.chars().count() as u16;
    let sort_w = sort.chars().count() as u16;
    let mut rects = vec![(clipped(area, x, area.y, state_w), Action::OpenFormat)];
    let sort_x = x + state_w + 3;
    if sort_w > 1 && sort.ends_with(['↑', '↓']) {
        rects.push((clipped(area, sort_x, area.y, sort_w - 1), Action::CycleSort));
        rects.push((clipped(area, sort_x + sort_w - 1, area.y, 1), Action::ReverseSort));
    } else {
        rects.push((clipped(area, sort_x, area.y, sort_w), Action::CycleSort));
    }
    let footer_w = app.side_footer(want_pr).chars().count() as u16;
    rects.push((clipped(area, area.x + 1, area.bottom().saturating_sub(1), footer_w), Action::OpenFormat));
    for (rect, action) in rects {
        if let Some(rect) = rect {
            app.hits.controls.push((rect, action, pane));
        }
    }
}

/// Adds a muted "clear all" to the Filters title and records its rect.
pub(super) fn clear_all_control(app: &mut App, area: Rect, title: &mut Line<'static>) {
    title.spans.push(Span::styled(" · ", Style::new().fg(muted())));
    title.spans.push(Span::styled("clear all", Style::new().fg(muted())));
    let x = area.x + 1 + (title.width() - 9) as u16;
    if let Some(rect) = clipped(area, x, area.y, 9) {
        app.hits.controls.push((rect, Action::ClearLabels, Pane::Labels));
    }
}

/// Diff strip on the tab line, right-aligned: previous file, next file,
/// split toggle. It draws only when the tabs leave room.
pub(super) fn diff_strip(f: &mut ratatui::Frame, tabs: Rect, app: &mut App) {
    const TEXT: &str = "‹ › split";
    let width = TEXT.chars().count() as u16;
    let titles = crate::app::tab_titles("Diff View");
    let used = titles.iter().map(|t| t.chars().count() as u16).sum::<u16>() + 6;
    if tabs.width < used + width {
        return;
    }
    let x = tabs.right() - width - 1;
    let strip = Rect::new(x, tabs.y, width, 1);
    f.render_widget(Paragraph::new(TEXT).style(Style::new().bg(ink()).fg(muted())), strip);
    for (dx, w, action) in [(0, 1, Action::DiffPrevFile), (2, 1, Action::DiffNextFile), (4, 5, Action::DiffSplit)] {
        app.hits.controls.push((Rect::new(x + dx, tabs.y, w, 1), action, Pane::Main));
    }
}

/// Draws the open actions menu: one row per entry, the cursor row in the
/// same amber marker as a focused list, the key at the right edge.
pub(super) fn render_menu(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.menu_rows.clear();
    let Some(menu) = &app.menu else { return };
    let label_w = menu.entries.iter().map(|e| e.label.chars().count()).max().unwrap_or(0);
    let key_w = menu.entries.iter().map(|e| e.key.chars().count()).max().unwrap_or(0);
    let width = ((2 + label_w + 2 + key_w + 1 + 2) as u16).min(area.width);
    let height = (menu.entries.len() as u16 + 2).min(area.height);
    let x = menu.anchor.0.min(area.right().saturating_sub(width));
    let y = menu.anchor.1.min(area.bottom().saturating_sub(height));
    let popup = Rect::new(x, y, width, height);
    // The menu is the focused layer, so it wears the focused-pane frame.
    let block = theme::pane_block(true);
    let inner = block.inner(popup);
    f.render_widget(Clear, popup);
    f.render_widget(block, popup);
    let visible = inner.height as usize;
    let offset = (menu.cursor + 1).saturating_sub(visible);
    let mut rows = vec![];
    for (i, entry) in menu.entries.iter().enumerate().skip(offset).take(visible) {
        let active = i == menu.cursor;
        let state = if active { theme::RowState::Active } else { theme::RowState::Plain };
        let bg = theme::row_bg(state, true);
        let (marker, marker_style) = theme::row_marker(state, true, "█ ");
        let gap = " ".repeat((inner.width as usize).saturating_sub(3 + entry.label.chars().count() + entry.key.chars().count()));
        let rect = Rect::new(inner.x, inner.y + (i - offset) as u16, inner.width, 1);
        let label_style = if active { Style::new().bg(bg).fg(paper()).bold() } else { Style::new().bg(bg).fg(paper()) };
        let line = Line::from(vec![
            Span::styled(marker, marker_style.bg(bg)),
            Span::styled(entry.label, label_style),
            Span::styled(gap, Style::new().bg(bg)),
            Span::styled(entry.key.clone(), Style::new().bg(bg).fg(amber())),
            Span::styled(" ", Style::new().bg(bg)),
        ]);
        f.render_widget(Paragraph::new(line).style(Style::new().bg(bg)), rect);
        rows.push((rect, i));
    }
    app.hits.menu_rows = rows;
    app.hits.popup = popup;
}
