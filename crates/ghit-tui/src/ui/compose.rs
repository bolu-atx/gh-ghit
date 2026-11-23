//! The comment editor modal: one box, Write or Preview, the target in the
//! top border with the mode switch, the key legend in the bottom border.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;
use ghit_core::core::{self, Dialog};
use crate::keymap::{Action, Context};
use crate::theme::{self, amber, base, ink, muted, paper};
use super::hints;

/// Widest popup and widest text column, in cells.
const POPUP_WIDTH: u16 = 96;
const TEXT_WIDTH: u16 = 80;
const SWITCH: &str = " Write  Preview ";

/// Popup rect: at most 96 cells wide, about two thirds of the height.
fn popup_rect(area: Rect) -> Rect {
    let width = area.width.min(POPUP_WIDTH);
    let height = (area.height * 2 / 3).clamp(area.height.min(12), area.height);
    Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height)
}

fn clip(text: &str, cells: usize) -> String {
    if text.chars().count() <= cells {
        return text.to_string();
    }
    let keep: String = text.chars().take(cells.saturating_sub(1)).collect();
    format!("{keep}…")
}

fn counter(body: &str) -> String {
    let chars = body.chars().count();
    let lines = if body.is_empty() { 0 } else { body.split('\n').count() };
    let plural = |n: usize, word: &str| if n == 1 { format!("{n} {word}") } else { format!("{n} {word}s") };
    format!("{} · {} ", plural(chars, "char"), plural(lines, "line"))
}

