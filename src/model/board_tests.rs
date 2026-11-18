//! Tests for model::board.

use super::*;
use super::board::card_pr_state;

#[test]
fn board_milestones_sort_and_push_none_last() {
    let item = |milestone: Option<&str>| BoardItem {
        title: "c".into(),
        number: None,
        status: None,
        labels: vec![],
        repo: None,
        milestone: milestone.map(str::to_string),
        content: serde_json::Value::Null,
    };
    let items = vec![item(None), item(Some("v2")), item(Some("v1")), item(Some("v1"))];
    assert_eq!(
        board_milestones(&items),
        vec![Some("v1".to_string()), Some("v2".to_string()), None]
    );
    assert!(board_milestones(&[]).is_empty());
}

#[test]
fn board_groups_unknowns() {
    let items = vec![
        BoardItem { title: "a".into(), number: Some(1), status: Some("Todo".into()), labels: vec![], repo: None, milestone: None, content: serde_json::Value::Null },
        BoardItem { title: "b".into(), number: Some(2), status: Some("Done".into()), labels: vec![], repo: None, milestone: None, content: serde_json::Value::Null },
        BoardItem { title: "c".into(), number: Some(3), status: Some("Mystery".into()), labels: vec![], repo: None, milestone: None, content: serde_json::Value::Null },
    ];
    let cols = board_columns(&items, &["Todo".to_string(), "Done".to_string()]);
    assert_eq!(cols.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["Todo", "Done", "Other"]);
    assert_eq!(cols[2].1.iter().map(|c| c.number).collect::<Vec<_>>(), vec![Some(3)]);
}

#[test]
fn card_footer_splits_labels_left_and_date_right() {
    let item = BoardItem { title: "a".into(), number: Some(1), status: Some("Todo".into()), labels: vec![Label { name: "bug".into() }], repo: None, milestone: None, content: serde_json::Value::Null };
    assert_eq!(card_footer_labels(&item), "● bug");
    let bare = BoardItem { labels: vec![], ..item.clone() };
    assert_eq!(card_footer_labels(&bare), "–");
    assert_eq!(card_footer_right(None, &CardFields::default()), "…");
    let d = BoardDetail { created: "2026-01-02".into(), updated: "2026-01-05T00:00:00Z".into(), comments: 3 };
    assert_eq!(card_footer_right(Some(&d), &CardFields::default()), "3 comments · 2026-01-05");
}

#[test]
fn card_pr_state_missing_is_draft_shows_nothing() {
    let card = |content: serde_json::Value| BoardItem {
        title: "a".into(),
        number: Some(1),
        status: Some("Todo".into()),
        labels: vec![],
        repo: None,
        milestone: None,
        content,
    };
    let pr = |draft: Option<bool>| {
        let mut map = serde_json::Map::new();
        map.insert("type".to_string(), serde_json::Value::String("PullRequest".to_string()));
        if let Some(draft) = draft {
            map.insert("isDraft".to_string(), serde_json::Value::Bool(draft));
        }
        card(serde_json::Value::Object(map))
    };
    assert_eq!(card_pr_state(&pr(Some(true))), Some("draft"));
    assert_eq!(card_pr_state(&pr(Some(false))), Some("ready"));
    // A warm cache entry predating `isDraft` shows nothing, never a
    // guessed "ready"; issues never show a state either.
    assert_eq!(card_pr_state(&pr(None)), None);
    assert_eq!(card_pr_state(&card(serde_json::Value::Null)), None);
}
