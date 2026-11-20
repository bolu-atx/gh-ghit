//! Clickable controls drawn outside rows: sidebar title and footer
//! controls, the Filters "clear all", the Main toolbar and the actions menu.
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

/// Pane buttons drawn over a border row, left-aligned: an amber key and
/// a label on a raised background. Only the button cells are drawn, so
/// the border shows between and after them. Each runs its action on click.
pub(super) fn button_row(f: &mut ratatui::Frame, app: &mut App, row: Rect, buttons: &[(&str, Action)], pane: Pane) {
    use crate::theme::line_hl;
    let mut x = row.x + 1;
    for (label, action) in buttons {
        let key = crate::keymap::describe(*action).map(|b| b.key_name()).unwrap_or_default();
        let width = (key.chars().count() + label.chars().count() + 3) as u16;
        if x + width > row.right() {
            break;
        }
        let rect = Rect::new(x, row.y, width, 1);
        let line = Line::from(vec![
            Span::styled(format!(" {key}"), Style::new().bg(line_hl()).fg(amber()).bold()),
            Span::styled(format!(" {label} "), Style::new().bg(line_hl()).fg(paper())),
        ]);
        f.render_widget(Paragraph::new(line), rect);
        app.hits.controls.push((rect, *action, pane));
        x += width + 1;
    }
}

/// One toolbar button: its glyph, short label and action.
struct Tool {
    icon: &'static str,
    label: &'static str,
    action: Action,
}

impl Tool {
    /// The key bound to the action, read from the keymap.
    fn key(&self) -> String {
        crate::keymap::describe(self.action).map(|b| b.key_name()).unwrap_or_default()
    }

    /// Cells the button takes: key, icon, and the label when `labeled`.
    fn width(&self, labeled: bool) -> u16 {
        let label = if labeled { 1 + self.label.chars().count() } else { 0 };
        (self.key().chars().count() + 1 + 1 + label) as u16
    }
}

/// Toolbar buttons, left to right. The split toggle shows on a PR's Diff
/// view only, the scope chip and fit on the map; comment and close need
/// an open item, and a merged PR cannot close or reopen.
fn tools(app: &App) -> Vec<Tool> {
    let mut tools = vec![];
    if app.visual_diff() {
        tools.push(Tool { icon: theme::TOOL_SPLIT, label: "split", action: Action::DiffSplit });
        let (icon, label) =
            if app.diff_will_expand() { (theme::TOOL_EXPAND, "expand") } else { (theme::TOOL_FOLD, "fold") };
        tools.push(Tool { icon, label, action: Action::DiffExpandAll });
        tools.push(Tool { icon: theme::TOOL_CONTEXT, label: app.diff_context_label(), action: Action::DiffContext });
    }
    if app.map_shown() {
        let label = if app.map.center().is_some() { "linked" } else { "repo" };
        tools.push(Tool { icon: theme::TOOL_SCOPE, label, action: Action::GraphScope });
        // On the map, `OpenFormat` opens the map filter.
        tools.push(Tool { icon: theme::TOOL_FILTER, label: "filter", action: Action::OpenFormat });
        tools.push(Tool { icon: theme::TOOL_FIT, label: "fit", action: Action::GraphFit });
        tools.push(Tool { icon: theme::TOOL_PHYSICS, label: "physics", action: Action::GraphPhysics });
        tools.push(Tool { icon: theme::TOOL_RANK, label: "ranks", action: Action::GraphStrip });
    }
    let Some(data) = app.nav.current.and_then(|n| app.issues.data.get(&n)) else { return tools };
    tools.push(Tool { icon: theme::TOOL_COMMENT, label: "comment", action: Action::Comment });
    match data.state.as_str() {
        "MERGED" => {}
        "CLOSED" => tools.push(Tool { icon: theme::TOOL_REOPEN, label: "reopen", action: Action::CloseItem }),
        _ => tools.push(Tool { icon: theme::TOOL_CLOSE, label: "close", action: Action::CloseItem }),
    }
    tools
}

/// Gap between toolbar buttons, and the file-step arrows' own width.
const TOOL_GAP: u16 = 2;
const STEP_WIDTH: u16 = 3;

