//! Helpers shared by the `App` tests.

use super::*;
use crate::model::{Author, IssueData};

pub(super) fn issue(number: u64, updated: &str) -> IssueData {
    IssueData {
        number,
        title: format!("issue {number}"),
        state: "OPEN".to_string(),
        author: Author { login: "bo".to_string() },
        labels: vec![],
        assignees: vec![],
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: updated.to_string(),
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

pub(super) fn sidebar_app(numbers: &[u64]) -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for &n in numbers {
        app.issues.data.insert(n, issue(n, "2026-01-05T00:00:00Z"));
        app.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    app.nav.current = numbers.first().copied();
    app.sync_selection();
    app
}

pub(super) fn temp_xdg() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir, Option<String>) {
    let guard = crate::cache::test_env_lock();
    let dir = tempfile::TempDir::new().unwrap();
    let saved = std::env::var("XDG_DATA_HOME").ok();
    std::env::set_var("XDG_DATA_HOME", dir.path());
    (guard, dir, saved)
}

pub(super) fn restore_xdg(saved: Option<String>) {
    match saved {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

pub(super) fn section_row(issue: IssueData) -> crate::github::Row {
    crate::github::Row {
        issue,
        statuses: vec![],
        detail: model::BoardDetail::default(),
        complete: false,
        window_at: None,
    }
}

pub(super) fn land_sections(app: &mut App, rows: Vec<crate::github::Row>) {
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Done(Ok(rows))).unwrap();
    poll_sections(app);
}

pub(super) fn pending_nav_app() -> App {
    let mut app = sidebar_app(&[7, 9]);
    app.show_issue(9);
    let d = app.issues.data.get_mut(&9).unwrap();
    d.is_pr = true;
    d.review_threads = vec![crate::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(2),
        comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app
}
