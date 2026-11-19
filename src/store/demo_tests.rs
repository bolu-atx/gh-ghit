//! A dense synthetic store for trying the dependency map by hand. It
//! writes through the real `open_at`, `Store::write` and `put_detail`, so
//! the demo can never drift from the schema. `scripts/graph-demo.sh`
//! runs it.

use super::open_at;
use crate::api::Row;
use crate::model::{BoardDetail, IssueData, IssueLinks};

const MILESTONES: [&str; 5] = ["Offline first", "Fast startup", "Clear review flow", "Safe sync", "Readable diffs"];

/// One demo item: number, PR or not, state, title, milestone index.
type Spec = (u64, bool, &'static str, &'static str, Option<usize>);

const ITEMS: &[Spec] = &[
    (1, false, "CLOSED", "Pick a storage engine", Some(0)),
    (2, false, "OPEN", "Write the schema", Some(0)),
    (3, false, "OPEN", "Store every listed row", Some(0)),
    (4, false, "OPEN", "Read rows back on start", Some(0)),
    (5, false, "OPEN", "Serve opens from the store", Some(0)),
    (6, false, "OPEN", "Work with no network at all", Some(0)),
    (7, false, "CLOSED", "Measure cold start", Some(1)),
    (8, false, "OPEN", "Defer the auth check", Some(1)),
    (9, false, "OPEN", "Paint before the first fetch", Some(1)),
    (10, false, "OPEN", "Start in under 50 ms", Some(1)),
    (11, false, "OPEN", "Show review threads inline", Some(2)),
    (12, false, "CLOSED", "Parse pending reviews", Some(2)),
    (13, false, "OPEN", "Submit a review in one key", Some(2)),
    (14, false, "OPEN", "Take a sync lease", Some(3)),
    (15, false, "OPEN", "Hand the lease over on exit", Some(3)),
    (16, false, "OPEN", "Split the fetch queue", Some(1)),
    (17, false, "OPEN", "Fetch the current issue first", Some(1)),
    (18, false, "OPEN", "Cap fetches in flight", Some(1)),
    (19, false, "OPEN", "Back off when rate limited", Some(3)),
    (20, false, "OPEN", "Record sync start times", Some(3)),
    (21, false, "OPEN", "Diff rows since the last sync", Some(3)),
    (22, false, "OPEN", "Detect deleted items", Some(3)),
    (23, false, "OPEN", "Catch every change on refresh", Some(3)),
    (24, false, "OPEN", "One local copy per repo", Some(0)),
    (25, false, "OPEN", "Search the local copy", Some(0)),
    (26, false, "OPEN", "Finder reads the store", Some(0)),
    (27, false, "OPEN", "Board reads the store", None),
    (28, false, "OPEN", "Labels pane reads the store", None),
    (29, false, "OPEN", "History survives restarts", Some(0)),
    (30, false, "OPEN", "Pins live in the store", None),
    (31, false, "OPEN", "Backfill the whole repo", Some(3)),
    (32, false, "OPEN", "Parse unified diffs", Some(4)),
    (33, false, "CLOSED", "Word-level highlights", Some(4)),
    (34, false, "OPEN", "Side-by-side layout", Some(4)),
    (35, false, "OPEN", "Readable diffs on 80 columns", Some(4)),
    (36, false, "OPEN", "Collect review comments", Some(2)),
    (37, false, "OPEN", "Thread replies", Some(2)),
    (38, false, "OPEN", "Resolve threads", Some(2)),
    (39, false, "OPEN", "Review without a browser", Some(2)),
    (40, false, "OPEN", "Store epic: tables", Some(0)),
    (41, false, "OPEN", "Items table", Some(0)),
    (42, false, "OPEN", "Details table", Some(0)),
    (43, false, "OPEN", "Edges table", Some(0)),
    (44, false, "OPEN", "Diff epic: rendering", Some(4)),
    (45, false, "OPEN", "Fold large files", Some(4)),
    (46, false, "OPEN", "Expand all files", Some(4)),
    (47, false, "CLOSED", "Old diff prototype", None),
    (48, false, "CLOSED", "Typo in the README", None),
    (49, false, "OPEN", "Theme toggle flickers", None),
    (50, false, "OPEN", "Help screen scrolls past the end", None),
    (51, false, "OPEN", "Support GitHub Enterprise hosts", None),
    (52, false, "OPEN", "Copy issue URL", None),
    (53, false, "OPEN", "Wrap long labels", None),
    (54, false, "OPEN", "Mouse wheel on the board", None),
    (55, false, "OPEN", "Document the drive protocol", None),
    (56, true, "OPEN", "Add the schema", Some(0)),
    (57, true, "OPEN", "Write rows on sync", Some(0)),
    (58, true, "OPEN", "Defer auth until first fetch", Some(1)),
    (59, true, "OPEN", "One-key review submit", Some(2)),
    (60, true, "OPEN", "Release the lease on quit", Some(3)),
    (61, true, "OPEN", "Local search over rows", Some(0)),
    (62, true, "OPEN", "Finder on the store", Some(0)),
    (63, true, "OPEN", "Fit diffs to 80 columns", Some(4)),
    (64, true, "OPEN", "Delta sync on refresh", Some(3)),
    (65, true, "OPEN", "Review flow end to end", Some(2)),
    (66, true, "OPEN", "Create the items table", Some(0)),
    (67, true, "MERGED", "Choose SQLite", Some(0)),
    (68, true, "MERGED", "Cold start probe", Some(1)),
    (69, true, "MERGED", "Pending review parser", Some(2)),
    (70, true, "MERGED", "Word diff spike", Some(4)),
    (71, true, "OPEN", "Bump dependencies", Some(1)),
    (72, true, "OPEN", "Trim the release binary", Some(1)),
    (73, true, "OPEN", "Tidy the sync module", Some(3)),
    (74, true, "OPEN", "Split the store file", Some(0)),
    (75, true, "OPEN", "Faster diff parse", Some(4)),
    (76, true, "OPEN", "Fix a typo in help", None),
    (77, true, "OPEN", "CI: cache cargo", None),
    (78, true, "OPEN", "Add a CONTRIBUTING note", None),
    (79, true, "OPEN", "Lint shell scripts", None),
    (80, true, "OPEN", "Update screenshots", None),
];

