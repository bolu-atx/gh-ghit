//! The map card, docked at the right of the map. A node card shows the
//! stored item's details and body with an open button; a milestone card
//! lists its items; an edge card names the link in words above a short
//! card per end. Buttons and end cards land in `app.hits.map_card_hits`.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::map::node_color;
use crate::app::{App, CardHit, MapCard};
use crate::model::{self, CheckState, MapNode, NodeState};
use crate::theme::{
    amber, base, check_color, check_glyph, column_color, column_icon, ink, muted, paper, teal,
};

/// Width of the field names on a node card.
const KEY_W: usize = 10;

pub(super) fn render_map_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(card) = app.map.card else { return };
    if area.width < 8 || area.height < 4 {
        return;
    }
    app.hits.map_card = area;
    let title = match card {
        MapCard::Node => match app.map.selected_index().map(|i| &app.map.graph.nodes[i]) {
            Some(MapNode { number: Some(n), .. }) => format!(" #{n} "),
            _ => " milestone ".to_string(),
        },
        MapCard::Edge { .. } => " link ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Span::styled(title, Style::new().fg(amber()).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(Clear, area);
    f.render_widget(block, area);
    let close = Rect::new(area.right() - 4, area.y, 3, 1);
    f.render_widget(Paragraph::new(" × ").style(Style::new().fg(paper())), close);
    app.hits.map_card_hits.push((close, CardHit::Close));

    let body = Rect { height: inner.height.saturating_sub(2), ..inner };
    let mut buttons = vec![];
    match (card, app.map.selected_index()) {
        (MapCard::Edge { .. }, _) => edge_card(f, body, app),
        (MapCard::Node, Some(index)) => {
            if app.map.graph.nodes[index].number.is_some() {
                buttons.push(("open", "↵", CardHit::Open));
                item_card(f, body, app, index);
            } else {
                hub_card(f, body, app, index);
            }
        }
        (MapCard::Node, None) => {}
    }
    buttons.push(("next link", "e", CardHit::NextEdge));
    buttons.push(("close", "esc", CardHit::Close));
    let row = Rect { y: inner.bottom() - 1, height: 1, ..inner };
    let mut x = row.x;
    for (label, key, part) in buttons {
        let width = (label.chars().count() + key.chars().count() + 3) as u16;
        if x + width > row.right() {
            break;
        }
        let rect = Rect::new(x, row.y, width, 1);
        let line = Line::from(vec![
            Span::styled(format!("[{label} "), Style::new().fg(paper())),
            Span::styled(key, Style::new().fg(amber()).add_modifier(Modifier::BOLD)),
            Span::styled("]", Style::new().fg(paper())),
        ]);
        f.render_widget(Paragraph::new(line), rect);
        app.hits.map_card_hits.push((rect, part));
        x += width + 1;
    }
}

/// An item: title, state, the stored fields, then the body excerpt.
fn item_card(f: &mut ratatui::Frame, area: Rect, app: &App, index: usize) {
    let node = &app.map.graph.nodes[index];
    let width = area.width as usize;
    let mut lines = vec![
        Line::from(Span::styled(model::slug(&node.title, width), heading())),
        Line::from(Span::styled(format!("● {}", node_words(app, node)), state_style(node))),
        Line::default(),
    ];
    let field = |key: &str, value: Span<'static>| {
        Line::from(vec![Span::styled(format!("{key:<KEY_W$}"), Style::new().fg(muted())), value])
    };
    let room = width.saturating_sub(KEY_W);
    let plain = |text: String| Span::styled(model::slug(&text, room), Style::new().fg(paper()));
    let detail = app.map.detail.as_ref().filter(|d| Some(d.number) == node.number);
    let item = app.map.items.iter().find(|item| Some(item.number) == node.number);
    let milestone = item.and_then(|item| item.milestone.clone()).unwrap_or("–".to_string());
    lines.push(field("milestone", plain(milestone)));
    let mut body = None;
    match detail.and_then(|d| d.row.as_ref()) {
        Some(row) => {
            let issue = &row.issue;
            let status = row.statuses.iter().find_map(|(_, status)| status.clone());
            lines.push(field("status", match status.as_deref() {
                Some(status) => {
                    let text = format!("{} {status}", column_icon(status));
                    Span::styled(model::slug(text.trim(), room), column_color(status))
                }
                None => plain("–".to_string()),
            }));
            let card = model::BoardItem::from_issue(issue, status.as_deref());
            lines.push(field("labels", plain(model::card_footer_labels(&card))));
            let logins: Vec<String> =
                issue.assignees.iter().map(|a| format!("@{}", a.login)).collect();
            let assignees = if logins.is_empty() { "–".to_string() } else { logins.join(" ") };
            lines.push(field("assignees", plain(assignees)));
            if issue.is_pr {
                lines.push(field("checks", match issue.checks.as_ref() {
                    Some(state) if *state != CheckState::Unknown => {
                        let word = match state {
                            CheckState::Pass => "passing",
                            CheckState::Fail => "failing",
                            _ => "pending",
                        };
                        Span::styled(format!("{} {word}", check_glyph(state)), check_color(state))
                    }
                    _ => plain("none".to_string()),
                }));
            }
            let fields = model::CardFields::default();
            let updated = model::card_footer_right(Some(&row.detail), &fields);
            lines.push(field("updated", plain(updated)));
            body = detail.and_then(|d| d.body.as_deref()).filter(|text| !text.trim().is_empty());
        }
        None => lines.push(Line::from(Span::styled("Not in the local copy yet.", muted()))),
    }
    lines.push(Line::default());
    let used = lines.len() as u16;
    f.render_widget(Paragraph::new(lines), area);
    let used = used.min(area.height);
    let rest = Rect { y: area.y + used, height: area.height - used, ..area };
    let text = match body {
        Some(text) => Paragraph::new(text.trim().to_string()).style(Style::new().fg(paper())),
        None => Paragraph::new("No stored body: open the item to read it.").style(muted()),
    };
    f.render_widget(text.wrap(Wrap { trim: true }), rest);
}

