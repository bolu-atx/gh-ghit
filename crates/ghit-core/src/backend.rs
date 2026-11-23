//! Issue data access, independent of any front end.

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;

use std::sync::mpsc::Sender;

use crate::github::{self, IssueBundle, IssueEvent, Row, SectionEvent};
use crate::model::{IssueData, ItemRef};

#[derive(Debug, Clone, Copy, Default)]
pub enum IssueBackend {
    #[default]
    GitHub,
}

impl IssueBackend {
    pub fn shares_github_sections(&self) -> bool {
        match self {
            Self::GitHub => true,
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
        }
    }

    pub async fn fetch_sub_issues(
        &self, repo: &str, number: ItemRef,
    ) -> Result<Vec<IssueData>, String> {
        match self {
            Self::GitHub => github::fetch_sub_issues(repo, number).await,
        }
    }

    pub async fn set_state(&self, repo: &str, number: ItemRef, open: bool) -> Result<(), String> {
        match self {
            Self::GitHub => github::set_state(repo, number, open).await,
        }
    }

    pub async fn post_comment(&self, repo: &str, number: ItemRef, body: &str) -> Result<(), String> {
        match self {
            Self::GitHub => github::post_thread_comment(repo, number, body).await,
        }
    }

    /// Only backends with separate issue sections may enter this path.
    pub async fn fetch_sections(
        &self,
        _repo: &str,
        _hours: u64,
        _show: [bool; 2],
        _refresh: github::Refresh,
        _progress: Option<Sender<SectionEvent>>,
    ) -> Result<Vec<Row>, String> {
        match self {
            Self::GitHub => {
                unreachable!("GitHub issue sections use the shared sections query")
            }
        }
    }
}
