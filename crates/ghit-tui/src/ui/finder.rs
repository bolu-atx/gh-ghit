//! The Ctrl+P finder: a query row, ranked results with the matched
//! characters in amber, the key legend and a result count in the bottom
//! border.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, FinderRow};
use ghit_core::core::Dialog;
use crate::keymap::Context;
use crate::theme::{self, amber, ink, muted, paper, state_color};
use super::hints;

const POPUP_WIDTH: u16 = 72;
const MAX_ROWS: u16 = 16;
const TITLE: &str = " Go to issue or PR ";
/// Query row, blank line, then the results.
const CHROME: u16 = 2;

/// The popup fits its `results`, from three rows up to `MAX_ROWS`. Its
/// top edge stays put while it grows, so typing does not make it jump.
fn popup_rect(area: Rect, results: usize) -> Rect {
    let width = area.width.min(POPUP_WIDTH);
    let most = MAX_ROWS.min(area.height.saturating_sub(CHROME + 4));
    let rows = (results as u16).clamp(3, most.max(3));
    let top = area.y + area.height.saturating_sub(most + CHROME + 2) / 3;
    let height = (rows + CHROME + 2).min(area.height);
    Rect::new(area.x + (area.width - width) / 2, top, width, height)
}

/// Spans for `text`, the chars at `lit` (offsets into `text`) in amber.
fn highlighted(text: &str, lit: &[usize], base: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = vec![];
    let mut run = String::new();
    let mut run_lit = false;
    for (i, c) in text.chars().enumerate() {
        let on = lit.contains(&i);
        if on != run_lit && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), if run_lit { base.fg(amber()).bold() } else { base }));
        }
        run_lit = on;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_lit { base.fg(amber()).bold() } else { base }));
    }
    spans
}

fn result_line(row: &FinderRow, active: bool, number_width: usize, width: usize) -> Line<'static> {
    let state = if active { theme::RowState::Active } else { theme::RowState::Plain };
    let bg = theme::row_bg(state, true);
    let (marker, marker_style) = theme::row_marker(state, true, "█ ");
    let number = row.number.display("");
    let lead = number.chars().count();
    let tag = if row.is_pr { "PR" } else { "" };
    let room = width.saturating_sub(2 + number_width + 2 + 1 + tag.len() + 1);
    let title: String = if row.title.chars().count() > room {
        row.title.chars().take(room.saturating_sub(1)).chain(std::iter::once('…')).collect()
    } else {
        row.title.clone()
    };
    let number_lit: Vec<usize> = row.indices.iter().copied().filter(|i| *i < lead).collect();
    let title_lit: Vec<usize> = row.indices.iter().filter_map(|i| i.checked_sub(lead + 1)).collect();
    let weight = |style: Style| if active { style.bold() } else { style };
    let mut spans = vec![Span::styled(marker, marker_style.bg(bg))];
    spans.extend(highlighted(&number, &number_lit, Style::new().bg(bg).fg(state_color(&row.state)).bold()));
    spans.push(Span::styled(" ".repeat(number_width - lead + 2), Style::new().bg(bg)));
    let used = 2 + number_width + 2 + title.chars().count();
    spans.extend(highlighted(&title, &title_lit, weight(Style::new().bg(bg).fg(paper()))));
    let gap = width.saturating_sub(used + tag.len() + 1);
    spans.push(Span::styled(" ".repeat(gap), Style::new().bg(bg)));
    spans.push(Span::styled(tag, Style::new().bg(bg).fg(muted())));
    spans.push(Span::styled(" ", Style::new().bg(bg)));
    Line::from(spans)
}

/// Draws the finder when it is open and records the result rows.
pub(super) fn render_finder(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.finder_rows.clear();
    let Some(finder) = app.finder() else { return };
    let (query, cursor) = (finder.query.clone(), finder.cursor);
    let rows = app.finder_rows();
    let total = app.finder_total();

    let popup = popup_rect(area, rows.len());
    app.hits.popup = popup;
    let block = theme::pane_block(true);
    let inner = block.inner(popup);
    f.render_widget(Clear, popup);
    f.render_widget(block, popup);
    let title = Rect::new(popup.x + 1, popup.y, (TITLE.chars().count() as u16).min(popup.width.saturating_sub(2)), 1);
    f.render_widget(Paragraph::new(Span::styled(TITLE, Style::new().bg(ink()).fg(paper()).bold())), title);
    if inner.height < CHROME + 1 || inner.width < 8 {
        return;
    }

    let query_room = (inner.width as usize).saturating_sub(4);
    let shown: String = query.chars().skip(query.chars().count().saturating_sub(query_room)).collect();
    let mut head = vec![Span::raw(" "), Span::styled("› ", Style::new().bg(ink()).fg(amber()).bold())];
    head.push(Span::styled(shown.clone(), Style::new().bg(ink()).fg(paper())));
    f.render_widget(Paragraph::new(Line::from(head)), Rect::new(inner.x, inner.y, inner.width, 1));
    f.set_cursor_position((inner.x + 3 + shown.chars().count() as u16, inner.y));

    let list = Rect::new(inner.x, inner.y + CHROME, inner.width, inner.height - CHROME);
    if rows.is_empty() {
        let text = if total == 0 {
            "No issues or PRs loaded yet.".to_string()
        } else {
            format!("No issue or PR matches \"{query}\".")
        };
        let line = Line::from(vec![Span::raw("  "), Span::styled(text, Style::new().bg(ink()).fg(muted()))]);
        f.render_widget(Paragraph::new(line), Rect::new(list.x, list.y, list.width, 1));
    }
    if rows.len() < 3 && !query.is_empty() && list.height as usize > rows.len().max(1) {
        let key = app.github_key();
        let text = format!("  Not here? Esc, / for a query, then {key} searches GitHub.");
        let at = Rect::new(list.x, list.y + rows.len().max(1) as u16, list.width, 1);
        f.render_widget(Paragraph::new(Span::styled(text, Style::new().bg(ink()).fg(muted()))), at);
    }
    let visible = list.height as usize;
    let offset = (cursor + 1).saturating_sub(visible);
    let number_width = rows.iter().skip(offset).take(visible)
        .map(|row| row.number.display("").len()).max().unwrap_or(0);
    for (i, row) in rows.iter().enumerate().skip(offset).take(visible) {
        let rect = Rect::new(list.x, list.y + (i - offset) as u16, list.width, 1);
        let line = result_line(row, i == cursor, number_width, list.width as usize);
        f.render_widget(Paragraph::new(line), rect);
        app.hits.finder_rows.push((rect, i));
    }

    let count = format!(" {} of {} ", rows.len(), total);
    let footer = Rect::new(popup.x + 2, popup.bottom() - 1, popup.width.saturating_sub(4), 1);
    let count_width = (count.chars().count() as u16).min(footer.width);
    let context = Context::Dialog(Dialog::Finder);
    let width = footer.width.saturating_sub(count_width + 2);
    let mut legend = hints::legend(context, app, width, 1).remove(0);
    legend.spans.insert(0, Span::styled(" ", theme::base()));
    legend.spans.push(Span::styled(" ", theme::base()));
    f.render_widget(Paragraph::new(legend), Rect::new(footer.x, footer.y, footer.width - count_width, 1));
    let hits = hints::legend_hits(context, app, width, 1, (footer.x + 1, footer.y));
    app.hits.legend.extend(hits);
    let at = Rect::new(footer.right() - count_width, footer.y, count_width, 1);
    f.render_widget(Paragraph::new(Span::styled(count, Style::new().bg(ink()).fg(muted()))), at);
}
