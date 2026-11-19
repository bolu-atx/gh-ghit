//! Tests for issue eviction and fetch queueing.

use super::*;
use super::test_support::{issue, land_sections, section_row, sidebar_app};

#[test]
fn evict_drops_row_and_cached_data_then_moves_cursor() {
    let mut app = sidebar_app(&[7, 9]);
    app.nav.current = Some(7);
    app.diff.expanded_rows.insert(7);
    app.diff.files.insert(7, vec![]);
    app.evict_current();
    assert!(!app.nav.visited.contains(&7));
    assert!(!app.issues.data.contains_key(&7), "data dropped");
    assert!(!app.issues.statuses.contains_key(&7), "status dropped");
    assert!(!app.diff.files.contains_key(&7), "diff dropped");
    assert!(!app.diff.expanded_rows.contains(&7), "expansion dropped");
    assert_eq!(app.nav.current, Some(9), "cursor falls to the next row");
    assert_eq!(app.issue_numbers(), vec![9]);
}

#[test]
fn evict_refuses_pinned_rows() {
    let mut app = sidebar_app(&[7]);
    app.nav.current = Some(7);
    app.nav.pinned.insert(7);
    app.nav.pin_order.push(7);
    app.evict_current();
    assert!(app.issues.data.contains_key(&7), "pinned row stays");
    assert!(app.nav.visited.contains(&7));
    assert_eq!(app.nav.current, Some(7));
    assert!(app.message.contains("pinned"), "{}", app.message);
}

#[test]
fn evict_falls_back_to_previous_row_at_the_end() {
    let mut app = sidebar_app(&[7, 9]);
    app.nav.current = Some(9);
    app.evict_current();
    assert_eq!(app.nav.current, Some(7));
    assert_eq!(app.issue_numbers(), vec![7]);
}

#[test]
fn switching_issue_moves_its_queued_fetch_up() {
    let mut app = App::new(Some("o/r".to_string()));
    app.fetches.hold = true;
    app.load_issue(1, false);
    app.load_issue(2, false);
    let view = fetch_queue::View { current: app.nav.requested, tab_shown: false };
    let order = app.fetches.queue_mut().order(&view);
    assert_eq!(order[0], (core::Kind::Details, Some(2)));
}

#[test]
fn evicted_row_can_be_refetched_on_revisit() {
    let mut app = sidebar_app(&[7, 9]);
    app.nav.current = Some(7);
    app.evict_current();
    assert!(!app.issues.data.contains_key(&7), "cache dropped so a revisit fetches");
    assert!(!app.issues.pending.contains(&7), "no stale in-flight marker");
    app.ensure_visited(7);
    assert!(app.issue_numbers().contains(&7), "row is listable again");
    // The refetch lands through the normal poll path.
    let fresh = issue(7, "2026-01-06T00:00:00Z");
    app.issues.tx.send((7, github::IssueEvent::Done(Ok(Box::new((fresh, vec![])))))).unwrap();
    poll_issues(&mut app);
    assert!(app.issues.data.contains_key(&7));
}

#[test]
fn slim_row_completes_on_open_with_refs_recomputed() {
    // Slim sections rows land partial with no body; opening one runs
    // the full fetch, which completes the row and recomputes refs.
    let mut app = sidebar_app(&[7]);
    let mut slim = issue(9, "2026-01-06T00:00:00Z");
    slim.body = String::new();
    land_sections(&mut app, vec![section_row(slim)]);
    assert!(app.issues.partial.contains(&9), "slim row lands partial");
    assert!(app.issues.data.get(&9).unwrap().body.is_empty());

    app.show_issue(9);
    assert!(app.issues.pending.contains(&9), "open runs the full fetch");
    assert!(app.refs.numbers.is_empty(), "no refs from an empty slim body");

    let mut full = issue(9, "2026-01-06T00:00:00Z");
    full.body = "see #7".to_string();
    full.paired = vec![model::PairedRef { number: 12, state: "OPEN".to_string() }];
    app.issues.tx.send((9, github::IssueEvent::Done(Ok(Box::new((full, vec![])))))).unwrap();
    poll_issues(&mut app);
    assert!(!app.issues.partial.contains(&9), "full fetch completes the row");
    assert_eq!(app.issues.data.get(&9).unwrap().body, "see #7");
    assert_eq!(app.refs.numbers, vec![7], "refs recompute from the full body");
    assert_eq!(app.refs.paired, vec![12]);
}
