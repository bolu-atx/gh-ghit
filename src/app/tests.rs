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
fn tab_click_columns_follow_title_widths() {
    let first = TABS[0].1.chars().count() as u16 + 1;
    let second = first + TABS[1].1.chars().count() as u16 + 1;
    assert_eq!(tab_at(0), Tab::Details);
    assert_eq!(tab_at(first - 1), Tab::Details, "the divider after a title selects it");
    assert_eq!(tab_at(first), Tab::Visual);
    assert_eq!(tab_at(second - 1), Tab::Visual);
    assert_eq!(tab_at(second), Tab::Map);
    assert_eq!(tab_at(200), Tab::Map, "the last tab takes the rest of the line");
}
