//! Jira issue detail, comments and child issues.

#[cfg(test)]
#[path = "detail_tests.rs"]
mod tests;

use std::sync::mpsc::Sender;

use serde_json::Value;

use super::{search, JiraClient};
use crate::api;
use crate::github::{self, IssueBundle, IssueEvent};
use crate::model::{adf_to_markdown, Author, Comment, IssueData, ItemRef};

const FIELDS: &str = "summary,status,labels,assignee,reporter,created,updated,\
    issuetype,priority,parent,description,issuelinks";

fn map_detail(project: &str, base: &str, value: &Value) -> Result<IssueBundle, String> {
    let row = search::map_issue(project, base, value)
        .ok_or("bad Jira response: missing issue key or fields")?;
    let mut issue = row.issue;
    issue.body = adf_to_markdown(&value["fields"]["description"]);
    Ok((issue, row.statuses))
}

fn map_comments(page: &Value) -> Result<(Vec<Comment>, usize), String> {
    let comments = page["comments"].as_array().ok_or("bad Jira response: no comments list")?;
    let total = page["total"].as_u64().ok_or("bad Jira response: no comment total")?;
    let comments = comments.iter().map(|value| Comment {
        author: Author {
            login: value["author"]["displayName"].as_str().unwrap_or_default().to_string(),
        },
        body: adf_to_markdown(&value["body"]),
        created_at: search::utc(value["created"].as_str().unwrap_or_default()),
    }).collect();
    Ok((comments, total as usize))
}

impl JiraClient {
    pub(crate) async fn fetch_number(
        &self,
        repo: &str,
        number: ItemRef,
        force: bool,
        progress: Option<Sender<(ItemRef, IssueEvent)>>,
    ) -> Result<IssueBundle, String> {
        if !force {
            let repo = repo.to_string();
            let hit = tokio::task::spawn_blocking(move || github::stored_bundle(&repo, number))
                .await.map_err(|e| format!("store read failed: {e}"))?;
            if let Some(bundle) = hit {
                return Ok(bundle);
            }
        }
        let path = format!("/rest/api/3/issue/{}-{}", self.project, number.number);
        let value = self.get_detail(&format!("{path}?fields={FIELDS}")).await?;
        let mut bundle = map_detail(&self.project, &self.base, &value)?;
        let mut start = 0;
        let mut done = 0;
        loop {
            let page = self.get_detail(&format!(
                "{path}/comment?startAt={start}&maxResults=100"
            )).await?;
            let (comments, total) = map_comments(&page)?;
            let count = comments.len();
            bundle.0.comments.extend(comments);
            start += count;
            done += 1;
            let size = page["maxResults"].as_u64().unwrap_or(100).max(1) as usize;
            let pages = total.div_ceil(size);
            if pages > 1 {
                if let Some(tx) = &progress {
                    let _ = tx.send((number, IssueEvent::Progress { done, total: pages }));
                }
            }
            if start >= total {
                break;
            }
            if count == 0 {
                return Err("bad Jira response: comment pagination made no progress".to_string());
            }
        }
        let repo = repo.to_string();
        let stored = bundle.clone();
        let _ = tokio::task::spawn_blocking(move || github::save_bundle(&repo, number, &stored))
            .await;
        Ok(bundle)
    }

    pub(crate) async fn fetch_sub_issues(&self, number: ItemRef) -> Result<Vec<IssueData>, String> {
        let key = search::quote(&format!("{}-{}", self.project, number.number));
        let found = self.search(&format!("parent = {key}"), None).await?;
        if found.capped {
            return Err("Jira sub-issues exceed the search row limit".to_string());
        }
        Ok(found.rows.into_iter().map(|row| row.issue).collect())
    }

    async fn get_detail(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{path}", self.base);
        let response = super::client().get(&url)
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
            .send().await.map_err(|e| format!("could not reach Jira: {e}"))?;
        let status = response.status().as_u16();
        if status == 429 {
            let header = response.headers().get("retry-after").and_then(|v| v.to_str().ok());
            let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
            let until = search::retry_until(header, now);
            api::note_rate_limit_until(until);
            return Err(format!("Jira rate limited: retry in {}s", until - now));
        }
        if status == 401 || status == 403 {
            return Err("Jira rejected the email or JIRA_API_TOKEN (401/403)".to_string());
        }
        if !response.status().is_success() {
            return Err(format!("Jira request failed: {status} for {url}"));
        }
        response.json().await.map_err(|e| format!("bad Jira response: {e}"))
    }
}
