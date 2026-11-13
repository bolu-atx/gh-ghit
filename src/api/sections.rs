use super::{Row, SectionRows};
use super::client::{client, graphql_auth_failure, graphql_url, post_json};
use crate::model::{Author, BoardDetail, IssueData, Label};
use serde_json::{json, Value};
use std::collections::HashSet;

const PR_FIELDS: &str = r#"
 number title state author { login } labels(first: 100) { nodes { name } }
 assignees(first: 100) { nodes { login } } createdAt updatedAt url
 milestone { title }
 comments(first: 1) { totalCount } mergedAt
 commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

const ISSUE_FIELDS: &str = r#"
 number title state author { login } labels(first: 100) { nodes { name } }
 assignees(first: 100) { nodes { login } } createdAt updatedAt url
 milestone { title }
 comments(first: 1) { totalCount }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

/// Total-row ceiling across the four section lists: one giant repo must not
/// blow up the sidebar (or memory) no matter how many pages it fills.
/// Fetching stops once the raw nodes reach this many, and `parse_rows`
/// truncates the merged rows here too.
pub const MAX_SECTION_ROWS: usize = 500;

/// Warm-refresh early stop: retire a section once a page holds this many
/// rows already in the last cache (the overlap anchor), or after this many
/// pages. A cold fetch (no cache) keeps walking to the row cap.
const OVERLAP_STOP: usize = 3;
const MAX_SECTION_PAGES: usize = 3;

// NOTE(#12): that issue also touches this sections query, and either may
// land first — so this change only adds the cursor/pageInfo plumbing that
// pagination needs, nothing else.
fn query() -> String {
    format!(r#"query($owner: String!, $repo: String!, $openAfter: String, $mergedAfter: String, $openIssuesAfter: String, $closedIssuesAfter: String) {{
 repository(owner: $owner, name: $repo) {{
 open: pullRequests(first: 50, states: OPEN, after: $openAfter, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}
 merged: pullRequests(first: 50, states: MERGED, after: $mergedAfter, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}
 openIssues: issues(first: 50, states: OPEN, after: $openIssuesAfter, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}
 closedIssues: issues(first: 50, states: CLOSED, after: $closedIssuesAfter, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}
 }} }}"#)
}

fn within_window(s: Option<&str>, now: chrono::DateTime<chrono::Utc>, hours: u64) -> bool {
    if hours == 0 {
        return true;
    }
    s.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|d| {
            let hours = hours.min(24 * 365 * 10) as i64;
            d.with_timezone(&chrono::Utc) >= now - chrono::Duration::hours(hours)
        })
}

fn parse_rows(pages: &[Value], now: chrono::DateTime<chrono::Utc>, hours: u64) -> Result<Vec<Row>, String> {
    for page in pages {
        if let Some(e) = graphql_auth_failure(page) {
            return Err(e);
        }
    }
    pages.first().and_then(|v| v.pointer("/data/repository")).filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", pages.first().and_then(|v| v.get("errors")).unwrap_or(&Value::Null)))?;
    let mut rows = Vec::new();
    // (alias, is_pr, date key, filter to window?)
    for (section, is_pr, date_key, windowed) in [
        ("open", true, "createdAt", false),
        ("merged", true, "mergedAt", true),
        ("openIssues", false, "createdAt", false),
        ("closedIssues", false, "updatedAt", true),
    ] {
        // Pages merge in fetch order, so row order matches creation/update
        // order within each section across page boundaries.
        for page in pages {
            let repo = page.pointer("/data/repository").filter(|v| v.is_object()).ok_or("missing section rows")?;
            let nodes = repo.pointer(&format!("/{section}/nodes")).and_then(Value::as_array).ok_or("missing section rows")?;
            for n in nodes {
                if rows.len() >= MAX_SECTION_ROWS { return Ok(rows); }
                let window_at = windowed.then(|| n.get(date_key).and_then(Value::as_str)).flatten().map(str::to_owned);
                if windowed && !within_window(window_at.as_deref(), now, hours) { continue; }
            let string = |k: &str| n.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let number = n.get("number").and_then(Value::as_u64).ok_or("missing row number")?;
            let labels = n.pointer("/labels/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("name")?.as_str().map(|s| Label { name: s.to_owned() })).collect();
            let assignees = n.pointer("/assignees/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("login")?.as_str().map(|s| Author { login: s.to_owned() })).collect();
            let statuses = n.pointer("/projectItems/nodes").and_then(Value::as_array).into_iter().flatten().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
            let created = string("createdAt");
            let updated = string("updatedAt");
            let detail = BoardDetail { created: created.get(..10).unwrap_or("").to_owned(), updated: updated.get(..10).unwrap_or("").to_owned(), comments: n.pointer("/comments/totalCount").and_then(Value::as_u64).unwrap_or(0) as usize };
            // Slim listing row: body and review threads load on open
            // through the full fetch, so they stay empty here even when a
            // payload carries them.
            let milestone = n.pointer("/milestone/title").and_then(Value::as_str).map(str::to_owned);
            rows.push(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: String::new(), comments: vec![], review_threads: vec![], paired: vec![], checks: crate::model::parse_check_state(n), milestone, is_pr }, statuses, detail, complete: false, window_at });
            }
        }
    }
    Ok(rows)
}

