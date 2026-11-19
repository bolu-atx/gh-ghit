//! Tests for model::graph.

use super::graph::*;
use super::physics::EdgeStyle;

fn item(number: u64, state: &str, milestone: Option<&str>) -> MapItem {
    MapItem { number, is_pr: false, state: state.to_string(), title: format!("Item {number}"), milestone: milestone.map(str::to_string), labels: vec![] }
}

fn link(src: u64, dst: u64, kind: LinkKind) -> MapLink {
    MapLink { src, dst, kind }
}

/// The default filter with `scope`.
fn scoped(scope: MapScope) -> MapFilter {
    MapFilter { scope, ..MapFilter::default() }
}

fn numbers(graph: &MapGraph) -> Vec<u64> {
    graph.nodes.iter().filter_map(|node| node.number).collect()
}

fn fixture() -> (Vec<MapItem>, Vec<MapLink>) {
    let mut pr = item(20, "OPEN", None);
    pr.is_pr = true;
    let items = vec![
        item(1, "OPEN", Some("Ship it")),
        item(2, "OPEN", Some("Ship it")),
        item(3, "CLOSED", None),
        item(4, "OPEN", None),
        item(5, "CLOSED", None),
        item(9, "OPEN", Some("Later")),
        pr,
    ];
    let links = vec![
        link(1, 2, LinkKind::Blocks),
        link(3, 1, LinkKind::Blocks),
        link(5, 3, LinkKind::Parent),
        link(20, 2, LinkKind::Closes),
        link(4, 99, LinkKind::Blocks),
        link(99, 1, LinkKind::Blocks),
    ];
    (items, links)
}

#[test]
fn component_scope_walks_links_from_the_open_issue() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), &scoped(MapScope::Component));
    assert_eq!(numbers(&graph), vec![1, 2, 3, 20], "#5 is closed and blocks nothing; #4 and #9 are elsewhere");
    let hubs: Vec<&str> = graph.nodes.iter().filter(|n| n.state == NodeState::Hub).map(|n| n.title.as_str()).collect();
    assert_eq!(hubs, vec!["Ship it"], "only the component's milestones");
}

#[test]
fn repo_scope_shows_every_open_item_and_every_hub() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), &scoped(MapScope::Repo));
    assert_eq!(numbers(&graph), vec![1, 2, 3, 4, 9, 20]);
    let hubs = graph.nodes.iter().filter(|n| n.state == NodeState::Hub).count();
    assert_eq!(hubs, 2);
}

#[test]
fn component_scope_with_no_center_shows_the_repo() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, None, &scoped(MapScope::Component));
    assert_eq!(numbers(&graph), numbers(&build_map(&items, &links, None, &scoped(MapScope::Repo))));
    assert_eq!(numbers(&graph), vec![1, 2, 3, 4, 9, 20]);
}

#[test]
fn closed_items_hide_unless_they_block_an_open_item() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), &scoped(MapScope::Component));
    let state = |n: u64| graph.nodes.iter().find(|node| node.number == Some(n)).map(|node| node.state);
    assert_eq!(state(3), Some(NodeState::Closed), "closed blocker of an open issue shows dimmed");
    assert_eq!(state(5), None);
    assert_eq!(state(1), Some(NodeState::Ready), "a closed blocker does not block");
    assert_eq!(state(2), Some(NodeState::Blocked));
    assert_eq!(state(20), Some(NodeState::Pr));
    let closed_center = build_map(&items, &links, Some(5), &scoped(MapScope::Component));
    assert!(numbers(&closed_center).contains(&5), "the open issue always shows");
}

#[test]
fn edges_carry_styles_and_hub_edges_join_milestones() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), &scoped(MapScope::Component));
    let at = |n: u64| graph.index_of(n).unwrap();
    let hub = graph.index_of(hub_id("Ship it")).unwrap();
    assert!(graph.edges.contains(&(at(1), at(2), EdgeStyle::Directed)));
    assert!(graph.edges.contains(&(at(20), at(2), EdgeStyle::Link)));
    assert!(graph.edges.contains(&(at(1), hub, EdgeStyle::Hub)));
    assert!(graph.edges.contains(&(at(2), hub, EdgeStyle::Hub)));
    assert_eq!(graph.edges.iter().filter(|e| e.2 == EdgeStyle::Hub).count(), 2);
}

