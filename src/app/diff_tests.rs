//! Tests for review-thread submit.

use super::*;
use super::test_support::sidebar_app;

#[test]
fn pending_submit_counts_threads_and_lands() {
    let mut app = sidebar_app(&[9]);
    let d = app.issues.data.get_mut(&9).unwrap();
    d.is_pr = true;
    d.review_threads = vec![crate::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(2),
        comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app.nav.current = Some(9);
    assert_eq!(app.pending_threads(), 1);
    // Landing a submit refetches so the Diff tab shows submitted threads.
    app.compose.submitting.insert(9);
    app.compose.review_tx.send((9, Ok(()))).unwrap();
    assert!(poll_review_submit(&mut app));
    assert!(!app.compose.submitting.contains(&9));
    assert_eq!(app.message, "pending review submitted on #9");
    assert!(app.issues.pending.contains(&9));
    // No pending threads means a note, never a send.
    let d = app.issues.data.get_mut(&9).unwrap();
    d.review_threads.clear();
    assert_eq!(app.pending_threads(), 0);
    app.submit_pending_review();
    assert!(app.compose.submitting.is_empty());
    assert_eq!(app.message, "no pending comments to submit");
}
