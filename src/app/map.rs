//! Dependency map state: the stored items and links, the built graph,
//! its force layout, the camera, the selection and a drag in progress.
//! One layout lives for the whole session, so moving between issues of
//! one component keeps every surviving node where it was.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::App;
use crate::core::map::{Camera, ZOOM_STEP};
use crate::github;
use crate::model::physics::{Layout, Physics};
use crate::model::{self, MapGraph, MapItem, MapLink, MapScope};

/// Most layout steps per frame while the map moves: a few, so the motion
/// reads as animation.
const MAX_STEPS_PER_FRAME: usize = 4;
/// Time one frame may spend on layout steps. A step is O(n²), so a large
/// repo takes fewer steps per frame and input stays responsive.
const FRAME_BUDGET: Duration = Duration::from_millis(4);
/// Steps every change runs at least, so a layout at rest starts moving.
const WAKE_STEPS: usize = 40;
/// Layout seed. Fixed, so a repo lays out the same way every run.
const SEED: u64 = 270;
/// Cells one pan key moves the view.
const PAN_CELLS: f64 = 6.0;
/// Cells one move key shifts the selected node.
const NUDGE_CELLS: i32 = 2;

/// A held left button on the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MapDrag {
    /// Pressed on the node with layout id `id`. An id, not an index, so a
    /// rebuild mid-drag never moves another node. A release with no
    /// motion shows the node's card, or opens the node when its card
    /// already showed (`shown`).
    Node { id: u64, moved: bool, shown: bool },
    /// Pressed off the nodes: the view follows the pointer from this cell.
    /// A release with no motion on an edge shows its card.
    Pan { col: u16, row: u16, edge: Option<(u64, u64)>, moved: bool },
}

/// The card docked at the right of the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapCard {
    /// The selected node's details.
    Node,
    /// One edge, named by the layout ids of its ends.
    Edge { src: u64, dst: u64 },
}

/// A clickable part of the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardHit {
    Open,
    Close,
    NextEdge,
    /// An end of the shown edge, by layout id.
    End(u64),
}

/// The stored row and body of the item a node card shows. Read once per
/// selection and store change, never per frame.
pub(crate) struct CardDetail {
    pub(crate) number: u64,
    /// Store change counter at the read.
    pub(crate) version: Option<i64>,
    pub(crate) row: Option<github::Row>,
    pub(crate) body: Option<String>,
}

pub(crate) struct MapView {
    pub(crate) items: Vec<MapItem>,
    pub(crate) links: Vec<MapLink>,
    /// True once the store was read.
    pub(crate) loaded: bool,
    /// Store change counter at the last read.
    pub(crate) version: Option<i64>,
    pub(crate) scope: MapScope,
    pub(crate) graph: MapGraph,
    /// Center and scope `graph` was built for; `None` forces a rebuild.
    built: Option<(Option<u64>, MapScope)>,
    /// Center of the last build. A data reread keeps the selection; a new
    /// center takes it.
    center: Option<u64>,
    pub(crate) layout: Layout,
    pub(crate) camera: Camera,
    /// The camera fits the whole graph each frame until a pan or zoom.
    pub(crate) follow: bool,
    /// Layout id of the selected node.
    pub(crate) selected: Option<u64>,
    pub(crate) drag: Option<MapDrag>,
    pub(crate) card: Option<MapCard>,
    pub(crate) detail: Option<CardDetail>,
    /// The next frame brings the card's subject into view.
    reveal: bool,
    /// Map area the camera last fit for.
    view: Rect,
    /// Steps still owed before the layout may rest.
    wake: usize,
}

impl Default for MapView {
    fn default() -> Self {
        MapView {
            items: vec![],
            links: vec![],
            loaded: false,
            version: None,
            scope: MapScope::default(),
            graph: MapGraph::default(),
            built: None,
            center: None,
            layout: Layout::new(SEED, Physics::default()),
            camera: Camera::default(),
            follow: true,
            selected: None,
            drag: None,
            card: None,
            detail: None,
            reveal: false,
            view: Rect::default(),
            wake: 0,
        }
    }
}

