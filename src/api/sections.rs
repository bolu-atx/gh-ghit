use super::{Row, SectionRows};
use super::client::{client, graphql_auth_failure, graphql_url, post_json, RATE_LIMIT_FIELD};
use crate::model::{Author, BoardDetail, IssueData, Label, SubSummary};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) const PR_FIELDS: &str = r#"
 number title state author { login } labels(first: 20) { nodes { name } }
 assignees(first: 10) { nodes { login } } createdAt updatedAt url
 milestone { title }
 comments(first: 1) { totalCount } mergedAt
 commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

pub(super) const ISSUE_FIELDS: &str = r#"
 number title state author { login } labels(first: 20) { nodes { name } }
 assignees(first: 10) { nodes { login } } createdAt updatedAt url
 milestone { title }
 comments(first: 1) { totalCount }
 parent { number }
 subIssuesSummary { total completed }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

const STATUS_PR_FIELDS: &str = r#"
 commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

const STATUS_ISSUE_FIELDS: &str = r#"
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

/// Numbers per status query.
const STATUS_CHUNK: usize = 50;

/// Silent-change fields of one row: CI rollup and project statuses.
pub struct StatusUpdate {
    pub number: u64,
    pub checks: Option<crate::model::CheckState>,
    pub statuses: super::Statuses,
}

/// The open rows whose CI or project status can change without bumping
/// `updatedAt`: `(number, is_pr)`.
pub fn status_targets(rows: &[Row]) -> Vec<(u64, bool)> {
    rows.iter().filter(|r| r.issue.state == "OPEN").map(|r| (r.issue.number, r.issue.is_pr)).collect()
}

/// One aliased query per chunk of 50 targets: `p<n>` pull requests,
/// `i<n>` issues.
fn status_queries(targets: &[(u64, bool)]) -> Vec<String> {
    targets.chunks(STATUS_CHUNK).map(|chunk| {
        let fields: String = chunk.iter().map(|&(n, is_pr)| if is_pr {
            format!(" p{n}: pullRequest(number: {n}) {{ {STATUS_PR_FIELDS} }}")
        } else {
            format!(" i{n}: issue(number: {n}) {{ {STATUS_ISSUE_FIELDS} }}")
        }).collect();
        format!("query($owner: String!, $repo: String!) {{\n repository(owner: $owner, name: $repo) {{{fields} }}\n {RATE_LIMIT_FIELD} }}")
    }).collect()
}

fn parse_status_page(value: &Value, chunk: &[(u64, bool)]) -> Result<Vec<StatusUpdate>, String> {
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
    let repo = value.pointer("/data/repository").filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    Ok(chunk.iter().filter_map(|&(number, is_pr)| {
        let n = repo.get(format!("{}{number}", if is_pr { "p" } else { "i" })).filter(|v| v.is_object())?;
        let statuses = n.pointer("/projectItems/nodes").and_then(Value::as_array).into_iter().flatten().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
        Some(StatusUpdate { number, checks: crate::model::parse_check_state(n), statuses })
    }).collect())
}

/// Fetch CI and project status for the open `targets`, chunked.
pub async fn fetch_statuses(repo: &str, targets: &[(u64, bool)], bearer: &str) -> Result<Vec<StatusUpdate>, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    let mut updates = Vec::new();
    for (query, chunk) in status_queries(targets).iter().zip(targets.chunks(STATUS_CHUNK)) {
        let body = json!({"query": query, "variables": {"owner": owner, "repo": name}});
        updates.extend(parse_status_page(&post_json(&client, &url, bearer, &body).await?, chunk)?);
    }
    Ok(updates)
}

