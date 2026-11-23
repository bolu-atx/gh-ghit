//! Issue data access, independent of any front end.

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;

use std::sync::mpsc::Sender;
use std::sync::Arc;

use crate::github::{self, IssueBundle, IssueEvent, Row, SectionEvent};
use crate::jira::JiraClient;
use crate::model::{IssueData, ItemRef, RepoMeta, Source};

#[derive(Debug, Clone, Default)]
pub enum IssueBackend {
    #[default]
    GitHub,
    Jira(Arc<JiraClient>),
}

/// The Jira connection a front end may show. The token is never part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraInfo {
    pub site: String,
    pub project: String,
    pub email: String,
    pub config_path: std::path::PathBuf,
    pub token_source: crate::jira::auth::TokenSource,
}

impl IssueBackend {
    pub fn shares_github_sections(&self) -> bool {
        match self {
            Self::GitHub => true,
            Self::Jira(_) => false,
        }
    }

    /// The Jira project key item labels carry; empty for GitHub.
    pub fn project_key(&self) -> &str {
        match self {
            Self::GitHub => "",
            Self::Jira(client) => client.project(),
        }
    }

    /// The source of this backend's issues when they are not GitHub's.
    pub fn own_source(&self) -> Option<Source> {
        match self {
            Self::GitHub => None,
            Self::Jira(_) => Some(Source::Jira),
        }
    }

    /// Label names and milestones for the Format pickers: this backend's
    /// issue side plus, when the backend brings its own issues, GitHub's
    /// PR side. An arm that fails keeps the other arm's lists and rides in
    /// `RepoMeta::error`.
    pub async fn fetch_meta(&self, repo: &str) -> Result<RepoMeta, String> {
        match self {
            Self::GitHub => {
                let (labels, milestones) = github::fetch_repo_meta(repo).await?;
                Ok(RepoMeta::side(Source::GitHub, labels, milestones))
            }
            Self::Jira(client) => {
                let issues = client.fetch_meta(repo).await;
                let prs = github::fetch_repo_meta(repo).await;
                match (issues, prs) {
                    (Ok((labels, milestones)), Ok((pr_labels, pr_milestones))) => {
                        let issues = RepoMeta::side(Source::Jira, labels, milestones);
                        Ok(issues.merge(RepoMeta::side(Source::GitHub, pr_labels, pr_milestones)))
                    }
                    (Ok((labels, milestones)), Err(error)) => {
                        Ok(RepoMeta::side(Source::Jira, labels, milestones).with_error(error))
                    }
                    (Err(error), Ok((labels, milestones))) => {
                        Ok(RepoMeta::side(Source::GitHub, labels, milestones).with_error(error))
                    }
                    (Err(error), Err(_)) => Err(error),
                }
            }
        }
    }

    /// The server search of a backend with its own issue sections.
    pub async fn search(
        &self, query: &str, progress: Option<Sender<SectionEvent>>,
    ) -> Result<Vec<Row>, String> {
        match self {
            Self::GitHub => unreachable!("GitHub search runs through github::search_repo"),
            Self::Jira(client) => client.search_rows(query, progress.as_ref()).await,
        }
    }

    /// The Jira connection, or `None` for GitHub.
    pub fn jira_info(&self) -> Option<JiraInfo> {
        match self {
            Self::GitHub => None,
            Self::Jira(client) => Some(JiraInfo {
                site: client.site().to_string(),
                project: client.project().to_string(),
                email: client.email().to_string(),
                config_path: client.config_path().to_path_buf(),
                token_source: client.token_source(),
            }),
        }
    }

    /// Detail includes the full comment thread and its pagination progress.
    pub async fn fetch_number(
        &self,
        repo: &str,
        number: ItemRef,
        force: bool,
        progress: Option<Sender<(ItemRef, IssueEvent)>>,
    ) -> Result<IssueBundle, String> {
        match self {
            Self::GitHub => github::fetch_number(repo, number, force, progress).await,
            Self::Jira(client) => client.fetch_number(repo, number, force, progress).await,
        }
    }

    pub async fn fetch_sub_issues(
        &self, repo: &str, number: ItemRef,
    ) -> Result<Vec<IssueData>, String> {
        match self {
            Self::GitHub => github::fetch_sub_issues(repo, number).await,
            Self::Jira(client) => client.fetch_sub_issues(number).await,
        }
    }

    pub async fn set_state(&self, repo: &str, number: ItemRef, open: bool) -> Result<(), String> {
        match self {
            Self::GitHub => github::set_state(repo, number, open).await,
            Self::Jira(_) => Err("issue state changes not supported for Jira yet".to_string()),
        }
    }

    pub async fn post_comment(&self, repo: &str, number: ItemRef, body: &str) -> Result<(), String> {
        match self {
            Self::GitHub => github::post_thread_comment(repo, number, body).await,
            Self::Jira(_) => Err("comments not supported for Jira yet".to_string()),
        }
    }

    /// Only backends with separate issue sections may enter this path.
    pub async fn fetch_sections(
        &self,
        repo: &str,
        hours: u64,
        show: [bool; 2],
        refresh: github::Refresh,
        progress: Option<Sender<SectionEvent>>,
    ) -> Result<Vec<Row>, String> {
        match self {
            Self::GitHub => {
                unreachable!("GitHub issue sections use the shared sections query")
            }
            Self::Jira(client) => client.fetch_sections(repo, hours, show, refresh, progress).await,
        }
    }
}
