//! The Map tab's dependency map. Edges draw first as box-drawing lines
//! with arrowheads at the blocked end; labels draw on top, the selected
//! node and its blocker chain last so they win overlaps. Every label rect
//! lands in `app.hits.map_nodes` and every edge cell in `app.hits.map_edges`
//! for the mouse. An open card docks at the right, the ranking strip at
//! the left, and the map takes the rest.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::app::App;
use ghit_core::core::map::{arrow, line, stroke, LabelDetail};
use ghit_core::model::physics::EdgeStyle;
use ghit_core::model::{self, NodeState};
use crate::theme::{amber, green, muted, paper, pr_ref, red, teal};

/// Card width bounds, in cells.
const CARD_MIN: u16 = 30;
const CARD_MAX: u16 = 42;

/// A label as placed: left cell, row and width, in view cells.
#[derive(Clone, Copy)]
struct Place {
    x: i32,
    y: i32,
    width: i32,
}

impl Place {
    fn holds(&self, (col, row): (i32, i32)) -> bool {
        row == self.y && col >= self.x && col < self.x + self.width
    }
}

pub(super) fn render_map(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.map_nodes.clear();
    app.hits.map_edges.clear();
    app.hits.map_card = Rect::default();
    app.hits.map_card_hits.clear();
    app.hits.map_strip = Rect::default();
    app.hits.map_strip_rows.clear();
    let area = if app.map.card.is_some() && !app.map.graph.nodes.is_empty() {
        let width = (area.width * 2 / 5).clamp(CARD_MIN, CARD_MAX).min(area.width);
        let [map, card] = Layout::horizontal([Constraint::Min(0), Constraint::Length(width)])
            .areas(area);
        super::map_card::render_map_card(f, card, app);
        map
    } else {
        area
    };
    let strip_w = (area.width / 3).min(super::map_strip::STRIP_MAX);
    let area = if app.map.strip && !app.map.graph.nodes.is_empty() && strip_w >= super::map_strip::STRIP_MIN {
        let [strip, map] = Layout::horizontal([Constraint::Length(strip_w), Constraint::Min(0)]).areas(area);
        super::map_strip::render_map_strip(f, strip, app);
        map
    } else {
        area
    };
    app.hits.map_area = area;
    app.hits.map_edges.reset(area);
    if area.width < 4 || area.height < 2 {
        return;
    }
    let buf = f.buffer_mut();
    if app.map.graph.nodes.is_empty() {
        let text = match app.map.center() {
            _ if !app.map.loaded => "Reading the local copy…".to_string(),
            Some(n) => format!(
                "{n} is not in the local copy yet. Press s to see the whole repo.",
                n = n.display("")
            ),
            None if app.map.filter.changed() => {
                "Nothing matches the map filter. Press f to change it.".to_string()
            }
            None => "Nothing to map.".to_string(),
        };
        put(buf, area, (0, 0), &text, Style::new().fg(muted()));
        return;
    }
    app.map.refresh_chain();
    let bodies: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    let camera = app.map.camera;
    let graph = &app.map.graph;
    let card_edge = app.map.card_edge();
    let detail = camera.detail();
    let centers: Vec<(i32, i32)> = bodies.iter().map(|&(x, y)| camera.cell(x, y, area.width, area.height)).collect();
    let selected = app.map.selected_index();
    // The selection always names itself, whatever the zoom.
    let labels: Vec<String> = graph.nodes.iter().enumerate().map(|(i, node)| {
        let detail = if Some(i) == selected { LabelDetail::Title(24).max_title(detail) } else { detail };
        label(node, detail)
    }).collect();
    let places: Vec<Place> = centers.iter().zip(&labels).map(|(&(col, row), text)| {
        let width = Span::raw(text.as_str()).width() as i32;
        Place { x: col - width / 2, y: row, width }
    }).collect();
    let no_chain = HashSet::new();
    let chain = app.map.chain().unwrap_or(&no_chain);
    let lit = |i: usize| Some(i) == selected || chain.contains(&i);

    // Hub edges first, so links and blocks draw over their dots; the
    // card's edge last, so it reads whole.
    let mut edges: Vec<usize> = (0..graph.edges.len()).collect();
    edges.sort_by_key(|&e| (Some(e) == card_edge, graph.edges[e].2 != EdgeStyle::Hub));
    for e in edges {
        let (a, b, style) = graph.edges[e];
        let (from, to) = (centers[a], centers[b]);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let on_chain = style == EdgeStyle::Directed && lit(a) && lit(b);
        let pen = match (Some(e) == card_edge, on_chain) {
            (true, _) => Style::new().fg(teal()).add_modifier(Modifier::BOLD),
            (false, true) => Style::new().fg(amber()).add_modifier(Modifier::BOLD),
            (false, false) => Style::new().fg(muted()),
        };
        let glyph = if style == EdgeStyle::Hub { '·' } else { stroke(dx, dy) };
        let cells = line(from, to);
        for &cell in &cells {
            let free = !places[a].holds(cell) && !places[b].holds(cell);
            if free && put_char(buf, area, cell, glyph, pen) {
                let (x, y) = screen(area, cell);
                app.hits.map_edges.set(x, y, graph.nodes[a].id, graph.nodes[b].id);
            }
        }
        if style == EdgeStyle::Directed {
            // The arrowhead sits where the line leaves the blocked label.
            if let Some(&cell) = cells.iter().rev().find(|cell| !places[b].holds(**cell) && !places[a].holds(**cell)) {
                let lit_edge = on_chain || Some(e) == card_edge;
                let head = if lit_edge { pen } else { Style::new().fg(red()) };
                if put_char(buf, area, cell, arrow(dx, dy), head) {
                    let (x, y) = screen(area, cell);
                    app.hits.map_edges.set(x, y, graph.nodes[a].id, graph.nodes[b].id);
                }
            }
        }
    }

    // Plain items first, then hubs, then the chain, then the selection.
    let rank = |i: usize| match (Some(i) == selected, chain.contains(&i), graph.nodes[i].state) {
        (true, _, _) => 3,
        (_, true, _) => 2,
        (_, _, NodeState::Hub) => 1,
        _ => 0,
    };
    let mut order: Vec<usize> = (0..graph.nodes.len()).collect();
    order.sort_by_key(|&i| rank(i));
    let mut hits = Vec::with_capacity(order.len());
    for i in order {
        let node = &graph.nodes[i];
        let mut style = Style::new().fg(node_color(node.state));
        if node.state == NodeState::Closed {
            style = style.add_modifier(Modifier::DIM);
        }
        if lit(i) {
            style = style.add_modifier(Modifier::BOLD);
        }
        if chain.contains(&i) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        let place = places[i];
        if let Some(rect) = put(buf, area, (place.x, place.y), &labels[i], style) {
            hits.push((rect, node.id));
        }
        if Some(i) == selected {
            put_char(buf, area, (place.x - 1, place.y), '▌', Style::new().fg(amber()));
        }
    }
    hits.reverse();
    app.hits.map_nodes = hits;

    let items = graph.nodes.iter().filter(|node| node.number.is_some()).count();
    let pinned = app.map.layout.bodies().iter().filter(|body| body.pinned).count();
    let scope = match app.map.center() {
        Some(n) => format!("linked to {n}", n = n.display("")),
        None => "whole repo".to_string(),
    };
    let mut summary = format!(" {items} items · {scope}");
    if app.map.filter.changed() {
        summary.push_str(" · filtered");
    }
    if pinned > 0 {
        summary.push_str(&format!(" · {pinned} pinned"));
    }
    put(buf, area, (0, i32::from(area.height) - 1), &format!("{summary} "), Style::new().fg(paper()).add_modifier(Modifier::DIM));
}