impl MapView {
    /// Index of the selected node in `graph`.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.graph.index_of(self.selected?)
    }

    /// Nodes that pick and open: items, not hubs.
    fn item_order(&self) -> Vec<usize> {
        (0..self.graph.nodes.len()).filter(|&i| self.graph.nodes[i].number.is_some()).collect()
    }

    /// The layout still moves, or owes steps after a change.
    pub(crate) fn moving(&self) -> bool {
        self.wake > 0 || !self.layout.settled()
    }

    fn wake(&mut self) {
        self.wake = WAKE_STEPS;
    }

    /// Index of the edge the card shows.
    pub(crate) fn card_edge(&self) -> Option<usize> {
        match self.card? {
            MapCard::Edge { src, dst } => self.graph.edge_between(src, dst),
            MapCard::Node => None,
        }
    }

    /// Item number the node card shows.
    fn card_number(&self) -> Option<u64> {
        match self.card? {
            MapCard::Node => self.graph.nodes[self.selected_index()?].number,
            MapCard::Edge { .. } => None,
        }
    }

    /// Show `card` and bring it into view.
    fn show(&mut self, card: MapCard) {
        self.card = Some(card);
        self.reveal = true;
    }

    /// Show the card of edge `edge`.
    fn show_edge(&mut self, edge: usize) {
        let (a, b, _) = self.graph.edges[edge];
        self.show(MapCard::Edge { src: self.graph.nodes[a].id, dst: self.graph.nodes[b].id });
    }
}

impl App {
    /// Replace the stored items and links the map draws from. Data equal
    /// to what the map holds changes nothing, so a sync that touched no
    /// map field leaves the layout at rest.
    pub(crate) fn set_map_data(&mut self, items: Vec<MapItem>, links: Vec<MapLink>) {
        if self.map.loaded && items == self.map.items && links == self.map.links {
            return;
        }
        self.map.items = items;
        self.map.links = links;
        self.map.loaded = true;
        self.map.built = None;
    }

    /// Per-frame map work while the Graph tab shows: reread the store after
    /// a write, rebuild on a new center or scope, then step the layout a
    /// few times within the frame budget while it moves. True when the map
    /// changed on screen.
    pub(crate) fn map_tick(&mut self) -> bool {
        if !self.visual_graph() {
            return false;
        }
        if let Some(link) = self.sections.link.as_ref() {
            let version = link.data_version();
            if !self.map.loaded || version != self.map.version {
                self.map.version = version;
                if let Some((items, links)) = link.map_data() {
                    self.set_map_data(items, links);
                }
            }
        }
        let mut changed = self.rebuild_map();
        changed |= self.read_card_detail();
        if self.map.moving() {
            self.step_map(FRAME_BUDGET);
            changed = true;
        }
        changed
    }

    /// While the camera follows, fit the whole graph in the map's last
    /// drawn area. True when the camera moved.
    fn fit_map_view(&mut self) -> bool {
        let area = self.hits.map_area;
        self.map.view = area;
        if !self.map.follow || !map_drawable(area) || self.map.graph.nodes.is_empty() {
            return false;
        }
        let bodies: Vec<(f64, f64)> = self.map.layout.bodies().iter().map(|b| (b.x, b.y)).collect();
        let camera = Camera::fit(&bodies, area.width, area.height);
        std::mem::replace(&mut self.map.camera, camera) != camera
    }

    /// Before a render: while the camera follows, fit the graph as it is
    /// now in the map's last drawn area.
    pub(crate) fn map_before_draw(&mut self) {
        if self.visual_graph() {
            self.fit_map_view();
        }
    }

    /// After a render, which records the map's area: fit a new area and
    /// bring a pending card subject into view. True when the camera moved,
    /// so the frame draws again.
    pub(crate) fn map_after_draw(&mut self) -> bool {
        if !self.visual_graph() {
            return false;
        }
        let mut moved = false;
        if self.hits.map_area != self.map.view {
            moved |= self.fit_map_view();
        }
        let area = self.hits.map_area;
        if self.map.reveal && map_drawable(area) && !self.map.graph.nodes.is_empty() {
            self.map.reveal = false;
            if !self.map.follow {
                moved |= self.reveal_card_subject(area);
            }
        }
        moved
    }

