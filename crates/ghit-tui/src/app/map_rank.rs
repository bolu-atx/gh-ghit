//! The ranking strip beside the map: show or hide it, and pick a row.
//! A pick selects the row's node, so the map lights its blocker chain,
//! and centers the view on it.

use crate::app::{App, MapView};

impl MapView {
    /// Node indexes of the strip's rows, top down: ready work, then
    /// bottlenecks. A node may show in both lists.
    pub(crate) fn strip_nodes(&self) -> Vec<usize> {
        let ranking = &self.ranking;
        ranking.ready.iter().copied().chain(ranking.bottlenecks.iter().map(|&(node, _)| node)).collect()
    }

    /// The strip row that reads as picked: the last picked row while it
    /// still holds the selected node, else the first row that does.
    pub(crate) fn strip_cursor(&self) -> Option<usize> {
        let selected = self.selected_index()?;
        let nodes = self.strip_nodes();
        self.strip_row.filter(|&row| nodes.get(row) == Some(&selected))
            .or_else(|| nodes.iter().position(|&node| node == selected))
    }
}

impl App {
    pub(crate) fn map_toggle_strip(&mut self) {
        self.map.strip = !self.map.strip;
    }

    /// Pick the next (`1`) or previous (`-1`) strip row, wrapping.
    pub(crate) fn map_strip_step(&mut self, delta: isize) {
        let rows = self.map.strip_nodes().len();
        if rows == 0 {
            self.notify("no ready work or bottlenecks on this map".to_string());
            return;
        }
        let next = match self.map.strip_cursor() {
            Some(at) => (at as isize + delta).rem_euclid(rows as isize) as usize,
            None if delta < 0 => rows - 1,
            None => 0,
        };
        self.map_strip_pick(next);
    }

    /// Select the node of strip row `row` and center the view on it.
    pub(crate) fn map_strip_pick(&mut self, row: usize) {
        let Some(&node) = self.map.strip_nodes().get(row) else { return };
        self.map.strip_row = Some(row);
        self.map.selected = Some(self.map.graph.nodes[node].id);
        let body = &self.map.layout.bodies()[node];
        (self.map.camera.x, self.map.camera.y) = (body.x, body.y);
        self.map.follow = false;
    }
}
