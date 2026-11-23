//! Ranks on the dependency map: the open issues that can start now, the
//! ones that hold up the most others, and the longest open blocking chain
//! in each milestone. Pure: reads a built `MapGraph`, so it ranks what
//! the map shows.

use std::collections::HashSet;

use super::graph::{MapGraph, NodeState};
use super::physics::EdgeStyle;

/// What the ranking strip lists. Every number is a node index of the
/// graph it was built from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranking {
    /// Open issues in a blocking relation with no open blocker: the most
    /// unblocked first, then in node order.
    pub ready: Vec<usize>,
    /// Open issues that block at least one open issue, with the count of
    /// open issues they block directly or through a chain: the most first,
    /// then in node order.
    pub bottlenecks: Vec<(usize, usize)>,
    /// The longest open blocking chain of each milestone hub, blocker
    /// first, in hub order. Only chains of two or more show.
    pub critical: Vec<(usize, Vec<usize>)>,
}

/// Rank the open issues of `graph`. PRs, closed items and hubs never
/// rank. A cycle of blocks links counts each issue once.
pub fn rank(graph: &MapGraph) -> Ranking {
    let blocks = open_blocks(graph);
    let unblocks: Vec<usize> = (0..graph.nodes.len()).map(|i| reach(&blocks, i)).collect();
    let in_relation: HashSet<usize> = graph.edges.iter()
        .filter(|edge| edge.2 == EdgeStyle::Directed)
        .flat_map(|&(src, dst, _)| [src, dst])
        .collect();
    let mut ready: Vec<usize> = (0..graph.nodes.len())
        .filter(|&i| graph.nodes[i].state == NodeState::Ready && in_relation.contains(&i))
        .collect();
    ready.sort_by_key(|&i| (std::cmp::Reverse(unblocks[i]), i));
    let mut bottlenecks: Vec<(usize, usize)> =
        unblocks.iter().copied().enumerate().filter(|&(_, count)| count > 0).collect();
    bottlenecks.sort_by_key(|&(i, count)| (std::cmp::Reverse(count), i));
    let critical = (0..graph.nodes.len())
        .filter(|&hub| graph.nodes[hub].state == NodeState::Hub)
        .filter_map(|hub| {
            let members: HashSet<usize> = graph.edges.iter()
                .filter(|&&(src, dst, style)| style == EdgeStyle::Hub && dst == hub && is_open_issue(graph, src))
                .map(|&(src, _, _)| src)
                .collect();
            let chain = longest_chain(&blocks, &members);
            (chain.len() >= 2).then_some((hub, chain))
        })
        .collect();
    Ranking { ready, bottlenecks, critical }
}

/// An open issue: blocked or ready, not a PR, closed item or hub.
fn is_open_issue(graph: &MapGraph, index: usize) -> bool {
    matches!(graph.nodes[index].state, NodeState::Blocked | NodeState::Ready)
}

/// Blocks edges between open issues, as out-neighbors per node, sorted.
fn open_blocks(graph: &MapGraph) -> Vec<Vec<usize>> {
    let mut blocks = vec![vec![]; graph.nodes.len()];
    for &(src, dst, style) in &graph.edges {
        if style == EdgeStyle::Directed && is_open_issue(graph, src) && is_open_issue(graph, dst) {
            blocks[src].push(dst);
        }
    }
    for list in &mut blocks {
        list.sort_unstable();
        list.dedup();
    }
    blocks
}

/// Nodes reachable from `start` along `blocks`, not counting `start`.
fn reach(blocks: &[Vec<usize>], start: usize) -> usize {
    let mut seen = HashSet::from([start]);
    let mut queue = vec![start];
    while let Some(node) = queue.pop() {
        for &next in &blocks[node] {
            if seen.insert(next) {
                queue.push(next);
            }
        }
    }
    seen.len() - 1
}

/// The longest chain along `blocks` inside `members`, blocker first. A
/// depth-first walk drops the links that close a cycle, so the rest is
/// acyclic and the longest chain is exact there. Ties go to the lowest
/// node index.
fn longest_chain(blocks: &[Vec<usize>], members: &HashSet<usize>) -> Vec<usize> {
    let mut roots: Vec<usize> = members.iter().copied().collect();
    roots.sort_unstable();
    // Walk state per node: 0 unseen, 1 on the walk, 2 done.
    let mut state = vec![0u8; blocks.len()];
    // Chain length from each done node, and its next node on that chain.
    let mut length = vec![0usize; blocks.len()];
    let mut next: Vec<Option<usize>> = vec![None; blocks.len()];
    for &root in &roots {
        if state[root] != 0 {
            continue;
        }
        state[root] = 1;
        let mut stack = vec![(root, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (node, at) = *top;
            let children = &blocks[node];
            if let Some(&child) = children.get(at) {
                top.1 += 1;
                if members.contains(&child) && state[child] == 0 {
                    state[child] = 1;
                    stack.push((child, 0));
                }
                continue;
            }
            // Children done: links to nodes still on the walk close a
            // cycle and do not count.
            let best = children.iter().copied()
                .filter(|&child| members.contains(&child) && state[child] == 2)
                .max_by_key(|&child| (length[child], std::cmp::Reverse(child)));
            length[node] = 1 + best.map_or(0, |child| length[child]);
            next[node] = best;
            state[node] = 2;
            stack.pop();
        }
    }
    let Some(start) = roots.iter().copied().max_by_key(|&i| (length[i], std::cmp::Reverse(i))) else {
        return vec![];
    };
    let mut chain = vec![start];
    let mut node = start;
    while let Some(after) = next[node] {
        chain.push(after);
        node = after;
    }
    chain
}