    /// Center the view on the card's subject when it sits outside `area`:
    /// the selected node, or the middle of the shown edge. True when the
    /// camera moved.
    fn reveal_card_subject(&mut self, area: Rect) -> bool {
        let bodies = self.map.layout.bodies();
        let target = match self.map.card_edge() {
            Some(e) => {
                let (a, b, _) = self.map.graph.edges[e];
                ((bodies[a].x + bodies[b].x) / 2.0, (bodies[a].y + bodies[b].y) / 2.0)
            }
            None => match self.map.selected_index() {
                Some(i) => (bodies[i].x, bodies[i].y),
                None => return false,
            },
        };
        let (col, row) = self.map.camera.cell(target.0, target.1, area.width, area.height);
        if col < 1 || row < 0 || col >= i32::from(area.width) - 1 || row >= i32::from(area.height) - 1 {
            (self.map.camera.x, self.map.camera.y) = target;
            return true;
        }
        false
    }

    /// Step the layout until `budget` elapses: at least one step, at most
    /// `MAX_STEPS_PER_FRAME`. Returns the steps taken.
    pub(crate) fn step_map(&mut self, budget: Duration) -> usize {
        let start = Instant::now();
        let mut steps = 0;
        while steps < MAX_STEPS_PER_FRAME {
            self.map.layout.step();
            steps += 1;
            if start.elapsed() >= budget {
                break;
            }
        }
        self.map.wake = self.map.wake.saturating_sub(steps);
        steps
    }

    /// Rebuild the graph when the center or scope moved. A new center
    /// takes the selection.
    fn rebuild_map(&mut self) -> bool {
        let center = self.nav.current;
        let want = (center, self.map.scope);
        if self.map.built == Some(want) {
            return false;
        }
        let map = &mut self.map;
        let new_center = map.center != center;
        map.center = center;
        map.graph = model::build_map(&map.items, &map.links, center, map.scope);
        map.layout.rebuild(&map.graph.ids(), &map.graph.edges);
        map.built = Some(want);
        if new_center || map.selected_index().is_none() {
            map.selected = center.filter(|n| map.graph.index_of(*n).is_some());
        }
        // A card whose subject left the graph closes.
        let gone = match map.card {
            Some(MapCard::Node) => map.selected_index().is_none(),
            Some(MapCard::Edge { .. }) => map.card_edge().is_none(),
            None => false,
        };
        if gone {
            map.card = None;
        }
        map.wake();
        true
    }

    /// Read the stored row and body for the node card when its item or
    /// the store changed. True when a new read landed.
    fn read_card_detail(&mut self) -> bool {
        let Some(number) = self.map.card_number() else { return false };
        let version = self.map.version;
        if self.map.detail.as_ref().is_some_and(|d| d.number == number && d.version == version) {
            return false;
        }
        let read = self.sections.link.as_ref().and_then(|link| link.item_card(number));
        let (row, body) = match read {
            Some((row, body)) => (Some(row), body),
            None => (None, None),
        };
        self.map.detail = Some(CardDetail { number, version, row, body });
        true
    }

    /// Run the layout to rest, for frame tests.
    #[cfg(test)]
    pub(crate) fn settle_map(&mut self) {
        self.rebuild_map();
        for _ in 0..3000 {
            if !self.map.moving() {
                break;
            }
            self.map.layout.step();
            self.map.wake = self.map.wake.saturating_sub(1);
        }
    }

    /// Select the next (`1`) or previous (`-1`) item node, wrapping.
    pub(crate) fn map_step(&mut self, delta: isize) {
        let order = self.map.item_order();
        if order.is_empty() {
            return;
        }
        let at = self.map.selected_index().and_then(|i| order.iter().position(|&o| o == i));
        let next = match at {
            Some(at) => (at as isize + delta).rem_euclid(order.len() as isize) as usize,
            None if delta < 0 => order.len() - 1,
            None => 0,
        };
        self.map.selected = Some(self.map.graph.nodes[order[next]].id);
    }

