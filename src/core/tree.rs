use std::collections::{HashMap, HashSet};

use crate::model::{IssueData, LazyChildren, SideRow};

/// Deepest indent level drawn; GitHub allows 8, the sidebar shows 4.
const MAX_DEPTH: usize = 4;

/// True when the tree should fetch REST sub-issues for `parent`: the
/// summary reports children the fetched rows do not cover, and the
/// session holds neither a result nor an in-flight request.
pub(crate) fn needs_lazy_children(
    parent: u64,
    data: &HashMap<u64, IssueData>,
    lazy: &HashMap<u64, LazyChildren>,
) -> bool {
    if lazy.contains_key(&parent) {
        return false;
    }
    let Some(summary) = data.get(&parent).and_then(|d| d.sub_summary) else {
        return false;
    };
    if summary.total == 0 {
        return false;
    }
    let fetched = data.values().filter(|d| d.parent == Some(parent)).count() as u64;
    fetched < summary.total
}

/// Tree input that stays constant across the descent.
struct TreeCtx<'a> {
    data: &'a HashMap<u64, IssueData>,
    children: &'a HashMap<u64, Vec<u64>>,
    folded: &'a HashSet<u64>,
    lazy: &'a HashMap<u64, LazyChildren>,
    visible: &'a HashSet<u64>,
}

pub(crate) fn tree_rows(
    rows: &[u64],
    data: &HashMap<u64, IssueData>,
    folded: &HashSet<u64>,
    lazy: &HashMap<u64, LazyChildren>,
) -> Vec<SideRow> {
    let visible: HashSet<u64> = rows.iter().copied().collect();
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut roots = Vec::new();
    let mut nested = HashSet::new();
    for &n in rows {
        let mut parent = data.get(&n).and_then(|d| d.parent);
        let mut seen = HashSet::from([n]);
        let mut cursor = parent;
        while let Some(p) = cursor.filter(|p| visible.contains(p)) {
            if !seen.insert(p) { parent = None; break; }
            cursor = data.get(&p).and_then(|d| d.parent);
        }
        if let Some(p) = parent.filter(|p| visible.contains(p)) {
            children.entry(p).or_default().push(n);
            nested.insert(n);
        } else {
            roots.push(n);
        }
    }
    // REST children join the tree under their parent. A child the query
    // already fetched stops being a root, so it renders once at normal
    // brightness; the rest render as outside-query rows.
    for &parent in rows {
        let Some(LazyChildren::Loaded(list)) = lazy.get(&parent) else { continue };
        for &child in list {
            if child == parent || !nested.insert(child) { continue; }
            roots.retain(|n| *n != child);
            children.entry(parent).or_default().push(child);
        }
    }
    let ctx = TreeCtx { data, children: &children, folded, lazy, visible: &visible };
    let mut out = Vec::new();
    for n in roots {
        visit(n, 0, "", "", &ctx, &mut out);
    }
    out
}

/// `guide` is this row's connector column (`│ `, `├─`, `└─` per level);
/// `prefix` is what its children inherit. Both stop growing past
/// `MAX_DEPTH` levels so deep chains keep their indent.
fn visit(n: u64, depth: usize, guide: &str, prefix: &str, ctx: &TreeCtx, out: &mut Vec<SideRow>) {
    let descendants = ctx.children.get(&n);
    let has_children = descendants.is_some() || summary_total(ctx.data, n) > 0;
    // A parent whose children exist only on GitHub shows folded until its
    // first expand loads them: `▾` over nothing would read as empty.
    let unloaded = descendants.is_none() && has_children && !ctx.lazy.contains_key(&n);
    let folded = ctx.folded.contains(&n) || unloaded;
    out.push(SideRow::Tree { number: n, depth: depth.min(MAX_DEPTH), guide: tail(guide),
        has_children, folded, outside: !ctx.visible.contains(&n) });
    if folded { return; }
    if let Some(list) = descendants {
        for (i, &child) in list.iter().enumerate() {
            let last = i + 1 == list.len();
            let guide = format!("{prefix}{}", if last { "└─" } else { "├─" });
            let next = format!("{prefix}{}", if last { "  " } else { "│ " });
            visit(child, depth + 1, &guide, &next, ctx, out);
        }
    }
    if matches!(ctx.lazy.get(&n), Some(LazyChildren::Loading)) {
        out.push(SideRow::Loading { guide: tail(&format!("{prefix}└─")) });
    }
}

/// Sub-issue count from the summary; only `total > 0` marks a parent
/// (GraphQL ships `{0, 0}` for every leaf).
fn summary_total(data: &HashMap<u64, IssueData>, n: u64) -> u64 {
    data.get(&n).and_then(|d| d.sub_summary).map_or(0, |s| s.total)
}

/// Last `MAX_DEPTH` connector cells of `guide`.
fn tail(guide: &str) -> String {
    let cells: Vec<char> = guide.chars().collect();
    cells[cells.len().saturating_sub(2 * MAX_DEPTH)..].iter().collect()
}