/// The color a node state reads in, on the map and on its card.
pub(super) fn node_color(state: NodeState) -> Color {
    match state {
        NodeState::Blocked => red(),
        NodeState::Ready => green(),
        NodeState::Pr => pr_ref(),
        NodeState::Closed => muted(),
        NodeState::Hub => amber(),
    }
}

/// Label text at a zoom: `#n title…`, `#n` or `•`; hubs keep a box.
fn label(node: &model::MapNode, detail: LabelDetail) -> String {
    match (node.number, detail) {
        (Some(n), LabelDetail::Title(width)) => {
            format!("{n} {}", model::slug(&node.title, width), n = n.display(""))
        }
        (Some(n), LabelDetail::Number) => n.display(""),
        (Some(_), LabelDetail::Dot) => "•".to_string(),
        (None, LabelDetail::Dot) => "◆".to_string(),
        (None, LabelDetail::Number) => format!("╭ {} ╮", model::slug(&node.title, 14)),
        (None, LabelDetail::Title(_)) => format!("╭ {} ╮", model::slug(&node.title, 28)),
    }
}

/// One glyph at view cell `cell`, dropped when it falls outside `area`.
/// True when it was drawn.
fn put_char(buf: &mut Buffer, area: Rect, cell: (i32, i32), glyph: char, style: Style) -> bool {
    let (col, row) = cell;
    if col < 0 || row < 0 || col >= i32::from(area.width) || row >= i32::from(area.height) {
        return false;
    }
    buf[screen(area, cell)].set_char(glyph).set_style(style);
    true
}

/// Screen cell of view cell `(col, row)`, which must lie inside `area`.
fn screen(area: Rect, (col, row): (i32, i32)) -> (u16, u16) {
    (area.x + col as u16, area.y + row as u16)
}

/// `text` from view cell `(col, row)`, clipped to `area`. Returns the
/// screen rect it covers, or `None` when nothing shows.
fn put(buf: &mut Buffer, area: Rect, (col, row): (i32, i32), text: &str, style: Style) -> Option<Rect> {
    if row < 0 || row >= i32::from(area.height) {
        return None;
    }
    let (mut x, mut first, mut last) = (col, None, None);
    for ch in text.chars() {
        let width = Span::raw(ch.to_string()).width() as i32;
        if x >= 0 && x + width <= i32::from(area.width) {
            let cell = (area.x + x as u16, area.y + row as u16);
            buf[cell].set_char(ch).set_style(style);
            first.get_or_insert(x);
            last = Some(x + width);
        }
        x += width;
    }
    let (first, last) = (first?, last?);
    Some(Rect::new(area.x + first as u16, area.y + row as u16, (last - first) as u16, 1))
}
