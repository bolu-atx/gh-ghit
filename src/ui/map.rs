//! The Graph tab's dependency map. Edges draw first as box-drawing lines
//! with arrowheads at the blocked end; labels draw on top, the selected
//! node and its blocker chain last so they win overlaps. Every label rect
//! lands in `app.hits.map_nodes` for the mouse.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::app::App;
use crate::core::map::{arrow, line, stroke, Camera, LabelDetail};
use crate::model::physics::EdgeStyle;
use crate::model::{self, MapScope, NodeState};
use crate::theme::{amber, green, muted, paper, pr_ref, red};

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
    app.hits.map_area = area;
    app.hits.map_nodes.clear();
    if area.width < 4 || area.height < 2 {
        return;
    }
    let buf = f.buffer_mut();
    if app.map.graph.nodes.is_empty() {
        let text = match app.nav.current {
            _ if !app.map.loaded => "Reading the local copy…".to_string(),
            Some(n) => format!("#{n} is not in the local copy yet. Press s to see the whole repo."),
            None => "Nothing to map.".to_string(),
        };
        put(buf, area, (0, 0), &text, Style::new().fg(muted()));
        return;
    }
    let bodies: Vec<(f64, f64)> = app.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
    if app.map.follow {
        app.map.camera = Camera::fit(&bodies, area.width, area.height);
    }
    let camera = app.map.camera;
    let graph = &app.map.graph;
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
    let chain: HashSet<usize> = selected.map(|i| graph.blockers_of(i)).unwrap_or_default();
    let lit = |i: usize| Some(i) == selected || chain.contains(&i);

    // Hub edges first, so links and blocks draw over their dots.
    let mut edges = graph.edges.clone();
    edges.sort_by_key(|&(_, _, style)| style != EdgeStyle::Hub);
    for &(a, b, style) in &edges {
        let (from, to) = (centers[a], centers[b]);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let on_chain = style == EdgeStyle::Directed && lit(a) && lit(b);
        let pen = if on_chain { Style::new().fg(amber()).add_modifier(Modifier::BOLD) } else { Style::new().fg(muted()) };
        let glyph = if style == EdgeStyle::Hub { '·' } else { stroke(dx, dy) };
        let cells = line(from, to);
        for &cell in &cells {
            if !places[a].holds(cell) && !places[b].holds(cell) {
                put_char(buf, area, cell, glyph, pen);
            }
        }
        if style == EdgeStyle::Directed {
            // The arrowhead sits where the line leaves the blocked label.
            if let Some(&cell) = cells.iter().rev().find(|cell| !places[b].holds(**cell) && !places[a].holds(**cell)) {
                let head = if on_chain { pen } else { Style::new().fg(red()) };
                put_char(buf, area, cell, arrow(dx, dy), head);
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
        let mut style = Style::new().fg(match node.state {
            NodeState::Blocked => red(),
            NodeState::Ready => green(),
            NodeState::Pr => pr_ref(),
            NodeState::Closed => muted(),
            NodeState::Hub => amber(),
        });
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
            hits.push((rect, i));
        }
        if Some(i) == selected {
            put_char(buf, area, (place.x - 1, place.y), '▌', Style::new().fg(amber()));
        }
    }
    hits.reverse();
    app.hits.map_nodes = hits;

    let items = graph.nodes.iter().filter(|node| node.number.is_some()).count();
    let pinned = app.map.layout.bodies().iter().filter(|body| body.pinned).count();
    let scope = match (app.map.scope, app.nav.current) {
        (MapScope::Component, Some(n)) => format!("linked to #{n}"),
        _ => "whole repo".to_string(),
    };
    let mut summary = format!(" {items} items · {scope}");
    if pinned > 0 {
        summary.push_str(&format!(" · {pinned} pinned"));
    }
    put(buf, area, (0, i32::from(area.height) - 1), &format!("{summary} "), Style::new().fg(paper()).add_modifier(Modifier::DIM));
}

/// Label text at a zoom: `#n title…`, `#n` or `•`; hubs keep a box.
fn label(node: &model::MapNode, detail: LabelDetail) -> String {
    match (node.number, detail) {
        (Some(n), LabelDetail::Title(width)) => format!("#{n} {}", model::slug(&node.title, width)),
        (Some(n), LabelDetail::Number) => format!("#{n}"),
        (Some(_), LabelDetail::Dot) => "•".to_string(),
        (None, LabelDetail::Dot) => "◆".to_string(),
        (None, LabelDetail::Number) => format!("╭ {} ╮", model::slug(&node.title, 14)),
        (None, LabelDetail::Title(_)) => format!("╭ {} ╮", model::slug(&node.title, 28)),
    }
}

/// One glyph at view cell `cell`, dropped when it falls outside `area`.
fn put_char(buf: &mut Buffer, area: Rect, (col, row): (i32, i32), glyph: char, style: Style) {
    if col < 0 || row < 0 || col >= i32::from(area.width) || row >= i32::from(area.height) {
        return;
    }
    buf[(area.x + col as u16, area.y + row as u16)].set_char(glyph).set_style(style);
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
