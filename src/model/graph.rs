//! Dependency map: issues and PRs as nodes, milestones as hubs, stored
//! links as edges. Pure: built from the local copy's items and links,
//! then laid out by `physics`.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::physics::EdgeStyle;

/// Top bit of a hub id, so a milestone hub never shares an id with an
/// item number.
const HUB_BIT: u64 = 1 << 63;

/// One stored item, as the map needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapItem {
    pub number: u64,
    pub is_pr: bool,
    /// GraphQL state: `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub title: String,
    pub milestone: Option<String>,
}

/// What a stored link means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub src: u64,
    pub dst: u64,
    pub kind: LinkKind,
}

/// Which items the map shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapScope {
    /// The linked component of the open issue, with its milestone hubs.
    #[default]
    Component,
    /// Every item of the repo, pulled together by milestone hubs.
    Repo,
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

/// One node: an item, or a milestone hub (`number` is `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapNode {
    /// Stable layout id: the item number, or a hub id.
    pub id: u64,
    pub number: Option<u64>,
    pub title: String,
    pub state: NodeState,
}

/// The built map: nodes, then edges as node index pairs. A `Directed`
/// edge runs from the blocker to the blocked node; a `Hub` edge from an
/// item to its milestone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapGraph {
    pub nodes: Vec<MapNode>,
    pub edges: Vec<(usize, usize, EdgeStyle)>,
}

impl MapGraph {
    /// Layout ids in node order.
    pub fn ids(&self) -> Vec<u64> {
        self.nodes.iter().map(|node| node.id).collect()
    }

    /// Index of the node with layout id `id`.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
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
pub fn hub_id(title: &str) -> u64 {
    let hash = title.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    HUB_BIT | (hash >> 1)
}

/// Build the map. `center` is the open issue: the component scope walks
/// links out from it, and it always shows. Links to numbers missing from
/// `items` drop. Closed items hide, except closed blockers of an open item
/// in scope.
pub fn build_map(items: &[MapItem], links: &[MapLink], center: Option<u64>, scope: MapScope) -> MapGraph {
    let by_number: HashMap<u64, &MapItem> = items.iter().map(|item| (item.number, item)).collect();
    let links: Vec<MapLink> = links.iter().copied()
        .filter(|link| link.src != link.dst && by_number.contains_key(&link.src) && by_number.contains_key(&link.dst))
        .collect();
    let in_scope: HashSet<u64> = match scope {
        MapScope::Repo => by_number.keys().copied().collect(),
        MapScope::Component => match center.filter(|n| by_number.contains_key(n)) {
            Some(center) => component(&links, center),
            None => HashSet::new(),
        },
    };
    // Blocks links indexed both ways, so each node checks only its own.
    let mut blocks: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut blocked_by: HashMap<u64, Vec<u64>> = HashMap::new();
    for link in links.iter().filter(|link| link.kind == LinkKind::Blocks) {
        blocks.entry(link.src).or_default().push(link.dst);
        blocked_by.entry(link.dst).or_default().push(link.src);
    }
    let open = |n: u64| by_number.get(&n).is_some_and(|item| item.state == "OPEN");
    let visible = |n: u64| {
        open(n)
            || Some(n) == center
            || blocks.get(&n).is_some_and(|dsts| dsts.iter().any(|&dst| in_scope.contains(&dst) && open(dst)))
    };
    let mut numbers: Vec<u64> = in_scope.iter().copied().filter(|&n| visible(n)).collect();
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
        MapNode { id: n, number: Some(n), title: item.title.clone(), state }
    }).collect();
    let index: HashMap<u64, usize> = numbers.iter().enumerate().map(|(i, &n)| (n, i)).collect();

    let mut edges: Vec<(usize, usize, EdgeStyle)> = links.iter().filter_map(|link| {
        let (src, dst) = (*index.get(&link.src)?, *index.get(&link.dst)?);
        let style = match link.kind {
            LinkKind::Blocks => EdgeStyle::Directed,
            LinkKind::Parent | LinkKind::Closes => EdgeStyle::Link,
        };
        Some((src, dst, style))
    }).collect();
    edges.sort_by_key(|&(src, dst, _)| (src, dst));
    edges.dedup_by_key(|edge| (edge.0, edge.1));

    let mut milestones: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, n) in numbers.iter().enumerate() {
        if let Some(title) = by_number[n].milestone.as_deref() {
            milestones.entry(title).or_default().push(i);
        }
    }
    for (title, members) in milestones {
        let hub = nodes.len();
        nodes.push(MapNode { id: hub_id(title), number: None, title: title.to_string(), state: NodeState::Hub });
        edges.extend(members.into_iter().map(|i| (i, hub, EdgeStyle::Hub)));
    }
    MapGraph { nodes, edges }
}

/// Numbers linked to `center` through any chain of links, either way.
fn component(links: &[MapLink], center: u64) -> HashSet<u64> {
    let mut neighbors: HashMap<u64, Vec<u64>> = HashMap::new();
    for link in links {
        neighbors.entry(link.src).or_default().push(link.dst);
        neighbors.entry(link.dst).or_default().push(link.src);
    }
    let mut seen = HashSet::from([center]);
    let mut queue = vec![center];
    while let Some(node) = queue.pop() {
        for &next in neighbors.get(&node).into_iter().flatten() {
            if seen.insert(next) {
                queue.push(next);
            }
        }
    }
    seen
}