#[test]
fn dangling_links_drop() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(4), &scoped(MapScope::Component));
    assert_eq!(numbers(&graph), vec![4], "#99 is not stored, so #4 stands alone");
    assert!(graph.edges.is_empty());
    assert!(build_map(&items, &links, Some(77), &scoped(MapScope::Component)).nodes.is_empty(), "unknown center");
}

#[test]
fn blocker_chain_is_transitive() {
    let items: Vec<MapItem> = (1..=4).map(|n| item(n, "OPEN", None)).collect();
    let links = vec![link(1, 2, LinkKind::Blocks), link(2, 3, LinkKind::Blocks), link(4, 3, LinkKind::Parent)];
    let graph = build_map(&items, &links, Some(3), &scoped(MapScope::Component));
    let mut chain: Vec<u64> = graph.blockers_of(graph.index_of(3).unwrap()).into_iter().map(|i| graph.nodes[i].id).collect();
    chain.sort_unstable();
    assert_eq!(chain, vec![1, 2], "parent links are not blockers");
}

#[test]
fn hub_ids_never_collide_with_numbers() {
    assert!(hub_id("v1") > u64::from(u32::MAX));
    assert_ne!(hub_id("v1"), hub_id("v2"));
}

#[test]
fn relations_read_as_words() {
    let (items, mut links) = fixture();
    links.push(link(1, 4, LinkKind::Parent));
    let graph = build_map(&items, &links, Some(2), &scoped(MapScope::Repo));
    let edge = |a: u64, b: u64| graph.edge_between(a, b).unwrap();
    assert_eq!(graph.relation(edge(1, 2)), "#1 blocks #2");
    assert_eq!(graph.relation(edge(20, 2)), "#20 closes #2");
    assert_eq!(graph.relation(edge(1, 4)), "#1 is the parent of #4");
    assert_eq!(graph.relation(edge(9, hub_id("Later"))), "#9 is in milestone Later");
    assert_eq!(graph.edge_between(2, 1), None, "edges have a direction");
    let one = graph.index_of(1).unwrap();
    let touching: Vec<String> = graph.edges_of(one).into_iter().map(|e| graph.relation(e)).collect();
    assert_eq!(touching, vec!["#1 blocks #2", "#1 is the parent of #4", "#3 blocks #1", "#1 is in milestone Ship it"]);
}

#[test]
fn an_edge_keeps_every_kind_it_carries() {
    let items: Vec<MapItem> = (1..=2).map(|n| item(n, "OPEN", None)).collect();
    // The parent link comes first, so a first-wins dedup would lose the blocker.
    let links = vec![link(1, 2, LinkKind::Parent), link(1, 2, LinkKind::Blocks), link(1, 2, LinkKind::Parent)];
    let mut filter = scoped(MapScope::Component);
    let graph = build_map(&items, &links, Some(2), &filter);
    let edge = graph.edge_between(1, 2).unwrap();
    assert_eq!(graph.edges[edge].2, EdgeStyle::Directed, "a blocks link draws as one");
    assert_eq!(graph.kinds[edge], vec![LinkKind::Blocks, LinkKind::Parent]);
    assert_eq!(graph.relation(edge), "#1 blocks and is the parent of #2");
    assert_eq!(graph.edges.len(), 1, "one edge per ordered pair");

    filter.blocks = false;
    let graph = build_map(&items, &links, Some(2), &filter);
    let edge = graph.edge_between(1, 2).unwrap();
    assert_eq!((graph.edges[edge].2, graph.kinds[edge].clone()), (EdgeStyle::Link, vec![LinkKind::Parent]));
    assert_eq!(graph.relation(edge), "#1 is the parent of #2");
    let two = graph.index_of(2).unwrap();
    assert_eq!(graph.nodes[two].state, NodeState::Blocked, "state counts the hidden blocker");

    filter.parents = false;
    let graph = build_map(&items, &links, Some(2), &filter);
    assert_eq!(numbers(&graph), vec![2], "with neither kind drawn, #1 leaves the component");
}