    /// Select the first or last item node.
    pub(crate) fn map_select_end(&mut self, last: bool) {
        let order = self.map.item_order();
        let pick = if last { order.last() } else { order.first() };
        if let Some(&i) = pick {
            self.map.selected = Some(self.map.graph.nodes[i].id);
        }
    }

    /// Open the selected item in the viewer. A hub only names itself.
    pub(crate) fn map_open(&mut self) {
        let Some(node) = self.map.selected_index().map(|i| &self.map.graph.nodes[i]) else { return };
        match node.number {
            Some(n) if Some(n) != self.nav.current => self.open_issue(n),
            Some(_) => {}
            None => self.notify(format!("milestone: {}", node.title)),
        }
    }

    /// Move the view by pan steps; it stops following the graph.
    pub(crate) fn map_pan(&mut self, cols: f64, rows: f64) {
        self.map_scroll(cols * PAN_CELLS, rows * PAN_CELLS / 2.0);
    }

    /// Move the view by `cols` x `rows` cells; it stops following the graph.
    pub(crate) fn map_scroll(&mut self, cols: f64, rows: f64) {
        self.map.camera.pan(cols, rows);
        self.map.follow = false;
    }

    /// Zoom in (`1`) or out (`-1`) one step about the view center.
    pub(crate) fn map_zoom(&mut self, steps: i32) {
        self.map.camera.zoom_by(ZOOM_STEP.powi(steps));
        self.map.follow = false;
    }

    /// Fit the whole graph in view and keep it fitted while it moves.
    pub(crate) fn map_fit(&mut self) {
        self.map.follow = true;
    }

    /// Flip between the open issue's component and the whole repo.
    pub(crate) fn map_toggle_scope(&mut self) {
        self.map.scope = match self.map.scope {
            MapScope::Component => MapScope::Repo,
            MapScope::Repo => MapScope::Component,
        };
        self.map.follow = true;
        self.notify(match self.map.scope {
            MapScope::Component => "map: linked items".to_string(),
            MapScope::Repo => "map: whole repo".to_string(),
        });
    }

    /// Release the selected node's pin so the layout moves it again.
    pub(crate) fn map_unpin(&mut self) {
        let Some(i) = self.map.selected_index() else { return };
        if !self.map.layout.bodies()[i].pinned {
            self.notify("node is not pinned: drag it to pin".to_string());
            return;
        }
        self.map.layout.unpin(i);
        self.map.wake();
        self.notify("unpinned".to_string());
    }

    /// Shift the selected node a few cells and pin it there.
    pub(crate) fn map_nudge(&mut self, cols: i32, rows: i32) {
        let Some(i) = self.map.selected_index() else { return };
        let area = self.hits.map_area;
        let body = &self.map.layout.bodies()[i];
        let (col, row) = self.map.camera.cell(body.x, body.y, area.width, area.height);
        self.map_place(i, col + cols * NUDGE_CELLS, row + rows * NUDGE_CELLS / 2);
    }

    /// Put node `index` at view cell `(col, row)` and pin it. The view
    /// stops following, so it holds still under the placed node.
    fn map_place(&mut self, index: usize, col: i32, row: i32) {
        self.map.follow = false;
        let area = self.hits.map_area;
        let (x, y) = self.map.camera.world(col, row, area.width, area.height);
        self.map.layout.set_position(index, x, y);
        self.map.layout.pin(index);
        self.map.wake();
    }

