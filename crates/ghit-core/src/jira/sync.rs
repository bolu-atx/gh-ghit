//! Jira listing sync: search the project, keep its rows in the repo store
//! next to the GitHub rows, and return what the issue sections show.

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;

use std::collections::HashSet;
use std::sync::mpsc::Sender;

use super::search::{delta_minutes, jql};
use super::JiraClient;
use crate::api::{self, Row, MAX_SECTION_ROWS};
use crate::github::{self, Refresh, SectionEvent};
use crate::model::{ItemRef, Source};
use crate::store;

impl JiraClient {
    /// Sync the project's recent issues into the repo store, then return
    /// the rows the open and closed issue lists show. A full refresh (or
    /// no earlier Jira sync) lists every open issue plus the closed window;
    /// otherwise only issues changed since the last Jira sync come over.
    pub(crate) async fn fetch_sections(
        &self,
        repo: &str,
        hours: u64,
        show: [bool; 2],
        refresh: Refresh,
        progress: Option<Sender<SectionEvent>>,
    ) -> Result<Vec<Row>, String> {
        let started = chrono::Utc::now();
        let read_repo = repo.to_string();
        let (last, stored) = tokio::task::spawn_blocking(move || load_rows(&read_repo))
            .await
            .unwrap_or_default();
        let since = api::delta_since(last.as_deref(), refresh == Refresh::Full);
        let minutes = since.map(|since| delta_minutes(since, started));
        let query = jql(&self.project, minutes, github::sync_hours(hours));
        let found = self.search(&query, progress.as_ref()).await?;
        let saved = found.rows.clone();
        let rows = overlay(stored, found.rows);
        let (save_repo, stamp) = (repo.to_string(), started.to_rfc3339());
        let _ = tokio::task::spawn_blocking(move || save_rows(&save_repo, &stamp, &saved)).await;
        let shown = shown_rows(rows, chrono::Utc::now(), hours, show);
        if found.capped || shown.len() >= MAX_SECTION_ROWS {
            if let Some(tx) = &progress {
                let _ = tx.send(SectionEvent::Capped);
            }
        }
        Ok(shown)
    }
}

/// The last Jira sync start and the stored Jira rows. GitHub rows in the
/// same file are not read. Blocking.
fn load_rows(repo: &str) -> (Option<String>, Vec<Row>) {
    let Ok(store) = store::open(repo) else { return (None, Vec::new()) };
    let rows = store.rows().unwrap_or_default();
    (store.jira_last_sync(), rows.into_iter().filter(is_jira).collect())
}

/// Upsert `rows` and record `started_at` as the last Jira sync start. Best
/// effort: the next sync rewrites. Blocking.
fn save_rows(repo: &str, started_at: &str, rows: &[Row]) {
    if let Ok(mut store) = store::open(repo) {
        let _ = store.write_jira(started_at, rows);
    }
}

fn is_jira(row: &Row) -> bool {
    row.issue.number.source == Source::Jira
}

/// `stored` with each `fetched` row replacing the stored row of its number.
fn overlay(stored: Vec<Row>, fetched: Vec<Row>) -> Vec<Row> {
    let fresh: HashSet<ItemRef> = fetched.iter().map(|row| row.issue.number).collect();
    stored.into_iter().filter(|row| !fresh.contains(&row.issue.number)).chain(fetched).collect()
}

/// The open and closed issue lists (`show`) of the Jira rows in `rows`.
fn shown_rows(
    rows: Vec<Row>, now: chrono::DateTime<chrono::Utc>, hours: u64, show: [bool; 2],
) -> Vec<Row> {
    let jira = rows.into_iter().filter(is_jira).collect();
    api::visible_rows(jira, now, hours, [false, false, show[0], show[1]])
}
