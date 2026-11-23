//! Dependency map: issues and PRs as nodes, milestones as hubs, stored
//! links as edges. Pure: built from the local copy's items and links,
//! then laid out by `physics`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::physics::EdgeStyle;
use crate::model::ItemRef;

/// Top bit of a hub id, so a milestone hub never shares an id with an
/// item number.
const HUB_BIT: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeId {
    Item(ItemRef),
    Hub(u64),
}

/// One stored item, as the map needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapItem {
    pub number: ItemRef,
    pub is_pr: bool,
    /// GraphQL state: `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub title: String,
    pub milestone: Option<String>,
    pub labels: Vec<String>,
}

/// What a stored link means. Declaration order is the order an edge
/// lists its kinds in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LinkKind {
    /// `src` blocks `dst`.
    Blocks,
    /// `src` is the parent of `dst`.
    Parent,
    /// The pull request `src` closes the issue `dst`.
    Closes,
}

/// One stored link between two item numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapLink {
    pub src: ItemRef,
    pub dst: ItemRef,
    pub kind: LinkKind,
}

/// Which items the map shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapScope {
    /// The linked component of the open issue and its milestone's
    /// members, with their milestone hubs.
    #[default]
    Component,
    /// Every item of the repo, pulled together by milestone hubs.
    Repo,
}

/// What the map draws. The default draws the open item's component with
/// every PR, hub and link kind, closed items only as blockers of open
/// items, and no chips. The open item always shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapFilter {
    pub scope: MapScope,
    /// Every closed item in scope shows, not only the closed blockers of
    /// open items.
    pub closed: bool,
    pub prs: bool,
    /// Milestone hub nodes and the edges that join items to them.
    pub hubs: bool,
    pub blocks: bool,
    pub parents: bool,
    pub closes: bool,
    /// Only items in one of these milestones show. Empty means any.
    pub milestones: BTreeSet<String>,
    /// Only items with one of these labels show. Empty means any.
    pub labels: BTreeSet<String>,
}

impl Default for MapFilter {
    fn default() -> Self {
        MapFilter {
            scope: MapScope::default(),
            closed: false,
            prs: true,
            hubs: true,
            blocks: true,
            parents: true,
            closes: true,
            milestones: BTreeSet::new(),
            labels: BTreeSet::new(),
        }
    }
}

impl MapFilter {
    /// The switch for links of `kind`.
    pub fn link_mut(&mut self, kind: LinkKind) -> &mut bool {
        match kind {
            LinkKind::Blocks => &mut self.blocks,
            LinkKind::Parent => &mut self.parents,
            LinkKind::Closes => &mut self.closes,
        }
    }

    /// Links of `kind` draw.
    pub fn draws(&self, kind: LinkKind) -> bool {
        match kind {
            LinkKind::Blocks => self.blocks,
            LinkKind::Parent => self.parents,
            LinkKind::Closes => self.closes,
        }
    }

    /// `item` passes the PR switch and both chip sets.
    pub fn keeps(&self, item: &MapItem) -> bool {
        let milestone = self.milestones.is_empty()
            || item.milestone.as_ref().is_some_and(|title| self.milestones.contains(title));
        let label = self.labels.is_empty()
            || item.labels.iter().any(|name| self.labels.contains(name));
        (self.prs || !item.is_pr) && milestone && label
    }

    /// The filter differs from the default in more than its scope.
    pub fn changed(&self) -> bool {
        *self != MapFilter { scope: self.scope, ..MapFilter::default() }
    }
}

/// How a node reads on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// An open issue with an open blocker.
    Blocked,
    /// An open issue with no open blocker.
    Ready,
    /// An open pull request.
    Pr,
    /// Closed or merged: shown only as a blocker of an open item, or as
    /// the open issue itself.
    Closed,
    /// A milestone.
    Hub,
}

impl NodeState {
    /// The state in a word or two, for cards.
    pub fn word(self) -> &'static str {
        match self {
            NodeState::Blocked => "blocked",
            NodeState::Ready => "ready",
            NodeState::Pr => "open PR",
            NodeState::Closed => "closed",
            NodeState::Hub => "milestone",
        }
    }
}

/// One node: an item, or a milestone hub (`number` is `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapNode {
    /// Stable layout id: the item number, or a hub id.
    pub id: NodeId,
    pub number: Option<ItemRef>,
    pub title: String,
    pub state: NodeState,
}

