//! Dependency map state: the stored items and links, the built graph,
//! its force layout, the camera, the selection and a drag in progress.
//! One layout lives for the whole session, so moving between issues of
//! one component keeps every surviving node where it was.

use crate::app::App;
use crate::core::map::{Camera, ZOOM_STEP};
use crate::model::physics::{Layout, Physics};
use crate::model::{self, MapGraph, MapItem, MapLink, MapScope};

/// Layout steps per frame while the map moves: a few, so a frame stays
/// cheap and the motion reads as animation.
const STEPS_PER_FRAME: usize = 4;
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
    /// rebuild mid-drag never moves another node. A release with no motion
    /// on the node that was already selected opens it.
    Node { id: u64, moved: bool, was_selected: bool },
    /// Pressed on empty space: the view follows the pointer from this cell.
    Pan { col: u16, row: u16 },
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
    /// few times while it moves. True when the map changed on screen.
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
        if self.map.moving() {
            for _ in 0..STEPS_PER_FRAME {
                self.map.layout.step();
            }
            self.map.wake = self.map.wake.saturating_sub(STEPS_PER_FRAME);
            changed = true;
        }
        changed
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
        map.wake();
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

    /// Move the view by whole cells; it stops following the graph.
    pub(crate) fn map_pan(&mut self, cols: f64, rows: f64) {
        self.map.camera.pan(cols * PAN_CELLS, rows * PAN_CELLS / 2.0);
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

    /// Put node `index` at view cell `(col, row)` and pin it.
    fn map_place(&mut self, index: usize, col: i32, row: i32) {
        let area = self.hits.map_area;
        let (x, y) = self.map.camera.world(col, row, area.width, area.height);
        self.map.layout.set_position(index, x, y);
        self.map.layout.pin(index);
        self.map.wake();
    }

    /// Left press on the map at view cell `(col, row)`: on node `node` it
    /// selects and grabs it, elsewhere it starts a pan.
    pub(crate) fn map_press(&mut self, node: Option<usize>, col: u16, row: u16) {
        self.map.drag = Some(match node {
            Some(index) => {
                let id = self.map.graph.nodes[index].id;
                let was_selected = self.map.selected == Some(id);
                self.map.selected = Some(id);
                MapDrag::Node { id, moved: false, was_selected }
            }
            None => MapDrag::Pan { col, row },
        });
    }

    /// The held button moved to view cell `(col, row)`.
    pub(crate) fn map_drag_to(&mut self, col: u16, row: u16) {
        match self.map.drag {
            Some(MapDrag::Node { id, was_selected, .. }) => {
                // A rebuild dropped the grabbed node: the drag ends.
                let Some(index) = self.map.graph.index_of(id) else {
                    self.map.drag = None;
                    return;
                };
                // The view holds still under a dragged node.
                self.map.follow = false;
                self.map_place(index, i32::from(col), i32::from(row));
                self.map.drag = Some(MapDrag::Node { id, moved: true, was_selected });
            }
            Some(MapDrag::Pan { col: from_col, row: from_row }) => {
                let (dc, dr) = (f64::from(from_col) - f64::from(col), f64::from(from_row) - f64::from(row));
                self.map.camera.pan(dc, dr);
                self.map.follow = false;
                self.map.drag = Some(MapDrag::Pan { col, row });
            }
            None => {}
        }
    }

    /// The button came up: a still press on the selected node opens it.
    pub(crate) fn map_release(&mut self) {
        if let Some(MapDrag::Node { id, moved: false, was_selected: true }) = self.map.drag.take() {
            if self.map.selected == Some(id) && self.map.graph.index_of(id).is_some() {
                self.map_open();
            }
        }
    }
}
