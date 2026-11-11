use crate::model::{Author, BoardDetail, DiffFile, IssueData, Label};
use serde_json::{json, Value};
use std::sync::OnceLock;

type Statuses = Vec<(String, Option<String>)>;
#[derive(Debug)]
pub struct Row {
    pub issue: IssueData,
    pub statuses: Statuses,
    pub detail: BoardDetail,
    pub complete: bool,
}

pub fn token() -> Option<&'static str> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN.get_or_init(|| {
        for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
            if let Ok(t) = std::env::var(var) {
                let t = t.trim().to_owned();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
        let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
        if !out.status.success() { return None; }
        let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
        (!token.is_empty()).then_some(token)
    }).as_deref()
}

const PR_FIELDS: &str = r#"
 number title state author { login } labels(first: 100) { nodes { name } }
 assignees(first: 100) { nodes { login } } createdAt updatedAt url body
 comments(first: 1) { totalCount } mergedAt
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

const ISSUE_FIELDS: &str = r#"
 number title state author { login } labels(first: 100) { nodes { name } }
 assignees(first: 100) { nodes { login } } createdAt updatedAt url body
 comments(first: 1) { totalCount }
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

/// Total-row ceiling across the four section lists: one giant repo must not
/// blow up the sidebar (or memory) no matter how many pages it fills.
/// Fetching stops once the raw nodes reach this many, and `parse_rows`
/// truncates the merged rows here too.
const MAX_SECTION_ROWS: usize = 500;

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
    s.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|d| d.with_timezone(&chrono::Utc) >= now - chrono::Duration::hours(hours as i64))
}

fn parse_rows(pages: &[Value], now: chrono::DateTime<chrono::Utc>, hours: u64) -> Result<Vec<Row>, String> {
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
                if windowed && !within_window(n.get(date_key).and_then(Value::as_str), now, hours) { continue; }
            let string = |k: &str| n.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let number = n.get("number").and_then(Value::as_u64).ok_or("missing row number")?;
            let labels = n.pointer("/labels/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("name")?.as_str().map(|s| Label { name: s.to_owned() })).collect();
            let assignees = n.pointer("/assignees/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("login")?.as_str().map(|s| Author { login: s.to_owned() })).collect();
            let statuses = n.pointer("/projectItems/nodes").and_then(Value::as_array).into_iter().flatten().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
            let created = string("createdAt");
            let updated = string("updatedAt");
            let detail = BoardDetail { created: created.get(..10).unwrap_or("").to_owned(), updated: updated.get(..10).unwrap_or("").to_owned(), comments: n.pointer("/comments/totalCount").and_then(Value::as_u64).unwrap_or(0) as usize };
            rows.push(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: string("body"), comments: vec![], is_pr }, statuses, detail, complete: false });
            }
        }
    }
    Ok(rows)
}

/// Shared HTTP client: one connection pool for all background fetches.
/// `Client::clone` is cheap; every task shares the same pool.
fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("ghit")
                .build()
                .expect("reqwest client")
        })
        .clone()
}

async fn get(client: &reqwest::Client, url: &str, bearer: &str) -> Result<Value, String> {
    client
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
}

async fn post_json(client: &reqwest::Client, url: &str, bearer: &str, body: &Value) -> Result<Value, String> {
    client
        .post(url)
        .bearer_auth(bearer)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
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
fn absorb_page(value: &mut Value, live: &mut [bool; 4], cursors: &mut [Option<String>; 4]) -> usize {
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
        fetched += value.pointer(&format!("/data/repository/{section}/nodes")).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        let (more, cursor) = section_cursor(value, section);
        if more && cursor.is_some() { cursors[i] = cursor; } else { live[i] = false; cursors[i] = None; }
    }
    fetched
}

pub async fn sections(repo: &str, hours: u64, bearer: &str) -> Result<Vec<Row>, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    let client = client();
    // One cursor per section, same order as `parse_rows` lists them: open,
    // merged, openIssues, closedIssues. Same page-loop shape as `pull_files`:
    // fetch, append, stop a section when its page has no next cursor — plus
    // a stop for every section once the raw nodes reach the row cap.
    let mut cursors: [Option<String>; 4] = [None, None, None, None];
    let mut live = [true; 4];
    let mut pages = Vec::new();
    let mut fetched: usize = 0;
    loop {
        let mut value = post_json(&client, &url, bearer, &json!({"query": query(), "variables": {"owner": owner, "repo": name, "openAfter": cursors[0], "mergedAfter": cursors[1], "openIssuesAfter": cursors[2], "closedIssuesAfter": cursors[3]}})).await?;
        fetched += absorb_page(&mut value, &mut live, &mut cursors);
        pages.push(value);
        if live.iter().all(|l| !l) || fetched >= MAX_SECTION_ROWS { break; }
    }
    parse_rows(&pages, chrono::Utc::now(), hours)
}