/// The built map: nodes, then edges as node index pairs. One edge per
/// ordered pair: a `Directed` edge carries a blocks link and runs from
/// the blocker to the blocked node; a `Hub` edge runs from an item to its
/// milestone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapGraph {
    pub nodes: Vec<MapNode>,
    pub edges: Vec<(usize, usize, EdgeStyle)>,
    /// Every link kind each edge carries, in `LinkKind` order. Empty for
    /// a hub edge.
    pub kinds: Vec<Vec<LinkKind>>,
    /// Layout id to node index, built with the graph.
    index: HashMap<NodeId, usize>,
}

impl MapGraph {
    pub fn new(nodes: Vec<MapNode>, edges: Vec<(usize, usize, EdgeStyle)>, kinds: Vec<Vec<LinkKind>>) -> Self {
        let index = nodes.iter().enumerate().map(|(i, node)| (node.id, i)).collect();
        MapGraph { nodes, edges, kinds, index }
    }

    /// Layout ids in node order.
    pub fn ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|node| node.id).collect()
    }

    /// Index of the node with layout id `id`.
    pub fn index_of(&self, id: NodeId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// Indexes of the edges that touch node `index`, in edge order.
    pub fn edges_of(&self, index: usize) -> Vec<usize> {
        let touches = |&e: &usize| self.edges[e].0 == index || self.edges[e].1 == index;
        (0..self.edges.len()).filter(touches).collect()
    }

    /// Index of the edge from layout id `src` to layout id `dst`.
    pub fn edge_between(&self, src: NodeId, dst: NodeId) -> Option<usize> {
        let (a, b) = (self.index_of(src)?, self.index_of(dst)?);
        self.edges.iter().position(|&(from, to, _)| from == a && to == b)
    }

    /// Edge `edge` in words: "#16 blocks #17", "#40 closes #25", "#3 is
    /// the parent of #8", "#3 blocks and is the parent of #8", "#16 is in
    /// milestone Fast startup".
    pub fn relation(&self, edge: usize) -> String {
        let (a, b, style) = self.edges[edge];
        let (from, to) = (&self.nodes[a], &self.nodes[b]);
        let name = |node: &MapNode| node.number.map_or(node.title.clone(), |n| n.display(""));
        if style == EdgeStyle::Hub {
            return format!("{} is in milestone {}", name(from), to.title);
        }
        let verbs: Vec<&str> = self.kinds.get(edge).into_iter().flatten().map(|kind| match kind {
            LinkKind::Blocks => "blocks",
            LinkKind::Parent => "is the parent of",
            LinkKind::Closes => "closes",
        }).collect();
        let verb = if verbs.is_empty() { "links to".to_string() } else { verbs.join(" and ") };
        format!("{} {verb} {}", name(from), name(to))
    }

    /// Every node that blocks `index`, directly or through a chain.
    pub fn blockers_of(&self, index: usize) -> HashSet<usize> {
        let mut found = HashSet::new();
        let mut queue = vec![index];
        while let Some(node) = queue.pop() {
            for &(src, dst, style) in &self.edges {
                if style == EdgeStyle::Directed && dst == node && src != index && found.insert(src) {
                    queue.push(src);
                }
            }
        }
        found
    }
}

/// Layout id of the hub for milestone `title`: FNV-1a with the hub bit.
pub fn hub_id(title: &str) -> NodeId {
    let hash = title.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    NodeId::Hub(HUB_BIT | (hash >> 1))
}

