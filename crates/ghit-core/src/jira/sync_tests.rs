use super::*;
use crate::model::IssueData;

fn row(source: Source, number: u64, state: &str, title: &str) -> Row {
    let now = chrono::Utc::now().to_rfc3339();
    let issue = IssueData {
        number: ItemRef { source, number },
        title: title.to_string(),
        state: state.to_string(),
        created_at: now.clone(),
        updated_at: now.clone(),
        ..Default::default()
    };
    Row {
        detail: Default::default(),
        statuses: vec![],
        complete: false,
        window_at: (state == "CLOSED").then_some(now),
        issue,
    }
}

fn jira(number: u64, state: &str, title: &str) -> Row {
    row(Source::Jira, number, state, title)
}

fn numbers(rows: &[Row]) -> Vec<u64> {
    rows.iter().map(|row| row.issue.number.number).collect()
}

#[test]
fn fetched_rows_replace_stored_rows_of_the_same_key() {
    let stored = vec![jira(1, "OPEN", "old"), jira(2, "OPEN", "kept")];
    let merged = overlay(stored, vec![jira(1, "CLOSED", "new")]);
    assert_eq!(numbers(&merged), vec![2, 1]);
    assert_eq!(merged[1].issue.title, "new");
}

#[test]
fn a_github_number_never_collides_with_a_jira_key() {
    let stored = vec![row(Source::GitHub, 1, "OPEN", "gh")];
    let merged = overlay(stored, vec![jira(1, "OPEN", "jira")]);
    assert_eq!(merged.len(), 2);
}

#[test]
fn the_lists_show_only_jira_rows_by_state() {
    let mut pr = row(Source::GitHub, 5, "OPEN", "pr");
    pr.issue.is_pr = true;
    let rows = vec![
        jira(1, "OPEN", "a"),
        jira(2, "CLOSED", "b"),
        row(Source::GitHub, 3, "OPEN", "github issue"),
        pr,
    ];
    let now = chrono::Utc::now();
    assert_eq!(numbers(&shown_rows(rows.clone(), now, 24, [true, true])), vec![1, 2]);
    assert_eq!(numbers(&shown_rows(rows.clone(), now, 24, [true, false])), vec![1]);
    assert_eq!(numbers(&shown_rows(rows, now, 24, [false, false])), Vec::<u64>::new());
}

#[test]
fn closed_rows_outside_the_window_drop() {
    let mut old = jira(2, "CLOSED", "old");
    old.window_at = Some("2020-01-01T00:00:00Z".to_string());
    let rows = vec![old, jira(3, "CLOSED", "new")];
    let shown = shown_rows(rows, chrono::Utc::now(), 24, [true, true]);
    assert_eq!(numbers(&shown), vec![3]);
}