/// `(hasNextPage, endCursor)` for one section connection of a sections page.
/// A missing pageInfo counts as done; `parse_rows` still rejects the page
/// itself when its nodes are absent.
fn section_cursor(value: &Value, section: &str) -> (bool, Option<String>) {
    let info = value.pointer(&format!("/data/repository/{section}/pageInfo"));
    let more = info.and_then(|v| v.get("hasNextPage")).and_then(Value::as_bool).unwrap_or(false);
    let cursor = info.and_then(|v| v.get("endCursor")).and_then(Value::as_str).map(str::to_owned);
    (more, cursor)
}

/// Section aliases in `parse_rows` order. Cursors and liveness use the same
/// order.
const ALIASES: [&str; 4] = ["open", "merged", "openIssues", "closedIssues"];

/// Fold one fetched page into the page loop and return its live node count.
/// The query is static, so a section that retired before this fetch comes
/// back as its first page again (`after: null`). Drop those already-seen
/// nodes before the page is merged, or `parse_rows` appends them twice.
///
/// With a cache, a section also retires early: `OVERLAP_STOP` rows it
/// already holds anchor the walk (open rows are created-desc, merged and
/// closed updated-desc, so overlap marks the cached tail), and
/// `MAX_SECTION_PAGES` bounds a burst of new rows. Without a cache the
/// walk keeps today's shape.
fn absorb_page(
    value: &mut Value,
    live: &mut [bool; 4],
    cursors: &mut [Option<String>; 4],
    page_count: &mut [usize; 4],
    cached: Option<&HashSet<u64>>,
) -> usize {
    let was_live = *live;
    for (i, section) in ALIASES.iter().enumerate() {
        if was_live[i] { continue; }
        if let Some(nodes) = value.pointer_mut(&format!("/data/repository/{section}/nodes")) {
            *nodes = Value::Array(Vec::new());
        }
    }
    let mut fetched = 0;
    for (i, section) in ALIASES.iter().enumerate() {
        if !live[i] { continue; }
        let nodes = value.pointer(&format!("/data/repository/{section}/nodes")).and_then(Value::as_array);
        fetched += nodes.map(Vec::len).unwrap_or(0);
        page_count[i] += 1;
        let overlap = cached.map_or(0, |c| {
            nodes.into_iter().flatten()
                .filter(|n| n.get("number").and_then(Value::as_u64).is_some_and(|n| c.contains(&n)))
                .count()
        });
        let (more, cursor) = section_cursor(value, section);
        let anchored = cached.is_some() && overlap >= OVERLAP_STOP;
        let capped = cached.is_some() && page_count[i] >= MAX_SECTION_PAGES;
        if more && cursor.is_some() && !anchored && !capped {
            cursors[i] = cursor;
        } else {
            live[i] = false;
            cursors[i] = None;
        }
    }
    fetched
}

