use crate::model::{
    self, Author, BoardDetail, Comment, DiffFile, GraphData, GraphEdgeKind, GraphLinkedPr,
    GraphNode, IssueData, Label,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::OnceLock;

type Statuses = Vec<(String, Option<String>)>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub issue: IssueData,
    pub statuses: Statuses,
    pub detail: BoardDetail,
    pub complete: bool,
    /// Section timestamp the window filtered on (`mergedAt` for merged PRs,
    /// `updatedAt` for closed issues). `None` for rows the window never
    /// filters. Cached rows re-check this anchor when they merge forward,
    /// so rows that aged out while cached leave.
    #[serde(default)]
    pub window_at: Option<String>,
}

/// Scopes ghit needs on the token: repo data (issues, PRs, comments,
/// search, diffs), org boards, and Projects v2 fields.
pub const REQUIRED_SCOPES: &[&str] = &["repo", "read:org", "project"];

/// Exact re-auth command naming every required scope, so the prompt
/// shows it and the user can also run it by hand.
pub fn reauth_command() -> String {
    let mut cmd = String::from("gh auth refresh");
    for s in REQUIRED_SCOPES {
        cmd.push_str(" -s ");
        cmd.push_str(s);
    }
    cmd
}

/// True when a fetch failure smells like auth (bad/expired token or
/// missing scopes) rather than a bad repo, number, payload, or rate
/// limit. Bare 403s stay out: rate limits are 403s with a quota body,
/// and only a scope-denial body (or a 401) may point at re-auth.
pub fn is_auth_error(e: &str) -> bool {
    const MARKERS: &[&str] = &[
        "401 Unauthorized", "Bad credentials", "Requires authentication",
        "Resource not accessible", "requires authentication",
        "missing scope", "Insufficient scopes", "Could not resolve to a Repository",
        "FORBIDDEN", "UNAUTHORIZED", "SAML", "Must have admin rights",
    ];
    MARKERS.iter().any(|m| e.contains(m))
}

/// Append the re-auth command to an auth-class failure, so a bare
/// status line still tells the user how to fix it.
pub fn with_auth_hint(e: String) -> String {
    if is_auth_error(&e) {
        format!("{e} — re-auth with more permissions: {}", reauth_command())
    } else {
        e
    }
}

/// Scopes still missing from a granted set: pure so tests pin it
/// without the network.
pub fn missing_scopes(granted: &[&str]) -> Vec<String> {
    REQUIRED_SCOPES
        .iter()
        .filter(|s| !granted.contains(s))
        .map(|s| s.to_string())
        .collect()
}

/// Early auth probe: validates the token against `GET /rate_limit`.
/// That endpoint answers for every token kind (OAuth, PAT, GitHub App
/// installation): a dead token 401s, anything alive 200s. Scope gaps
/// need a repo, so those wait for `check_repo_access` below.
pub async fn check_auth(bearer: &str) -> Result<Vec<String>, String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let url = format!("{base}/rate_limit");
    let res = client()
        .get(&url)
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub auth check failed", &url, res).await);
    }
    Ok(vec![])
}

/// Repo-scoped access check, run once the repo is known: a 200 proves
/// the token reaches this repo, and the OAuth scopes header (absent on
/// installation and fine-grained tokens) names any missing scope.
/// Auth-class failures carry the re-auth hint; a 404 names the repo
/// plainly, since re-auth cannot fix a wrong slug.
pub async fn check_repo_access(bearer: &str, repo: &str) -> Result<Vec<String>, String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let url = format!("{base}/repos/{repo}");
    let res = client()
        .get(&url)
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub repo access check failed", &url, res).await);
    }
    let granted: Vec<&str> = res
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();
    // No scopes header (installation or fine-grained token, or a blank
    // one): reachability above is all we know, and a blank list proves
    // no gap.
    if granted.iter().all(|s| s.is_empty()) {
        return Ok(vec![]);
    }
    Ok(missing_scopes(&granted))
}

static TOKEN_CACHE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn token() -> Option<String> {
    // Env wins and is re-read every call, so an export mid-run applies.
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            let t = t.trim().to_owned();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    let mut cached = TOKEN_CACHE.lock().unwrap();
    if let Some(t) = cached.clone() {
        return Some(t);
    }
    let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() { return None; }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    if token.is_empty() { return None; }
    *cached = Some(token.clone());
    Some(token)
}