/// `(blocker, blocked)`: chains of 2 to 6, a fork, a join, one blocker
/// shared by many, and two diamonds.
const BLOCKS: &[(u64, u64)] = &[
    (1, 2), (2, 3), (3, 4), (4, 5), (5, 6),
    (7, 8), (8, 9), (9, 10),
    (11, 12), (12, 13),
    (14, 15),
    (10, 16), (16, 17), (16, 18), (16, 19),
    (19, 20), (20, 23), (21, 23), (22, 23),
    (6, 24), (23, 24),
    (24, 25), (24, 26), (24, 27), (24, 28), (24, 29), (24, 30), (24, 31),
    (32, 33), (32, 34), (33, 35), (34, 35),
    (36, 37), (36, 38), (37, 39), (38, 39),
];

/// `(parent, child)`.
const PARENTS: &[(u64, u64)] = &[(3, 40), (40, 41), (40, 42), (40, 43), (47, 44), (44, 45), (44, 46)];

/// `(pull request, issue it closes)`.
const CLOSES: &[(u64, u64)] = &[
    (56, 2), (57, 3), (58, 8), (59, 13), (60, 15), (61, 25), (62, 26), (63, 35), (64, 23), (65, 39),
    (66, 41), (67, 1), (68, 7), (69, 12), (70, 33),
];

fn demo_rows() -> Vec<Row> {
    ITEMS.iter().map(|&(number, is_pr, state, title, milestone)| {
        let incoming = |pairs: &[(u64, u64)]| pairs.iter().filter(|p| p.1 == number).map(|p| p.0).collect::<Vec<u64>>();
        let path = if is_pr { "pull" } else { "issues" };
        let issue = IssueData {
            number,
            is_pr,
            state: state.to_string(),
            title: title.to_string(),
            url: format!("https://github.com/demo/graph/{path}/{number}"),
            created_at: "2026-11-01T00:00:00Z".to_string(),
            updated_at: format!("2026-11-{:02}T12:00:00Z", 2 + number % 17),
            milestone: milestone.map(|i| MILESTONES[i].to_string()),
            parent: incoming(PARENTS).first().copied(),
            links: IssueLinks { blocked_by: incoming(BLOCKS), closed_by: incoming(CLOSES) },
            ..IssueData::default()
        };
        let window_at = (state != "OPEN").then(|| issue.updated_at.clone());
        Row { issue, statuses: vec![], detail: BoardDetail::default(), complete: false, window_at }
    }).collect()
}

#[test]
fn demo_items_are_well_formed() {
    let rows = demo_rows();
    assert_eq!(rows.len(), 80);
    assert_eq!(rows.iter().filter(|row| row.issue.is_pr).count(), 25);
    let known = |n: u64| rows.iter().any(|row| row.issue.number == n);
    for &(a, b) in BLOCKS.iter().chain(PARENTS).chain(CLOSES) {
        assert!(known(a) && known(b), "{a} -> {b}");
    }
}

#[test]
#[ignore = "writes the graph demo store; scripts/graph-demo.sh runs it"]
fn write_graph_demo_store() {
    let path = std::path::PathBuf::from(std::env::var("GHIT_DEMO_STORE").expect("GHIT_DEMO_STORE names the store file"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("create the store dir");
    }
    let _ = std::fs::remove_file(&path);
    let mut store = open_at(&path).expect("store opens");
    let rows = demo_rows();
    store.write("2026-11-19T00:00:00Z", &rows).expect("store writes");
    // Stored details let every open serve offline, with no request.
    for row in &rows {
        let mut issue = row.issue.clone();
        issue.body = format!("Demo item #{} for the dependency map.", issue.number);
        let bundle = (issue, Vec::<(String, Option<String>)>::new());
        store.put_detail(super::Detail::Issue, row.issue.number, &row.issue.updated_at, &bundle).expect("detail writes");
    }
    let stored = store.map_items().expect("items read back");
    assert_eq!(stored.len(), 80);
    assert!(store.edges().expect("edges read back").len() >= BLOCKS.len() + PARENTS.len() + CLOSES.len());
}

