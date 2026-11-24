//! Tests for model::text.

use super::*;

use super::test_support::sample_issue;

#[test]
fn meta_spans_carry_roles_and_join_to_plain_text() {
    let d = sample_issue();
    let parts = meta_spans(&d, &[]);
    assert!(parts.iter().any(|(k, t)| *k == MetaKind::State && t == "OPEN"));
    assert!(parts.iter().any(|(k, t)| *k == MetaKind::Label && t == "needs-triage"));
    assert!(parts.iter().any(|(k, _)| *k == MetaKind::Url));
    let joined: String = parts.into_iter().map(|(_, t)| t).collect();
    assert!(joined.contains("project: —"));
}

#[test]
fn meta_spans_name_the_milestone_or_a_dash() {
    let mut d = sample_issue();
    d.milestone = Some("v0.3".to_string());
    let parts = meta_spans(&d, &[]);
    assert!(parts.iter().any(|(k, t)| *k == MetaKind::Milestone && t == "v0.3"));
    let joined: String = parts.into_iter().map(|(_, t)| t).collect();
    assert!(joined.contains("milestone: v0.3"));
    let bare = meta_spans(&sample_issue(), &[]);
    assert!(bare.iter().any(|(k, t)| *k == MetaKind::Milestone && t == "—"));
}

#[test]
fn status_and_meta() {
    assert_eq!(status_text(&[("Roadmap".into(), Some("In Progress".into()))]), "In Progress");
    assert_eq!(status_text(&[]), "");
}