/// Drop the cached `gh auth token` value so a re-auth (or logout)
/// takes effect without restarting the app.
pub fn forget_token() {
    *TOKEN_CACHE.lock().unwrap() = None;
}

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
const MAX_SECTION_ROWS: usize = 500;

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
    s.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|d| d.with_timezone(&chrono::Utc) >= now - chrono::Duration::hours(hours as i64))
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

/// Map a failed response: the status plus the API's own first body
/// line, so a 403 rate limit never reads as a scope denial. Auth-class
/// failures carry the re-auth hint; everything else stays a plain error
/// for the status line.
async fn http_fail(context: &str, url: &str, res: reqwest::Response) -> String {
    let status = res.status();
    let snippet: String = res
        .text()
        .await
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect();
    with_auth_hint(format!("{context}: {status} for {url}: {snippet}"))
}

async fn get(client: &reqwest::Client, url: &str, bearer: &str) -> Result<Value, String> {
    let res = client
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub request failed", url, res).await);
    }
    res.json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
}

async fn post_json(client: &reqwest::Client, url: &str, bearer: &str, body: &Value) -> Result<Value, String> {
    let res = client
        .post(url)
        .bearer_auth(bearer)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub request failed", url, res).await);
    }
    res.json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
}

/// Auth-class failure hiding inside a 200 GraphQL payload: the data
/// may be partial (nested fields denied), so surface it instead of
/// rendering the partial result as if it were whole.
fn graphql_auth_failure(value: &Value) -> Option<String> {
    let errors = value.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let first = errors.first().map(|v| v.to_string()).unwrap_or_default();
    is_auth_error(&first).then(|| with_auth_hint(format!("GitHub GraphQL: {first}")))
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
) -> Result<Vec<Row>, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
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
    parse_rows(&pages, chrono::Utc::now(), hours)
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

/// Fetch details for the actual board cards, including cards outside the sections window.
///
/// GraphQL kept: batch win — one aliased query for every card replaces
/// one REST `issues/{n}` call per card (probe: the REST fields used
/// here are per-number only; there is no multi-number REST read).
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
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
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
            checks: None,
            milestone: v.pointer("/milestone/title").and_then(Value::as_str).map(str::to_owned),
            is_pr,
        },
        statuses: vec![],
        detail,
        complete: false,
        window_at: None,
    })
}

/// Server-side issue/PR search: `repo:{repo} {query}` through the REST
/// search API, relevance-ranked, issues and PRs mixed.
pub async fn search(repo: &str, query: &str, bearer: &str) -> Result<Vec<Row>, String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let url = format!("{base}/search/issues");
    let res = client()
        .get(&url)
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .query(&[("q", format!("repo:{repo} {query}")), ("per_page", "50".to_string())])
        .send()
        .await
        .map_err(|e| format!("GitHub search failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub search failed", &url, res).await);
    }
    let value: Value = res
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
    // REST ships `milestone` as an object; flatten to its title so the
    // `Option<String>` field parses. Absent or null stays `None`.
    let milestone = v.pointer("/milestone/title").and_then(Value::as_str).map(str::to_owned);
    v["milestone"] = milestone.map(Value::String).unwrap_or(Value::Null);
    let mut data: IssueData = serde_json::from_value(v).map_err(|e| format!("bad issue payload: {e}"))?;
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let field = if is_pr { "pullRequest" } else { "issue" };
    let threads_field = if is_pr { "reviewThreads(first:50) { nodes { id path line isResolved comments(first:20) { nodes { id databaseId author { login } body state } } } }" } else { "" };
    // Will-close counterparts: an issue lists its closing PRs, a PR lists
    // the issues it will close. Never body mentions.
    let paired_field = if is_pr {
        "closingIssuesReferences(first: 5) { nodes { number state } }"
    } else {
        "closedByPullRequestsReferences(first: 5, includeClosedPrs: true) { nodes { number state } }"
    };
    // GraphQL kept for this follow-up only (the issue itself is REST
    // above): REST has no equivalent for any of its three fields.
    // Probes: REST review comments (rust-lang/rust PR 165882,
    // `pulls/{n}/comments`) carry id/path/line/in_reply_to_id but no
    // thread grouping or `isResolved`; REST `pulls/116` has no closing
    // field and the #114 timeline `cross-referenced` event has no
    // will-close flag, so `closingIssuesReferences` /
    // `closedByPullRequestsReferences` have no REST equivalent; and
    // Projects v2 status is GraphQL-only (see `sections`).
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
    // Sub-issues are not here: they load through REST
    // `issues/{n}/sub_issues` in `graph` (confirmed equivalent, #117).
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
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
    value
        .pointer("/data/repository/issue")
        .filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))
}

