use super::*;

use crate::api::Row;
use crate::model::{IssueData, ItemRef, Label};

fn milestone(title: &str, due: Option<&str>, released: bool) -> Milestone {
    Milestone {
        title: title.to_string(),
        due_on: due.map(str::to_owned),
        closed: released,
        ..Default::default()
    }
}

#[test]
fn versions_map_to_milestones() {
    let raw = serde_json::json!([
        {"id": "1", "name": "1.0", "released": true, "releaseDate": "2026-10-01"},
        {"id": "2", "name": "1.1", "released": false},
        {"id": "3", "name": ""},
        {"id": "4"},
    ]);
    assert_eq!(
        versions_of(&raw).unwrap(),
        vec![milestone("1.0", Some("2026-10-01"), true), milestone("1.1", None, false)]
    );
}

#[test]
fn a_versions_payload_that_is_not_a_list_is_an_error() {
    assert!(versions_of(&serde_json::json!({"values": []})).is_err());
    assert_eq!(versions_of(&serde_json::json!([])).unwrap(), vec![]);
}

fn row(source: Source, labels: &[&str]) -> Row {
    Row {
        issue: IssueData {
            number: ItemRef { source, number: 1 },
            labels: labels.iter().map(|name| Label { name: (*name).to_string() }).collect(),
            ..Default::default()
        },
        statuses: Vec::new(),
        detail: crate::model::BoardDetail::default(),
        complete: false,
        window_at: None,
    }
}

#[test]
fn labels_are_the_distinct_names_of_the_jira_rows() {
    let rows = vec![
        row(Source::Jira, &["ui", "bug"]),
        row(Source::Jira, &["bug", ""]),
        row(Source::GitHub, &["docs"]),
    ];
    assert_eq!(labels_of(&rows), vec!["bug".to_string(), "ui".to_string()]);
    assert_eq!(labels_of(&[]), Vec::<String>::new());
}