/// Recent section rows, head-first. `cached` carries the numbers in the
/// last persisted payload: it enables the per-section overlap and page
/// stops in `absorb_page`. `None` (cache miss) walks every page like
/// before.
/// GraphQL kept: REST has no equivalent for the Projects v2 status
/// (`projectItems … fieldValueByName(name: "Status")`); probe
/// `gh api repos/bolu-atx/gh-ghit/pulls/116` returns no project-status
/// field at all. Batch win too: one query pages all four sections.
pub async fn sections(
    repo: &str,
    hours: u64,
    bearer: &str,
    cached: Option<&HashSet<u64>>,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<SectionRows, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    // One cursor per section, same order as `parse_rows` lists them: open,
    // merged, openIssues, closedIssues. Same page-loop shape as `pull_files`:
    // fetch, append, stop a section when its page has no next cursor — plus
    // the warm-refresh overlap/page stops and the raw-node row cap.
    let mut cursors: [Option<String>; 4] = [None, None, None, None];
    let mut live = [true; 4];
    let mut page_count = [0usize; 4];
    let mut pages = Vec::new();
    let mut fetched: usize = 0;
    loop {
        let mut value = post_json(&client, &url, bearer, &json!({"query": query(), "variables": {"owner": owner, "repo": name, "openAfter": cursors[0], "mergedAfter": cursors[1], "openIssuesAfter": cursors[2], "closedIssuesAfter": cursors[3]}})).await?;
        fetched += absorb_page(&mut value, &mut live, &mut cursors, &mut page_count, cached);
        // Rows fetched vs the row cap, while more pages remain: a single
        // page is one request and stays indeterminate.
        if live.iter().any(|l| *l) && fetched < MAX_SECTION_ROWS {
            if let Some(report) = progress {
                report(fetched, MAX_SECTION_ROWS);
            }
        }
        pages.push(value);
        if live.iter().all(|l| !l) || fetched >= MAX_SECTION_ROWS { break; }
    }
    let rows = parse_rows(&pages, chrono::Utc::now(), hours)?;
    let capped = fetched >= MAX_SECTION_ROWS || rows.len() >= MAX_SECTION_ROWS;
    Ok(SectionRows { rows, capped })
}

