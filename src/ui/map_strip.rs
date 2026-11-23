//! The ranking strip, docked at the left of the map: ready work, then
//! bottlenecks, then each milestone's critical path. Ready and
//! bottleneck rows land in `app.hits.map_strip_rows` for the mouse; the
//! picked row carries the selection marker.

use ratatui::layout::{Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::map::node_color;
use crate::app::App;
use crate::model::{self, MapGraph};
use crate::theme::{amber, base, ink, muted, paper};

/// Strip width bounds, in cells.
pub(super) const STRIP_MAX: u16 = 30;
pub(super) const STRIP_MIN: u16 = 18;

pub(super) fn render_map_strip(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.map_strip = area;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Span::styled(" ranks ", Style::new().fg(amber()).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area).inner(Margin::new(1, 0));
    f.render_widget(Clear, area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let graph = &app.map.graph;
    let ranking = &app.map.ranking;
    let cursor = app.map.strip_cursor();
    let heading = |text: &str| (Line::from(Span::styled(text.to_string(), Style::new().fg(muted()))), None);
    let empty = |text: &str| (Line::from(Span::styled(text.to_string(), Style::new().fg(muted()).add_modifier(Modifier::DIM))), None);
    // Each line, with the strip row it picks.
    let mut lines: Vec<(Line<'static>, Option<usize>)> = vec![heading("Ready")];
    let mut row = 0;
    for &node in &ranking.ready {
        lines.push((item_line(graph, node, None, width, cursor == Some(row)), Some(row)));
        row += 1;
    }
    if ranking.ready.is_empty() {
        lines.push(empty("nothing ready"));
    }
    lines.push((Line::default(), None));
    lines.push(heading("Bottlenecks"));
    for &(node, count) in &ranking.bottlenecks {
        lines.push((item_line(graph, node, Some(count), width, cursor == Some(row)), Some(row)));
        row += 1;
    }
    if ranking.bottlenecks.is_empty() {
        lines.push(empty("nothing blocks open work"));
    }
    if !ranking.critical.is_empty() {
        lines.push((Line::default(), None));
        lines.push(heading("Critical path"));
    }
    for (hub, chain) in &ranking.critical {
        let title = model::slug(&graph.nodes[*hub].title, width);
        lines.push((
            Line::from(Span::styled(title, Style::new().fg(amber()))),
            None,
        ));
        let numbers: Vec<String> = chain
            .iter()
            .filter_map(|&i| graph.nodes[i].number)
            .map(|n| n.display(""))
            .collect();
        let text = format!(" {}", model::slug(&numbers.join(" → "), width - 1));
        lines.push((Line::from(Span::styled(text, Style::new().fg(paper()))), None));
    }

    // Scroll just enough to keep the picked row in view.
    let at = lines.iter().position(|(_, row)| row.is_some() && *row == cursor).unwrap_or(0);
    let skip = at.saturating_sub(inner.height as usize - 1);
    let rows = lines.into_iter().skip(skip).take(inner.height as usize);
    for (y, (line, row)) in (inner.y..).zip(rows) {
        let rect = Rect::new(inner.x, y, inner.width, 1);
        f.render_widget(Paragraph::new(line), rect);
        if let Some(row) = row {
            app.hits.map_strip_rows.push((rect, row));
        }
    }
}

/// One ranked item: `#n title`, or `#n unblocks k title` for a
/// bottleneck. The picked row is bold behind the marker.
fn item_line(graph: &MapGraph, node: usize, unblocks: Option<usize>, width: usize, picked: bool) -> Line<'static> {
    let item = &graph.nodes[node];
    let number = item.number.map_or(String::new(), |n| n.display(""));
    let mut style = Style::new().fg(node_color(item.state));
    if picked {
        style = style.add_modifier(Modifier::BOLD);
    }
    let marker = if picked { "▌" } else { " " };
    let mut spans = vec![Span::styled(marker, Style::new().fg(amber())), Span::styled(number.clone(), style)];
    let mut used = 1 + number.chars().count();
    if let Some(count) = unblocks {
        let text = format!(" unblocks {count}");
        used += text.chars().count();
        spans.push(Span::styled(text, Style::new().fg(paper())));
    }
    let room = width.saturating_sub(used + 1);
    if room > 2 {
        spans.push(Span::styled(format!(" {}", model::slug(&item.title, room)), Style::new().fg(muted())));
    }
    Line::from(spans)
}
