use crate::model::{Author, BoardDetail, IssueData, Label};
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

const FIELDS: &str = r#"
 number title state author { login } labels(first: 100) { nodes { name } }
 assignees(first: 100) { nodes { login } } createdAt updatedAt url body
 comments(first: 1) { totalCount } mergedAt
 projectItems(first: 10) { nodes { project { title } status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }
"#;

fn query() -> String {
    format!(r#"query($owner: String!, $repo: String!) {{
 repository(owner: $owner, name: $repo) {{
 open: pullRequests(first: 50, states: OPEN, orderBy: {{field: CREATED_AT, direction: DESC}}) {{ nodes {{ {FIELDS} }} }}
 merged: pullRequests(first: 50, states: MERGED, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ {FIELDS} }} }}
 }} }}"#)
}

fn parse_rows(value: &Value, now: chrono::DateTime<chrono::Utc>, hours: u64) -> Result<Vec<Row>, String> {
    let repo = value.pointer("/data/repository").filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    let mut rows = Vec::new();
    for section in ["open", "merged"] {
        let nodes = repo.pointer(&format!("/{section}/nodes")).and_then(Value::as_array).ok_or("missing PR rows")?;
        for n in nodes {
            let merged_at = n.get("mergedAt").and_then(Value::as_str).map(str::to_owned);
            if section == "merged" && !merged_at.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).is_some_and(|d| d.with_timezone(&chrono::Utc) >= now - chrono::Duration::hours(hours as i64)) { continue; }
            let string = |k: &str| n.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            let number = n.get("number").and_then(Value::as_u64).ok_or("missing PR number")?;
            let labels = n.pointer("/labels/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("name")?.as_str().map(|s| Label { name: s.to_owned() })).collect();
            let assignees = n.pointer("/assignees/nodes").and_then(Value::as_array).into_iter().flatten().filter_map(|v| v.get("login")?.as_str().map(|s| Author { login: s.to_owned() })).collect();
            let statuses = n.pointer("/projectItems/nodes").and_then(Value::as_array).into_iter().flatten().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
            let created = string("createdAt");
            let updated = string("updatedAt");
            let detail = BoardDetail { created: created.get(..10).unwrap_or("").to_owned(), updated: updated.get(..10).unwrap_or("").to_owned(), comments: n.pointer("/comments/totalCount").and_then(Value::as_u64).unwrap_or(0) as usize };
            rows.push(Row { issue: IssueData { number, title: string("title"), state: string("state"), author: Author { login: n.pointer("/author/login").and_then(Value::as_str).unwrap_or("").to_owned() }, labels, assignees, created_at: created, updated_at: updated, url: string("url"), body: string("body"), comments: vec![], is_pr: true }, statuses, detail, complete: false });
        }
    }
    Ok(rows)
}

pub fn sections(repo: &str, hours: u64, bearer: &str) -> Result<Vec<Row>, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let url = if host == "github.com" { "https://api.github.com/graphql".to_owned() } else { format!("https://{host}/api/graphql") };
    let response = ureq::post(&url).set("Authorization", &format!("Bearer {bearer}")).set("User-Agent", "ghit").send_json(json!({"query": query(), "variables": {"owner": owner, "repo": name}})).map_err(|e| format!("GitHub request failed: {e}"))?;
    let value: Value = response.into_json().map_err(|e| format!("bad GitHub response: {e}"))?;
    parse_rows(&value, chrono::Utc::now(), hours)
}

pub fn issue(repo: &str, number: u64, bearer: &str) -> Result<(IssueData, Statuses), String> {
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned());
    let base = if host == "github.com" { "https://api.github.com".to_owned() } else { format!("https://{host}/api/v3") };
    let get = |url: &str| -> Result<Value, String> {
        ureq::get(url).set("Authorization", &format!("Bearer {bearer}")).set("User-Agent", "ghit")
            .call().map_err(|e| format!("GitHub request failed: {e}"))?
            .into_json().map_err(|e| format!("bad GitHub response: {e}"))
    };
    let mut v = get(&format!("{base}/repos/{repo}/issues/{number}"))?;
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
        let items = get(&format!("{base}/repos/{repo}/issues/{number}/comments?per_page=100&page={page}"))?;
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
    let statuses = ureq::post(&url).set("Authorization", &format!("Bearer {bearer}")).set("User-Agent", "ghit").send_json(json!({"query":query,"variables":{"owner":owner,"repo":name,"num":number}})).ok()
        .and_then(|r| r.into_json::<Value>().ok()).and_then(|v| v.pointer(&format!("/data/repository/{field}/projectItems/nodes")).and_then(Value::as_array).cloned()).unwrap_or_default()
        .iter().map(|p| (p.pointer("/project/title").and_then(Value::as_str).unwrap_or("").to_owned(), p.pointer("/status/name").and_then(Value::as_str).map(str::to_owned))).collect();
    Ok((data, statuses))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_has_both_sections_and_metadata() {
        let q = query();
        for field in ["open: pullRequests", "merged: pullRequests", "projectItems", "totalCount", "$owner", "$repo"] { assert!(q.contains(field)); }
    }
    #[test]
    fn parses_rows_and_filters_merged_window() {
        let v = json!({"data":{"repository":{"open":{"nodes":[{"number":1,"title":"one","state":"OPEN","labels":{"nodes":[{"name":"bug"}]},"comments":{"totalCount":3},"projectItems":{"nodes":[{"project":{"title":"P"},"status":{"name":"Todo"}}]}}]},"merged":{"nodes":[{"number":2,"mergedAt":"2026-11-10T11:30:00Z"},{"number":3,"mergedAt":"2026-11-09T10:00:00Z"}]}}}});
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let rows = parse_rows(&v, now, 1).unwrap();
        assert_eq!(rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(rows[0].issue.labels[0].name, "bug");
        assert_eq!(rows[0].statuses[0].1.as_deref(), Some("Todo"));
        assert_eq!(rows[0].detail.comments, 3);
        assert!(parse_rows(&json!({"errors":[{"message":"denied"}]}), now, 1).is_err());
    }
}
