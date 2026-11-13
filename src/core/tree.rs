use std::collections::{HashMap, HashSet};

use crate::model::{IssueData, SideRow};

/// Deepest indent level drawn; GitHub allows 8, the sidebar shows 4.
const MAX_DEPTH: usize = 4;

pub(crate) fn tree_rows(
    rows: &[u64], data: &HashMap<u64, IssueData>, folded: &HashSet<u64>,
) -> Vec<SideRow> {
    let visible: HashSet<u64> = rows.iter().copied().collect();
    let mut children: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut roots = Vec::new();
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
        } else {
            roots.push(n);
        }
    }
    /// `guide` is this row's connector column (`│ `, `├─`, `└─` per
    /// level); `prefix` is what its children inherit. Both stop growing
    /// past `MAX_DEPTH` levels so deep chains keep their indent.
    fn visit(n: u64, depth: usize, guide: &str, prefix: &str,
             children: &HashMap<u64, Vec<u64>>, folded: &HashSet<u64>, out: &mut Vec<SideRow>) {
        let descendants = children.get(&n);
        let cells: Vec<char> = guide.chars().collect();
        let guide: String = cells[cells.len().saturating_sub(2 * MAX_DEPTH)..].iter().collect();
        out.push(SideRow::Tree { number: n, depth: depth.min(MAX_DEPTH), guide,
            has_children: descendants.is_some(), folded: folded.contains(&n) });
        if folded.contains(&n) { return; }
        if let Some(list) = descendants {
            for (i, &child) in list.iter().enumerate() {
                let last = i + 1 == list.len();
                let guide = format!("{prefix}{}", if last { "└─" } else { "├─" });
                let next = format!("{prefix}{}", if last { "  " } else { "│ " });
                visit(child, depth + 1, &guide, &next, children, folded, out);
            }
        }
    }
    let mut out = Vec::new();
    for n in roots { visit(n, 0, "", "", &children, folded, &mut out); }
    out
}