/// The full graph: center plus every neighbor, capped by `build_graph`.
/// Test-only since `graph` splices REST sub-issues in before building.
#[cfg(test)]
fn parse_graph(value: &Value) -> Result<GraphData, String> {
    let (center, neighbors) = parse_graph_parts(value)?;
    Ok(model::build_graph(center, neighbors))
}

/// Center plus GraphQL neighbors, before the REST sub-issues join.
fn parse_graph_parts(value: &Value) -> Result<(GraphNode, Vec<(GraphNode, GraphEdgeKind)>), String> {
    let issue = graph_issue(value)?;
    let center = parse_graph_issue(issue)?;
    Ok((center, parse_graph_neighbors(issue)))
}

/// REST API base honoring `GH_HOST`, shared by the REST ports below.
fn rest_base() -> String {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") }
}

/// One REST sub-issue object as a graph node. REST state is lowercase,
/// graph state is the GraphQL uppercase form; REST carries no project
/// status or closing-PR list, so those stay empty — the same fields
/// every REST issue payload in this file already lacks.
fn sub_issue_node(v: &Value) -> Option<GraphNode> {
    let labels = v.get("labels").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|l| l.get("name").and_then(Value::as_str).or_else(|| l.as_str()).map(|s| Label { name: s.to_owned() }))
        .collect();
    Some(GraphNode {
        number: v.get("number")?.as_u64()?,
        title: v.get("title").and_then(Value::as_str).unwrap_or("").to_owned(),
        state: v.get("state").and_then(Value::as_str).unwrap_or("").to_uppercase(),
        is_pr: false,
        labels,
        project_status: None,
        linked_prs: vec![],
    })
}

/// Sub-issue nodes out of a REST `issues/{n}/sub_issues` payload.
/// Malformed entries skip, like every other list parser here.
pub fn parse_sub_issues(value: &Value) -> Vec<GraphNode> {
    value.as_array().into_iter().flatten().filter_map(sub_issue_node).collect()
}

/// Splice REST sub-issues into GraphQL neighbors at their pane
/// priority slot: after blockers and parent, before every other kind,
/// so a duplicate number still wins as a child (`build_graph` keeps
/// the first edge).
fn with_sub_issues(mut neighbors: Vec<(GraphNode, GraphEdgeKind)>, subs: Vec<GraphNode>) -> Vec<(GraphNode, GraphEdgeKind)> {
    let at = neighbors.iter()
        .take_while(|(_, k)| matches!(k, GraphEdgeKind::Blocks | GraphEdgeKind::BlockedBy | GraphEdgeKind::Parent))
        .count();
    let children = subs.into_iter().map(|n| (n, GraphEdgeKind::Child));
    neighbors.splice(at..at, children);
    neighbors
}

/// Sub-issues through REST `GET issues/{n}/sub_issues`, capped at the
/// old GraphQL `first: 10`. Ported in #117: probe
/// `gh api repos/bolu-atx/gh-ghit/issues/117/sub_issues` returns 200
/// with the same issue objects REST returns everywhere else.
async fn fetch_sub_issues(client: &reqwest::Client, base: &str, repo: &str, number: u64, bearer: &str) -> Result<Vec<GraphNode>, String> {
    let value = get(client, &format!("{base}/repos/{repo}/issues/{number}/sub_issues?per_page=10"), bearer).await?;
    Ok(parse_sub_issues(&value))
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
///
/// GraphQL kept: batch win — one aliased query for every missing
/// number replaces one REST `issues/{n}` call per number, and it
/// carries the per-node Projects v2 status REST has no equivalent
/// for (see `sections`).
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
///
/// GraphQL kept for the star itself: batch win — one query replaces a
/// REST call per relation per node, and several relations have no
/// REST equivalent at all. Probes (#117): `issues/117/tracked` 404s
/// (no tracked-issues endpoint), the REST timeline has no
/// `willCloseTarget` (see `issue`), and every node carries Projects
/// v2 status (GraphQL-only, see `sections`). Sub-issues are the one
/// confirmed equivalent and load through REST below. Follow-up worth
/// probing: REST `issues/{n}/dependencies/blocked_by|blocking`
/// returned 200 on #117, but plain issue objects without project
/// status or closing-PR refs, so they stay in this query for now.
pub async fn graph(repo: &str, number: u64, bearer: &str) -> Result<GraphData, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" {
        "https://api.github.com/graphql".to_owned()
    } else {
        format!("https://{host}/api/graphql")
    };
    let client = client();
    let base = rest_base();
    let value = post_json(
        &client,
        &url,
        bearer,
        &json!({"query": graph_query(true), "variables": {"owner": owner, "repo": name, "num": number}}),
    )
    .await?;
    match parse_graph_parts(&value) {
        Ok((center, neighbors)) => {
            let subs = fetch_sub_issues(&client, &base, repo, number, bearer).await?;
            Ok(model::build_graph(center, with_sub_issues(neighbors, subs)))
        }
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
            let subs = fetch_sub_issues(&client, &base, repo, number, bearer).await?;
            let mut data = model::build_graph(center, with_sub_issues(neighbors, subs));
            data.from_references = true;
            Ok(data)
        }
        Err(e) => Err(e),
    }
}

