//! Shared fixtures for the model tests.

use super::*;
use crate::model::ItemRef;

pub(super) fn sample_issue() -> IssueData {
    IssueData {
        number: ItemRef::github(1),
        title: "some title here".to_string(),
        state: "OPEN".to_string(),
        issue_type: String::new(),
        priority: String::new(),
        author: Author { login: "bo".to_string() },
        labels: vec![Label { name: "needs-triage".to_string() }],
        assignees: vec![],
        created_at: "2026-01-02T00:00:00Z".to_string(),
        updated_at: "2026-01-05T00:00:00Z".to_string(),
        url: "https://github.com/o/r/issues/1".to_string(),
        body: "see (#1) and #2".to_string(),
        comments: vec![],
        review_threads: vec![],
        paired: vec![],
        checks: None,
        milestone: None,
        parent: None,
        sub_summary: None,
            links: Default::default(),
        is_pr: false,
    }
}