/// Write status-only updates into matching rows; every other field stays.
/// Numbers with no row are ignored.
pub fn apply_statuses(rows: &mut [Row], updates: Vec<StatusUpdate>) {
    for u in updates {
        if let Some(r) = rows.iter_mut().find(|r| r.issue.number == u.number) {
            if r.issue.is_pr { r.issue.checks = u.checks; }
            r.statuses = u.statuses;
        }
    }
}

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
fn query(show: [bool; 4]) -> String {
    let decl = |i: usize, d: &str| if show[i] { format!(", {d}") } else { String::new() };
    let decls = format!("{}{}{}{}", decl(0, "$openAfter: String"), decl(1, "$mergedAfter: String"), decl(2, "$openIssuesAfter: String"), decl(3, "$closedIssuesAfter: String"));
    let field = |i: usize, body: String| if show[i] { format!(" {body}\n") } else { String::new() };
    let open = field(0, format!("open: pullRequests(first: 50, states: OPEN, after: $openAfter, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}"));
    let merged = field(1, format!("merged: pullRequests(first: 50, states: MERGED, after: $mergedAfter, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}"));
    let open_issues = field(2, format!("openIssues: issues(first: 50, states: OPEN, after: $openIssuesAfter, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}"));
    let closed_issues = field(3, format!("closedIssues: issues(first: 50, states: CLOSED, after: $closedIssuesAfter, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }}"));
    format!("query($owner: String!, $repo: String!{decls}) {{\n repository(owner: $owner, name: $repo) {{\n{open}{merged}{open_issues}{closed_issues} }}\n {RATE_LIMIT_FIELD} }}")
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
            // A hidden section is left out of the query, so its alias is absent.
            let Some(nodes) = repo.pointer(&format!("/{section}/nodes")).and_then(Value::as_array) else { continue };
            for n in nodes {
                if rows.len() >= MAX_SECTION_ROWS { return Ok(rows); }
                let window_at = windowed.then(|| n.get(date_key).and_then(Value::as_str)).flatten().map(str::to_owned);
                if windowed && !within_window(window_at.as_deref(), now, hours) { continue; }
                rows.push(parse_node(n, is_pr, window_at)?);
            }
        }
    }
    Ok(rows)
}