/// Cells each button takes, with labels or icon-only. The widest form
/// that fits `room` wins; `None` means even icons do not fit.
fn fit_tools(tools: &[Tool], steps: bool, room: u16) -> Option<(bool, Vec<u16>)> {
    for labeled in [true, false] {
        let widths: Vec<u16> = tools.iter().map(|t| t.width(labeled)).collect();
        let count = widths.len() as u16 + u16::from(steps);
        let total = widths.iter().sum::<u16>() + if steps { STEP_WIDTH } else { 0 } + count.saturating_sub(1) * TOOL_GAP;
        if total <= room {
            return Some((labeled, widths));
        }
    }
    None
}

/// Action toolbar on the tab line, right-aligned: file stepping on a PR's
/// Diff view, then split, comment and close or reopen. Buttons are an amber
/// key, a muted icon and a short label while the tabs leave room; the label
/// goes first when tight, and the whole bar when the tabs need the space.
pub(super) fn toolbar(f: &mut ratatui::Frame, tabs: Rect, app: &mut App) {
    let tools = tools(app);
    let steps = app.visual_diff();
    let counter = app.search.active().then(|| app.search.counter());
    // The match counter leads the toolbar: `‹ 3/12 ›` steps like `N`/`n`.
    let lead = counter.as_ref().map_or(0, |c| c.chars().count() as u16 + STEP_WIDTH + 1);
    // Titles have no pad cells, only a one-cell divider between them, so
    // one more cell keeps the last title off the first button.
    let titles = crate::app::TABS.map(|(_, title)| title);
    let used = titles.iter().map(|t| t.chars().count() as u16).sum::<u16>() + titles.len() as u16;
    let room = tabs.width.saturating_sub(used + 1);
    let room_for_tools = room.saturating_sub(if lead > 0 { lead + TOOL_GAP } else { 0 });
    let Some((labeled, widths)) = fit_tools(&tools, steps, room_for_tools) else { return };
    let total = widths.iter().sum::<u16>() + if steps { STEP_WIDTH } else { 0 } + (widths.len() as u16 + u16::from(steps)).saturating_sub(1) * TOOL_GAP
        + if lead > 0 { lead + TOOL_GAP } else { 0 };
    let mut x = tabs.right() - total - 1;
    let style = Style::new().bg(ink()).fg(muted());
    let mut rects = vec![];
    if let Some(counter) = counter {
        let w = counter.chars().count() as u16;
        f.render_widget(Paragraph::new(format!("‹ {counter} ›")).style(style), Rect::new(x, tabs.y, lead, 1));
        rects.push((Rect::new(x, tabs.y, 1, 1), Action::ContentPrev));
        rects.push((Rect::new(x + 2, tabs.y, w + 2, 1), Action::ContentNext));
        x += lead + TOOL_GAP;
    }
    if steps {
        f.render_widget(Paragraph::new("‹ ›").style(style), Rect::new(x, tabs.y, STEP_WIDTH, 1));
        rects.push((Rect::new(x, tabs.y, 1, 1), Action::DiffPrevFile));
        rects.push((Rect::new(x + 2, tabs.y, 1, 1), Action::DiffNextFile));
        x += STEP_WIDTH + TOOL_GAP;
    }
    for tool in &tools {
        let rect = Rect::new(x, tabs.y, tool.width(labeled), 1);
        let text = if labeled { format!(" {} {}", tool.icon, tool.label) } else { format!(" {}", tool.icon) };
        let line = Line::from(vec![Span::styled(tool.key(), Style::new().bg(ink()).fg(amber())), Span::styled(text, style)]);
        f.render_widget(Paragraph::new(line), rect);
        x += rect.width + TOOL_GAP;
        rects.push((rect, tool.action));
    }
    for (rect, action) in rects {
        app.hits.controls.push((rect, action, Pane::Main));
    }
}

/// Draws the open actions menu: one row per entry, the cursor row in the
/// same amber marker as a focused list, the key at the right edge.
pub(super) fn render_menu(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.menu_rows.clear();
    let Some(menu) = app.menu() else { return };
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
