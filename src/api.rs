use crate::model::{
    self, Author, BoardDetail, Comment, DiffFile, GraphData, GraphEdgeKind, GraphLinkedPr,
    GraphNode, IssueData, Label,
};
use serde_json::{json, Value};
use std::collections::HashSet;
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
 reviewThreads(first: 50) { nodes { id path line isResolved comments(first: 20) { nodes { id author { login } body } } } }
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
            rows.push(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: string("body"), comments: vec![], review_threads: crate::model::parse_review_threads(n), paired: vec![], is_pr }, statuses, detail, complete: false });
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

pub async fn sections(
    repo: &str,
    hours: u64,
    bearer: &str,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<Vec<Row>, String> {
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
    parse_rows(&pages, chrono::Utc::now(), hours)
}

/// Fetch details for the actual board cards, including cards outside the sections window.
pub async fn board_details(cards: &[(String, u64)], bearer: &str) -> Result<Vec<Option<BoardDetail>>, String> {
    let mut fields = String::new();
    for (i, (repo, number)) in cards.iter().enumerate() {
        let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
        fields.push_str(&format!("c{i}: repository(owner: {}, name: {}) {{ issueOrPullRequest(number: {number}) {{ ... on Issue {{ createdAt updatedAt comments {{ totalCount }} }} ... on PullRequest {{ createdAt updatedAt comments {{ totalCount }} }} }} }} ", json!(owner), json!(name)));
    }
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    let value = post_json(&client(), &url, bearer, &json!({"query": format!("query {{ {fields} }}")})).await?;
    parse_board_details(&value, cards.len())
}

fn parse_board_details(value: &Value, count: usize) -> Result<Vec<Option<BoardDetail>>, String> {
    let data = value.get("data").filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    Ok((0..count).map(|i| {
        let node = data.pointer(&format!("/c{i}/issueOrPullRequest"))?;
        let created = node.get("createdAt")?.as_str()?.get(..10)?.to_owned();
        let updated = node.get("updatedAt")?.as_str()?.get(..10)?.to_owned();
        let comments = node.pointer("/comments/totalCount")?.as_u64()? as usize;
        Some(BoardDetail { created, updated, comments })
    }).collect())
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
            review_threads: vec![],
            paired: vec![],
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

/// Optional progress sink for multi-page fetches: called with
/// `(done, total)` as pages land. A shared `&dyn Fn` keeps the fetch
/// signatures free of channel types; the caller re-tags onto its own
/// channel. Single-page fetches never call it and stay indeterminate.
async fn fetch_comment_pages(
    client: &reqwest::Client,
    base: &str,
    repo: &str,
    number: u64,
    bearer: &str,
    count: u64,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<Vec<Comment>, String> {
    let mut pages = Vec::new();
    let total = count.div_ceil(100);
    // Bound the number of simultaneous requests for unusually large threads.
    for first in (1..=total).step_by(8) {
        let mut tasks = tokio::task::JoinSet::new();
        for page in first..=(first + 7).min(total) {
            let client = client.clone();
            let url = format!("{base}/repos/{repo}/issues/{number}/comments?per_page=100&page={page}");
            let bearer = bearer.to_owned();
            tasks.spawn(async move {
                let value = get(&client, &url, &bearer).await?;
                let items = value.as_array().ok_or("bad comments payload")?;
                let comments = items.iter().map(|c| Comment {
                    author: Author { login: c.pointer("/user/login").and_then(Value::as_str).unwrap_or("").to_owned() },
                    body: c["body"].as_str().unwrap_or("").to_owned(),
                    created_at: c["created_at"].as_str().unwrap_or("").to_owned(),
                }).collect();
                Ok::<_, String>((page as usize, comments))
            });
        }
        while let Some(result) = tasks.join_next().await {
            pages.push(result.map_err(|e| format!("comment fetch failed: {e}"))??);
        }
        // Multi-page threads only: a single page is one request and stays
        // indeterminate like every other single-request fetch.
        if total > 1 {
            if let Some(report) = progress {
                report(pages.len(), total as usize);
            }
        }
    }
    Ok(model::merge_comment_pages(pages))
}

pub async fn issue(
    repo: &str,
    number: u64,
    bearer: &str,
    cached_comments: Option<Vec<Comment>>,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<(IssueData, Statuses), String> {
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
    let comments = match cached_comments {
        Some(comments) => comments,
        None => fetch_comment_pages(&client, &base, repo, number, bearer, v["comments"].as_u64().ok_or("missing comment count")?, progress).await?,
    };
    v["comments"] = json!(comments);
    v["isPr"] = json!(is_pr);
    let mut data: IssueData = serde_json::from_value(v).map_err(|e| format!("bad issue payload: {e}"))?;
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let field = if is_pr { "pullRequest" } else { "issue" };
    let threads_field = if is_pr { "reviewThreads(first:50) { nodes { id path line isResolved comments(first:20) { nodes { id author { login } body } } } }" } else { "" };
    // Will-close counterparts: an issue lists its closing PRs, a PR lists
    // the issues it will close. Never body mentions.
    let paired_field = if is_pr {
        "closingIssuesReferences(first: 5) { nodes { number state } }"
    } else {
        "closedByPullRequestsReferences(first: 5, includeClosedPrs: true) { nodes { number state } }"
    };
    let query = format!(r#"query($owner:String!, $repo:String!, $num:Int!) {{ repository(owner:$owner,name:$repo) {{ {field}(number:$num) {{ {threads_field} {paired_field} projectItems(first:10) {{ nodes {{ project {{ title }} status: fieldValueByName(name:"Status") {{ ... on ProjectV2ItemFieldSingleSelectValue {{ name }} }} }} }} }} }} }}"#);
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    // Project statuses stay best-effort: a failed lookup yields none,
    // same as before, instead of failing the whole issue fetch.
    let response = post_json(&client, &url, bearer, &json!({"query":query,"variables":{"owner":owner,"repo":name,"num":number}})).await;
    let node = response.as_ref().ok().and_then(|v| v.pointer(&format!("/data/repository/{field}")));
    if let Some(node) = node {
        data.paired = crate::model::paired_refs(node, is_pr);
    }
    if let (true, Some(node)) = (is_pr, node) {
        data.review_threads = crate::model::parse_review_threads(node);
    }
    let statuses = node.and_then(|n| n.pointer("/projectItems/nodes")).and_then(Value::as_array).into_iter().flatten()
        .map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
    Ok((data, statuses))
}

/// Labels plus project status: valid on both Issues and PullRequests, so
/// every graph node shape reuses it.
const GRAPH_META: &str = r#"labels(first: 5) { nodes { name } }
 projectItems(first: 3) { nodes { status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }"#;

/// The dependency star around one issue. Blocker connections stay behind
/// `with_blockers` so a token or repo without those fields can retry
/// without them and derive reference edges from the body instead.
fn graph_query(with_blockers: bool) -> String {
    let mut q = String::from(
        "query($owner: String!, $repo: String!, $num: Int!) { repository(owner: $owner, name: $repo) { issue(number: $num) { number title state body ",
    );
    q.push_str(GRAPH_META);
    q.push_str(" parent { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } ");
    q.push_str("subIssues(first: 10) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
    if with_blockers {
        for conn in ["blockedBy", "blocking"] {
            q.push_str(&format!("{conn}(first: 10) {{ nodes {{ number title state "));
            q.push_str(GRAPH_META);
            q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
        }
    }
    q.push_str("trackedIssues(first: 5) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
    q.push_str("trackedInIssues(first: 5) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
    q.push_str("closedByPullRequestsReferences(first: 5, includeClosedPrs: true) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } totalCount } ");
    q.push_str("timelineItems(first: 20, itemTypes: [CROSS_REFERENCED_EVENT]) { nodes { __typename ... on CrossReferencedEvent { willCloseTarget source { __typename ... on Issue { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } ... on PullRequest { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } } } } } } } }");
    q
}

/// One aliased detail lookup per body-referenced number, for the fallback
/// path that resolves `#` refs without blocker fields.
fn graph_detail_query(owner: &str, name: &str, numbers: &[u64]) -> String {
    let mut fields = String::new();
    for (i, n) in numbers.iter().enumerate() {
        fields.push_str(&format!(
            "r{i}: repository(owner: {}, name: {}) {{ issueOrPullRequest(number: {n}) {{ __typename ... on Issue {{ number title state {GRAPH_META} }} ... on PullRequest {{ number title state {GRAPH_META} }} }} }} ",
            json!(owner),
            json!(name)
        ));
    }
    format!("query {{ {fields} }}")
}

fn parse_graph_labels(v: &Value) -> Vec<Label> {
    v.pointer("/labels/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| {
            l.get("name")?.as_str().map(|s| Label { name: s.to_owned() })
        })
        .collect()
}

fn parse_graph_status(v: &Value) -> Option<String> {
    v.pointer("/projectItems/nodes")
        .and_then(Value::as_array)?
        .first()?
        .pointer("/status/name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn parse_graph_prs(v: &Value) -> Vec<GraphLinkedPr> {
    v.pointer("/closedByPullRequestsReferences/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(GraphLinkedPr {
                number: p.get("number")?.as_u64()?,
                state: p.get("state").and_then(Value::as_str).unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// One issue-typed node: center, parent, sub-issues, blockers, tracked.
fn parse_graph_issue(v: &Value) -> Result<GraphNode, String> {
    let string = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(GraphNode {
        number: v.get("number").and_then(Value::as_u64).ok_or("graph node without a number")?,
        title: string("title"),
        state: string("state"),
        is_pr: false,
        labels: parse_graph_labels(v),
        project_status: parse_graph_status(v),
        linked_prs: parse_graph_prs(v),
    })
}

/// One polymorphic node: closing PRs and timeline sources carry
/// `__typename`; labeled the same, without nested closing references.
fn parse_graph_typed(v: &Value) -> Result<GraphNode, String> {
    let string = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(GraphNode {
        number: v.get("number").and_then(Value::as_u64).ok_or("graph node without a number")?,
        title: string("title"),
        state: string("state"),
        is_pr: v.get("__typename").and_then(Value::as_str).unwrap_or("") == "PullRequest",
        labels: parse_graph_labels(v),
        project_status: parse_graph_status(v),
        linked_prs: vec![],
    })
}

/// Neighbors in pane priority order: blockers, parent, children, closing
/// PRs, tracked, then timeline cross-references (`willCloseTarget` PRs
/// count as closing). First edge wins on duplicates via `build_graph`.
fn push_graph_conn(issue: &Value, ptr: &str, kind: GraphEdgeKind, out: &mut Vec<(GraphNode, GraphEdgeKind)>) {
    for n in issue.pointer(ptr).and_then(Value::as_array).into_iter().flatten() {
        if let Ok(node) = parse_graph_issue(n) {
            out.push((node, kind));
        }
    }
}

fn parse_graph_neighbors(issue: &Value) -> Vec<(GraphNode, GraphEdgeKind)> {
    let mut out: Vec<(GraphNode, GraphEdgeKind)> = vec![];
    push_graph_conn(issue, "/blocking/nodes", GraphEdgeKind::Blocks, &mut out);
    push_graph_conn(issue, "/blockedBy/nodes", GraphEdgeKind::BlockedBy, &mut out);
    if let Some(p) = issue.get("parent").filter(|v| v.is_object()) {
        if let Ok(node) = parse_graph_issue(p) {
            out.push((node, GraphEdgeKind::Parent));
        }
    }
    push_graph_conn(issue, "/subIssues/nodes", GraphEdgeKind::Child, &mut out);
    for n in issue
        .pointer("/closedByPullRequestsReferences/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Ok(node) = parse_graph_typed(n) {
            out.push((node, GraphEdgeKind::Closes));
        }
    }
    push_graph_conn(issue, "/trackedIssues/nodes", GraphEdgeKind::References, &mut out);
    push_graph_conn(issue, "/trackedInIssues/nodes", GraphEdgeKind::ReferencedBy, &mut out);
    for t in issue.pointer("/timelineItems/nodes").and_then(Value::as_array).into_iter().flatten() {
        let will_close = t.get("willCloseTarget").and_then(Value::as_bool).unwrap_or(false);
        let Some(src) = t.get("source").filter(|v| v.is_object()) else { continue };
        if let Ok(node) = parse_graph_typed(src) {
            let kind = if will_close && node.is_pr {
                GraphEdgeKind::Closes
            } else {
                GraphEdgeKind::ReferencedBy
            };
            out.push((node, kind));
        }
    }
    out
}

fn graph_issue(value: &Value) -> Result<&Value, String> {
    value
        .pointer("/data/repository/issue")
        .filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))
}

/// The full graph: center plus every neighbor, capped by `build_graph`.
fn parse_graph(value: &Value) -> Result<GraphData, String> {
    let issue = graph_issue(value)?;
    let center = parse_graph_issue(issue)?;
    Ok(model::build_graph(center, parse_graph_neighbors(issue)))
}

/// True when the fetch failed on the blocker connections (schema hides
/// them from some tokens): retry without them and derive references.
fn is_blocker_field_error(e: &str) -> bool {
    e.contains("blockedBy") || e.contains("blocking")
}

/// Fallback center plus neighbors from a blocker-free payload, with the
/// body-referenced numbers still to resolve. Pure so tests pin it without
/// the network.
fn parse_graph_fallback_parts(
    value: &Value,
    repo: &str,
) -> Result<(GraphNode, Vec<(GraphNode, GraphEdgeKind)>, Vec<u64>), String> {
    let issue = graph_issue(value)?;
    let center = parse_graph_issue(issue)?;
    let neighbors = parse_graph_neighbors(issue);
    let known: HashSet<u64> = neighbors
        .iter()
        .map(|(n, _)| n.number)
        .chain([center.number])
        .collect();
    let body = issue.get("body").and_then(Value::as_str).unwrap_or("");
    let room = model::MAX_GRAPH_NODES.saturating_sub(1 + neighbors.len());
    let missing: Vec<u64> = model::referenced_numbers(body, Some(repo))
        .into_iter()
        .filter(|n| !known.contains(n))
        .take(room)
        .collect();
    Ok((center, neighbors, missing))
}

/// Resolve fallback `#` refs to display nodes in one aliased query.
async fn resolve_graph_refs(
    client: &reqwest::Client,
    url: &str,
    bearer: &str,
    owner: &str,
    name: &str,
    numbers: &[u64],
) -> Result<Vec<GraphNode>, String> {
    if numbers.is_empty() {
        return Ok(vec![]);
    }
    let value = post_json(
        client,
        url,
        bearer,
        &json!({"query": graph_detail_query(owner, name, numbers), "variables": {}}),
    )
    .await?;
    let data = value.get("data").filter(|v| v.is_object()).ok_or_else(|| {
        format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null))
    })?;
    let mut out = vec![];
    for i in 0..numbers.len() {
        if let Some(raw) = data.pointer(&format!("/r{i}/issueOrPullRequest")).filter(|v| v.is_object()) {
            if let Ok(node) = parse_graph_typed(raw) {
                out.push(node);
            }
        }
    }
    Ok(out)
}

/// The dependency star around one issue: blockers, parent, sub-issues,
/// linked and closing PRs, cross-references. When the token or repo hides
/// the blocked-by/blocking fields the fetch retries without them and
/// derives reference edges from body `#` refs instead.
pub async fn graph(repo: &str, number: u64, bearer: &str) -> Result<GraphData, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" {
        "https://api.github.com/graphql".to_owned()
    } else {
        format!("https://{host}/api/graphql")
    };
    let client = client();
    let value = post_json(
        &client,
        &url,
        bearer,
        &json!({"query": graph_query(true), "variables": {"owner": owner, "repo": name, "num": number}}),
    )
    .await?;
    match parse_graph(&value) {
        Ok(data) => Ok(data),
        Err(e) if is_blocker_field_error(&e) => {
            let value = post_json(
                &client,
                &url,
                bearer,
                &json!({"query": graph_query(false), "variables": {"owner": owner, "repo": name, "num": number}}),
            )
            .await?;
            let (center, mut neighbors, missing) = parse_graph_fallback_parts(&value, repo)?;
            for node in resolve_graph_refs(&client, &url, bearer, owner, name, &missing).await? {
                neighbors.push((node, GraphEdgeKind::References));
            }
            let mut data = model::build_graph(center, neighbors);
            data.from_references = true;
            Ok(data)
        }
        Err(e) => Err(e),
    }
}

/// GraphQL endpoint honoring `GH_HOST`, shared by every mutation below.
fn graphql_url() -> String {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    if host == "github.com" {
        "https://api.github.com/graphql".to_owned()
    } else {
        format!("https://{host}/api/graphql")
    }
}

/// Unwrap a GraphQL response: transport errors already surfaced, so a
/// top-level `errors` array is the failure, and `data` the payload.
fn graphql_data(value: &Value) -> Result<&Value, String> {
    if let Some(errors) = value.get("errors") {
        return Err(format!("GitHub GraphQL: {errors}"));
    }
    value.get("data").filter(|v| v.is_object()).ok_or_else(|| format!("GitHub GraphQL: {value}"))
}

/// Post a plain issue/PR thread comment through REST. Validation errors
/// are strings for the status line, never panics.
pub async fn add_comment(repo: &str, number: u64, body: &str, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    post_json(&client(), &format!("{base}/repos/{repo}/issues/{number}/comments"), bearer, &json!({"body": body})).await?;
    Ok(())
}

/// Post a single-shot line comment on a PR through
/// `addPullRequestReviewThread`: no review flow, one thread per call.
/// Needs the PR's node ID, so it resolves `number` first, then mutates.
pub async fn add_review_thread(repo: &str, number: u64, body: &str, path: &str, line: u64, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    if path.is_empty() || line == 0 {
        return Err("line comment needs a file and a new-side line".to_string());
    }
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    let id_query = "query($owner:String!, $repo:String!, $num:Int!) { repository(owner:$owner,name:$repo) { pullRequest(number:$num) { id } } }";
    let id_value = post_json(&client, &url, bearer, &json!({"query": id_query, "variables": {"owner": owner, "repo": name, "num": number}})).await?;
    let pr_id = graphql_data(&id_value)?.pointer("/repository/pullRequest/id").and_then(Value::as_str).ok_or("PR not found: line comments need a pull request")?;
    let mutation = "mutation($prId:ID!, $path:String!, $line:Int!, $body:String!) { addPullRequestReviewThread(input:{pullRequestId:$prId, path:$path, line:$line, side:RIGHT, body:$body}) { thread { id } } }";
    let out = post_json(&client, &url, bearer, &json!({"query": mutation, "variables": {"prId": pr_id, "path": path, "line": line, "body": body}})).await?;
    graphql_data(&out)?.pointer("/addPullRequestReviewThread/thread/id").and_then(Value::as_str).ok_or("line comment failed: no thread returned")?;
    Ok(())
}

/// Reply inside an existing review thread. Wired end to end, but the
/// compose modal never targets it yet: picking the thread under the
/// cursor waits on #11, so callers surface its absence instead.
pub async fn add_thread_reply(thread_id: &str, body: &str, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    if thread_id.is_empty() {
        return Err("reply needs a thread: open the thread first".to_string());
    }
    let mutation = "mutation($threadId:ID!, $body:String!) { addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$threadId, body:$body}) { comment { id } } }";
    let out = post_json(&client(), &graphql_url(), bearer, &json!({"query": mutation, "variables": {"threadId": thread_id, "body": body}})).await?;
    graphql_data(&out)?.pointer("/addPullRequestReviewThreadReply/comment/id").and_then(Value::as_str).ok_or("reply failed: no comment returned")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_board_batch_and_marks_missing_cards_for_fallback() {
        let value = json!({"data": {"c0": {"issueOrPullRequest": {"createdAt": "2026-01-02T03:04:05Z", "updatedAt": "2026-02-03T04:05:06Z", "comments": {"totalCount": 7}}}, "c1": null, "c2": {"issueOrPullRequest": {"createdAt": "2026-01-02T03:04:05Z"}}}});
        let details = parse_board_details(&value, 3).unwrap();
        let detail = details[0].as_ref().unwrap();
        assert_eq!((&*detail.created, &*detail.updated, detail.comments), ("2026-01-02", "2026-02-03", 7));
        assert!(details[1].is_none());
        assert!(details[2].is_none());
        assert!(parse_board_details(&json!({"errors": [{"message": "denied"}]}), 1).is_err());
    }
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query();
        for field in ["open: pullRequests", "merged: pullRequests", "openIssues: issues", "closedIssues: issues", "projectItems", "totalCount", "$owner", "$repo", "pageInfo", "hasNextPage", "endCursor", "$openAfter", "$mergedAfter", "$openIssuesAfter", "$closedIssuesAfter"] { assert!(q.contains(field)); }
    }
    #[test]
    fn parses_rows_and_filters_merged_window() {
        let v = json!({"data":{"repository":{"open":{"nodes":[{"number":1,"title":"one","state":"OPEN","labels":{"nodes":[{"name":"bug"}]},"comments":{"totalCount":3},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]}}]},"merged":{"nodes":[{"number":2,"mergedAt":"2026-11-10T11:30:00Z"},{"number":3,"mergedAt":"2026-11-09T10:00:00Z"}]},"openIssues":{"nodes":[{"number":7,"title":"seven","state":"OPEN"}]},"closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"},{"number":9,"title":"nine","state":"CLOSED","updatedAt":"2026-11-09T10:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let mut v = v;
        v["data"]["repository"]["open"]["nodes"][0]["reviewThreads"] = json!({"nodes":[{"id":"thread-id","path":"src/a.rs","line":12,"isResolved":false,"comments":{"nodes":[{"id":"comment-id","author":{"login":"reviewer"},"body":"Please fix"}]}},{"id":"bad"}]});
        let rows = parse_rows(&[v], now, 1).unwrap();
        assert_eq!(rows[0].issue.review_threads.len(), 1);
        assert_eq!(rows[0].issue.review_threads[0].id, "thread-id");
        assert_eq!(rows[0].issue.review_threads[0].comments[0].id, "comment-id");
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
    fn graph_node(n: u64, title: &str) -> Value {
        json!({"number": n, "title": title, "state": "OPEN",
            "labels": {"nodes": [{"name": "bug"}]},
            "projectItems": {"nodes": [{"status": {"name": "Todo"}}]},
            "closedByPullRequestsReferences": {"nodes": [{"number": 44, "state": "MERGED"}]}})
    }

    fn graph_payload() -> Value {
        json!({"data": {"repository": {"issue": {
            "number": 7, "title": "center", "state": "OPEN", "body": "see #9",
            "labels": {"nodes": []}, "projectItems": {"nodes": []},
            "closedByPullRequestsReferences": {"nodes": []},
            "parent": {"number": 1, "title": "parent", "state": "CLOSED",
                "labels": {"nodes": []}, "projectItems": {"nodes": []},
                "closedByPullRequestsReferences": {"nodes": []}},
            "subIssues": {"nodes": [graph_node(8, "child")], "totalCount": 1},
            "blockedBy": {"nodes": [graph_node(9, "blocker")], "totalCount": 1},
            "blocking": {"nodes": [graph_node(10, "blocked")], "totalCount": 1},
            "trackedIssues": {"nodes": [], "totalCount": 0},
            "trackedInIssues": {"nodes": [], "totalCount": 0},
            "timelineItems": {"nodes": [
                {"__typename": "CrossReferencedEvent", "willCloseTarget": false,
                 "source": {"__typename": "Issue", "number": 11, "title": "ref",
                    "state": "OPEN", "labels": {"nodes": []}, "projectItems": {"nodes": []}}},
                {"__typename": "CrossReferencedEvent", "willCloseTarget": true,
                 "source": {"__typename": "PullRequest", "number": 12, "title": "fix",
                    "state": "OPEN", "labels": {"nodes": []}, "projectItems": {"nodes": []}}}
            ]}
        }}}})
    }

    #[test]
    fn graph_query_carries_every_relation() {
        let full = graph_query(true);
        for field in ["parent", "subIssues", "blockedBy", "blocking", "trackedIssues",
            "trackedInIssues", "closedByPullRequestsReferences", "timelineItems",
            "CROSS_REFERENCED_EVENT", "labels", "projectItems"]
        {
            assert!(full.contains(field), "missing {field}");
        }
        let slim = graph_query(false);
        assert!(!slim.contains("blockedBy"), "{slim}");
        assert!(!slim.contains("blocking"), "{slim}");
        assert!(slim.contains("subIssues"), "{slim}");
    }

    #[test]
    fn graph_queries_have_balanced_braces() {
        for q in [graph_query(true), graph_query(false), graph_detail_query("o", "r", &[1, 2])] {
            let (open, close) = (
                q.chars().filter(|&c| c == '{').count(),
                q.chars().filter(|&c| c == '}').count(),
            );
            assert_eq!(open, close, "unbalanced braces in graph query: {q}");
        }
    }

    #[test]
    fn parses_full_graph_with_labels_status_and_prs() {
        let data = parse_graph(&graph_payload()).unwrap();
        assert_eq!(data.center, 7);
        assert!(!data.from_references);
        // Center + parent + child + blocker + blocked + 2 timeline = 7 nodes.
        assert_eq!(data.nodes.len(), 7);
        let kinds: Vec<GraphEdgeKind> = data.edges.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                GraphEdgeKind::Blocks,
                GraphEdgeKind::BlockedBy,
                GraphEdgeKind::Parent,
                GraphEdgeKind::Child,
                GraphEdgeKind::ReferencedBy,
                GraphEdgeKind::Closes,
            ]
        );
        let child = data.nodes.iter().find(|n| n.number == 8).unwrap();
        assert_eq!(child.labels[0].name, "bug");
        assert_eq!(child.project_status.as_deref(), Some("Todo"));
        assert_eq!(child.linked_prs, vec![GraphLinkedPr { number: 44, state: "MERGED".to_string() }]);
        let closer = data.nodes.iter().find(|n| n.number == 12).unwrap();
        assert!(closer.is_pr);
        assert!(!data.nodes.iter().find(|n| n.number == 11).unwrap().is_pr);
        // The center blocks #10, so only it (and #7's edge) counts as blocking.
        assert!(model::graph_blocks_something(&data, 7));
        assert!(!model::graph_blocks_something(&data, 8));
        assert!(parse_graph(&json!({"errors": [{"message": "denied"}]})).is_err());
    }

    #[test]
    fn blocker_field_errors_route_to_the_fallback() {
        assert!(is_blocker_field_error("GitHub GraphQL: {\"message\":\"Field 'blockedBy' doesn't exist\"}"));
        assert!(is_blocker_field_error("Field 'blocking' doesn't exist on type 'Issue'"));
        assert!(!is_blocker_field_error("GitHub GraphQL: null"));
        // No blocker connections in the payload: body refs become missing.
        let mut slim = graph_payload();
        let issue = slim.pointer_mut("/data/repository/issue").unwrap();
        issue.as_object_mut().unwrap().remove("blockedBy");
        issue.as_object_mut().unwrap().remove("blocking");
        issue["body"] = json!("follow up in #9 and #10, dup of #9");
        let (center, neighbors, missing) = parse_graph_fallback_parts(&slim, "o/r").unwrap();
        assert_eq!(center.number, 7);
        // Deduped in first-seen order; the sub-issue (#8) is already known.
        assert_eq!(missing, vec![9, 10]);
        assert!(neighbors.iter().any(|(n, _)| n.number == 8));
    }

    #[test]
    fn fallback_derives_reference_edges_from_body_refs() {
        let mut slim = graph_payload();
        let issue = slim.pointer_mut("/data/repository/issue").unwrap();
        issue.as_object_mut().unwrap().remove("blockedBy");
        issue.as_object_mut().unwrap().remove("blocking");
        // #8 is already a sub-issue and #7 is the center: only #9 stays missing.
        issue["body"] = json!("follow up in #8 and #9, self #7");
        let (center, neighbors, missing) = parse_graph_fallback_parts(&slim, "o/r").unwrap();
        assert_eq!(center.number, 7);
        assert_eq!(missing, vec![9]);
        // Resolved nodes join as References edges under the cap.
        let detail = json!({"data": {
            "r0": {"issueOrPullRequest": {"__typename": "Issue", "number": 9,
                "title": "nine", "state": "OPEN",
                "labels": {"nodes": []}, "projectItems": {"nodes": []}}}
        }});
        let data = detail.get("data").unwrap();
        let raw = data.pointer("/r0/issueOrPullRequest").unwrap();
        let node = parse_graph_typed(raw).unwrap();
        let mut all = neighbors;
        all.push((node, GraphEdgeKind::References));
        let mut data = model::build_graph(center, all);
        data.from_references = true;
        assert!(data.from_references);
        assert!(data.edges.iter().any(|e| e.kind == GraphEdgeKind::References && e.to == 9));
    }

    #[test]
    fn graph_detail_query_aliases_each_number() {
        let q = graph_detail_query("o", "r", &[9, 10]);
        assert!(q.contains("r0:") && q.contains("r1:"));
        assert!(q.contains("issueOrPullRequest(number: 9)"));
        assert!(q.contains("issueOrPullRequest(number: 10)"));
    }

    #[test]
    fn graphql_errors_reject_and_data_unwraps() {
        let ok = json!({"data": {"addPullRequestReviewThread": {"thread": {"id": "t"}}}});
        assert_eq!(graphql_data(&ok).unwrap().pointer("/addPullRequestReviewThread/thread/id").and_then(Value::as_str), Some("t"));
        assert!(graphql_data(&json!({"errors": [{"message": "denied"}]})).is_err());
        assert!(graphql_data(&json!({"data": null})).is_err());
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
