//! Tests for review-thread submit.

use super::*;
use super::test_support::sidebar_app;
use ghit_core::model::ItemRef;

#[test]
fn pending_submit_counts_threads_and_lands() {
    let mut app = sidebar_app(&[ItemRef::github(9)]);
    let d = app.session.item_mut(ItemRef::github(9)).unwrap();
    d.is_pr = true;
    d.review_threads = vec![ghit_core::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(2),
        comments: vec![ghit_core::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app.nav.current = Some(ItemRef::github(9));
    assert_eq!(app.pending_threads(), 1);
    // Landing a submit refetches so the Diff tab shows submitted threads.
    app.session.sends_test().submitting.insert(ItemRef::github(9));
    app.session
        .sends_test()
        .review_tx
        .send((ItemRef::github(9), Ok(())))
        .unwrap();
    assert!(poll_session(&mut app));
    assert!(!app.session.sends_test().submitting.contains(&ItemRef::github(9)));
    assert_eq!(app.message, "pending review submitted on #9");
    assert!(app.session.issues_test().pending.contains(&ItemRef::github(9)));
    // No pending threads means a note, never a send.
    let d = app.session.item_mut(ItemRef::github(9)).unwrap();
    d.review_threads.clear();
    assert_eq!(app.pending_threads(), 0);
    app.submit_pending_review();
    assert!(app.session.sends_test().submitting.is_empty());
    assert_eq!(app.message, "no pending comments to submit");
}
