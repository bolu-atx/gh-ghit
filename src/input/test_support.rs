//! Shared fixtures for the input tests.

use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

use crate::model::{Author, IssueData};

pub(super) fn sample(number: u64) -> IssueData {
    IssueData {
        number,
        title: "Fix the thing".to_string(),
        state: "OPEN".to_string(),
        author: Author { login: "bo".to_string() },
        labels: vec![],
        assignees: vec![],
        created_at: "2026-01-02T00:00:00Z".to_string(),
        updated_at: "2026-01-05T00:00:00Z".to_string(),
        url: format!("https://github.com/o/r/issues/{number}"),
        body: String::new(),
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

pub(super) fn wheel(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::empty() }
}