/// Comment compose modal (`c`). Write mode draws a line-number gutter,
/// soft-wrapped text and the real terminal cursor; Preview renders the
/// markdown through the shared `GhitSheet` stylesheet in the same box.
/// While the background submit runs, the title reads Sending and editing
/// locks. The modal steals all keyboard and mouse focus, edits with
/// bash/readline chords, and offers `$EDITOR` on `Ctrl+O`; resolving
/// threads is out of scope and never offered here.
pub(super) fn render_compose(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(c) = app.draft() else { return };
    let body = c.body.clone();
    let cursor = c.cursor.min(body.chars().count());
    let (sending, mut view) = (c.sending, c.view);
    let item_title = app.session.issues.data.get(&c.number).map(|d| d.title.clone());
    let title = c.target.title(c.number, item_title.as_deref());

    let popup = popup_rect(area);
    app.hits.popup = popup;
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL).border_style(Style::new().bg(ink()).fg(muted())).style(base());
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let switch_w = SWITCH.chars().count() as u16;
    let mut head = vec![Span::raw(" ")];
    if sending {
        head.push(Span::styled(format!("{} Sending… ", super::spinner_frame(app.tick)), Style::new().bg(ink()).fg(amber()).bold()));
    }
    let room = popup.width.saturating_sub(switch_w + 5) as usize;
    head.push(Span::styled(clip(&title, room.saturating_sub(if sending { 12 } else { 0 })), Style::new().bg(ink()).fg(paper()).bold()));
    // A gap before the border line resumes, like every other pane title.
    head.push(Span::raw(" "));
    let title_rect = Rect::new(popup.x + 1, popup.y, popup.width.saturating_sub(2), 1);
    f.render_widget(Paragraph::new(Line::from(head)), title_rect);
    let switch_x = popup.right().saturating_sub(1 + switch_w);
    let (write_style, preview_style) = if view.preview {
        (Style::new().bg(ink()).fg(muted()), Style::new().bg(ink()).fg(amber()).bold())
    } else {
        (Style::new().bg(ink()).fg(amber()).bold(), Style::new().bg(ink()).fg(muted()))
    };
    let switch = Line::from(vec![
        Span::styled(" ", base()),
        Span::styled("Write", write_style),
        Span::styled("  ", base()),
        Span::styled("Preview", preview_style),
        Span::styled(" ", base()),
    ]);
    f.render_widget(Paragraph::new(switch), Rect::new(switch_x, popup.y, switch_w, 1));
    // Only the idle word clicks: it flips to the other mode.
    let idle = if view.preview { Rect::new(switch_x + 1, popup.y, 5, 1) } else { Rect::new(switch_x + 8, popup.y, 7, 1) };
    app.hits.legend.push((idle, Action::ComposeTogglePreview));

    let text_area = Rect { height: inner.height.saturating_sub(1), ..inner };
    let height = text_area.height as usize;
    app.hits.compose_text = Rect::default();
    if view.preview {
        let preview = if body.trim().is_empty() {
            ratatui::text::Text::from(Line::from(Span::styled("(nothing to preview)", Style::new().bg(ink()).fg(muted()).italic())))
        } else {
            theme::render_markdown(&body)
                .lines
                .into_iter()
                .map(|line| Line::from(line.spans.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
                .into()
        };
        view.scroll = view.scroll.min(preview.lines.len().saturating_sub(1));
        let pane = Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..text_area };
        f.render_widget(
            Paragraph::new(preview).style(base()).wrap(Wrap { trim: false }).scroll((view.scroll as u16, 0)),
            pane,
        );
    } else {
        let lines = body.split('\n').count();
        let digits = lines.to_string().len().max(2);
        let gutter = digits as u16 + 3;
        let cells = inner.width.saturating_sub(gutter + 1).min(TEXT_WIDTH + 1);
        let text = Rect { x: inner.x + gutter, width: cells, ..text_area };
        app.hits.compose_text = text;
        let rows = core::text_rows(&body, cells as usize);
        let (cursor_row, cursor_col) = core::cursor_pos(&rows, cursor);
        if view.followed != Some(cursor) {
            if cursor_row < view.scroll {
                view.scroll = cursor_row;
            } else if cursor_row >= view.scroll + height {
                view.scroll = cursor_row + 1 - height;
            }
            view.followed = Some(cursor);
        }
        view.scroll = view.scroll.min(rows.len().saturating_sub(height));
        let chars: Vec<char> = body.chars().collect();
        for (i, row) in rows.iter().enumerate().skip(view.scroll).take(height) {
            let y = inner.y + (i - view.scroll) as u16;
            let bg = if i == cursor_row && !sending { theme::line_hl() } else { ink() };
            let number = if core::starts_line(&rows, i) { format!(" {:>digits$}  ", row.line + 1) } else { " ".repeat(gutter as usize) };
            let shown: String = chars[row.start..row.end].iter().map(|c| if c.is_control() { ' ' } else { *c }).collect();
            let mut spans = vec![Span::styled(number, Style::new().bg(bg).fg(muted()))];
            if body.is_empty() {
                spans.push(Span::styled("Markdown works here. Ctrl+S sends.", Style::new().bg(bg).fg(muted()).italic()));
            } else {
                spans.push(Span::styled(shown, Style::new().bg(bg).fg(paper())));
            }
            f.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(bg)), Rect::new(inner.x, y, inner.width, 1));
        }
        if !sending && cursor_row >= view.scroll && cursor_row < view.scroll + height {
            f.set_cursor_position((text.x + cursor_col as u16, inner.y + (cursor_row - view.scroll) as u16));
        }
    }

    if inner.height > 0 {
        let count = counter(&body);
        let width = (count.chars().count() as u16).min(inner.width);
        let at = Rect::new(inner.right() - width, inner.bottom() - 1, width, 1);
        f.render_widget(Paragraph::new(count).style(Style::new().bg(ink()).fg(muted())), at);
    }
    if let Some(c) = app.draft_mut() {
        c.view = view;
    }

    let footer = Rect::new(popup.x + 2, popup.bottom() - 1, popup.width.saturating_sub(4), 1);
    if sending {
        let line = Line::styled(" sending… esc closes, the send still lands ", Style::new().bg(ink()).fg(muted()));
        f.render_widget(Paragraph::new(line), footer);
        return;
    }
    let context = Context::Dialog(Dialog::Compose);
    let width = footer.width.saturating_sub(2);
    let mut legend = hints::legend(context, app, width, 1).remove(0);
    legend.spans.insert(0, Span::styled(" ", base()));
    legend.spans.push(Span::styled(" ", base()));
    f.render_widget(Paragraph::new(legend), footer);
    let hits = hints::legend_hits(context, app, width, 1, (footer.x + 1, footer.y));
    app.hits.legend.extend(hits);
}
