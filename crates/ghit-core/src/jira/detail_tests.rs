use super::*;
use crate::model::Source;
use serde_json::json;

fn issue() -> Value {
    json!({"key": "PROJ-42", "fields": {
        "summary": "Fix login", "status": {
            "name": "In Progress", "statusCategory": {"key": "indeterminate"}
        },
        "issuetype": {"name": "Bug"}, "priority": {"name": "High"},
        "assignee": {"displayName": "Ada"}, "reporter": {"displayName": "Grace"},
        "labels": ["urgent"], "parent": {"key": "PROJ-7"},
        "created": "2026-11-01T09:30:00.000+0100",
        "updated": "2026-11-02T10:00:00.000+0000",
        "description": {"type": "doc", "version": 1, "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Login details", "marks": [{"type": "strong"}]}
            ]}
        ]}
    }})
}

#[test]
fn detail_maps_adf_people_metadata_and_dates() {
    let (data, statuses) = map_detail("PROJ", "https://jira.example", &issue()).unwrap();
    assert_eq!(data.number, ItemRef { source: Source::Jira, number: 42 });
    assert_eq!(data.title, "Fix login");
    assert_eq!(data.body.trim(), "**Login details**");
    assert_eq!(data.state, "OPEN");
    assert_eq!(data.issue_type, "Bug");
    assert_eq!(data.priority, "High");
    assert_eq!(data.author.login, "Grace");
    assert_eq!(data.assignees[0].login, "Ada");
    assert_eq!(data.labels[0].name, "urgent");
    assert_eq!(data.parent, Some(ItemRef { source: Source::Jira, number: 7 }));
    assert_eq!(data.created_at, "2026-11-01T08:30:00Z");
    assert_eq!(data.updated_at, "2026-11-02T10:00:00Z");
    assert_eq!(data.url, "https://jira.example/browse/PROJ-42");
    assert_eq!(statuses, vec![("PROJ".to_string(), Some("In Progress".to_string()))]);
}

#[test]
fn comments_map_display_names_adf_and_utc_dates() {
    let page = json!({"startAt": 0, "total": 101, "comments": [{
        "author": {"displayName": "Lin"}, "created": "2026-11-02T11:00:00.000+0100",
        "body": {"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "Reviewed"}
        ]}]}
    }]});
    let (comments, total) = map_comments(&page).unwrap();
    assert_eq!(total, 101);
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author.login, "Lin");
    assert_eq!(comments[0].body.trim(), "Reviewed");
    assert_eq!(comments[0].created_at, "2026-11-02T10:00:00Z");
    assert!(map_comments(&json!({"total": 0})).is_err());
    assert!(map_comments(&json!({"comments": []})).is_err());
    assert!(map_comments(&json!({"total": 0, "comments": []})).unwrap().0.is_empty());
}

#[test]
fn null_description_and_optional_fields_are_empty() {
    let value = json!({"key": "PROJ-1", "fields": {"description": null}});
    let (data, statuses) = map_detail("PROJ", "https://jira.example", &value).unwrap();
    assert!(data.body.is_empty());
    assert!(data.assignees.is_empty());
    assert!(data.priority.is_empty());
    assert!(statuses.is_empty());
    assert!(map_detail("OTHER", "https://jira.example", &value).is_err());
    assert!(map_detail("PROJ", "https://jira.example", &json!({})).is_err());
}

#[test]
fn jira_detail_uses_the_shared_store_and_invalidates_on_updated() {
    let _guard = crate::cache::test_env_lock();
    let dir = tempfile::tempdir().unwrap();
    let saved = std::env::var_os("XDG_DATA_HOME");
    std::env::set_var("XDG_DATA_HOME", dir.path());
    let mut bundle = map_detail("PROJ", "https://jira.example", &issue()).unwrap();
    bundle.0.comments = vec![Comment {
        author: Author { login: "Lin".into() },
        body: "Reviewed".into(),
        created_at: "2026-11-02T10:00:00Z".into(),
    }];
    let number = bundle.0.number;
    let mut listed = search::map_issue("PROJ", "https://jira.example", &issue()).unwrap();
    listed.issue.parent = None;
    listed.issue.updated_at = "2026-11-01T10:00:00Z".into();
    crate::store::open("o/r").unwrap().upsert_rows_newer(&[listed]).unwrap();
    github::save_bundle("o/r", number, &bundle);
    let client = JiraClient::new("https://jira.example", "email".into(), "token".into(),
        "PROJ".into());
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let backend = crate::backend::IssueBackend::Jira(std::sync::Arc::new(client));
    let cached = runtime.block_on(backend.fetch_number("o/r", number, false, None)).unwrap();
    assert_eq!(cached.0.body, bundle.0.body);
    assert_eq!(cached.0.issue_type, "Bug");
    assert_eq!(cached.0.comments[0].body, "Reviewed");
    assert_eq!(cached.0.comments[0].author.login, "Lin");
    let mut store = crate::store::open("o/r").unwrap();
    let mut row = store.row(number).unwrap();
    assert_eq!(row.issue.parent, bundle.0.parent);
    row.issue.updated_at = "2026-11-03T10:00:00Z".into();
    store.upsert_rows_newer(&[row]).unwrap();
    assert!(github::stored_bundle("o/r", number).is_none());
    match saved {
        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}