    /// Left press on the map at view cell `(col, row)`: on node `node` it
    /// selects and grabs it, elsewhere (on edge `edge` or empty space) it
    /// starts a pan.
    pub(crate) fn map_press(
        &mut self,
        node: Option<usize>,
        edge: Option<usize>,
        col: u16,
        row: u16,
    ) {
        self.map.drag = Some(match node {
            Some(index) => {
                let id = self.map.graph.nodes[index].id;
                let shown = self.map.selected == Some(id) && self.map.card == Some(MapCard::Node);
                self.map.selected = Some(id);
                MapDrag::Node { id, moved: false, shown }
            }
            None => {
                let graph = &self.map.graph;
                let ends = |e: usize| (graph.nodes[graph.edges[e].0].id, graph.nodes[graph.edges[e].1].id);
                MapDrag::Pan { col, row, edge: edge.map(ends), moved: false }
            }
        });
    }

    /// The held button moved to view cell `(col, row)`.
    pub(crate) fn map_drag_to(&mut self, col: u16, row: u16) {
        match self.map.drag {
            Some(MapDrag::Node { id, shown, .. }) => {
                // A rebuild dropped the grabbed node: the drag ends.
                let Some(index) = self.map.graph.index_of(id) else {
                    self.map.drag = None;
                    return;
                };
                self.map_place(index, i32::from(col), i32::from(row));
                self.map.drag = Some(MapDrag::Node { id, moved: true, shown });
            }
            Some(MapDrag::Pan { col: from_col, row: from_row, edge, .. }) => {
                if (from_col, from_row) == (col, row) {
                    return;
                }
                let (dc, dr) = (f64::from(from_col) - f64::from(col), f64::from(from_row) - f64::from(row));
                self.map_scroll(dc, dr);
                self.map.drag = Some(MapDrag::Pan { col, row, edge, moved: true });
            }
            None => {}
        }
    }

    /// The button came up. A still press on a node shows its card, or
    /// opens it when the card already showed it (the app's second click).
    /// A still press on an edge shows the edge's card.
    pub(crate) fn map_release(&mut self) {
        match self.map.drag.take() {
            Some(MapDrag::Node { id, moved: false, shown }) => {
                if self.map.selected != Some(id) || self.map.graph.index_of(id).is_none() {
                    return;
                }
                if shown {
                    self.map_open();
                } else {
                    self.map.show(MapCard::Node);
                }
            }
            Some(MapDrag::Pan { edge: Some((src, dst)), moved: false, .. }) => {
                if let Some(edge) = self.map.graph.edge_between(src, dst) {
                    self.map.show_edge(edge);
                }
            }
            _ => {}
        }
    }

    /// Show the selected node's card, or hide the card when it shows.
    pub(crate) fn map_toggle_card(&mut self) {
        if self.map.card == Some(MapCard::Node) {
            self.map.card = None;
        } else if self.map.selected_index().is_some() {
            self.map.show(MapCard::Node);
        }
    }

    pub(crate) fn map_close_card(&mut self) {
        self.map.card = None;
    }

    /// Show the card of the next (`1`) or previous (`-1`) edge of the
    /// selected node, wrapping.
    pub(crate) fn map_step_edge(&mut self, delta: isize) {
        let Some(index) = self.map.selected_index() else { return };
        let edges = self.map.graph.edges_of(index);
        if edges.is_empty() {
            self.notify("this node has no links".to_string());
            return;
        }
        let at = self.map.card_edge().and_then(|e| edges.iter().position(|&o| o == e));
        let next = match at {
            Some(at) => (at as isize + delta).rem_euclid(edges.len() as isize) as usize,
            None if delta < 0 => edges.len() - 1,
            None => 0,
        };
        self.map.show_edge(edges[next]);
    }

    /// A click on part of the card.
    pub(crate) fn map_card_click(&mut self, hit: CardHit) {
        match hit {
            CardHit::Open => self.map_open(),
            CardHit::Close => self.map_close_card(),
            CardHit::NextEdge => self.map_step_edge(1),
            CardHit::End(id) => {
                if self.map.graph.index_of(id).is_some() {
                    self.map.selected = Some(id);
                    self.map.show(MapCard::Node);
                }
            }
        }
    }
}

/// The map has room to draw in `area`.
fn map_drawable(area: Rect) -> bool {
    area.width >= 4 && area.height >= 2
}
