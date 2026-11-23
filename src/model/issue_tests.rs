//! Tests for model::issue.

use super::*;
use crate::model::ItemRef;

#[test]
fn parses_review_threads_in_unresolved_first_order() {
    let input = serde_json::json!({"reviewThreads":{"nodes":[
        {"id":"t1","path":"a.rs","line":null,"isResolved":true,"comments":{"nodes":[{"id":"c1","author":{"login":"ada"},"body":"done"}]}},
        {"id":"t2","path":"b.rs","line":42,"isResolved":false,"comments":{"nodes":[{"id":"c2","author":{"login":"bob"},"body":"**fix**"}]}},
        {"id":"bad"}, null
    ]}});
    let threads = parse_review_threads(&input);
    assert_eq!(threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t2", "t1"]);
    assert_eq!((&*threads[0].path, threads[0].line, threads[0].is_resolved), ("b.rs", Some(42), false));
    assert_eq!((&*threads[0].comments[0].id, &*threads[0].comments[0].author.as_ref().unwrap().login, &*threads[0].comments[0].body), ("c2", "bob", "**fix**"));
    assert_eq!(threads[1].line, None);
    assert!(threads[1].is_resolved);
}

#[test]
fn review_comments_track_pending_state() {
    let input = serde_json::json!({"reviewThreads":{"nodes":[
        {"id":"t1","path":"a.rs","line":1,"isResolved":false,"comments":{"nodes":[
            {"id":"c1","author":{"login":"ada"},"body":"draft","state":"PENDING"},
            {"id":"c2","author":{"login":"bob"},"body":"live","state":"SUBMITTED"}
        ]}},
        {"id":"t2","path":"b.rs","line":2,"isResolved":false,"comments":{"nodes":[
            {"id":"c3","author":{"login":"cam"},"body":"old"}
        ]}}
    ]}});
    let threads = parse_review_threads(&input);
    let pending = threads.iter().find(|t| t.id == "t1").unwrap();
    assert!(pending.has_pending());
    assert!(pending.comments[0].is_pending());
    assert!(!pending.comments[1].is_pending());
    // Old caches without a state field read as submitted.
    let old = threads.iter().find(|t| t.id == "t2").unwrap();
    assert!(!old.has_pending());
    assert_eq!(old.comments[0].state, "SUBMITTED");
}

#[test]
fn reply_target_is_the_first_comments_numeric_id() {
    let input = serde_json::json!({"reviewThreads":{"nodes":[
        {"id":"t1","path":"a.rs","line":1,"isResolved":false,"comments":{"nodes":[{"id":"c1","databaseId":99,"body":"hi"},{"id":"c2","databaseId":100,"body":"re"}]}},
        {"id":"t2","path":"a.rs","line":2,"isResolved":false,"comments":{"nodes":[{"id":"c3","body":"no numeric id"}]}}
    ]}});
    let threads = parse_review_threads(&input);
    assert_eq!(thread_reply_comment_id(&threads, "t1"), Some(99));
    assert_eq!(thread_reply_comment_id(&threads, "t2"), None);
    assert_eq!(thread_reply_comment_id(&threads, "missing"), None);
    assert_eq!(thread_reply_comment_id(&[], "t1"), None);
}

#[test]
fn paired_refs_parse_both_directions() {
    let issue = serde_json::json!({"closedByPullRequestsReferences":{"nodes":[
        {"number": 44, "state": "MERGED"}, {"number": 45, "state": "OPEN"}, null
    ]}});
    assert_eq!(
        paired_refs(&issue, false),
        vec![
            PairedRef {
                number: ItemRef::github(44),
                state: "MERGED".to_string()
            },
            PairedRef {
                number: ItemRef::github(45),
                state: "OPEN".to_string()
            },
        ]
    );
    let pr = serde_json::json!({"closingIssuesReferences":{"nodes":[
        {"number": 59, "state": "OPEN"}
    ]}});
    assert_eq!(
        paired_refs(&pr, true),
        vec![PairedRef {
            number: ItemRef::github(59),
            state: "OPEN".to_string()
        }]
    );
    assert!(paired_refs(&serde_json::json!({}), true).is_empty());
}

#[test]
fn check_rollup_parses_pass_fail_pending_and_absent() {
    use CheckState::{Fail, Pass, Pending};
    let node = |state: &str| serde_json::json!({"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":state}}}]}});
    assert_eq!(parse_check_state(&node("SUCCESS")), Some(Pass));
    assert_eq!(parse_check_state(&node("FAILURE")), Some(Fail));
    assert_eq!(parse_check_state(&node("ERROR")), Some(Fail));
    assert_eq!(parse_check_state(&node("PENDING")), Some(Pending));
    assert_eq!(parse_check_state(&node("EXPECTED")), Some(Pending));
    // Missing commits, empty nodes, or a null rollup mean no checks.
    assert_eq!(parse_check_state(&serde_json::json!({})), None);
    assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[]}})), None);
    assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}})), None);
    assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[{"commit":{}}]}})), None);
    // Unknown future states never fail the row.
    assert_eq!(parse_check_state(&node("QUEUED")), Some(CheckState::Unknown));
}

#[test]
fn cached_issue_without_optional_details_still_parses() {
    let payload = serde_json::json!({
        "number": ItemRef::github(7), "title": "cached", "state": "OPEN",
        "author": {"login": "bo"}, "labels": [], "assignees": [],
        "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
        "url": "https://github.com/o/r/issues/7", "body": "",
        "comments": [], "isPr": false, "reviewThreads": []
    });
    let data: IssueData = serde_json::from_value(payload).expect("cache parses");
    assert!(data.paired.is_empty());
    assert!(data.checks.is_none());
}

#[test]
fn merges_pages_in_oldest_first_order() {
    let comment = |body: &str| Comment { body: body.to_owned(), ..Comment::default() };
    let merged = merge_comment_pages(vec![(2, vec![comment("third")]), (1, vec![comment("first"), comment("second")])]);
    assert_eq!(merged.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(), vec!["first", "second", "third"]);
}
