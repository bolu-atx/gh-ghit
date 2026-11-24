//! Tests for model::rank. Graphs are built by hand, so the ranks do not
//! depend on how `build_map` scopes the items.

use super::graph::{LinkKind, MapGraph, MapNode, NodeState};
use super::physics::EdgeStyle;
use super::rank::{Ranking, rank};
use NodeState::{Blocked as B, Closed as C, Hub as H, Pr as P, Ready as R};
use crate::model::ItemRef;
use crate::model::NodeId;

/// Nodes in order, blocks links and hub links as node index pairs.
fn graph(states: &[NodeState], blocks: &[(usize, usize)], hubs: &[(usize, usize)]) -> MapGraph {
    let nodes = states
        .iter()
        .enumerate()
        .map(|(i, &state)| {
            let number = (state != NodeState::Hub).then_some(ItemRef::github(i as u64 + 1));
            MapNode {
                id: NodeId::Item(ItemRef::github(i as u64 + 1)),
                number,
                title: format!("Node {i}"),
                state,
            }
        })
        .collect();
    let edges: Vec<_> = blocks
        .iter()
        .map(|&(a, b)| (a, b, EdgeStyle::Directed))
        .chain(hubs.iter().map(|&(a, b)| (a, b, EdgeStyle::Hub)))
        .collect();
    let kinds = edges.iter().map(|e| if e.2 == EdgeStyle::Directed { vec![LinkKind::Blocks] } else { vec![] }).collect();
    MapGraph::new(nodes, edges, kinds)
}

#[test]
fn a_chain_ranks_its_head_ready_and_counts_what_it_unblocks() {
    // 0 blocks 1 blocks 2; 0 also blocks 3. 4 stands alone.
    let map = graph(&[R, B, B, B, R], &[(0, 1), (1, 2), (0, 3)], &[]);
    let ranking = rank(&map);
    assert_eq!(ranking.ready, vec![0], "a lone open issue is not in a blocking relation");
    assert_eq!(ranking.bottlenecks, vec![(0, 3), (1, 1)], "0 unblocks 1, 2 and 3 through the chain");
}

#[test]
fn closed_blockers_leave_their_issue_ready_and_never_rank() {
    // Closed 0 blocks 1; 1 blocks 2.
    let map = graph(&[C, R, B], &[(0, 1), (1, 2)], &[]);
    let ranking = rank(&map);
    assert_eq!(ranking.ready, vec![1], "its only blocker is closed");
    assert_eq!(ranking.bottlenecks, vec![(1, 1)], "a closed blocker unblocks nothing");
}

#[test]
fn a_ready_issue_with_only_closed_blockers_still_lists() {
    let map = graph(&[C, R], &[(0, 1)], &[]);
    let ranking = rank(&map);
    assert_eq!(ranking.ready, vec![1]);
    assert!(ranking.bottlenecks.is_empty());
}

#[test]
fn ready_work_that_unblocks_more_comes_first() {
    // 0 blocks 1; 2 blocks 3 and 4.
    let map = graph(&[R, B, R, B, B], &[(0, 1), (2, 3), (2, 4)], &[]);
    let ranking = rank(&map);
    assert_eq!(ranking.ready, vec![2, 0]);
    assert_eq!(ranking.bottlenecks, vec![(2, 2), (0, 1)]);
}

#[test]
fn prs_and_hubs_never_rank() {
    // PR 0 blocks 1; 1 is in hub 2.
    let map = graph(&[P, B, H], &[(0, 1)], &[(1, 2)]);
    let ranking = rank(&map);
    assert!(ranking.ready.is_empty(), "1 is blocked by an open PR");
    assert!(ranking.bottlenecks.is_empty(), "a PR is not an open issue");
    assert!(ranking.critical.is_empty());
}

#[test]
fn a_cycle_counts_each_issue_once_and_ends() {
    // 0 -> 1 -> 2 -> 0, and 2 -> 3.
    let map = graph(&[B, B, B, B], &[(0, 1), (1, 2), (2, 0), (2, 3)], &[]);
    let ranking = rank(&map);
    assert!(ranking.ready.is_empty(), "every issue in a cycle has an open blocker");
    assert_eq!(ranking.bottlenecks, vec![(0, 3), (1, 3), (2, 3)], "the start never counts itself");
}

#[test]
fn the_critical_path_is_the_longest_open_chain_inside_each_milestone() {
    // Hub 6 holds 0..=3: 0 -> 1 -> 2 and 0 -> 3. 4 is outside, so 2 -> 4
    // does not count. Hub 7 holds 5 alone.
    let map = graph(
        &[R, B, B, B, B, R, H, H],
        &[(0, 1), (1, 2), (0, 3), (2, 4)],
        &[(0, 6), (1, 6), (2, 6), (3, 6), (5, 7)],
    );
    let ranking = rank(&map);
    assert_eq!(ranking.critical, vec![(6, vec![0, 1, 2])], "a one-issue milestone has no chain");
}

#[test]
fn the_critical_path_skips_closed_members_and_survives_a_cycle() {
    // Hub 5 holds 0..=4: closed 0 -> 1, then 1 -> 2 -> 3 -> 1 and 3 -> 4.
    let map = graph(
        &[C, B, B, B, B, H],
        &[(0, 1), (1, 2), (2, 3), (3, 1), (3, 4)],
        &[(0, 5), (1, 5), (2, 5), (3, 5), (4, 5)],
    );
    let ranking = rank(&map);
    assert_eq!(ranking.critical, vec![(5, vec![1, 2, 3, 4])], "the link that closes the cycle drops");
}

#[test]
fn an_empty_graph_ranks_nothing() {
    assert_eq!(rank(&MapGraph::default()), Ranking::default());
}