/// Merge a warm fetch into the last cached payload. Fresh rows win for the
/// numbers they cover; the landing check in `poll_sections` is the second
/// line, keeping full fetches of visited rows intact. Cached rows survive
/// only while their window anchor is still inside the window, so rows that
/// aged out while cached leave, and the row cap still holds.
///
/// Rows regroup by section before the fresh-then-cached merge, so the
/// default (unsorted) sidebar keeps a full refresh's order.
pub fn merge_rows(fresh: Vec<Row>, cached: Vec<Row>, now: chrono::DateTime<chrono::Utc>, hours: u64) -> Vec<Row> {
    fn section(row: &Row) -> usize {
        match (row.issue.is_pr, row.issue.state.as_str()) {
            (true, "MERGED") => 1,
            (false, "OPEN") => 2,
            (false, _) => 3,
            (true, _) => 0,
        }
    }
    let mut seen: HashSet<u64> = fresh.iter().map(|r| r.issue.number).collect();
    let mut sections: [Vec<Row>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for row in fresh {
        sections[section(&row)].push(row);
    }
    for row in cached {
        let number = row.issue.number;
        if seen.contains(&number) { continue; }
        if let Some(at) = &row.window_at {
            if !within_window(Some(at.as_str()), now, hours) { continue; }
        }
        seen.insert(number);
        sections[section(&row)].push(row);
    }
    sections.into_iter().flatten().take(MAX_SECTION_ROWS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query();
        for field in ["open: pullRequests", "merged: pullRequests", "openIssues: issues", "closedIssues: issues", "projectItems", "totalCount", "$owner", "$repo", "pageInfo", "hasNextPage", "endCursor", "$openAfter", "$mergedAfter", "$openIssuesAfter", "$closedIssuesAfter", "commits(last: 1)", "statusCheckRollup"] { assert!(q.contains(field)); }
    }
    #[test]
    fn sections_query_stays_slim_and_rows_stay_partial() {
        // Startup fetches list-weight rows only: bodies and per-PR review
        // threads load when a row opens through the full fetch.
        let q = query();
        assert!(!q.contains("reviewThreads"), "sections query carries no threads");
        assert!(!q.contains("body"), "sections query carries no body");
        for field in ["number", "title", "state", "author", "labels", "assignees", "createdAt", "updatedAt", "url", "projectItems", "statusCheckRollup"] { assert!(q.contains(field), "sidebar/match field missing: {field}"); }
        let v = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"one","state":"OPEN","author":{"login":"bo"},"labels":{"nodes":[{"name":"bug"}]},"assignees":{"nodes":[{"login":"amy"}]},"createdAt":"2026-11-10T11:00:00Z","updatedAt":"2026-11-10T11:00:00Z","url":"https://github.com/o/r/pull/1","comments":{"totalCount":2},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"SUCCESS"}}}]},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]}}]},
            "merged":{"nodes":[]},
            "openIssues":{"nodes":[]},
            "closedIssues":{"nodes":[]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 24).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.issue.body.is_empty());
        assert!(row.issue.review_threads.is_empty());
        assert!(!row.complete);
        // Sidebar render and `/` match fields survive the slimming.
        assert_eq!(row.issue.title, "one");
        assert_eq!(row.issue.author.login, "bo");
        assert_eq!(row.issue.labels[0].name, "bug");
        assert_eq!(row.issue.assignees[0].login, "amy");
        assert_eq!(row.issue.url, "https://github.com/o/r/pull/1");
        assert_eq!(row.statuses[0].1.as_deref(), Some("Todo"));
        assert_eq!(row.detail.comments, 2);
        assert_eq!(row.issue.checks, Some(crate::model::CheckState::Pass));
    }
    #[test]
    fn parses_check_rollup_per_state() {
        use crate::model::CheckState;
        let pr = |n: u64, state: Option<&str>| {
            let commits = match state {
                Some(s) => json!({"nodes": [{"commit": {"statusCheckRollup": {"state": s}}}]}),
                None => json!({"nodes": []}),
            };
            json!({"number": n, "title": "pr", "state": "OPEN", "commits": commits})
        };
        let v = json!({"data": {"repository": {
            "open": {"nodes": [pr(1, Some("SUCCESS")), pr(2, Some("FAILURE")), pr(3, Some("PENDING")), pr(4, None)]},
            "merged": {"nodes": []},
            "openIssues": {"nodes": [{"number": 7, "title": "seven", "state": "OPEN"}]},
            "closedIssues": {"nodes": []}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 24).unwrap();
        let checks = |n: u64| rows.iter().find(|r| r.issue.number == n).unwrap().issue.checks;
        assert_eq!(checks(1), Some(CheckState::Pass));
        assert_eq!(checks(2), Some(CheckState::Fail));
        assert_eq!(checks(3), Some(CheckState::Pending));
        assert_eq!(checks(4), None);
        assert_eq!(checks(7), None);
    }
    #[test]
    fn parses_rows_and_filters_merged_window() {
        let v = json!({"data":{"repository":{"open":{"nodes":[{"number":1,"title":"one","state":"OPEN","labels":{"nodes":[{"name":"bug"}]},"comments":{"totalCount":3},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]},"body":"listing body should stay out","reviewThreads":{"nodes":[{"id":"thread-id"}]}}]},"merged":{"nodes":[{"number":2,"mergedAt":"2026-11-10T11:30:00Z"},{"number":3,"mergedAt":"2026-11-09T10:00:00Z"}]},"openIssues":{"nodes":[{"number":7,"title":"seven","state":"OPEN"}]},"closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"},{"number":9,"title":"nine","state":"CLOSED","updatedAt":"2026-11-09T10:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 1).unwrap();
        assert!(rows[0].issue.body.is_empty(), "slim rows carry no body");
        assert!(rows[0].issue.review_threads.is_empty(), "slim rows carry no threads");
        assert!(!rows[0].complete, "slim rows stay partial so open runs the full fetch");
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2, 7, 8]);
        assert_eq!(rows[0].issue.labels[0].name, "bug");
        assert_eq!(rows[0].statuses[0].1.as_deref(), Some("Todo"));
        assert_eq!(rows[0].detail.comments, 3);
        assert!(rows.iter().find(|r| r.issue.number == 1).unwrap().issue.is_pr);
        assert!(!rows.iter().find(|r| r.issue.number == 7).unwrap().issue.is_pr);
        assert!(parse_rows(&[json!({"errors":[{"message":"denied"}]})], now, 1).is_err());
    }
    #[test]
    fn merges_section_pages_in_order_with_window() {
        // Two pages: open rows split across the boundary, merged and closed
        // rows split with one stale row per section that the window drops.
        let page1 = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"one","state":"OPEN"},{"number":2,"title":"two","state":"OPEN"}],"pageInfo":{"hasNextPage":true,"endCursor":"c1"}},
            "merged":{"nodes":[{"number":3,"title":"three","state":"MERGED","mergedAt":"2026-11-10T11:30:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"c2"}},
            "openIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes":[{"number":9,"title":"nine","state":"CLOSED","updatedAt":"2026-11-09T10:00:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"c3"}}}}});
        let page2 = json!({"data":{"repository":{
            "open":{"nodes":[{"number":4,"title":"four","state":"OPEN"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "merged":{"nodes":[{"number":5,"title":"five","state":"MERGED","mergedAt":"2026-11-09T10:00:00Z"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "openIssues":{"nodes":[{"number":7,"title":"seven","state":"OPEN"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[page1, page2], now, 1).unwrap();
        // Open pages merge in order, stale merged 5 and stale closed 9 drop.
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2, 4, 3, 7, 8]);
        assert!(rows.iter().find(|r| r.issue.number == 4).unwrap().issue.is_pr);
        assert!(!rows.iter().find(|r| r.issue.number == 7).unwrap().issue.is_pr);
    }
    #[test]
    fn retired_section_does_not_duplicate_rows_across_pages() {
        // Page 1: merged retires (no next page) while open continues. The
        // static query then refetches merged page 1 with `after: null` on
        // page 2, so `absorb_page` must drop those nodes before merging.
        let mut page1 = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"one","state":"OPEN"},{"number":2,"title":"two","state":"OPEN"}],"pageInfo":{"hasNextPage":true,"endCursor":"o1"}},
            "merged":{"nodes":[{"number":3,"title":"three","state":"MERGED","mergedAt":"2026-11-10T11:30:00Z"},{"number":4,"title":"four","state":"MERGED","mergedAt":"2026-11-10T11:10:00Z"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "openIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let mut page2 = json!({"data":{"repository":{
            "open":{"nodes":[{"number":5,"title":"five","state":"OPEN"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "merged":{"nodes":[{"number":3,"title":"three","state":"MERGED","mergedAt":"2026-11-10T11:30:00Z"},{"number":4,"title":"four","state":"MERGED","mergedAt":"2026-11-10T11:10:00Z"}],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "openIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        let mut pages = Vec::new();
        absorb_page(&mut page1, &mut live, &mut cursors, &mut count, None);
        assert!(live[0] && !live[1], "open stays live, merged retires on page 1");
        pages.push(page1);
        absorb_page(&mut page2, &mut live, &mut cursors, &mut count, None);
        assert!(page2.pointer("/data/repository/merged/nodes").and_then(Value::as_array).is_some_and(Vec::is_empty), "retired merged nodes are cleared from page 2");
        pages.push(page2);
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&pages, now, 1).unwrap();
        let numbers = |state: &str| rows.iter().filter(|r| r.issue.state == state).map(|r| r.issue.number).collect::<Vec<_>>();
        assert_eq!(numbers("OPEN"), vec![1, 2, 5], "open rows merge in page order");
        assert_eq!(numbers("MERGED"), vec![3, 4], "each merged row appears exactly once");
    }
    #[test]
    fn section_rows_stop_at_named_cap() {
        let nodes: Vec<Value> = (1..=MAX_SECTION_ROWS as u64 + 10)
            .map(|n| json!({"number": n, "title": "row", "state": "OPEN"}))
            .collect();
        let (head, tail) = nodes.split_at(50);
        let page = |ns: &[Value]| json!({"data":{"repository":{
            "open":{"nodes": ns, "pageInfo":{"hasNextPage":true,"endCursor":"c"}},
            "merged":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "openIssues":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[page(head), page(tail)], now, 24).unwrap();
        assert_eq!(rows.len(), MAX_SECTION_ROWS);
        assert_eq!(rows[0].issue.number, 1);
        assert_eq!(rows[MAX_SECTION_ROWS - 1].issue.number, MAX_SECTION_ROWS as u64);
    }
    /// One sections page with `open` nodes; the other sections are empty
    /// and done, so only the open section stays live.
    fn open_page(numbers: &[u64], more: bool, cursor: Option<&str>) -> Value {
        let nodes: Vec<Value> = numbers.iter()
            .map(|n| json!({"number": n, "title": "row", "state": "OPEN"}))
            .collect();
        json!({"data":{"repository":{
            "open":{"nodes": nodes, "pageInfo":{"hasNextPage":more,"endCursor":cursor}},
            "merged":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "openIssues":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes": [], "pageInfo":{"hasNextPage":false,"endCursor":null}}}}})
    }
    fn section_row(number: u64, title: &str, state: &str, is_pr: bool, window_at: Option<&str>) -> Row {
        Row {
            issue: IssueData {
                number, title: title.to_owned(), state: state.to_owned(),
                author: Author::default(), labels: vec![], assignees: vec![],
                created_at: String::new(), updated_at: String::new(),
                url: String::new(), body: String::new(), comments: vec![],
                is_pr, review_threads: vec![], paired: vec![], checks: None, milestone: None,
            },
            statuses: vec![],
            detail: BoardDetail::default(),
            complete: false,
            window_at: window_at.map(str::to_owned),
        }
    }
    #[test]
    fn overlap_retires_a_warm_section_before_its_last_page() {
        let cached: HashSet<u64> = [1, 2, 3].into();
        let mut page = open_page(&[1, 2, 3, 4], true, Some("c1"));
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        absorb_page(&mut page, &mut live, &mut cursors, &mut count, Some(&cached));
        assert!(!live[0], "OVERLAP_STOP cached rows anchor the walk");
        assert!(cursors[0].is_none(), "a retired section holds no cursor");
        assert_eq!(count[0], 1);
    }
    #[test]
    fn warm_section_keeps_paging_below_the_overlap() {
        let cached: HashSet<u64> = [1, 2].into();
        let mut page = open_page(&[1, 2, 3, 4], true, Some("c1"));
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        absorb_page(&mut page, &mut live, &mut cursors, &mut count, Some(&cached));
        assert!(live[0], "two cached rows are not enough to stop");
        assert_eq!(cursors[0].as_deref(), Some("c1"));
    }
    #[test]
    fn warm_section_stops_at_the_page_cap_while_a_cold_walk_continues() {
        let cached = HashSet::new();
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        for page in 1..=MAX_SECTION_PAGES {
            let mut value = open_page(&[100 + page as u64], true, Some("c"));
            absorb_page(&mut value, &mut live, &mut cursors, &mut count, Some(&cached));
        }
        assert!(!live[0], "no overlap: MAX_SECTION_PAGES retires the section");
        assert_eq!(count[0], MAX_SECTION_PAGES);

        // Cache miss: same synthetic pages, but the walk keeps today's shape.
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        for page in 1..=MAX_SECTION_PAGES + 1 {
            let mut value = open_page(&[100 + page as u64], true, Some("c"));
            absorb_page(&mut value, &mut live, &mut cursors, &mut count, None);
        }
        assert!(live[0], "a cache miss never early-stops");
        assert_eq!(cursors[0].as_deref(), Some("c"));
        assert_eq!(count[0], MAX_SECTION_PAGES + 1);
    }
    #[test]
    fn merge_keeps_fresh_rows_and_dedups_by_number() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let fresh = vec![
            section_row(1, "fresh", "OPEN", true, None),
            section_row(2, "new", "OPEN", true, None),
            section_row(4, "merged", "MERGED", true, Some("2026-11-10T11:30:00Z")),
        ];
        let cached = vec![
            section_row(1, "stale", "OPEN", true, None),
            section_row(3, "older", "OPEN", true, None),
            section_row(3, "duplicate", "OPEN", true, None),
            section_row(5, "closed issue", "CLOSED", false, Some("2026-11-10T11:30:00Z")),
        ];
        let rows = merge_rows(fresh, cached, now, 24);
        // Fresh-then-cached within each section keeps the cold-refresh order.
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert_eq!(rows[0].issue.title, "fresh", "the fetched row wins over the cached one");
    }
    #[test]
    fn merge_drops_cached_rows_that_aged_out_of_the_window() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let cached = vec![
            section_row(1, "still merged", "MERGED", true, Some("2026-11-10T11:30:00Z")),
            section_row(2, "aged merged", "MERGED", true, Some("2026-11-09T10:00:00Z")),
            section_row(3, "open stays", "OPEN", false, None),
        ];
        let rows = merge_rows(vec![], cached, now, 1);
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 3]);
    }
    #[test]
    fn parsed_rows_carry_the_window_anchor_used_at_merge_time() {
        let v = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"one","state":"OPEN"}]},
            "merged":{"nodes":[{"number":2,"title":"two","state":"MERGED","mergedAt":"2026-11-10T11:30:00Z"}]},
            "openIssues":{"nodes":[]},
            "closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 24).unwrap();
        let anchor = |n: u64| rows.iter().find(|r| r.issue.number == n).unwrap().window_at.clone();
        assert_eq!(anchor(1), None, "open rows never age out");
        assert_eq!(anchor(2).as_deref(), Some("2026-11-10T11:30:00Z"));
        assert_eq!(anchor(8).as_deref(), Some("2026-11-10T11:00:00Z"));
    }
    #[test]
    fn cached_rows_roundtrip_through_json() {
        let row = section_row(7, "cached", "MERGED", true, Some("2026-11-10T11:30:00Z"));
        let raw = serde_json::to_string(&row).unwrap();
        let back: Row = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.issue.number, 7);
        assert_eq!(back.issue.title, "cached");
        assert_eq!(back.window_at.as_deref(), Some("2026-11-10T11:30:00Z"));
    }

    #[test]
    fn benign_field_errors_still_parse_rows() {
        // Non-auth field errors stay best-effort: rows still parse.
        let benign = json!({
            "data": {"repository": {
                "open": {"nodes": []}, "merged": {"nodes": []},
                "openIssues": {"nodes": []}, "closedIssues": {"nodes": []}}},
            "errors": [{"message": "something odd on one field"}],
        });
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        assert!(parse_rows(&[benign], now, 24).unwrap().is_empty());
        // An auth-class payload surfaces instead of parsing.
        let denied = json!({
            "data": {"repository": {"open": {"nodes": []}}},
            "errors": [{"type": "FORBIDDEN", "message": "Resource not accessible by personal access token"}],
        });
        assert!(parse_rows(&[denied], now, 24).is_err());
    }
}