/// One GitHub issue-search hit as a sidebar row. Search items carry no
/// project or merged info, so statuses stay empty and the row is partial:
/// opening it runs the full fetch.
pub fn search_row(v: &Value) -> Result<Row, String> {
    let number = v.get("number").and_then(Value::as_u64).ok_or("search hit without a number")?;
    let string = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let is_pr = v.get("pull_request").is_some_and(|p| !p.is_null());
    let labels = v.get("labels").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|l| l.get("name")?.as_str().map(|s| Label { name: s.to_owned() })).collect();
    let assignees = v.get("assignees").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|a| a.get("login")?.as_str().map(|s| Author { login: s.to_owned() })).collect();
    let created = string("created_at");
    let updated = string("updated_at");
    let mut state = string("state").to_uppercase();
    if state.is_empty() {
        state = "OPEN".to_string();
    }
    let detail = BoardDetail {
        created: created.get(..10).unwrap_or("").to_owned(),
        updated: updated.get(..10).unwrap_or("").to_owned(),
        comments: v.get("comments").and_then(Value::as_u64).unwrap_or(0) as usize,
    };
    Ok(Row {
        issue: IssueData {
            number,
            title: string("title"),
            state,
            author: Author { login: v.pointer("/user/login").and_then(Value::as_str).unwrap_or("").to_owned() },
            labels,
            assignees,
            created_at: created,
            updated_at: updated,
            url: string("html_url"),
            body: string("body"),
            comments: vec![],
            is_pr,
        },
        statuses: vec![],
        detail,
        complete: false,
    })
}