/// A milestone: its items on the map, each a click away.
fn hub_card(f: &mut ratatui::Frame, area: Rect, app: &mut App, index: usize) {
    let graph = &app.map.graph;
    let members: Vec<usize> = graph.edges.iter().filter(|e| e.1 == index).map(|e| e.0).collect();
    let width = area.width as usize;
    let lines = vec![
        Line::from(Span::styled(model::slug(&graph.nodes[index].title, width), heading())),
        Line::from(Span::styled(
            format!("◆ milestone · {} items on the map", members.len()),
            amber(),
        )),
    ];
    f.render_widget(Paragraph::new(lines), area);
    let mut hits = vec![];
    for (row, &member) in (3..area.height).zip(&members) {
        let node = &graph.nodes[member];
        let rect = Rect::new(area.x, area.y + row, area.width, 1);
        let label = format!("#{} {}", node.number.unwrap_or_default(), node.title);
        f.render_widget(Paragraph::new(model::slug(&label, width)).style(state_style(node)), rect);
        hits.push((rect, CardHit::End(node.id)));
    }
    app.hits.map_card_hits.extend(hits);
}

/// An edge: the link in words, then a two-line card per end.
fn edge_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(edge) = app.map.card_edge() else { return };
    let graph = &app.map.graph;
    let width = area.width as usize;
    let relation = graph.relation(edge, &app.map.links);
    let head = Span::styled(model::slug(&relation, width), Style::new().fg(teal()).bold());
    f.render_widget(Paragraph::new(head), Rect { height: 1.min(area.height), ..area });
    let (a, b, _) = graph.edges[edge];
    let mut hits = vec![];
    for (slot, end) in [a, b].into_iter().enumerate() {
        let y = 2 + slot as u16 * 3;
        if y + 2 > area.height {
            break;
        }
        let node = &graph.nodes[end];
        let rect = Rect::new(area.x, area.y + y, area.width, 2);
        let marker = if app.map.selected == Some(node.id) { "▌" } else { " " };
        let name = match node.number {
            Some(n) => format!("#{n} {}", node.title),
            None => format!("◆ {}", node.title),
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(marker, Style::new().fg(amber())),
                Span::styled(model::slug(&name, width.saturating_sub(1)), heading()),
            ]),
            Line::from(Span::styled(format!("  {}", node_words(app, node)), state_style(node))),
        ];
        f.render_widget(Paragraph::new(lines), rect);
        hits.push((rect, CardHit::End(node.id)));
    }
    app.hits.map_card_hits.extend(hits);
}

/// Kind, state and blocked or ready, as in "issue · open · blocked".
fn node_words(app: &App, node: &MapNode) -> String {
    let item = node.number.and_then(|n| app.map.items.iter().find(|item| item.number == n));
    let Some(item) = item else {
        return node.state.word().to_string();
    };
    let kind = if item.is_pr { "PR" } else { "issue" };
    let state = item.state.to_lowercase();
    match node.state {
        NodeState::Blocked | NodeState::Ready => {
            format!("{kind} · {state} · {}", node.state.word())
        }
        _ => format!("{kind} · {state}"),
    }
}

/// Titles on the card.
fn heading() -> Style {
    Style::new().fg(paper()).add_modifier(Modifier::BOLD)
}

/// A node's state words, in its map color.
fn state_style(node: &MapNode) -> Style {
    Style::new().fg(node_color(node.state))
}
