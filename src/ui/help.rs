//! Help screen: rendered from the keymap registry, so it cannot drift
//! from what the keys do. The context the screen was opened from comes
//! first and is marked; the other contexts follow.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::App;
use super::hints;
use crate::core::Dialog;
use crate::keymap::{self, Context};
use crate::theme::{amber, base, ink, muted, paper, teal};

/// Widest key column; longer key lists wrap into the label column.
const KEY_COLUMN_MAX: usize = 26;

const BANNER: [&str; 4] = [
    "  ___ _   _ ___ _____",
    " / __| | | |_ _|_   _|",
    "| (_ | |_| || |  | |",
    " \\___|____|___|  |_|",
];

/// Every help line, sections in order. Pure of layout, so tests can read it.
pub(super) fn help_lines(app: &App) -> Vec<Line<'static>> {
    let sections: Vec<_> = keymap::help_sections(app).into_iter()
        .map(|(context, current)| (context, current, keymap::help_rows(context)))
        .collect();
    let key_width = sections.iter().flat_map(|(_, _, rows)| rows)
        .map(|row| row.keys.chars().count())
        .max().unwrap_or(0).min(KEY_COLUMN_MAX) + 2;
    let mut lines = vec![];
    for (context, current, rows) in &sections {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        let title = if *current { format!("▸ {} · here", context.title()) } else { context.title().to_string() };
        let color = if *current { amber() } else { paper() };
        lines.push(Line::styled(title, Style::new().bg(ink()).fg(color).bold()));
        let grouped = rows.windows(2).any(|pair| pair[0].group != pair[1].group);
        let mut group = "";
        for row in rows {
            if grouped && row.group != group {
                group = row.group;
                lines.push(Line::styled(format!("  {group}"), Style::new().bg(ink()).fg(muted()).italic()));
            }
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<width$}", row.keys, width = key_width.max(row.keys.chars().count() + 2)), Style::new().bg(ink()).fg(amber())),
                Span::styled(row.label, Style::new().bg(ink()).fg(muted())),
                Span::styled(
                    row.mouse.map(|gesture| format!(" · {gesture}")).unwrap_or_default(),
                    Style::new().bg(ink()).fg(muted()).italic(),
                ),
            ]));
        }
    }
    lines
}

pub(super) fn render_shortcuts(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let width = area.width.saturating_sub(4).clamp(1, 84);
    let height = area.height.saturating_sub(2).clamp(1, 40);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base()).title(" Shortcuts ");
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = help_lines(app);
    // Credit footer (display-only: the modal swallows all mouse input,
    // so the URL below is never clickable).
    lines.push(Line::from(""));
    for row in BANNER {
        lines.push(Line::styled(row, Style::new().bg(ink()).fg(amber())));
    }
    lines.push(Line::styled("created by bolu-atx", Style::new().bg(ink()).fg(muted())));
    lines.push(Line::styled(
        "https://github.com/bolu-atx/gh-ghit",
        Style::new().bg(ink()).fg(teal()).underlined(),
    ));

    let body_height = inner.height.saturating_sub(1);
    let max_scroll = (lines.len() as u16).saturating_sub(body_height);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let body = Rect::new(inner.x, inner.y, inner.width, body_height);
    f.render_widget(Paragraph::new(lines).scroll((app.help_scroll, 0)), body);

    let footer = Rect::new(inner.x, inner.y + body_height, inner.width, inner.height.min(1));
    let legend = hints::legend(Context::Dialog(Dialog::Shortcuts), app, footer.width, 1);
    f.render_widget(Paragraph::new(legend), footer);
}
