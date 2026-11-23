use super::*;

fn issue(key: &str, status: &str, category: &str) -> Value {
    json!({
        "key": key,
        "fields": {
            "summary": "Fix login",
            "status": {"name": status, "statusCategory": {"key": category}},
            "labels": ["backend", "urgent"],
            "assignee": {"displayName": "Ada Lovelace"},
            "reporter": {"displayName": "Grace Hopper"},
            "created": "2026-11-01T09:30:00.000+0100",
            "updated": "2026-11-02T10:00:00.000+0000",
            "issuetype": {"name": "Bug"},
            "priority": {"name": "High"},
            "parent": {"key": "PROJ-7"}
        }
    })
}

#[test]
fn full_jql_lists_open_plus_the_closed_window() {
    assert_eq!(
        jql("PROJ", None, 168),
        "project = \"PROJ\" AND (statusCategory != Done OR \
         (statusCategory = Done AND updated >= \"-168h\")) ORDER BY updated DESC"
    );
}

#[test]
fn full_jql_with_no_window_lists_everything() {
    assert_eq!(jql("PROJ", None, 0), "project = \"PROJ\" ORDER BY updated DESC");
}

#[test]
fn incremental_jql_reaches_back_by_relative_minutes() {
    assert_eq!(
        jql("PROJ", Some(48), 168),
        "project = \"PROJ\" AND updated >= \"-48m\" ORDER BY updated DESC"
    );
}

#[test]
fn jql_quotes_the_project_key() {
    assert_eq!(quote(r#"A"B\C"#), r#""A\"B\\C""#);
    assert!(jql(r#"X" OR 1=1 --"#, None, 0).starts_with(r#"project = "X\" OR 1=1 --""#));
}

#[test]
fn delta_minutes_pads_the_gap() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-02T12:00:00Z").unwrap().to_utc();
    let since = chrono::DateTime::parse_from_rfc3339("2026-11-02T11:15:30Z").unwrap().to_utc();
    assert_eq!(delta_minutes(since, now), 44 + 3);
    assert_eq!(delta_minutes(now + chrono::TimeDelta::minutes(5), now), 3, "a future start clamps");
}

#[test]
fn retry_after_seconds_set_the_wait_and_a_default_covers_a_missing_header() {
    assert_eq!(retry_until(Some("17"), 1000), 1017);
    assert_eq!(retry_until(Some(" 5 "), 1000), 1005);
    assert_eq!(retry_until(None, 1000), 1030);
    assert_eq!(retry_until(Some("Wed, 21 Oct 2026 07:28:00 GMT"), 1000), 1030);
}

#[test]
fn an_open_issue_maps_to_a_row() {
    let raw = issue("PROJ-123", "In Progress", "indeterminate");
    let row = map_issue("PROJ", "https://x.atlassian.net", &raw).unwrap();
    let data = &row.issue;
    assert_eq!(data.number, ItemRef { source: Source::Jira, number: 123 });
    assert_eq!((data.title.as_str(), data.state.as_str()), ("Fix login", "OPEN"));
    assert_eq!(data.author.login, "Grace Hopper");
    assert_eq!(data.assignees.len(), 1);
    assert_eq!(data.assignees[0].login, "Ada Lovelace");
    let labels: Vec<&str> = data.labels.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(labels, ["backend", "urgent"]);
    assert_eq!(data.created_at, "2026-11-01T08:30:00Z", "offsets fold to UTC");
    assert_eq!(data.updated_at, "2026-11-02T10:00:00Z");
    assert_eq!(data.url, "https://x.atlassian.net/browse/PROJ-123");
    assert!(!data.is_pr);
    assert_eq!(data.parent, Some(ItemRef { source: Source::Jira, number: 7 }));
    assert_eq!(row.statuses, vec![("PROJ".to_string(), Some("In Progress".to_string()))]);
    assert_eq!(row.window_at, None);
}

#[test]
fn a_done_issue_is_closed_and_anchors_its_window_on_the_update() {
    let row = map_issue("PROJ", "https://x", &issue("PROJ-9", "Done", "done")).unwrap();
    assert_eq!(row.issue.state, "CLOSED");
    assert_eq!(row.window_at.as_deref(), Some("2026-11-02T10:00:00Z"));
}

#[test]
fn to_do_counts_as_open() {
    let row = map_issue("PROJ", "https://x", &issue("PROJ-1", "To Do", "new")).unwrap();
    assert_eq!(row.issue.state, "OPEN");
}

#[test]
fn keys_of_other_projects_are_rejected() {
    assert!(map_issue("PROJ", "https://x", &issue("OTHER-5", "To Do", "new")).is_none());
    assert!(map_issue("PROJ", "https://x", &issue("PROJ-x", "To Do", "new")).is_none());
    let mut foreign_parent = issue("PROJ-2", "To Do", "new");
    foreign_parent["fields"]["parent"] = json!({"key": "OTHER-1"});
    assert_eq!(map_issue("PROJ", "https://x", &foreign_parent).unwrap().issue.parent, None);
}

#[test]
fn a_sparse_issue_still_maps() {
    let sparse =
        json!({"key": "proj-4", "fields": {"summary": "s", "assignee": null, "labels": []}});
    let row = map_issue("PROJ", "https://x", &sparse).unwrap();
    assert_eq!(row.issue.number.number, 4, "keys compare without case");
    assert!(row.issue.assignees.is_empty() && row.statuses.is_empty());
    assert_eq!(row.issue.state, "OPEN");
}

#[test]
fn a_page_yields_its_rows_and_the_next_token() {
    let page = json!({
        "issues": [issue("PROJ-1", "To Do", "new"), issue("OTHER-2", "To Do", "new")],
        "nextPageToken": "abc"
    });
    let (rows, next) = parse_page("PROJ", "https://x", &page).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(next.as_deref(), Some("abc"));
}

#[test]
fn the_last_page_has_no_next_token() {
    let page = json!({"issues": [], "isLast": true, "nextPageToken": "stale"});
    assert_eq!(parse_page("PROJ", "https://x", &page).unwrap().1, None);
    let bare = json!({"issues": []});
    assert_eq!(parse_page("PROJ", "https://x", &bare).unwrap().1, None);
}

#[test]
fn a_reply_without_an_issues_list_is_an_error() {
    let reply = json!({"errorMessages": ["bad jql"]});
    let error = parse_page("PROJ", "https://x", &reply).unwrap_err();
    assert!(error.contains("no issues list"), "{error}");
}