/// GraphQL endpoint honoring `GH_HOST`, shared by the GraphQL calls
/// below that REST cannot cover (see their keep-comments).
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

/// Head-commit SHA out of a REST `pulls/{n}` payload: the
/// `commit_id` a REST review comment anchors to.
fn parse_head_sha(v: &Value) -> Result<String, String> {
    v.pointer("/head/sha")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "PR not found: line comments need a pull request".to_string())
}

/// REST review-comment body for one new line comment.
fn review_comment_payload(body: &str, commit_id: &str, path: &str, line: u64) -> Value {
    json!({"body": body, "commit_id": commit_id, "path": path, "line": line, "side": "RIGHT"})
}

/// Post a single-shot line comment on a PR through REST
/// `POST pulls/{n}/comments`: no review flow, one thread per call.
/// Ported from the GraphQL `addPullRequestReviewThread` mutation in
/// #117 (probe: REST review comments on rust-lang/rust PR 165882
/// carry the same path/line/side fields); this also drops the old
/// GraphQL node-id lookup — REST needs the head SHA, which
/// `GET pulls/{n}` returns as `head.sha` (probed on PR 116).
pub async fn add_review_thread(repo: &str, number: u64, body: &str, path: &str, line: u64, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    if path.is_empty() || line == 0 {
        return Err("line comment needs a file and a new-side line".to_string());
    }
    repo.split_once('/').ok_or("repository must be owner/repo")?;
    let base = rest_base();
    let client = client();
    let pr = get(&client, &format!("{base}/repos/{repo}/pulls/{number}"), bearer).await?;
    let commit_id = parse_head_sha(&pr)?;
    let out = post_json(&client, &format!("{base}/repos/{repo}/pulls/{number}/comments"), bearer, &review_comment_payload(body, &commit_id, path, line)).await?;
    out.get("id").and_then(Value::as_u64).ok_or("line comment failed: no comment returned")?;
    Ok(())
}

/// Submit the viewer's pending review on a PR. `addPullRequestReviewThread`
/// lands in a pending review by design, so line comments stay invisible
/// until this runs with a COMMENT event.
///
/// GraphQL kept: this mutation postdates the #117 audit and the REST
/// list-reviews probe came back empty, so no equivalent is confirmed
/// yet — re-probe before porting (follow-up).
pub async fn submit_pending_review(repo: &str, number: u64, bearer: &str) -> Result<(), String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    let id_query = "query($owner:String!, $repo:String!, $num:Int!) { repository(owner:$owner,name:$repo) { pullRequest(number:$num) { id } } }";
    let id_value = post_json(&client, &url, bearer, &json!({"query": id_query, "variables": {"owner": owner, "repo": name, "num": number}})).await?;
    let pr_id = graphql_data(&id_value)?.pointer("/repository/pullRequest/id").and_then(Value::as_str).ok_or("PR not found: pending submit needs a pull request")?;
    let mutation = "mutation($prId:ID!) { submitPullRequestReview(input:{pullRequestId:$prId, event:COMMENT}) { pullRequestReview { id state } } }";
    let out = post_json(&client, &url, bearer, &json!({"query": mutation, "variables": {"prId": pr_id}})).await?;
    graphql_data(&out)?.pointer("/submitPullRequestReview/pullRequestReview/id").and_then(Value::as_str).ok_or("submit failed: no review returned")?;
    Ok(())
}

