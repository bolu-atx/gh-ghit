//! Board modal: columns, cards, filter form and links.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

use crate::app::App;
use ghit_core::core::Dialog;
use crate::keymap::{Action, Context};
use ghit_core::model::self;
use super::hints;

use super::{sized_popup, scroll_to, centered};

/// Board format form (`f` with the board open): the text row, one
/// milestone pill per milestone on the board, and card field checkboxes.
/// The active widget carries the `▌` marker; `space` toggles a pill or a
/// field, typing on the text row edits it, Enter applies, Esc clears.
/// Empty pills select every card.
pub(super) fn render_board_filter(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, base, ink, muted, paper};
    let pills = app.board_pills();
    // Nerd Font Font Awesome fa-check-square / fa-square-o, same as the
    // sidebar filter toggles. The marker echoes the board card selection.
    let marker = |active: bool| {
        Span::styled(
            if active { "▌ " } else { "  " },
            Style::new().bg(ink()).fg(if active { amber() } else { muted() }).bold(),
        )
    };
    let empty = app.sidebar.query_draft.is_empty();
    let mut text_row = vec![
        marker(app.board.filter_focus == 0),
        Span::styled("filter ", Style::new().bg(ink()).fg(muted())),
        Span::styled(
            if empty { "label, title, #num …".to_string() } else { app.sidebar.query_draft.clone() },
            Style::new().bg(ink()).fg(if empty { muted() } else { paper() }),
        ),
    ];
    if app.query_edit() {
        text_row.push(Span::styled("▌", Style::new().bg(ink()).fg(amber()).bold()));
    } else {
        text_row.push(Span::styled(" (e)".to_string(), Style::new().bg(ink()).fg(muted())));
    }
    let mut focus_line = 0;
    let mut lines = vec![
        Line::from(text_row),
        Line::from(""),
        Line::from(Span::styled("milestones", Style::new().bg(ink()).fg(muted()))),
    ];
    if pills.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no milestones on this board",
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    for (i, pill) in pills.iter().enumerate() {
        if !app.query_edit() && app.board.filter_focus == i + 1 {
            focus_line = lines.len();
        }
        let on = app.board.milestone_draft.contains(pill);
        let box_glyph = if on { crate::theme::nf("\u{f14a}", "■") } else { crate::theme::nf("\u{f096}", "□") };
        let name = pill.clone().unwrap_or_else(|| "(no milestone)".to_string());
        lines.push(Line::from(vec![
            marker(app.board.filter_focus == i + 1),
            Span::styled(
                format!("{box_glyph} {name}"),
                Style::new().bg(ink()).fg(if on { paper() } else { muted() }),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("card fields", Style::new().bg(ink()).fg(muted()))));
    for (i, name) in model::CardFields::ORDER.iter().enumerate() {
        if !app.query_edit() && app.board.filter_focus == pills.len() + 1 + i {
            focus_line = lines.len();
        }
        let on = app.board.fields.shown(i).unwrap_or(false);
        let box_glyph = if on { crate::theme::nf("\u{f14a}", "■") } else { crate::theme::nf("\u{f096}", "□") };
        lines.push(Line::from(vec![
            marker(app.board.filter_focus == pills.len() + 1 + i),
            Span::styled(
                format!("{box_glyph} {name}"),
                Style::new().bg(ink()).fg(if on { paper() } else { muted() }),
            ),
        ]));
    }
    lines.push(Line::from(""));
    let popup = sized_popup(area, 62, lines.len() + 1);
    let legend_width = popup.width.saturating_sub(2);
    let legend = hints::legend(Context::Dialog(Dialog::BoardFilter), app, legend_width, 1);
    let legend_line = lines.len();
    lines.extend(legend);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Line::from(vec![
            Span::styled(format!(" {}Format ", crate::theme::nf("\u{f0b0} ", "")), Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled("· board cards ", Style::new().bg(ink()).fg(muted())),
        ]));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let scroll = scroll_to(inner, focus_line, app.query_edit());
    app.hits.form_popup = popup;
    if let Some(y) = legend_line.checked_sub(scroll as usize).filter(|y| *y < inner.height as usize) {
        let at = (inner.x, inner.y + y as u16);
        let legend_hits = hints::legend_hits(Context::Dialog(Dialog::BoardFilter), app, legend_width, 1, at);
        app.hits.legend.extend(legend_hits);
    }
    // Widget rows: the text row, then pills and card fields at the
    // lines `lines` laid out (2 header lines, blank and label gaps).
    let first_pill = 3 + usize::from(pills.is_empty());
    let mut widget_lines = vec![0];
    widget_lines.extend((0..pills.len()).map(|i| first_pill + i));
    let fields_at = first_pill + pills.len() + 2;
    widget_lines.extend((0..model::CardFields::ORDER.len()).map(|i| fields_at + i));
    for (index, line) in widget_lines.into_iter().enumerate() {
        let Some(y) = line.checked_sub(scroll as usize).filter(|y| *y < inner.height as usize)
        else { continue };
        app.hits.form_cells.push((Rect::new(inner.x, inner.y + y as u16, inner.width, 1), index, None));
    }
    f.render_widget(Paragraph::new(lines).style(base()).scroll((scroll, 0)), inner);
    if app.query_edit() {
        let x = inner.x + 9 + app.sidebar.query_draft.chars().count() as u16;
        f.set_cursor_position((x, inner.y));
    }
}

/// Collapsed card: title, labels, spacer. Expanded adds status, url, excerpt.
const BASE_CARD_H: u16 = 3;

const EXPANDED_CARD_H: u16 = 6;

fn card_height(expanded: bool) -> u16 {
    if expanded {
        EXPANDED_CARD_H
    } else {
        BASE_CARD_H
    }
}

/// Narrowest board column: below this the card lines clip.
const MIN_COL_W: u16 = 36;

/// Column areas with a 1-cell gutter so borders never touch. Columns
/// share the area evenly, but never shrink past `MIN_COL_W`; when even
/// the minimum does not fit, they keep it and overflow as before.
fn column_layout(area: Rect, n_cols: usize) -> Vec<Rect> {
    if n_cols == 0 {
        return Vec::new();
    }
    let gutters = n_cols - 1;
    let needed = n_cols * MIN_COL_W as usize + gutters;
    let widths: Vec<u16> = if area.width as usize >= needed {
        let share = (area.width as usize - gutters) / n_cols;
        let extra = (area.width as usize - gutters) % n_cols;
        (0..n_cols)
            .map(|i| (if i < extra { share + 1 } else { share }) as u16)
            .collect()
    } else {
        vec![MIN_COL_W; n_cols]
    };
    let mut constraints = Vec::with_capacity(n_cols * 2);
    for (i, width) in widths.into_iter().enumerate() {
        constraints.push(Constraint::Length(width));
        if i + 1 < n_cols {
            constraints.push(Constraint::Length(1));
        }
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    chunks.iter().step_by(2).copied().collect()
}

pub(super) fn render_board(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, base};
    let popup = centered(area, 92, 92);
    app.hits.popup = popup;
    f.render_widget(Clear, popup);
    // Nerd Font Font Awesome fa-columns; the board name keeps its label.
    // Column headers stay icon-free: their vocabularies are user-defined
    // and already color-coded, so glyphs would add noise, not signal.
    let title = match &app.board.data {
        Some(b) => Line::from(vec![
            Span::styled(format!(" {}", crate::theme::nf("\u{f0db} ", "")), Style::new().bg(ink()).fg(muted())),
            Span::styled(format!("{} ", b.title), Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled(
                format!("{} cards", b.items.len()),
                Style::new().bg(ink()).fg(muted()),
            ),
        ]),
        None => Line::from(vec![
            Span::styled(format!(" {}Fetching board … ", crate::theme::nf("\u{f0db} ", "")), Style::new().bg(ink()).fg(amber()).bold()),
        ]),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(title);
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    render_board_links(f, popup, app);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (inner, hint_area) = (chunks[0], chunks[1]);
    let filtered = app.board_filtered();
    let mut hint = vec![];
    if !app.board.filter.trim().is_empty() {
        hint.push(Span::styled("filter ", Style::new().bg(ink()).fg(muted())));
        hint.push(Span::styled(
            format!("\"{}\"", app.board.filter),
            Style::new().bg(ink()).fg(amber()).bold(),
        ));
        hint.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
    }
    if !app.board.milestones.is_empty() {
        let names: Vec<String> = app
            .board_pills()
            .into_iter()
            .filter(|p| app.board.milestones.contains(p))
            .map(|p| p.unwrap_or_else(|| "(no milestone)".to_string()))
            .collect();
        hint.push(Span::styled("milestones ", Style::new().bg(ink()).fg(muted())));
        hint.push(Span::styled(
            names.join(", "),
            Style::new().bg(ink()).fg(amber()).bold(),
        ));
        hint.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
    }
    let used: usize = hint.iter().map(|span| span.content.chars().count()).sum();
    let room = hint_area.width.saturating_sub(used as u16);
    for line in hints::legend(Context::Dialog(Dialog::Board), app, room, 1) {
        hint.extend(line.spans);
    }
    let legend_at = (hint_area.x + used as u16, hint_area.y);
    let legend_hits = hints::legend_hits(Context::Dialog(Dialog::Board), app, room, 1, legend_at);
    app.hits.legend.extend(legend_hits);
    f.render_widget(Paragraph::new(Line::from(hint)), hint_area);

    let Some(board) = &app.board.data else {
        f.render_widget(
            Paragraph::new(super::loading_body("Loading board…", app.tick, usize::from(inner.height))).style(base()),
            inner,
        );
        return;
    };
    let cols = crate::app::board_columns(board, &app.board.filter, &app.board.milestones);
    if cols.is_empty() {
        f.render_widget(Paragraph::new("empty board").style(base()), inner);
        return;
    }
    while app.board.states.len() < cols.len() {
        app.board.states.push(ListState::default());
    }
    let col_areas = column_layout(inner, cols.len());
    app.hits.board_cells.clear();
    for (i, (name, cards)) in cols.iter().enumerate() {
        let Some(a) = col_areas.get(i) else { continue };
        let inner_w = a.width.saturating_sub(2);
        let items: Vec<ListItem> = cards
            .iter()
            .enumerate()
            .map(|(ri, c)| {
                let sel = app.board.col == i && app.board.row == ri;
                let detail = c.number.and_then(|n| app.board.details.get(&n));
                let expanded = app.board.expanded == Some((i, ri));
                board_card(c, sel, expanded, detail, inner_w, &app.board.fields)
            })
            .collect();
        let active = app.board.col == i;
        let color = crate::theme::column_color(name);
        let icon = crate::theme::column_icon(name);
        let mut head = vec![];
        if icon.is_empty() {
            head.push(Span::styled(
                format!(" {name} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
        } else {
            head.push(Span::styled(
                format!(" {icon} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
            head.push(Span::styled(
                format!("{name} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
        }
        head.push(Span::styled(
            format!("({})", cards.len()),
            Style::new().bg(ink()).fg(muted()),
        ));
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(if active { amber() } else { muted() }))
                .style(base())
                .title(Line::from(head)),
        );
        let state = &mut app.board.states[i];
        state.select(if active { Some(app.board.row) } else { None });
        f.render_stateful_widget(list, *a, state);
        let col_inner = Block::default().borders(Borders::ALL).inner(*a);
        if cards.is_empty() {
            // An empty lane keeps its width and says so, instead of
            // reading as a broken fetch. A filter that hides every card
            // reads as no match, not as an empty column.
            let label = if filtered {
                hints::tip("  no matches · ", Context::Dialog(Dialog::Board), Action::Back, " clears the filter")
            } else {
                Line::styled("  no cards", Style::new().bg(ink()).fg(muted()))
            };
            f.render_widget(
                Paragraph::new(label).style(base()).wrap(Wrap { trim: false }),
                col_inner,
            );
        }
        // Map visible cards back to click areas using the widget's scroll
        // offset. Cards have variable height, so walk line offsets.
        let offset = app.board.states[i].offset();
        let mut y = col_inner.y;
        for (ri, _) in cards.iter().enumerate().skip(offset) {
            let h = card_height(app.board.expanded == Some((i, ri)));
            if y + h > col_inner.y + col_inner.height {
                break;
            }
            app.hits.board_cells.push((Rect::new(col_inner.x, y, col_inner.width, h), i, ri));
            y += h;
        }
    }
}

/// Top-right header icons: a flag for the repo milestones page plus an
/// external-link for the project board itself. Nerd Font glyphs only,
/// in the same teal-underlined URL style as card links, so the header
/// stays one row tall. Hit areas land in `app.hits.board_links`.
fn render_board_links(f: &mut ratatui::Frame, popup: Rect, app: &mut App) {
    use crate::theme::{base, ink, teal};
    app.hits.board_links.clear();
    let mut links: Vec<(String, String)> = vec![];
    // Nerd Font Font Awesome fa-flag: the repo milestones page.
    if let Some(repo) = app.session.repo_flag.clone().or_else(|| app.session.repo_slug.clone()) {
        links.push((
            crate::theme::nf("\u{f024}", "⚑").to_string(),
            format!("https://github.com/{repo}/milestones"),
        ));
    }
    // Nerd Font Font Awesome fa-external-link: the project board page,
    // from `gh project list` (old caches predate it and show no icon
    // until the board refetches).
    if let Some(url) = app.board.data.as_ref().map(|b| b.url.clone()) {
        if !url.is_empty() {
            links.push((crate::theme::nf("\u{f08e}", "↗").to_string(), url));
        }
    }
    if links.is_empty() || popup.width < 12 {
        return;
    }
    let width = links.len() as u16 * 3;
    let x = popup.x + popup.width.saturating_sub(width + 1);
    let spans: Vec<Span> = links
        .iter()
        .map(|(glyph, _)| {
            Span::styled(
                format!(" {glyph} "),
                Style::new().bg(ink()).fg(teal()).underlined(),
            )
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(base()),
        Rect::new(x, popup.y, width, 1),
    );
    for (i, (_, url)) in links.iter().enumerate() {
        app.hits.board_links
            .push((Rect::new(x + i as u16 * 3, popup.y, 3, 1), url.clone()));
    }
}

/// What kind of item this is: issue, PR, or draft.
fn card_kind(c: &model::BoardItem) -> String {
    match c.content.get("type").and_then(|t| t.as_str()) {
        Some("PullRequest") => "PR".to_string(),
        Some("DraftIssue") => "DRAFT".to_string(),
        Some("Issue") => "ISSUE".to_string(),
        Some(other) => other.to_string(),
        None => "ITEM".to_string(),
    }
}

fn card_excerpt(c: &model::BoardItem, width: usize) -> String {
    let body = c.content.get("body").and_then(|b| b.as_str()).unwrap_or("");
    match body.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(first) => model::slug(first, width),
        None => "–".to_string(),
    }
}

/// A board card: 3 lines collapsed (head, meta or breathing room,
/// footer with labels left and dates right), 6 lines expanded (status,
/// URL, excerpt, dates plus comment count). Collapsed lines honor the
/// Card fields toggles; off means undrawn. Matches `BASE_CARD_H` /
/// `EXPANDED_CARD_H`, so the height never changes with the toggles.
fn board_card(
    c: &model::BoardItem,
    sel: bool,
    expanded: bool,
    detail: Option<&model::BoardDetail>,
    inner_width: u16,
    fields: &model::CardFields,
) -> ListItem<'static> {
    use crate::theme::{amber, column_color, ink, muted, paper, teal};
    // The slug takes what the line leaves after the 2-cell marker and
    // the `#num ` prefix, so a wider column shows more title. The number
    // toggle governs the collapsed card; the expanded card is a detail
    // view and always names its number.
    let title_width = (inner_width as usize).saturating_sub(2);
    let show_number = fields.number || expanded;
    let head = match (c.number, show_number) {
        (Some(n), true) => {
            let prefix = format!("{n} ", n = n.display(""));
            let room = title_width.saturating_sub(prefix.chars().count()).max(1);
            format!("{prefix}{}", model::slug(&c.title, room))
        }
        (Some(_), false) => model::slug(&c.title, title_width.max(1)),
        (None, _) => {
            format!("◇ {}", model::slug(&c.title, title_width.saturating_sub(2).max(1)))
        }
    };
    let first = if sel {
        Line::from(vec![
            Span::styled("▌ ", Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled(head, Style::new().bg(ink()).fg(paper()).bold()),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::new().bg(ink())),
            Span::styled(head, Style::new().bg(ink()).fg(paper())),
        ])
    };
    if !expanded {
        // Labels own the left; the swimlane carries the status, so the
        // collapsed card never repeats it unless its field is on.
        let room = (inner_width as usize).saturating_sub(4);
        let right = model::card_footer_right(detail, fields);
        let max_left = room.saturating_sub(right.chars().count() + 2);
        let mut left = if fields.labels { model::card_footer_labels(c) } else { String::new() };
        if left.chars().count() > max_left && max_left > 1 {
            left = format!("{}…", left.chars().take(max_left.saturating_sub(1)).collect::<String>());
        }
        let pad = room.saturating_sub(left.chars().count() + right.chars().count()).max(1);
        let footer = format!("    {left}{}{right}", " ".repeat(pad));
        let meta = model::card_meta(c, fields, room);
        let second = if meta.is_empty() {
            Line::from(Span::styled("", Style::new().bg(ink())))
        } else {
            Line::from(Span::styled(format!("    {meta}"), Style::new().bg(ink()).fg(muted())))
        };
        return ListItem::new(vec![
            first,
            second,
            Line::from(Span::styled(footer, Style::new().bg(ink()).fg(muted()))),
        ]);
    }
    let status = c.status.clone().unwrap_or_else(|| "No status".to_string());
    let icon = crate::theme::column_icon(&status);
    let status_label = if icon.is_empty() {
        status.clone()
    } else {
        format!("{icon} {status}")
    };
    let mut meta = format!("{} · {}", status_label, card_kind(c));
    if let Some(login) = model::card_assignee(c) {
        meta.push_str(&format!(" · @{login}"));
    }
    // No icon here: at the minimum column width the date line already
    // fills the card, so a glyph would push the comment count off-screen.
    let dates = match detail {
        Some(d) => format!("{} → {} · {}", d.created, d.updated, comment_word(d.comments)),
        None => "…".to_string(),
    };
    let excerpt_width = (inner_width as usize).saturating_sub(4).max(1);
    ListItem::new(vec![
        first,
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(meta, Style::new().bg(ink()).fg(column_color(&status))),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(
                c.url().unwrap_or_else(|| "–".to_string()),
                Style::new().bg(ink()).fg(teal()).underlined(),
            ),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(card_excerpt(c, excerpt_width), Style::new().bg(ink()).fg(paper())),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(dates, Style::new().bg(ink()).fg(muted())),
        ]),
        Line::from(Span::styled("", Style::new().bg(ink()))),
    ])
}

fn comment_word(n: usize) -> String {
    if n == 0 {
        "no comments".to_string()
    } else {
        format!("{} comment{}", n, if n == 1 { "" } else { "s" })
    }
}