#[test]
fn switches_hide_prs_hubs_and_link_kinds() {
    let (items, links) = fixture();
    let base = scoped(MapScope::Component);
    let build = |filter: &MapFilter| build_map(&items, &links, Some(2), filter);
    assert_eq!(numbers(&build(&base)), vec![1, 2, 3, 20]);
    assert_eq!(numbers(&build(&MapFilter { prs: false, ..base.clone() })), vec![1, 2, 3]);
    assert_eq!(numbers(&build(&MapFilter { closes: false, ..base.clone() })), vec![1, 2, 3], "#20 links only by closes");
    let hubless = build(&MapFilter { hubs: false, ..base.clone() });
    assert!(hubless.nodes.iter().all(|node| node.state != NodeState::Hub));
    assert!(hubless.edges.iter().all(|edge| edge.2 != EdgeStyle::Hub));
    let full = build(&base);
    assert_eq!(full.kinds.len(), full.edges.len(), "every edge has its kinds");
    let hub = full.index_of(hub_id("Ship it")).unwrap();
    let hub_edge = full.edges.iter().position(|edge| edge.1 == hub).unwrap();
    assert!(full.kinds[hub_edge].is_empty(), "a hub edge carries no link kind");
}

#[test]
fn the_closed_switch_shows_every_closed_item_in_scope() {
    let (items, links) = fixture();
    let filter = MapFilter { closed: true, ..scoped(MapScope::Component) };
    let graph = build_map(&items, &links, Some(2), &filter);
    assert_eq!(numbers(&graph), vec![1, 2, 3, 5, 20], "#5 is closed and blocks nothing, yet shows");
    let repo = MapFilter { closed: true, ..scoped(MapScope::Repo) };
    assert_eq!(numbers(&build_map(&items, &links, None, &repo)), vec![1, 2, 3, 4, 5, 9, 20]);
}

#[test]
fn chips_keep_matching_items_and_the_open_item() {
    let (mut items, links) = fixture();
    items[3].labels = vec!["bug".to_string()];
    let mut filter = scoped(MapScope::Repo);
    filter.milestones.insert("Ship it".to_string());
    let graph = build_map(&items, &links, None, &filter);
    assert_eq!(numbers(&graph), vec![1, 2], "only Ship it items");
    let hubs: Vec<&str> = graph.nodes.iter().filter(|n| n.state == NodeState::Hub).map(|n| n.title.as_str()).collect();
    assert_eq!(hubs, vec!["Ship it"]);
    assert_eq!(graph.edges.iter().filter(|e| e.2 != EdgeStyle::Hub).count(), 1, "only the edge between kept items");

    let labeled = MapFilter { labels: ["bug".to_string()].into(), ..scoped(MapScope::Repo) };
    assert_eq!(numbers(&build_map(&items, &links, None, &labeled)), vec![4]);
    assert_eq!(numbers(&build_map(&items, &links, Some(9), &labeled)), vec![4, 9], "the open item always shows");
    let both = MapFilter { labels: ["bug".to_string()].into(), ..filter };
    assert!(numbers(&build_map(&items, &links, None, &both)).is_empty(), "both chip sets must match");
}

#[test]
fn a_filter_reads_old_or_partial_prefs_and_names_its_changes() {
    assert_eq!(serde_json::from_str::<MapFilter>("{}").unwrap(), MapFilter::default());
    let partial: MapFilter = serde_json::from_str(r#"{"closed":true,"scope":"Repo"}"#).unwrap();
    assert_eq!(partial, MapFilter { closed: true, ..scoped(MapScope::Repo) });
    assert!(!MapFilter::default().changed());
    assert!(!scoped(MapScope::Repo).changed(), "the scope alone is no filter");
    assert!(partial.changed());
}
