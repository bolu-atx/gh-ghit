//! The Jira picker lists: the labels of the project's synced rows and its
//! fix versions.

#[cfg(test)]
#[path = "meta_tests.rs"]
mod tests;

use std::collections::BTreeSet;

use serde_json::Value;

use super::JiraClient;
use crate::cache::{self, Key};
use crate::github::Row;
use crate::model::{Milestone, Source};
use crate::store;

/// One entry of the REST `project/<KEY>/versions` listing. `releaseDate`
/// orders the sidebar's milestone groups, and `released` closes the pill.
fn versions_of(value: &Value) -> Result<Vec<Milestone>, String> {
    let items = value.as_array().ok_or("bad Jira versions payload")?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let title = item.get("name").and_then(Value::as_str)?;
            if title.is_empty() {
                return None;
            }
            Some(Milestone {
                title: title.to_owned(),
                due_on: item.get("releaseDate").and_then(Value::as_str).map(str::to_owned),
                closed: item.get("released").and_then(Value::as_bool).unwrap_or(false),
                ..Default::default()
            })
        })
        .collect())
}

/// The distinct label names the Jira rows carry, sorted.
fn labels_of(rows: &[Row]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for row in rows.iter().filter(|row| row.issue.number.source == Source::Jira) {
        for label in &row.issue.labels {
            if !label.name.is_empty() {
                names.insert(label.name.clone());
            }
        }
    }
    names.into_iter().collect()
}

/// The labels of the project's stored rows. A missing or unreadable store
/// lists none. Blocking.
fn stored_labels(repo: &str) -> Vec<String> {
    let Ok(store) = store::open(repo) else { return Vec::new() };
    labels_of(&store.rows().unwrap_or_default())
}

impl JiraClient {
    /// Picker meta of the issue side: the labels its synced rows carry,
    /// read from the store with no request, and its fix versions as
    /// milestones.
    pub(crate) async fn fetch_meta(
        &self, repo: &str,
    ) -> Result<(Vec<String>, Vec<Milestone>), String> {
        let repo = repo.to_string();
        let labels = tokio::task::spawn_blocking(move || stored_labels(&repo))
            .await
            .map_err(|e| format!("store read failed: {e}"))?;
        let milestones = self.versions().await?;
        Ok((labels, milestones))
    }

    /// The project's fix versions through the 30-minute cache.
    async fn versions(&self) -> Result<Vec<Milestone>, String> {
        let name = Key::JiraVersions { site: self.base.clone(), project: self.project.clone() }
            .name();
        let read_name = name.clone();
        let hit: Option<Vec<Milestone>> =
            tokio::task::spawn_blocking(move || cache::cache_get(&read_name))
                .await
                .map_err(|e| format!("cache read failed: {e}"))?;
        if let Some(versions) = hit {
            return Ok(versions);
        }
        let path = format!("/rest/api/3/project/{}/versions", self.project);
        let fresh = versions_of(&self.get_detail(&path).await?)?;
        let saved = fresh.clone();
        let _ = tokio::task::spawn_blocking(move || cache::cache_put(&name, &saved)).await;
        Ok(fresh)
    }
}
