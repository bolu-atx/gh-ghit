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

fn query() -> String {
    format!(r#"query($owner: String!, $repo: String!) {{
 repository(owner: $owner, name: $repo) {{
 open: pullRequests(first: 50, states: OPEN, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} }}
 merged: pullRequests(first: 50, states: MERGED, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {PR_FIELDS} }} }}
 openIssues: issues(first: 50, states: OPEN, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} }}
 closedIssues: issues(first: 50, states: CLOSED, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {ISSUE_FIELDS} }} }}
 }} }}"#)
}

fn within_window(s: Option<&str>, now: chrono::DateTime<chrono::Utc>, hours: u64) -> bool {
    s.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|d| d.with_timezone(&chrono::Utc) >= now - chrono::Duration::hours(hours as i64))
}

fn parse_rows(value: &Value, now: chrono::DateTime<chrono::Utc>, hours: u64) -> Result<Vec<Row>, String> {
    let repo = value.pointer("/data/repository").filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    let mut rows = Vec::new();
    // (alias, is_pr, date key, filter to window?)
    for (section, is_pr, date_key, windowed) in [
        ("open", true, "createdAt", false),
        ("merged", true, "mergedAt", true),
        ("openIssues", false, "createdAt", false),
        ("closedIssues", false, "updatedAt", true),
    ] {
        let nodes = repo.pointer(&format!("/{section}/nodes")).and_then(Value::as_array).ok_or("missing section rows")?;
        for n in nodes {
            if windowed && !within_window(n.get(date_key).and_then(Value::as_str), now, hours) { continue; }
            let string = |k: &str| n.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let number = n.get("number").and_then(Value::as_u64).ok_or("missing row number")?;
            let labels = n.pointer("/labels/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("name")?.as_str().map(|s| Label { name: s.to_owned() })).collect();
            let assignees = n.pointer("/assignees/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("login")?.as_str().map(|s| Author { login: s.to_owned() })).collect();
            let statuses = n.pointer("/projectItems/nodes").and_then(Value::as_array).into_iter().flatten().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
            let created = string("createdAt");
            let updated = string("updatedAt");
            let detail = BoardDetail { created: created.get(..10).unwrap_or("").to_owned(), updated: updated.get(..10).unwrap_or("").to_owned(), comments: n.pointer("/comments/totalCount").and_then(Value::as_u64).unwrap_or(0) as usize };
            rows.push(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: string("body"), comments: vec![], review_threads: crate::model::parse_review_threads(n), is_pr }, statuses, detail, complete: false });
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

pub async fn sections(repo: &str, hours: u64, bearer: &str) -> Result<Vec<Row>, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    let value = post_json(&client(), &url, bearer, &json!({"query": query(), "variables": {"owner": owner, "repo": name}})).await?;
    parse_rows(&value, chrono::Utc::now(), hours)
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
    let mut data: IssueData = serde_json::from_value(v).map_err(|e| format!("bad issue payload: {e}"))?;
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let field = if is_pr { "pullRequest" } else { "issue" };
    let threads_field = if is_pr { "reviewThreads(first:50) { nodes { id path line isResolved comments(first:20) { nodes { id author { login } body } } } }" } else { "" };
    let query = format!(r#"query($owner:String!, $repo:String!, $num:Int!) {{ repository(owner:$owner,name:$repo) {{ {field}(number:$num) {{ {threads_field} projectItems(first:10) {{ nodes {{ project {{ title }} status: fieldValueByName(name:"Status") {{ ... on ProjectV2ItemFieldSingleSelectValue {{ name }} }} }} }} }} }} }}"#);
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    // Project statuses stay best-effort: a failed lookup yields none,
    // same as before, instead of failing the whole issue fetch.
    let response = post_json(&client, &url, bearer, &json!({"query":query,"variables":{"owner":owner,"repo":name,"num":number}})).await;
    let node = response.as_ref().ok().and_then(|v| v.pointer(&format!("/data/repository/{field}")));
    if let (true, Some(node)) = (is_pr, node) {
        data.review_threads = crate::model::parse_review_threads(node);
    }
    let statuses = node.and_then(|n| n.pointer("/projectItems/nodes")).and_then(Value::as_array).into_iter().flatten()
        .map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
    Ok((data, statuses))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query();
        for field in ["open: pullRequests", "merged: pullRequests", "openIssues: issues", "closedIssues: issues", "projectItems", "totalCount", "$owner", "$repo"] { assert!(q.contains(field)); }
    }
    #[test]
    fn parses_rows_and_filters_merged_window() {
        let v = json!({"data":{"repository":{"open":{"nodes":[{"number":1,"title":"one","state":"OPEN","labels":{"nodes":[{"name":"bug"}]},"comments":{"totalCount":3},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]}}]},"merged":{"nodes":[{"number":2,"mergedAt":"2026-11-10T11:30:00Z"},{"number":3,"mergedAt":"2026-11-09T10:00:00Z"}]},"openIssues":{"nodes":[{"number":7,"title":"seven","state":"OPEN"}]},"closedIssues":{"nodes":[{"number":8,"title":"eight","state":"CLOSED","updatedAt":"2026-11-10T11:00:00Z"},{"number":9,"title":"nine","state":"CLOSED","updatedAt":"2026-11-09T10:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let mut v = v;
        v["data"]["repository"]["open"]["nodes"][0]["reviewThreads"] = json!({"nodes":[{"id":"thread-id","path":"src/a.rs","line":12,"isResolved":false,"comments":{"nodes":[{"id":"comment-id","author":{"login":"reviewer"},"body":"Please fix"}]}},{"id":"bad"}]});
        let rows = parse_rows(&v, now, 1).unwrap();
        assert_eq!(rows[0].issue.review_threads.len(), 1);
        assert_eq!(rows[0].issue.review_threads[0].id, "thread-id");
        assert_eq!(rows[0].issue.review_threads[0].comments[0].id, "comment-id");
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2, 7, 8]);
        assert_eq!(rows[0].issue.labels[0].name, "bug");
        assert_eq!(rows[0].statuses[0].1.as_deref(), Some("Todo"));
        assert_eq!(rows[0].detail.comments, 3);
        assert!(rows.iter().find(|r| r.issue.number == 1).unwrap().issue.is_pr);
        assert!(!rows.iter().find(|r| r.issue.number == 7).unwrap().issue.is_pr);
        assert!(parse_rows(&json!({"errors":[{"message":"denied"}]}), now, 1).is_err());
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