/// REST review-comment body for a reply inside an existing thread:
/// `in_reply_to` is the numeric id of a comment already in it.
fn review_reply_payload(body: &str, comment_id: u64) -> Value {
    json!({"body": body, "in_reply_to": comment_id})
}

/// Reply inside an existing review thread through REST
/// `POST pulls/{n}/comments` with `in_reply_to`. Ported from the
/// GraphQL `addPullRequestReviewThreadReply` mutation in #117:
/// probe shows REST review comments carry numeric `id` and
/// `in_reply_to_id` (rust-lang/rust PR 165882), so the first
/// comment's numeric id names the thread. Callers resolve it from
/// the thread under the cursor (`model::thread_reply_comment_id`).
pub async fn add_thread_reply(repo: &str, number: u64, comment_id: u64, body: &str, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    if comment_id == 0 {
        return Err("reply needs a thread: open the thread first".to_string());
    }
    repo.split_once('/').ok_or("repository must be owner/repo")?;
    let base = rest_base();
    let out = post_json(&client(), &format!("{base}/repos/{repo}/pulls/{number}/comments"), bearer, &review_reply_payload(body, comment_id)).await?;
    out.get("id").and_then(Value::as_u64).ok_or("reply failed: no comment returned")?;
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
    fn auth_failures_carry_the_reauth_command() {
        for e in [
            "GitHub request failed: 401 Unauthorized for https://api.github.com/user: {\"message\":\"Bad credentials\"}",
            "GitHub request failed: 403 Forbidden for https://api.github.com/graphql: {\"message\":\"Resource not accessible by personal access token\"}",
            "GitHub GraphQL: Bad credentials",
            "GitHub GraphQL: Resource not accessible by personal access token",
            "Could not resolve to a Repository with the name of o/r",
        ] {
            assert!(is_auth_error(e), "auth: {e}");
            let hinted = with_auth_hint(e.to_string());
            assert!(hinted.contains("gh auth refresh -s repo -s read:org -s project"), "hint: {hinted}");
        }
        for e in [
            "GitHub request failed: 404 Not Found for https://api.github.com/repos/o/r/issues/401: {\"message\":\"Not Found\"}",
            "GitHub request failed: 403 Forbidden for https://api.github.com/rate_limit: {\"message\":\"API rate limit exceeded\"}",
            "bad search payload",
            "missing row number",
        ] {
            assert!(!is_auth_error(e), "not auth: {e}");
            assert_eq!(with_auth_hint(e.to_string()), e, "untouched: {e}");
        }
    }

    #[test]
    fn graphql_auth_errors_fail_loudly_not_partially() {
        // Nested scope denial rides a 200 with partial data: surface it
        // instead of rendering the partial result as whole.
        let denied = json!({
            "data": {"repository": {"open": {"nodes": []}}},
            "errors": [{"type": "FORBIDDEN", "message": "Resource not accessible by personal access token"}],
        });
        let err = parse_board_details(&denied, 1).unwrap_err();
        assert!(err.contains("gh auth refresh"), "hint: {err}");
        // Non-auth field errors stay best-effort: rows still parse.
        let benign = json!({
            "data": {"repository": {
                "open": {"nodes": []}, "merged": {"nodes": []},
                "openIssues": {"nodes": []}, "closedIssues": {"nodes": []}}},
            "errors": [{"message": "something odd on one field"}],
        });
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        assert!(parse_rows(&[benign], now, 24).unwrap().is_empty());
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
        for field in ["parent", "blockedBy", "blocking", "trackedIssues",
            "trackedInIssues", "closedByPullRequestsReferences", "timelineItems",
            "CROSS_REFERENCED_EVENT", "labels", "projectItems"]
        {
            assert!(full.contains(field), "missing {field}");
        }
        // Sub-issues load through REST `issues/{n}/sub_issues` (#117).
        assert!(!full.contains("subIssues"), "{full}");
        let slim = graph_query(false);
        assert!(!slim.contains("blockedBy"), "{slim}");
        assert!(!slim.contains("blocking"), "{slim}");
        assert!(!slim.contains("subIssues"), "{slim}");
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
        // Center + parent + blocker + blocked + 2 timeline = 6 nodes.
        // Sub-issues join from REST, not this payload (next test).
        assert_eq!(data.nodes.len(), 6);
        let kinds: Vec<GraphEdgeKind> = data.edges.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                GraphEdgeKind::Blocks,
                GraphEdgeKind::BlockedBy,
                GraphEdgeKind::Parent,
                GraphEdgeKind::ReferencedBy,
                GraphEdgeKind::Closes,
            ]
        );
        let blocker = data.nodes.iter().find(|n| n.number == 9).unwrap();
        assert_eq!(blocker.labels[0].name, "bug");
        assert_eq!(blocker.project_status.as_deref(), Some("Todo"));
        assert_eq!(blocker.linked_prs, vec![GraphLinkedPr { number: 44, state: "MERGED".to_string() }]);
        let closer = data.nodes.iter().find(|n| n.number == 12).unwrap();
        assert!(closer.is_pr);
        assert!(!data.nodes.iter().find(|n| n.number == 11).unwrap().is_pr);
        // The center blocks #10, so only it (and #7's edge) counts as blocking.
        assert!(model::graph_blocks_something(&data, 7));
        assert!(!model::graph_blocks_something(&data, 11));
        assert!(parse_graph(&json!({"errors": [{"message": "denied"}]})).is_err());
    }

    #[test]
    fn rest_sub_issues_parse_and_join_as_children() {
        let subs = parse_sub_issues(&json!([
            {"number": 8, "title": "child", "state": "open", "labels": [{"name": "bug"}]},
            {"number": 9, "title": "also blocker", "state": "closed", "labels": []},
            {"title": "no number"},
            null,
        ]));
        assert_eq!(subs.len(), 2);
        assert_eq!((subs[0].number, &*subs[0].title, &*subs[0].state), (8, "child", "OPEN"));
        assert_eq!(subs[0].labels[0].name, "bug");
        assert_eq!(subs[0].project_status, None);
        assert!(subs[0].linked_prs.is_empty());
        assert_eq!(subs[1].state, "CLOSED");
        assert!(parse_sub_issues(&json!({"not": "a list"})).is_empty());

        // Children splice after blockers/parent and win on duplicates:
        // #9 stays the blocker the GraphQL query already found.
        let (center, neighbors) = parse_graph_parts(&graph_payload()).unwrap();
        let data = model::build_graph(center, with_sub_issues(neighbors, subs));
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
        assert_eq!(child.title, "child");
        assert!(data.edges.iter().any(|e| e.kind == GraphEdgeKind::Child && e.to == 8));
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
        // Deduped in first-seen order.
        assert_eq!(missing, vec![9, 10]);
        assert!(neighbors.iter().any(|(n, _)| n.number == 1), "parent still resolves");
    }

    #[test]
    fn fallback_derives_reference_edges_from_body_refs() {
        let mut slim = graph_payload();
        let issue = slim.pointer_mut("/data/repository/issue").unwrap();
        issue.as_object_mut().unwrap().remove("blockedBy");
        issue.as_object_mut().unwrap().remove("blocking");
        // #7 is the center: only #9 stays missing.
        issue["body"] = json!("follow up in #9, self #7");
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
    fn rest_review_comment_payloads_carry_the_anchor_fields() {
        // New line comment: head SHA + path/line on the new side.
        let p = review_comment_payload("note", "abc123", "src/a.rs", 42);
        assert_eq!(p, json!({"body": "note", "commit_id": "abc123", "path": "src/a.rs", "line": 42, "side": "RIGHT"}));
        // Reply: the numeric id of a comment already in the thread.
        assert_eq!(review_reply_payload("re", 99), json!({"body": "re", "in_reply_to": 99}));
        // The head SHA comes off the REST PR payload, not a GraphQL id.
        assert_eq!(parse_head_sha(&json!({"head": {"sha": "abc123"}})).unwrap(), "abc123");
        assert!(parse_head_sha(&json!({"head": {}})).is_err());
        assert!(parse_head_sha(&json!({})).is_err());
    }

    #[tokio::test]
    async fn rest_review_mutations_validate_before_the_network() {
        assert!(add_review_thread("o/r", 1, "  ", "a.rs", 1, "t").await.is_err());
        assert!(add_review_thread("o/r", 1, "hi", "", 1, "t").await.is_err());
        assert!(add_review_thread("o/r", 1, "hi", "a.rs", 0, "t").await.is_err());
        assert!(add_review_thread("badrepo", 1, "hi", "a.rs", 1, "t").await.is_err());
        assert!(add_thread_reply("o/r", 1, 99, "  ", "t").await.is_err());
        assert!(add_thread_reply("o/r", 1, 0, "hi", "t").await.is_err());
        assert!(add_thread_reply("badrepo", 1, 99, "hi", "t").await.is_err());
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
