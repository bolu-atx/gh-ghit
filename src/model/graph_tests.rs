//! Tests for model::graph.

use super::*;
use super::graph::{GRAPH_SECTION_ORDER, GraphEdge, edge_for};

fn node(number: u64) -> GraphNode {
    GraphNode {
        number,
        title: format!("issue {number}"),
        state: "OPEN".to_string(),
        is_pr: false,
        labels: vec![],
        project_status: None,
        linked_prs: vec![],
    }
}

#[test]
fn edge_kinds_point_the_documented_way() {
    assert_eq!(edge_for(7, 9, GraphEdgeKind::Blocks), GraphEdge { from: 7, to: 9, kind: GraphEdgeKind::Blocks });
    assert_eq!(edge_for(7, 9, GraphEdgeKind::BlockedBy), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::BlockedBy });
    assert_eq!(edge_for(7, 9, GraphEdgeKind::Parent), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::Parent });
    assert_eq!(edge_for(7, 9, GraphEdgeKind::Child), GraphEdge { from: 7, to: 9, kind: GraphEdgeKind::Child });
    assert_eq!(edge_for(7, 9, GraphEdgeKind::Closes), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::Closes });
}

#[test]
fn build_dedups_and_caps() {
    let neighbors = vec![
        (node(9), GraphEdgeKind::Blocks),
        (node(9), GraphEdgeKind::References),
        (node(7), GraphEdgeKind::Child),
    ];
    let data = build_graph(node(7), neighbors);
    assert_eq!(data.nodes.iter().map(|n| n.number).collect::<Vec<_>>(), vec![7, 9]);
    assert_eq!(data.edges, vec![edge_for(7, 9, GraphEdgeKind::Blocks)]);
    assert!(!data.truncated);

    let many: Vec<(GraphNode, GraphEdgeKind)> =
        (1..=40).map(|n| (node(n), GraphEdgeKind::References)).collect();
    let capped = build_graph(node(100), many);
    assert_eq!(capped.nodes.len(), MAX_GRAPH_NODES);
    assert!(capped.truncated);
}

#[test]
fn blocker_end_of_either_kind_counts_as_blocking() {
    let data = build_graph(node(7), vec![(node(9), GraphEdgeKind::Blocks), (node(10), GraphEdgeKind::BlockedBy)]);
    assert!(graph_blocks_something(&data, 7));
    assert!(!graph_blocks_something(&data, 9));
    assert!(graph_blocks_something(&data, 10));
    assert!(!graph_blocks_something(&data, 999));
}

#[test]
fn sections_follow_blast_radius_order_and_order_leads_with_center() {
    let data = build_graph(
        node(7),
        vec![
            (node(11), GraphEdgeKind::ReferencedBy),
            (node(9), GraphEdgeKind::Blocks),
            (node(10), GraphEdgeKind::Parent),
        ],
    );
    let labels: Vec<GraphEdgeKind> =
        graph_sections(&data).into_iter().map(|(k, _)| k).collect();
    assert_eq!(labels, vec![GraphEdgeKind::Blocks, GraphEdgeKind::Parent, GraphEdgeKind::ReferencedBy]);
    let order: Vec<u64> = graph_order(&data).into_iter().map(|i| data.nodes[i].number).collect();
    assert_eq!(order, vec![7, 9, 10, 11]);
}

#[test]
fn edge_labels_cover_every_kind() {
    for kind in GRAPH_SECTION_ORDER {
        assert!(!graph_edge_label(kind).is_empty());
    }
}