/// One sections node (Issue or PullRequest fields) as a slim listing row.
pub(super) fn parse_node(n: &Value, is_pr: bool, window_at: Option<String>) -> Result<Row, String> {
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
    let parent = n.pointer("/parent/number").and_then(Value::as_u64);
    let summary = n.get("subIssuesSummary").filter(|s| s.is_object());
    let sub_summary = summary.map(|s| SubSummary {
        total: s.get("total").and_then(Value::as_u64).unwrap_or(0),
        completed: s.get("completed").and_then(Value::as_u64).unwrap_or(0),
    });
    Ok(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: String::new(), comments: vec![], review_threads: vec![], paired: vec![], checks: crate::model::parse_check_state(n), milestone, parent, sub_summary, is_pr }, statuses, detail, complete: false, window_at })
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
///
/// The merged and closed-issue lists are updated-desc and `updatedAt >=
/// mergedAt`, so once a page's last node is older than the window no later
/// page holds an in-window row: the section retires. `hours == 0` (all time)
/// never retires this way, and open sections are unaffected.
fn absorb_page(
    value: &mut Value,
    live: &mut [bool; 4],
    cursors: &mut [Option<String>; 4],
    page_count: &mut [usize; 4],
    cached: Option<&HashSet<u64>>,
    now: chrono::DateTime<chrono::Utc>,
    hours: u64,
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
        // Sections 1 (merged) and 3 (closedIssues) are updated-desc.
        let past_window = hours > 0 && (i == 1 || i == 3) && nodes
            .and_then(|n| n.last())
            .and_then(|n| n.get("updatedAt")?.as_str())
            .is_some_and(|t| !within_window(Some(t), now, hours));
        if more && cursor.is_some() && !anchored && !capped && !past_window {
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
    show: [bool; 4],
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<SectionRows, String> {
    if !show.iter().any(|s| *s) {
        return Ok(SectionRows { rows: Vec::new(), capped: false });
    }
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    // One cursor per section, same order as `parse_rows` lists them: open,
    // merged, openIssues, closedIssues. Same page-loop shape as `pull_files`:
    // fetch, append, stop a section when its page has no next cursor — plus
    // the warm-refresh overlap/page stops and the raw-node row cap.
    let mut cursors: [Option<String>; 4] = [None, None, None, None];
    // A hidden section starts retired and is left out of the query text.
    let mut live = show;
    let query = query(show);
    let mut page_count = [0usize; 4];
    let mut pages = Vec::new();
    let mut fetched: usize = 0;
    loop {
        let mut variables = json!({"owner": owner, "repo": name});
        for (i, var) in ["openAfter", "mergedAfter", "openIssuesAfter", "closedIssuesAfter"].into_iter().enumerate() {
            if show[i] { variables[var] = json!(cursors[i]); }
        }
        let mut value = post_json(&client, &url, bearer, &json!({"query": query, "variables": variables})).await?;
        fetched += absorb_page(&mut value, &mut live, &mut cursors, &mut page_count, cached, chrono::Utc::now(), hours);
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
fn merge_rows(fresh: Vec<Row>, cached: Vec<Row>, now: chrono::DateTime<chrono::Utc>, hours: u64) -> Vec<Row> {
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

/// Search-index lag margin: the change-only search reaches back this far
/// before the last fetch start.
const DELTA_OVERLAP: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// A cached payload whose fetch start is older than this takes the full
/// walk instead of the change-only search.
const DELTA_MAX_AGE_SECS: i64 = 3600;

/// Delta vs full: the cached fetch start to search from, or `None` for the
/// full walk (forced, no cache, missing or bad timestamp, or too old).
pub fn delta_since(
    started: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
    full: bool,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if full {
        return None;
    }
    let at = chrono::DateTime::parse_from_rfc3339(started?).ok()?.with_timezone(&chrono::Utc);
    let age = now - at;
    (age >= chrono::TimeDelta::zero() && age < chrono::TimeDelta::seconds(DELTA_MAX_AGE_SECS))
        .then_some(at)
}

/// Issue-search text for every issue and PR changed since `started`, minus
/// the index-lag overlap.
fn delta_search_query(repo: &str, started: chrono::DateTime<chrono::Utc>) -> String {
    let since = (started - DELTA_OVERLAP).format("%Y-%m-%dT%H:%M:%SZ");
    format!("repo:{repo} updated:>={since} sort:updated-desc")
}

/// Change-only search: same row fields as the sections lists, either kind.
fn delta_query() -> String {
    format!("query($q: String!, $after: String) {{\n search(type: ISSUE, query: $q, first: 50, after: $after) {{ nodes {{ __typename ... on Issue {{ {ISSUE_FIELDS} }} ... on PullRequest {{ {PR_FIELDS} }} }} pageInfo {{ hasNextPage endCursor }} }}\n {RATE_LIMIT_FIELD} }}")
}

/// Rows plus `(hasNextPage, endCursor)` for one change-only search page.
/// The window anchor matches `parse_rows`: `mergedAt` for merged PRs,
/// `updatedAt` for closed issues, none otherwise.
fn parse_delta_page(value: &Value) -> Result<(Vec<Row>, bool, Option<String>), String> {
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
    let search = value.pointer("/data/search").filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    let mut rows = Vec::new();
    for n in search.get("nodes").and_then(Value::as_array).into_iter().flatten() {
        let is_pr = match n.get("__typename").and_then(Value::as_str) {
            Some("PullRequest") => true,
            Some("Issue") => false,
            _ => continue,
        };
        let anchor = match (is_pr, n.get("state").and_then(Value::as_str)) {
            (true, Some("MERGED")) => Some("mergedAt"),
            (false, Some("CLOSED")) => Some("updatedAt"),
            _ => None,
        };
        let window_at = anchor.and_then(|k| n.get(k)?.as_str()).map(str::to_owned);
        rows.push(parse_node(n, is_pr, window_at)?);
    }
    let info = search.get("pageInfo");
    let more = info.and_then(|v| v.get("hasNextPage")).and_then(Value::as_bool).unwrap_or(false);
    let cursor = info.and_then(|v| v.get("endCursor")).and_then(Value::as_str).map(str::to_owned);
    Ok((rows, more, cursor))
}

/// Every issue and PR in `repo` updated since `started` (minus the
/// overlap), one search page at a time up to the row cap.
pub async fn changed_rows(
    repo: &str,
    started: chrono::DateTime<chrono::Utc>,
    bearer: &str,
) -> Result<Vec<Row>, String> {
    let url = graphql_url();
    let client = client();
    let query = delta_query();
    let q = delta_search_query(repo, started);
    let mut after: Option<String> = None;
    let mut rows = Vec::new();
    loop {
        let body = json!({"query": query, "variables": {"q": q, "after": after}});
        let value = post_json(&client, &url, bearer, &body).await?;
        let (page, more, cursor) = parse_delta_page(&value)?;
        rows.extend(page);
        if !more || cursor.is_none() || rows.len() >= MAX_SECTION_ROWS { break; }
        after = cursor;
    }
    Ok(rows)
}

/// Sidebar list a row belongs to; `None` for closed, unmerged PRs, which
/// no list shows.
fn listed_section(row: &Row) -> Option<usize> {
    match (row.issue.is_pr, row.issue.state.as_str()) {
        (true, "OPEN") => Some(0),
        (true, "MERGED") => Some(1),
        (false, "OPEN") => Some(2),
        (false, "CLOSED") => Some(3),
        _ => None,
    }
}

/// True when a shown list holds `row` and its window anchor, if any, is
/// inside the window.
fn shown(row: &Row, now: chrono::DateTime<chrono::Utc>, hours: u64, show: [bool; 4]) -> bool {
    listed_section(row).is_some_and(|i| show[i])
        && row.window_at.as_deref().is_none_or(|at| within_window(Some(at), now, hours))
}

/// Sidebar view of stored rows: the shown ones, grouped by list, each
/// list in its server order (`list_order`), up to the row cap.
pub fn visible_rows(rows: Vec<Row>, now: chrono::DateTime<chrono::Utc>, hours: u64, show: [bool; 4]) -> Vec<Row> {
    let mut rows: Vec<Row> = rows.into_iter().filter(|r| shown(r, now, hours, show)).collect();
    rows.sort_by(|a, b| list_order(b).cmp(&list_order(a)));
    merge_rows(Vec::new(), rows, now, hours)
}

/// Sort key, descending, that matches each list's server order from row
/// data alone: open rows by creation, merged PRs by merge time (their
/// window anchor), closed issues by last update; ties by number. GitHub
/// timestamps share one RFC 3339 form, so text order is time order.
fn list_order(row: &Row) -> (&str, u64) {
    let issue = &row.issue;
    let at = match (issue.is_pr, issue.state.as_str()) {
        (true, "MERGED") => row.window_at.as_deref().unwrap_or(&issue.updated_at),
        (_, "CLOSED") => &issue.updated_at,
        _ => &issue.created_at,
    };
    (at, issue.number)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc)
    }
    fn status_row(number: u64, is_pr: bool, state: &str) -> Row {
        let mut r = parse_node(&json!({"number": number, "title": "t", "state": state, "createdAt": "2026-11-01T00:00:00Z", "updatedAt": "2026-11-02T00:00:00Z", "url": "u"}), is_pr, None).unwrap();
        r.issue.is_pr = is_pr;
        r
    }

    #[test]
    fn status_queries_alias_and_chunk_at_fifty() {
        let targets: Vec<(u64, bool)> = (1..=120).map(|n| (n, n % 2 == 0)).collect();
        let qs = status_queries(&targets);
        assert_eq!(qs.len(), 3);
        assert!(qs[0].contains(" p2: pullRequest(number: 2)") && qs[0].contains(" i1: issue(number: 1)"));
        assert!(qs[0].contains("statusCheckRollup") && qs[0].contains(RATE_LIMIT_FIELD));
        assert!(qs[2].contains("i101:") && !qs[2].contains("i100:"), "chunks split 50/50/20");
        let q = status_queries(&[(5, false)]).remove(0);
        assert!(!q.contains("statusCheckRollup"), "issues ask project status only");
        assert!(q.contains("projectItems"));
    }

    #[test]
    fn status_targets_take_open_rows_only() {
        let rows = vec![status_row(1, true, "OPEN"), status_row(2, true, "MERGED"), status_row(3, false, "OPEN"), status_row(4, false, "CLOSED")];
        assert_eq!(status_targets(&rows), vec![(1, true), (3, false)]);
    }

    #[test]
    fn status_updates_touch_only_ci_and_project_status() {
        let mut rows = vec![status_row(1, true, "OPEN"), status_row(3, false, "OPEN")];
        rows[0].issue.title = "keep".into();
        let value = json!({"data": {"repository": {
            "p1": {"commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]},
                   "projectItems": {"nodes": [{"project": {"title": "Board"}, "status": {"name": "Done"}}]}},
            "i3": {"projectItems": {"nodes": [{"project": {"title": "Board"}, "status": {"name": "Todo"}}]}},
            "p99": {"projectItems": {"nodes": []}}}}});
        let updates = parse_status_page(&value, &[(1, true), (3, false), (99, true), (7, true)]).unwrap();
        assert_eq!(updates.len(), 3, "a missing alias is skipped");
        apply_statuses(&mut rows, updates);
        assert_eq!(rows[0].issue.checks, Some(crate::model::CheckState::Fail));
        assert_eq!(rows[0].statuses[0].1.as_deref(), Some("Done"));
        assert_eq!(rows[0].issue.title, "keep");
        assert_eq!(rows[0].issue.updated_at, "2026-11-02T00:00:00Z");
        assert_eq!(rows[1].statuses[0].1.as_deref(), Some("Todo"));
        assert_eq!(rows[1].issue.checks, None);
        assert_eq!(rows.len(), 2, "unknown numbers add no rows");
    }

    #[test]
    fn subset_query_omits_hidden_aliases_and_variables() {
        let q = query([true, false, true, false]);
        assert!(q.contains("open: pullRequests") && q.contains("openIssues: issues"));
        assert!(q.contains("$openAfter") && q.contains("$openIssuesAfter"));
        for gone in ["merged: pullRequests", "closedIssues: issues", "$mergedAfter", "$closedIssuesAfter"] {
            assert!(!q.contains(gone), "hidden section leaked: {gone}");
        }
        assert!(q.contains("$owner: String!, $repo: String!, $openAfter: String, $openIssuesAfter: String)"));
    }
    #[test]
    fn parse_rows_tolerates_missing_aliases() {
        let v = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"one","state":"OPEN"}]}}}});
        let rows = parse_rows(&[v], test_now(), 24).unwrap();
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1]);
    }
    #[test]
    fn merged_and_closed_stop_at_the_window_cutoff_while_open_keeps_paging() {
        let mut page = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"updatedAt":"2026-01-01T00:00:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"o1"}},
            "merged":{"nodes":[{"number":2,"updatedAt":"2026-11-10T11:30:00Z"},{"number":3,"updatedAt":"2026-11-09T10:00:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"m1"}},
            "openIssues":{"nodes":[],"pageInfo":{"hasNextPage":true,"endCursor":"i1"}},
            "closedIssues":{"nodes":[{"number":4,"updatedAt":"2026-11-10T11:30:00Z"},{"number":5,"updatedAt":"2026-11-09T10:00:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"c1"}}}}});
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        absorb_page(&mut page, &mut live, &mut cursors, &mut count, None, test_now(), 1);
        assert_eq!(live, [true, false, true, false], "past-window merged and closed retire");
        assert_eq!(cursors[0].as_deref(), Some("o1"));
        assert_eq!(cursors[2].as_deref(), Some("i1"));
        assert!(cursors[1].is_none() && cursors[3].is_none());
    }
    #[test]
    fn in_window_last_node_and_all_time_keep_merged_paging() {
        let page = || json!({"data":{"repository":{
            "open":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "merged":{"nodes":[{"number":2,"updatedAt":"2026-11-10T11:30:00Z"}],"pageInfo":{"hasNextPage":true,"endCursor":"m1"}},
            "openIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}},
            "closedIssues":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}});
        let old = || { let mut p = page(); p["data"]["repository"]["merged"]["nodes"][0]["updatedAt"] = json!("2020-01-01T00:00:00Z"); p };
        let run = |mut p: Value, hours: u64| {
            let mut live = [true; 4];
            let mut cursors: [Option<String>; 4] = [None, None, None, None];
            let mut count = [0usize; 4];
            absorb_page(&mut p, &mut live, &mut cursors, &mut count, None, test_now(), hours);
            live[1]
        };
        assert!(run(page(), 1), "last node inside the window keeps paging");
        assert!(run(old(), 0), "hours 0 never stops early");
        assert!(!run(old(), 1));
    }
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query([true; 4]);
        for field in ["open: pullRequests", "merged: pullRequests", "openIssues: issues", "closedIssues: issues", "projectItems", "totalCount", "$owner", "$repo", "pageInfo", "hasNextPage", "endCursor", "$openAfter", "$mergedAfter", "$openIssuesAfter", "$closedIssuesAfter", "commits(last: 1)", "statusCheckRollup", "parent { number }", "subIssuesSummary { total completed }"] { assert!(q.contains(field)); }
    }
    #[test]
    fn sections_query_stays_slim_and_rows_stay_partial() {
        // Startup fetches list-weight rows only: bodies and per-PR review
        // threads load when a row opens through the full fetch.
        let q = query([true; 4]);
        assert!(q.contains("labels(first: 20)") && q.contains("assignees(first: 10)"));
        assert!(!q.contains("labels(first: 100)") && !q.contains("assignees(first: 100)"));
        assert!(q.contains("rateLimit { cost remaining resetAt }"));
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
    fn parses_parent_and_sub_issue_summary_on_issue_nodes() {
        let v = json!({"data":{"repository":{
            "open":{"nodes":[{"number":1,"title":"pr","state":"OPEN"}]},
            "merged":{"nodes":[]},
            "openIssues":{"nodes":[
                {"number":7,"title":"child","state":"OPEN",
                 "parent":{"number":3},"subIssuesSummary":{"total":4,"completed":1}},
                {"number":8,"title":"plain","state":"OPEN"}]},
            "closedIssues":{"nodes":[]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 24).unwrap();
        let row = |n: u64| rows.iter().find(|r| r.issue.number == n).unwrap();
        assert_eq!(row(7).issue.parent, Some(3));
        assert_eq!(row(7).issue.sub_summary, Some(SubSummary { total: 4, completed: 1 }));
        assert_eq!(row(8).issue.parent, None, "a top-level issue has no parent");
        assert_eq!(row(8).issue.sub_summary, None, "a leaf issue has no sub-issues");
        assert_eq!(row(1).issue.parent, None, "PR rows carry no parent");
        assert_eq!(row(1).issue.sub_summary, None, "PR rows carry no sub-issue summary");
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
        absorb_page(&mut page1, &mut live, &mut cursors, &mut count, None, test_now(), 0);
        assert!(live[0] && !live[1], "open stays live, merged retires on page 1");
        pages.push(page1);
        absorb_page(&mut page2, &mut live, &mut cursors, &mut count, None, test_now(), 0);
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
    #[test]
    fn visible_rows_order_each_list_from_row_dates() {
        let now = test_now();
        let dated = |n: u64, is_pr: bool, state: &str, created: &str, updated: &str, merged: Option<&str>| {
            let mut row = section_row(n, "", state, is_pr, merged.or((state == "CLOSED").then_some(updated)));
            row.issue.created_at = created.to_owned();
            row.issue.updated_at = updated.to_owned();
            row
        };
        let rows = vec![
            // Open: newest created first, whatever the update time.
            dated(1, true, "OPEN", "2026-11-10T01:00:00Z", "2026-11-10T11:00:00Z", None),
            dated(2, true, "OPEN", "2026-11-10T05:00:00Z", "2026-11-10T05:00:00Z", None),
            // Merged: newest merged first; a later update does not lift it.
            dated(3, true, "MERGED", "2026-11-01T00:00:00Z", "2026-11-10T11:30:00Z", Some("2026-11-10T02:00:00Z")),
            dated(4, true, "MERGED", "2026-11-01T00:00:00Z", "2026-11-10T04:00:00Z", Some("2026-11-10T04:00:00Z")),
            // Open issues tie on creation: higher number first.
            dated(5, false, "OPEN", "2026-11-10T03:00:00Z", "2026-11-10T03:00:00Z", None),
            dated(6, false, "OPEN", "2026-11-10T03:00:00Z", "2026-11-10T03:00:00Z", None),
            // Closed issues: newest updated first.
            dated(7, false, "CLOSED", "2026-11-10T08:00:00Z", "2026-11-10T03:00:00Z", None),
            dated(8, false, "CLOSED", "2026-11-01T00:00:00Z", "2026-11-10T06:00:00Z", None),
        ];
        let order: Vec<u64> = visible_rows(rows, now, 24, [true; 4]).iter().map(|r| r.issue.number).collect();
        assert_eq!(order, vec![2, 1, 4, 3, 6, 5, 8, 7]);
    }
    fn section_row(number: u64, title: &str, state: &str, is_pr: bool, window_at: Option<&str>) -> Row {
        Row {
            issue: IssueData {
                number, title: title.to_owned(), state: state.to_owned(),
                author: Author::default(), labels: vec![], assignees: vec![],
                created_at: String::new(), updated_at: String::new(),
                url: String::new(), body: String::new(), comments: vec![],
                is_pr, review_threads: vec![], paired: vec![], checks: None, milestone: None,
                parent: None, sub_summary: None,
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
        absorb_page(&mut page, &mut live, &mut cursors, &mut count, Some(&cached), test_now(), 0);
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
        absorb_page(&mut page, &mut live, &mut cursors, &mut count, Some(&cached), test_now(), 0);
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
            absorb_page(&mut value, &mut live, &mut cursors, &mut count, Some(&cached), test_now(), 0);
        }
        assert!(!live[0], "no overlap: MAX_SECTION_PAGES retires the section");
        assert_eq!(count[0], MAX_SECTION_PAGES);

        // Cache miss: same synthetic pages, but the walk keeps today's shape.
        let mut live = [true; 4];
        let mut cursors: [Option<String>; 4] = [None, None, None, None];
        let mut count = [0usize; 4];
        for page in 1..=MAX_SECTION_PAGES + 1 {
            let mut value = open_page(&[100 + page as u64], true, Some("c"));
            absorb_page(&mut value, &mut live, &mut cursors, &mut count, None, test_now(), 0);
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
        let mut row = section_row(7, "cached", "MERGED", true, Some("2026-11-10T11:30:00Z"));
        row.issue.parent = Some(3);
        row.issue.sub_summary = Some(SubSummary { total: 2, completed: 1 });
        let raw = serde_json::to_string(&row).unwrap();
        let back: Row = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.issue.number, 7);
        assert_eq!(back.issue.title, "cached");
        assert_eq!(back.issue.parent, Some(3));
        assert_eq!(back.issue.sub_summary, Some(SubSummary { total: 2, completed: 1 }));
        assert_eq!(back.window_at.as_deref(), Some("2026-11-10T11:30:00Z"));
    }
    #[test]
    fn old_cached_rows_without_sub_issue_fields_still_parse() {
        let old = json!({
            "issue": {"number": 7, "title": "cached", "state": "OPEN"},
            "statuses": [],
            "detail": {},
            "complete": false,
        });
        let row: Row = serde_json::from_value(old).unwrap();
        assert_eq!(row.issue.parent, None);
        assert_eq!(row.issue.sub_summary, None);
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
    #[test]
    fn delta_search_query_names_repo_and_reaches_back_two_minutes() {
        let started = test_now();
        let q = delta_search_query("o/r", started);
        assert_eq!(q, "repo:o/r updated:>=2026-11-10T11:58:00Z sort:updated-desc");
        let gql = delta_query();
        for field in ["search(type: ISSUE, query: $q, first: 50, after: $after)", "... on Issue {",
            "... on PullRequest {", "__typename", "pageInfo { hasNextPage endCursor }",
            "subIssuesSummary", "mergedAt", "statusCheckRollup", RATE_LIMIT_FIELD] {
            assert!(gql.contains(field), "delta query missing: {field}");
        }
    }
    #[test]
    fn delta_or_full_follows_cache_age_and_timestamp() {
        let now = test_now();
        let at = |s: &str| delta_since(Some(s), now, false);
        assert_eq!(at("2026-11-10T11:30:00Z"), Some(now - chrono::TimeDelta::minutes(30)));
        assert!(at("2026-11-10T11:00:01Z").is_some(), "just under an hour old");
        assert_eq!(at("2026-11-10T11:00:00Z"), None, "an hour old takes the full walk");
        assert_eq!(at("2026-11-10T13:00:00Z"), None, "a future timestamp is not trusted");
        assert_eq!(at("garbage"), None, "a bad timestamp takes the full walk");
        assert_eq!(delta_since(None, now, false), None, "no timestamp takes the full walk");
        assert_eq!(delta_since(Some("2026-11-10T11:30:00Z"), now, true), None, "forced full");
    }
    #[test]
    fn delta_page_parses_issues_and_prs_into_section_rows() {
        let v = json!({"data": {"search": {
            "nodes": [
                {"__typename": "PullRequest", "number": 1, "title": "pr", "state": "MERGED",
                 "mergedAt": "2026-11-10T11:00:00Z", "updatedAt": "2026-11-10T11:05:00Z",
                 "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "SUCCESS"}}}]}},
                {"__typename": "Issue", "number": 2, "title": "bug", "state": "CLOSED",
                 "updatedAt": "2026-11-10T11:10:00Z", "parent": {"number": 9},
                 "labels": {"nodes": [{"name": "bug"}]}},
                {"__typename": "Issue", "number": 3, "title": "open", "state": "OPEN",
                 "updatedAt": "2026-11-10T11:20:00Z"},
                {"__typename": "PullRequest", "number": 4, "title": "draft", "state": "OPEN"},
                {}],
            "pageInfo": {"hasNextPage": true, "endCursor": "c1"}}}});
        let (rows, more, cursor) = parse_delta_page(&v).unwrap();
        assert!(more);
        assert_eq!(cursor.as_deref(), Some("c1"));
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        let row = |n: u64| rows.iter().find(|r| r.issue.number == n).unwrap();
        assert!(row(1).issue.is_pr && !row(2).issue.is_pr && row(4).issue.is_pr);
        assert_eq!(row(1).window_at.as_deref(), Some("2026-11-10T11:00:00Z"), "merged anchors on mergedAt");
        assert_eq!(row(2).window_at.as_deref(), Some("2026-11-10T11:10:00Z"), "closed anchors on updatedAt");
        assert_eq!(row(3).window_at, None);
        assert_eq!(row(1).issue.checks, Some(crate::model::CheckState::Pass));
        assert_eq!(row(2).issue.parent, Some(9));
        assert_eq!(row(2).issue.labels[0].name, "bug");
        assert!(parse_delta_page(&json!({"errors": [{"message": "boom"}]})).is_err());
    }
}
