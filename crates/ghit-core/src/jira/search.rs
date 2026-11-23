//! Jira issue search: the JQL, the `search/jql` paging client and the
//! mapping from Jira issues to sidebar rows.

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

use std::sync::mpsc::Sender;

use serde_json::{json, Value};

use super::JiraClient;
use crate::api::{self, Row, MAX_SECTION_ROWS};
use crate::github::SectionEvent;
use crate::model::{Author, BoardDetail, IssueData, ItemRef, Label, Source};

const PAGE_SIZE: usize = 100;

/// Seconds to wait after a 429 that carries no usable `Retry-After`.
const DEFAULT_RETRY_SECS: u64 = 30;

const FIELDS: [&str; 10] = [
    "summary", "status", "labels", "assignee", "reporter", "created", "updated", "issuetype",
    "priority", "parent",
];

/// Quote a JQL string value.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The JQL of one sync. `delta_minutes` reaches back that far for changed
/// issues only (relative times need no time zone, unlike absolute ones,
/// which Jira reads in the account's zone). Otherwise it lists every open
/// issue plus the closed ones updated within `closed_hours` (0 = all).
pub(super) fn jql(project: &str, delta_minutes: Option<i64>, closed_hours: u64) -> String {
    let key = quote(project);
    let filter = match (delta_minutes, closed_hours) {
        (Some(minutes), _) => format!(" AND updated >= {}", quote(&format!("-{minutes}m"))),
        (None, 0) => String::new(),
        (None, hours) => format!(
            " AND (statusCategory != Done OR (statusCategory = Done AND updated >= {}))",
            quote(&format!("-{hours}h"))
        ),
    };
    format!("project = {key}{filter} ORDER BY updated DESC")
}

/// Minutes from `since` to `now`, with a margin for clock skew and the
/// minute resolution of JQL times.
pub(super) fn delta_minutes(
    since: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>,
) -> i64 {
    (now - since).num_minutes().max(0) + 3
}

/// When a 429 lets calls resume: `Retry-After` seconds, else a default wait.
pub(super) fn retry_until(retry_after: Option<&str>, now: u64) -> u64 {
    let secs = retry_after.and_then(|s| s.trim().parse::<u64>().ok());
    now + secs.unwrap_or(DEFAULT_RETRY_SECS)
}

/// The issue number of `key` when it belongs to `project`.
fn item_of(project: &str, key: &str) -> Option<ItemRef> {
    let (prefix, number) = key.rsplit_once('-')?;
    if !prefix.eq_ignore_ascii_case(project) {
        return None;
    }
    Some(ItemRef { source: Source::Jira, number: number.parse().ok()? })
}

/// Jira's `2024-05-01T10:00:00.000+0000` as the UTC RFC 3339 form the
/// sidebar compares as text. Unparsable text stays as it is.
fn utc(stamp: &str) -> String {
    chrono::DateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(stamp))
        .map(|at| at.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|_| stamp.to_string())
}

/// One Jira issue as a sidebar row. `None` for a key of another project.
/// The status name rides in the project-status slot under the project key.
fn map_issue(project: &str, base: &str, issue: &Value) -> Option<Row> {
    let key = issue.get("key")?.as_str()?;
    let number = item_of(project, key)?;
    let fields = issue.get("fields")?;
    let text = |pointer: &str| fields.pointer(pointer).and_then(Value::as_str).map(str::to_owned);
    let done = text("/status/statusCategory/key").as_deref() == Some("done");
    let person = |pointer: &str| text(pointer).map(|login| Author { login });
    let labels = fields.get("labels").and_then(Value::as_array).into_iter().flatten();
    let issue_data = IssueData {
        number,
        title: text("/summary").unwrap_or_default(),
        state: if done { "CLOSED" } else { "OPEN" }.to_string(),
        author: person("/reporter/displayName").unwrap_or_default(),
        labels: labels
            .filter_map(Value::as_str)
            .map(|name| Label { name: name.to_string() })
            .collect(),
        assignees: person("/assignee/displayName").into_iter().collect(),
        created_at: text("/created").map(|at| utc(&at)).unwrap_or_default(),
        updated_at: text("/updated").map(|at| utc(&at)).unwrap_or_default(),
        url: format!("{base}/browse/{key}"),
        parent: text("/parent/key").and_then(|parent| item_of(project, &parent)),
        ..Default::default()
    };
    Some(Row {
        statuses: text("/status/name")
            .map(|name| (project.to_string(), Some(name)))
            .into_iter()
            .collect(),
        detail: BoardDetail::from_issue(&issue_data),
        window_at: done.then(|| issue_data.updated_at.clone()),
        issue: issue_data,
        complete: false,
    })
}

/// One search reply: the rows of this project and the next page token.
fn parse_page(
    project: &str, base: &str, page: &Value,
) -> Result<(Vec<Row>, Option<String>), String> {
    let issues = page
        .get("issues")
        .and_then(Value::as_array)
        .ok_or_else(|| "bad Jira response: no issues list".to_string())?;
    let rows = issues.iter().filter_map(|issue| map_issue(project, base, issue)).collect();
    let last = page.get("isLast").and_then(Value::as_bool).unwrap_or(false);
    let next = page.get("nextPageToken").and_then(Value::as_str).filter(|_| !last);
    Ok((rows, next.map(str::to_owned)))
}

/// What a search returned: the rows, and whether the row cap cut it short.
pub(super) struct Found {
    pub rows: Vec<Row>,
    pub capped: bool,
}

impl JiraClient {
    /// Run `jql`, paging on `nextPageToken` up to the sidebar row cap.
    pub(super) async fn search(
        &self, jql: &str, progress: Option<&Sender<SectionEvent>>,
    ) -> Result<Found, String> {
        let mut rows: Vec<Row> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut body = json!({"jql": jql, "fields": FIELDS, "maxResults": PAGE_SIZE});
            if let Some(token) = &token {
                body["nextPageToken"] = json!(token);
            }
            let page = self.post_search(&body).await?;
            let (found, next) = parse_page(&self.project, &self.base, &page)?;
            rows.extend(found);
            if rows.len() >= MAX_SECTION_ROWS {
                rows.truncate(MAX_SECTION_ROWS);
                return Ok(Found { rows, capped: next.is_some() });
            }
            let Some(next) = next else { return Ok(Found { rows, capped: false }) };
            if let Some(tx) = progress {
                let (done, total) = (rows.len(), MAX_SECTION_ROWS);
                let _ = tx.send(SectionEvent::Progress { done, total });
            }
            token = Some(next);
        }
    }

    async fn post_search(&self, body: &Value) -> Result<Value, String> {
        let url = format!("{}/rest/api/3/search/jql", self.base);
        let res = super::client()
            .post(&url)
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| format!("could not reach Jira: {e}"))?;
        let status = res.status().as_u16();
        if status == 429 {
            let header = res.headers().get("retry-after").and_then(|v| v.to_str().ok());
            let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
            let until = retry_until(header, now);
            api::note_rate_limit_until(until);
            return Err(format!("Jira rate limited: retry in {}s", until - now));
        }
        if status == 401 || status == 403 {
            return Err("Jira rejected the email or JIRA_API_TOKEN (401/403)".to_string());
        }
        if !res.status().is_success() {
            return Err(format!("Jira search failed: {status} for {url}"));
        }
        res.json().await.map_err(|e| format!("bad Jira response: {e}"))
    }
}