/// Server-side issue/PR search: `repo:{repo} {query}` through the REST
/// search API, relevance-ranked, issues and PRs mixed.
pub async fn search(repo: &str, query: &str, bearer: &str) -> Result<Vec<Row>, String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let value: Value = client()
        .get(format!("{base}/search/issues"))
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .query(&[("q", format!("repo:{repo} {query}")), ("per_page", "50".to_string())])
        .send()
        .await
        .map_err(|e| format!("GitHub search failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub search failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("bad search response: {e}"))?;
    value
        .get("items")
        .and_then(Value::as_array)
        .ok_or("bad search payload")?
        .iter()
        .map(search_row)
        .collect()
}

/// One entry of the REST `pulls/{n}/files` listing. `patch` is absent
/// for binary or oversized files; those render as header-only rows.
pub fn diff_file(v: &Value) -> Result<DiffFile, String> {
    Ok(DiffFile {
        filename: v.get("filename").and_then(Value::as_str).ok_or("diff entry without a filename")?.to_owned(),
        status: v.get("status").and_then(Value::as_str).unwrap_or("").to_owned(),
        additions: v.get("additions").and_then(Value::as_u64).unwrap_or(0) as usize,
        deletions: v.get("deletions").and_then(Value::as_u64).unwrap_or(0) as usize,
        patch: v.get("patch").and_then(Value::as_str).unwrap_or("").to_owned(),
    })
}

/// Changed files for one PR, oldest first, across all result pages.
pub async fn pull_files(repo: &str, number: u64, bearer: &str) -> Result<Vec<DiffFile>, String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let client = client();
    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let items = get(&client, &format!("{base}/repos/{repo}/pulls/{number}/files?per_page=100&page={page}"), bearer).await?;
        let items = items.as_array().ok_or("bad diff payload")?;
        for v in items {
            out.push(diff_file(v)?);
        }
        if items.len() < 100 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

pub async fn issue(repo: &str, number: u64, bearer: &str) -> Result<(IssueData, Statuses), String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let client = client();
    let mut v = get(&client, &format!("{base}/repos/{repo}/issues/{number}"), bearer).await?;
    let is_pr = !v["pull_request"].is_null();
    v["author"] = v["user"].clone();
    for (old, new) in [("created_at", "createdAt"), ("updated_at", "updatedAt"), ("html_url", "url")] { v[new] = v[old].clone(); }
    if v["author"].is_null() { v["author"] = json!({}); }
    if v["body"].is_null() { v["body"] = json!(""); }
    if is_pr && !v["pull_request"]["merged_at"].is_null() { v["state"] = json!("MERGED"); }
    else if let Some(state) = v["state"].as_str() { v["state"] = json!(state.to_uppercase()); }
    let mut comments = Vec::new();
    let mut page = 1;
    loop {
        let items = get(&client, &format!("{base}/repos/{repo}/issues/{number}/comments?per_page=100&page={page}"), bearer).await?;
        let items = items.as_array().ok_or("bad comments payload")?;
        for c in items { comments.push(json!({"author": c.get("user").filter(|u| !u.is_null()).cloned().unwrap_or(json!({})), "body": c["body"].as_str().unwrap_or(""), "createdAt": c["created_at"]})); }
        if items.len() < 100 { break; }
        page += 1;
    }
    v["comments"] = json!(comments);
    v["isPr"] = json!(is_pr);
    let data = serde_json::from_value(v).map_err(|e| format!("bad issue payload: {e}"))?;
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let field = if is_pr { "pullRequest" } else { "issue" };
    let query = format!(r#"query($owner:String!, $repo:String!, $num:Int!) {{ repository(owner:$owner,name:$repo) {{ {field}(number:$num) {{ projectItems(first:10) {{ nodes {{ project {{ title }} status: fieldValueByName(name:"Status") {{ ... on ProjectV2ItemFieldSingleSelectValue {{ name }} }} }} }} }} }} }}"#);
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    // Project statuses stay best-effort: a failed lookup yields none,
    // same as before, instead of failing the whole issue fetch.
    let statuses = post_json(&client, &url, bearer, &json!({"query":query,"variables":{"owner":owner,"repo":name,"num":number}})).await.ok()
        .and_then(|v| v.pointer(&format!("/data/repository/{field}/projectItems/nodes")).and_then(Value::as_array).cloned()).unwrap_or_default()
        .iter().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
    Ok((data, statuses))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query();
        for field in ["open: pullRequests", "merged: pullRequests", "openIssues: issues", "closedIssues: issues", "projectItems", "totalCount", "$owner", "$repo", "pageInfo", "hasNextPage", "endCursor", "$openAfter", "$mergedAfter", "$openIssuesAfter", "$closedIssuesAfter"] { assert!(q.contains(field)); }
    }
    #[test]
    fn parses_rows_and_filters_merged_window() {
        let v = json!({"data":{"repository":{"open":{"nodes":[{"number":1,"title":"one","state":"OPEN","labels":{"nodes":[{"name":"bug"}]},"comments":{"totalCount":3},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]}}]},"merged":{"nodes":[{"number":2,"mergedAt":"2026-11-10T11:30:00Z"},{"number":3,"mergedAt":"2026-11-09T10:00:00Z"}]},"openIssues":{"nodes":[{"number":7,"title":"seven","state":"OPEN"}]},"closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"},{"number":9,"title":"nine","state":"CLOSED","updatedAt":"2026-11-09T10:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&[v], now, 1).unwrap();
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
        let mut pages = Vec::new();
        absorb_page(&mut page1, &mut live, &mut cursors);
        assert!(live[0] && !live[1], "open stays live, merged retires on page 1");
        pages.push(page1);
        absorb_page(&mut page2, &mut live, &mut cursors);
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
    #[test]
    fn search_row_marks_prs_and_stays_partial() {
        let pr = json!({"number":12,"title":"a pr","state":"open","user":{"login":"bo"},"labels":[{"name":"bug"}],"assignees":[],"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-05T00:00:00Z","html_url":"https://github.com/o/r/pull/12","body":"hi","comments":4,"pull_request":{"url":"https://api.github.com/x"}});
        let row = search_row(&pr).unwrap();
        assert!(row.issue.is_pr);
        assert_eq!(row.issue.state, "OPEN");
        assert!(!row.complete);
        assert_eq!(row.detail.comments, 4);
        let issue = json!({"number":13,"title":"an issue","state":"closed","user":{"login":"bo"},"labels":[],"assignees":[],"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-05T00:00:00Z","html_url":"https://github.com/o/r/issues/13","body":"","comments":0});
        let row = search_row(&issue).unwrap();
        assert!(!row.issue.is_pr);
        assert_eq!(row.issue.state, "CLOSED");
        assert!(search_row(&json!({"title":"no number"})).is_err());
    }
    #[test]
    fn diff_file_maps_patch_and_binary() {
        let f = diff_file(&json!({"filename":"src/a.rs","status":"modified","additions":3,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+new"})).unwrap();
        assert_eq!(f.filename, "src/a.rs");
        assert_eq!((f.additions, f.deletions), (3, 1));
        assert!(f.patch.contains("@@"));
        let bin = diff_file(&json!({"filename":"img.png","status":"added","additions":0,"deletions":0})).unwrap();
        assert_eq!(bin.patch, "");
        assert!(diff_file(&json!({"status":"modified"})).is_err());
    }
}
