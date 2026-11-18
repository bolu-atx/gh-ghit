//! Dependency graph types, building and section ordering.

use super::*;

/// Cap on nodes in one dependency graph (center included). The graph is a
/// 1-hop star around the current issue; a full-repo layout is out of scope.
pub const MAX_GRAPH_NODES: usize = 24;

/// One relationship in the dependency graph, named from the center issue's
/// perspective: `Blocks` means the center blocks the neighbor, `BlockedBy`
/// the reverse, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphEdgeKind {
    Blocks,
    BlockedBy,
    Parent,
    Child,
    References,
    ReferencedBy,
    Closes,
}

/// Section order for the graph pane: blast radius first, then origins,
/// then plain references.
pub const GRAPH_SECTION_ORDER: [GraphEdgeKind; 7] = [
    GraphEdgeKind::Blocks,
    GraphEdgeKind::BlockedBy,
    GraphEdgeKind::Parent,
    GraphEdgeKind::Child,
    GraphEdgeKind::Closes,
    GraphEdgeKind::References,
    GraphEdgeKind::ReferencedBy,
];

/// Pane section header per edge kind.
pub fn graph_edge_label(kind: GraphEdgeKind) -> &'static str {
    match kind {
        GraphEdgeKind::Blocks => "Blocks",
        GraphEdgeKind::BlockedBy => "Blocked by",
        GraphEdgeKind::Parent => "Parent",
        GraphEdgeKind::Child => "Sub-issues",
        GraphEdgeKind::References => "References",
        GraphEdgeKind::ReferencedBy => "Referenced by",
        GraphEdgeKind::Closes => "Closing PRs",
    }
}

/// A PR linked to a graph node (closing reference), with its merge state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphLinkedPr {
    pub number: u64,
    #[serde(default)]
    pub state: String,
}

/// One issue or PR in the dependency graph: number, slug title, state,
/// project status, label chips, and linked PR numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_pr: bool,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub project_status: Option<String>,
    #[serde(default)]
    pub linked_prs: Vec<GraphLinkedPr>,
}

/// A directed relationship between two graph nodes. `Blocks` edges point
/// from the blocker to the blocked; every other kind points from the
/// neighbor toward the center or vice versa per `edge_for`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: u64,
    pub to: u64,
    pub kind: GraphEdgeKind,
}

/// The 1-hop dependency star around one issue: the center plus its capped
/// neighbors. `truncated` means neighbors were dropped at the cap;
/// `from_references` means blocked-by/blocking fields were unavailable and
/// reference edges were derived from body `#` refs instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphData {
    pub center: u64,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub from_references: bool,
}

/// Directed edge for one neighbor `kind`, from the center's perspective.
pub fn edge_for(center: u64, neighbor: u64, kind: GraphEdgeKind) -> GraphEdge {
    match kind {
        GraphEdgeKind::Blocks
        | GraphEdgeKind::Child
        | GraphEdgeKind::References => GraphEdge { from: center, to: neighbor, kind },
        GraphEdgeKind::BlockedBy
        | GraphEdgeKind::Parent
        | GraphEdgeKind::ReferencedBy
        | GraphEdgeKind::Closes => GraphEdge { from: neighbor, to: center, kind },
    }
}

/// Build the graph from the center plus neighbors in priority order (first
/// edge wins on duplicates). Nodes past `MAX_GRAPH_NODES` drop and set
/// `truncated`. The center node itself carries no edge.
pub fn build_graph(center: GraphNode, neighbors: Vec<(GraphNode, GraphEdgeKind)>) -> GraphData {
    let center_number = center.number;
    let mut nodes = vec![center];
    let mut edges = vec![];
    let mut truncated = false;
    for (node, kind) in neighbors {
        if node.number == center_number || nodes.iter().any(|n| n.number == node.number) {
            continue;
        }
        if nodes.len() >= MAX_GRAPH_NODES {
            truncated = true;
            continue;
        }
        edges.push(edge_for(center_number, node.number, kind));
        nodes.push(node);
    }
    GraphData { center: center_number, nodes, edges, truncated, from_references: false }
}

/// True when `number` blocks something: it is the blocker end of a
/// blocking relationship. Both `Blocks` and `BlockedBy` edges point from
/// the blocker to the blocked (the kind names the center's perspective),
/// so the blocker is always `from`. Nodes without one render gray/dim.
#[cfg(test)]
pub fn graph_blocks_something(data: &GraphData, number: u64) -> bool {
    data.edges.iter().any(|e| {
        e.from == number && matches!(e.kind, GraphEdgeKind::Blocks | GraphEdgeKind::BlockedBy)
    })
}

/// Node indices grouped by edge kind in `GRAPH_SECTION_ORDER`. The center
/// node carries no edge, so it never appears here; see `graph_order`.
pub fn graph_sections(data: &GraphData) -> Vec<(GraphEdgeKind, Vec<usize>)> {
    let kind_of = |idx: usize| {
        let n = data.nodes[idx].number;
        data.edges
            .iter()
            .find(|e| (e.from == n && e.to == data.center) || (e.from == data.center && e.to == n))
            .map(|e| e.kind)
    };
    let mut out = vec![];
    for kind in GRAPH_SECTION_ORDER {
        let mut group = vec![];
        for (idx, node) in data.nodes.iter().enumerate() {
            if node.number != data.center && kind_of(idx) == Some(kind) {
                group.push(idx);
            }
        }
        if !group.is_empty() {
            out.push((kind, group));
        }
    }
    out
}

/// Flat navigation order: the center first, then neighbors section by
/// section. Indices into `data.nodes`.
pub fn graph_order(data: &GraphData) -> Vec<usize> {
    let mut order = vec![];
    if let Some(center) = data.nodes.iter().position(|n| n.number == data.center) {
        order.push(center);
    }
    for (_, group) in graph_sections(data) {
        order.extend(group);
    }
    order
}
