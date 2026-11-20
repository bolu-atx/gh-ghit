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
    let needle = app.help_filter.to_lowercase();
    let sections: Vec<_> = keymap::help_sections(app).into_iter()
        .map(|(context, current)| (context, current, filter_rows(context, &needle)))
        .filter(|(_, _, rows)| !rows.is_empty())
        .collect();
    if sections.is_empty() {
        return vec![Line::styled(format!("  no shortcut matches \"{}\"", app.help_filter), Style::new().bg(ink()).fg(muted()))];
    }
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

/// A section's rows that match the filter. A match on the section title
/// keeps the whole section; otherwise a row matches on its keys, label,
/// group or mouse gesture. Case-insensitive; an empty filter keeps all.
fn filter_rows(context: Context, needle: &str) -> Vec<keymap::HelpRow> {
    let rows = keymap::help_rows(context);
    if needle.is_empty() || context.title().to_lowercase().contains(needle) {
        return rows;
    }
    rows.into_iter().filter(|row| {
        [row.keys.as_str(), row.label, row.group, row.mouse.unwrap_or("")]
            .iter().any(|field| field.to_lowercase().contains(needle))
    }).collect()
}

pub(super) fn render_shortcuts(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let width = area.width.saturating_sub(4).clamp(1, 84);
    let height = area.height.saturating_sub(2).clamp(1, 40);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);
    app.hits.popup = popup;
    f.render_widget(Clear, popup);
    let title = match (app.help_typing, app.help_filter.is_empty()) {
        (true, _) => format!(" Shortcuts · /{}▏ ", app.help_filter),
        (false, false) => format!(" Shortcuts · /{} ", app.help_filter),
        (false, true) => " Shortcuts ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base()).title(title);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = help_lines(app);
    // Credit footer (display-only: the modal swallows all mouse input,
    // so the URL below is never clickable).
    lines.push(Line::from(""));
    for row in BANNER {
        lines.push(Line::styled(row, Style::new().bg(ink()).fg(amber())));
    }
    lines.push(Line::styled(format!("ghit v{} · created by bolu-atx", env!("CARGO_PKG_VERSION")), Style::new().bg(ink()).fg(muted())));
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
    let legend_hits = hints::legend_hits(Context::Dialog(Dialog::Shortcuts), app, footer.width, 1, (footer.x, footer.y));
    app.hits.legend.extend(legend_hits);
}
