//! Tests for model::graph.

use super::graph::*;
use super::physics::EdgeStyle;

fn item(number: u64, state: &str, milestone: Option<&str>) -> MapItem {
    MapItem { number, is_pr: false, state: state.to_string(), title: format!("Item {number}"), milestone: milestone.map(str::to_string) }
}

fn link(src: u64, dst: u64, kind: LinkKind) -> MapLink {
    MapLink { src, dst, kind }
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
    let graph = build_map(&items, &links, Some(2), MapScope::Component);
    assert_eq!(numbers(&graph), vec![1, 2, 3, 20], "#5 is closed and blocks nothing; #4 and #9 are elsewhere");
    let hubs: Vec<&str> = graph.nodes.iter().filter(|n| n.state == NodeState::Hub).map(|n| n.title.as_str()).collect();
    assert_eq!(hubs, vec!["Ship it"], "only the component's milestones");
}

#[test]
fn repo_scope_shows_every_open_item_and_every_hub() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), MapScope::Repo);
    assert_eq!(numbers(&graph), vec![1, 2, 3, 4, 9, 20]);
    let hubs = graph.nodes.iter().filter(|n| n.state == NodeState::Hub).count();
    assert_eq!(hubs, 2);
}

#[test]
fn closed_items_hide_unless_they_block_an_open_item() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), MapScope::Component);
    let state = |n: u64| graph.nodes.iter().find(|node| node.number == Some(n)).map(|node| node.state);
    assert_eq!(state(3), Some(NodeState::Closed), "closed blocker of an open issue shows dimmed");
    assert_eq!(state(5), None);
    assert_eq!(state(1), Some(NodeState::Ready), "a closed blocker does not block");
    assert_eq!(state(2), Some(NodeState::Blocked));
    assert_eq!(state(20), Some(NodeState::Pr));
    let closed_center = build_map(&items, &links, Some(5), MapScope::Component);
    assert!(numbers(&closed_center).contains(&5), "the open issue always shows");
}

#[test]
fn edges_carry_styles_and_hub_edges_join_milestones() {
    let (items, links) = fixture();
    let graph = build_map(&items, &links, Some(2), MapScope::Component);
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
    let graph = build_map(&items, &links, Some(4), MapScope::Component);
    assert_eq!(numbers(&graph), vec![4], "#99 is not stored, so #4 stands alone");
    assert!(graph.edges.is_empty());
    assert!(build_map(&items, &links, Some(77), MapScope::Component).nodes.is_empty(), "unknown center");
}

#[test]
fn blocker_chain_is_transitive() {
    let items: Vec<MapItem> = (1..=4).map(|n| item(n, "OPEN", None)).collect();
    let links = vec![link(1, 2, LinkKind::Blocks), link(2, 3, LinkKind::Blocks), link(4, 3, LinkKind::Parent)];
    let graph = build_map(&items, &links, Some(3), MapScope::Component);
    let mut chain: Vec<u64> = graph.blockers_of(graph.index_of(3).unwrap()).into_iter().map(|i| graph.nodes[i].id).collect();
    chain.sort_unstable();
    assert_eq!(chain, vec![1, 2], "parent links are not blockers");
}

#[test]
fn hub_ids_never_collide_with_numbers() {
    assert!(hub_id("v1") > u64::from(u32::MAX));
    assert_ne!(hub_id("v1"), hub_id("v2"));
}
