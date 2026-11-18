//! Tests for `App` state at the root.

use super::*;
use super::test_support::sidebar_app;

#[test]
fn spin_owed_paces_spinner_repaints() {
    let mut app = sidebar_app(&[7]);
    assert!(!app.spin_owed(Instant::now()), "idle owes nothing");
    app.notify("loading".to_string());
    assert!(!app.spin_owed(Instant::now()), "fresh paint owes nothing");
    let stale = Instant::now() - Duration::from_millis(SPIN_FRAME_MS + 50);
    assert!(app.spin_owed(stale), "stale frame owes a repaint");
    app.message_at = Some(Instant::now() - Duration::from_secs(5));
    assert!(!app.spin_owed(stale), "expired message owes nothing");
}

#[test]
fn tab_split_matches_first_title_width() {
    let first = tab_titles("Diff View")[0].chars().count() as u16;
    assert_eq!(TAB_SPLIT_X, first + 2, "click split follows the first title plus divider");
}