/// Build the map. `center` is the open item: the component scope walks
/// links out from it, and it always shows. With no center the component
/// scope shows the whole repo. Links to numbers missing from `items`
/// drop. `filter` picks the items, link kinds and hubs that draw; the
/// component walks only drawn links. Closed items hide, except closed
/// blockers of an open item in scope, unless `filter.closed` shows them.
/// A node's state counts every stored blocks link, drawn or not.
pub fn build_map(
    items: &[MapItem],
    links: &[MapLink],
    center: Option<ItemRef>,
    filter: &MapFilter,
) -> MapGraph {
    let by_number: HashMap<ItemRef, &MapItem> =
        items.iter().map(|item| (item.number, item)).collect();
    let stored: Vec<MapLink> = links
        .iter()
        .copied()
        .filter(|link| {
            link.src != link.dst
                && by_number.contains_key(&link.src)
                && by_number.contains_key(&link.dst)
        })
        .collect();
    let kept = |n: ItemRef| {
        Some(n) == center || by_number.get(&n).is_some_and(|item| filter.keeps(item))
    };
    let links: Vec<MapLink> = stored.iter().copied()
        .filter(|link| filter.draws(link.kind) && kept(link.src) && kept(link.dst))
        .collect();
    let in_scope: HashSet<ItemRef> = match (filter.scope, center) {
        (MapScope::Repo, _) | (MapScope::Component, None) => {
            by_number.keys().copied().filter(|&n| kept(n)).collect()
        }
        (MapScope::Component, Some(center)) if by_number.contains_key(&center) => {
            // With hubs on, the open item's milestone pulls in its members
            // too, one hop: other milestones' hubs only decorate.
            let milestone = by_number[&center].milestone.as_deref().filter(|_| filter.hubs);
            let seeds = by_number.values()
                .filter(|item| item.number == center
                    || (milestone.is_some() && item.milestone.as_deref() == milestone && kept(item.number)))
                .map(|item| item.number);
            component(&links, seeds)
        }
        (MapScope::Component, Some(_)) => HashSet::new(),
    };
    // Blocks links indexed both ways, so each node checks only its own.
    let mut blocks: HashMap<ItemRef, Vec<ItemRef>> = HashMap::new();
    for link in links.iter().filter(|link| link.kind == LinkKind::Blocks) {
        blocks.entry(link.src).or_default().push(link.dst);
    }
    let mut blocked_by: HashMap<ItemRef, Vec<ItemRef>> = HashMap::new();
    for link in stored.iter().filter(|link| link.kind == LinkKind::Blocks) {
        blocked_by.entry(link.dst).or_default().push(link.src);
    }
    let open = |n: ItemRef| by_number.get(&n).is_some_and(|item| item.state == "OPEN");
    let visible = |n: ItemRef| {
        open(n)
            || filter.closed
            || Some(n) == center
            || blocks.get(&n).is_some_and(|dsts| dsts.iter().any(|&dst| in_scope.contains(&dst) && open(dst)))
    };
    let mut numbers: Vec<ItemRef> = in_scope.iter().copied().filter(|&n| visible(n)).collect();
    numbers.sort_unstable();

    let mut nodes: Vec<MapNode> = numbers.iter().map(|&n| {
        let item = by_number[&n];
        let blocked = blocked_by.get(&n).is_some_and(|srcs| srcs.iter().any(|&src| open(src)));
        let state = match (open(n), item.is_pr, blocked) {
            (false, _, _) => NodeState::Closed,
            (true, true, _) => NodeState::Pr,
            (true, false, true) => NodeState::Blocked,
            (true, false, false) => NodeState::Ready,
        };
        MapNode { id: NodeId::Item(n), number: Some(n), title: item.title.clone(), state }
    }).collect();
    let index: HashMap<ItemRef, usize> = numbers.iter().enumerate().map(|(i, &n)| (n, i)).collect();

    // One edge per ordered pair, carrying every kind linked that way.
    let mut pairs: BTreeMap<(usize, usize), Vec<LinkKind>> = BTreeMap::new();
    for link in &links {
        if let (Some(&src), Some(&dst)) = (index.get(&link.src), index.get(&link.dst)) {
            pairs.entry((src, dst)).or_default().push(link.kind);
        }
    }
    let mut edges = Vec::with_capacity(pairs.len());
    let mut kinds = Vec::with_capacity(pairs.len());
    for ((src, dst), mut found) in pairs {
        found.sort_unstable();
        found.dedup();
        let blocks = found.contains(&LinkKind::Blocks);
        edges.push((src, dst, if blocks { EdgeStyle::Directed } else { EdgeStyle::Link }));
        kinds.push(found);
    }

    if filter.hubs {
        let mut milestones: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, n) in numbers.iter().enumerate() {
            if let Some(title) = by_number[n].milestone.as_deref() {
                milestones.entry(title).or_default().push(i);
            }
        }
        for (title, members) in milestones {
            let hub = nodes.len();
            nodes.push(MapNode { id: hub_id(title), number: None, title: title.to_string(), state: NodeState::Hub });
            for i in members {
                edges.push((i, hub, EdgeStyle::Hub));
                kinds.push(vec![]);
            }
        }
    }
    MapGraph::new(nodes, edges, kinds)
}

/// Numbers linked to any of `seeds` through any chain of links, either way.
fn component(links: &[MapLink], seeds: impl Iterator<Item = ItemRef>) -> HashSet<ItemRef> {
    let mut neighbors: HashMap<ItemRef, Vec<ItemRef>> = HashMap::new();
    for link in links {
        neighbors.entry(link.src).or_default().push(link.dst);
        neighbors.entry(link.dst).or_default().push(link.src);
    }
    let mut seen: HashSet<ItemRef> = seeds.collect();
    let mut queue: Vec<ItemRef> = seen.iter().copied().collect();
    while let Some(node) = queue.pop() {
        for &next in neighbors.get(&node).into_iter().flatten() {
            if seen.insert(next) {
                queue.push(next);
            }
        }
    }
    seen
}
