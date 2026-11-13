mod board;
mod client;
mod graph;
mod sections;

pub use client::{
    check_auth, check_repo_access, forget_token, is_auth_error, reauth_command, token,
    REQUIRED_SCOPES,
};
pub use board::fetch_board;
pub use graph::graph;
pub use sections::{merge_rows, sections, MAX_SECTION_ROWS};

use crate::model::{self, Author, BoardDetail, Comment, DiffFile, IssueData, Label};
use client::{
    client, get, graphql_auth_failure, graphql_data, graphql_url, http_fail, post_json,
    rest_base,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    let url = graphql_url();
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
/// project status, so `statuses` stays empty and the row is partial:
/// opening it runs the full fetch. Merged PRs still report `MERGED` from
/// `pull_request.merged_at`.
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
    let merged = v.pointer("/pull_request/merged_at").is_some_and(|m| !m.is_null());
    if merged {
        state = "MERGED".to_string();
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

/// Fetched rows plus whether the fetch stopped at the shared sidebar
/// row cap.
#[derive(Debug)]
pub struct SectionRows {
    pub rows: Vec<Row>,
    pub capped: bool,
}

/// Absorb one REST search page into `rows`. Returns the hit count on the
/// page: the caller pages again while it is 100 and the row cap is not
/// reached. Hits past the cap are ignored.
fn absorb_search_page(value: &Value, rows: &mut Vec<Row>) -> Result<usize, String> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or("bad search payload")?;
    let room = MAX_SECTION_ROWS.saturating_sub(rows.len());
    for item in items.iter().take(room) {
        rows.push(search_row(item)?);
    }
    Ok(items.len())
}

/// Server-side issue/PR search: one `repo:{repo} {query}` REST search,
/// one page of 100 hits at a time, paged until the results end or the
/// shared sidebar row cap lands.
pub async fn search(
    repo: &str,
    query: &str,
    bearer: &str,
    progress: Option<&(dyn Fn(usize, usize) + Send + Sync)>,
) -> Result<SectionRows, String> {
    let url = format!("{}/search/issues", rest_base());
    let client = client();
    let q = format!("repo:{repo} {query}");
    let mut rows = Vec::new();
    let mut page = 1;
    loop {
        let res = client
            .get(&url)
            .bearer_auth(bearer)
            .header("Accept", "application/vnd.github+json")
            .query(&[
                ("q", q.clone()),
                ("per_page", "100".to_string()),
                ("page", page.to_string()),
            ])
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
        let hits = absorb_search_page(&value, &mut rows)?;
        if rows.len() >= MAX_SECTION_ROWS {
            return Ok(SectionRows { rows, capped: true });
        }
        if hits < 100 {
            break;
        }
        if let Some(report) = progress {
            report(rows.len(), MAX_SECTION_ROWS);
        }
        page += 1;
    }
    Ok(SectionRows { rows, capped: false })
}

/// Repository labels for the Format dialog's picker, one REST page.
pub async fn repo_labels(repo: &str, bearer: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/repos/{repo}/labels?per_page=100", rest_base());
    let value = get(&client(), &url, bearer).await?;
    let items = value.as_array().ok_or("bad labels payload")?;
    Ok(items
        .iter()
        .filter_map(|v| v.get("name").and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Repository milestones (open and closed) for the Format dialog's
/// picker and the milestone group order. `due_on` rides along.
pub async fn repo_milestones(repo: &str, bearer: &str) -> Result<Vec<model::Milestone>, String> {
    let url = format!("{}/repos/{repo}/milestones?state=all&per_page=100", rest_base());
    let value = get(&client(), &url, bearer).await?;
    let items = value.as_array().ok_or("bad milestones payload")?;
    Ok(items
        .iter()
        .filter_map(|v| {
            let title = v.get("title").and_then(Value::as_str)?;
            if title.is_empty() {
                return None;
            }
            Some(model::Milestone {
                title: title.to_owned(),
                due_on: v.get("due_on").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .collect())
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
    let base = rest_base();
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
    let base = rest_base();
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
    let url = graphql_url();
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


/// Post a plain issue/PR thread comment through REST. Validation errors
/// are strings for the status line, never panics.
pub async fn add_comment(repo: &str, number: u64, body: &str, bearer: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("comment is empty: write something first".to_string());
    }
    let base = rest_base();
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
    fn search_row_marks_prs_and_stays_partial() {
        let pr = json!({"number":12,"title":"a pr","state":"open","user":{"login":"bo"},"labels":[{"name":"bug"}],"assignees":[],"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-05T00:00:00Z","html_url":"https://github.com/o/r/pull/12","body":"hi","comments":4,"milestone":{"title":"v0.6"},"pull_request":{"url":"https://api.github.com/x"}});
        let row = search_row(&pr).unwrap();
        assert!(row.issue.is_pr);
        assert_eq!(row.issue.state, "OPEN");
        assert_eq!(row.issue.milestone.as_deref(), Some("v0.6"), "search rows carry their milestone");
        assert!(!row.complete);
        assert_eq!(row.detail.comments, 4);
        let issue = json!({"number":13,"title":"an issue","state":"closed","user":{"login":"bo"},"labels":[],"assignees":[],"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-05T00:00:00Z","html_url":"https://github.com/o/r/issues/13","body":"","comments":0});
        let row = search_row(&issue).unwrap();
        assert!(!row.issue.is_pr);
        assert_eq!(row.issue.state, "CLOSED");
        let merged = json!({"number":14,"title":"merged","state":"closed","user":{"login":"bo"},"labels":[],"assignees":[],"created_at":"2026-11-01T00:00:00Z","updated_at":"2026-11-05T00:00:00Z","html_url":"https://github.com/o/r/pull/14","body":"","comments":0,"pull_request":{"merged_at":"2026-11-04T00:00:00Z"}});
        assert_eq!(search_row(&merged).unwrap().issue.state, "MERGED");
        assert!(search_row(&json!({"title":"no number"})).is_err());
    }

    fn search_hit(number: u64) -> Value {
        json!({
            "number": number,
            "title": format!("hit {number}"),
            "state": "open",
            "user": {"login": "bo"},
            "labels": [{"name": "bug"}],
            "created_at": "2026-11-01T00:00:00Z",
            "updated_at": "2026-11-05T00:00:00Z",
            "html_url": format!("https://github.com/o/r/issues/{number}"),
            "comments": 0,
        })
    }

    fn search_page(start: u64, count: usize) -> Value {
        json!({"items": (start..start + count as u64).map(search_hit).collect::<Vec<_>>()})
    }

    #[test]
    fn paged_search_absorbs_every_page_and_stops_at_the_cap() {
        let mut rows = vec![];
        assert_eq!(absorb_search_page(&search_page(1, 100), &mut rows), Ok(100));
        assert_eq!(absorb_search_page(&search_page(101, 2), &mut rows), Ok(2));
        assert_eq!(rows.len(), 102, "both pages land");
        assert_eq!(rows[101].issue.number, 102);
        // A page of 100 means "ask again"; the cap ignores later hits.
        let mut capped = vec![];
        for page in 0..5 {
            absorb_search_page(&search_page(page * 100 + 1, 100), &mut capped).unwrap();
        }
        assert_eq!(capped.len(), MAX_SECTION_ROWS);
        assert_eq!(absorb_search_page(&search_page(9999, 3), &mut capped), Ok(3));
        assert_eq!(capped.len(), MAX_SECTION_ROWS, "hits past the cap stay out");
        assert!(absorb_search_page(&json!({"message": "no items"}), &mut rows).is_err());
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
