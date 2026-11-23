//! Tests for issue eviction and fetch queueing.

use ghit_core::github;
use super::*;
use super::test_support::{issue, land_sections, section_row, sidebar_app};
use ghit_core::model::ItemRef;

#[test]
fn evict_drops_row_and_cached_data_then_moves_cursor() {
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9)]);
    app.nav.current = Some(ItemRef::github(7));
    app.diff.expanded_rows.insert(ItemRef::github(7));
    app.session.diff.files.insert(ItemRef::github(7), vec![]);
    app.evict_current();
    assert!(!app.nav.visited.contains(&ItemRef::github(7)));
    assert!(
        !app.session.issues.data.contains_key(&ItemRef::github(7)),
        "data dropped"
    );
    assert!(
        !app.session.issues.statuses.contains_key(&ItemRef::github(7)),
        "status dropped"
    );
    assert!(
        !app.session.diff.files.contains_key(&ItemRef::github(7)),
        "diff dropped"
    );
    assert!(
        !app.diff.expanded_rows.contains(&ItemRef::github(7)),
        "expansion dropped"
    );
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(9)),
        "cursor falls to the next row"
    );
    assert_eq!(app.issue_numbers(), vec![ItemRef::github(9)]);
}

#[test]
fn evict_refuses_pinned_rows() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.nav.current = Some(ItemRef::github(7));
    app.session.pins.set.insert(ItemRef::github(7));
    app.session.pins.order.push(ItemRef::github(7));
    app.evict_current();
    assert!(
        app.session.issues.data.contains_key(&ItemRef::github(7)),
        "pinned row stays"
    );
    assert!(app.nav.visited.contains(&ItemRef::github(7)));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.message.contains("pinned"), "{}", app.message);
}

#[test]
fn evict_falls_back_to_previous_row_at_the_end() {
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9)]);
    app.nav.current = Some(ItemRef::github(9));
    app.evict_current();
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert_eq!(app.issue_numbers(), vec![ItemRef::github(7)]);
}

#[test]
fn switching_issue_moves_its_queued_fetch_up() {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.fetches.hold = true;
    app.load_issue(ItemRef::github(1), false);
    app.load_issue(ItemRef::github(2), false);
    let view = fetch_queue::View { current: app.nav.requested, tab_shown: false };
    let order = app.session.fetches.queue_mut().order(&view);
    assert_eq!(order[0], (core::Kind::Details, Some(ItemRef::github(2))));
}

#[test]
fn evicted_row_can_be_refetched_on_revisit() {
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9)]);
    app.nav.current = Some(ItemRef::github(7));
    app.evict_current();
    assert!(
        !app.session.issues.data.contains_key(&ItemRef::github(7)),
        "cache dropped so a revisit fetches"
    );
    assert!(
        !app.session.issues.pending.contains(&ItemRef::github(7)),
        "no stale in-flight marker"
    );
    app.ensure_visited(ItemRef::github(7));
    assert!(
        app.issue_numbers().contains(&ItemRef::github(7)),
        "row is listable again"
    );
    // The refetch lands through the normal poll path.
    let fresh = issue(ItemRef::github(7), "2026-01-06T00:00:00Z");
    app.session.issues
        .tx
        .send((
            ItemRef::github(7),
            github::IssueEvent::Done(Ok(Box::new((fresh, vec![])))),
        ))
        .unwrap();
    poll_session(&mut app);
    assert!(app.session.issues.data.contains_key(&ItemRef::github(7)));
}

#[test]
fn slim_row_completes_on_open_with_refs_recomputed() {
    // Slim sections rows land partial with no body; opening one runs
    // the full fetch, which completes the row and recomputes refs.
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let mut slim = issue(ItemRef::github(9), "2026-01-06T00:00:00Z");
    slim.body = String::new();
    land_sections(&mut app, vec![section_row(slim)]);
    assert!(
        app.session.issues.partial.contains(&ItemRef::github(9)),
        "slim row lands partial"
    );
    assert!(app
        .session
        .issues
        .data
        .get(&ItemRef::github(9))
        .unwrap()
        .body
        .is_empty());

    app.show_issue(ItemRef::github(9));
    assert!(app.session.issues.pending.contains(&ItemRef::github(9)), "open runs the full fetch");
    assert!(app.session.refs.numbers.is_empty(), "no refs from an empty slim body");

    let mut full = issue(ItemRef::github(9), "2026-01-06T00:00:00Z");
    full.body = "see #7".to_string();
    full.paired = vec![model::PairedRef {
        number: ItemRef::github(12),
        state: "OPEN".to_string(),
    }];
    app.session.issues
        .tx
        .send((
            ItemRef::github(9),
            github::IssueEvent::Done(Ok(Box::new((full, vec![])))),
        ))
        .unwrap();
    poll_session(&mut app);
    assert!(
        !app.session.issues.partial.contains(&ItemRef::github(9)),
        "full fetch completes the row"
    );
    assert_eq!(
        app.session.issues.data.get(&ItemRef::github(9)).unwrap().body,
        "see #7"
    );
    assert_eq!(
        app.session.refs.numbers,
        vec![ItemRef::github(7)],
        "refs recompute from the full body"
    );
    assert_eq!(app.session.refs.paired, vec![ItemRef::github(12)]);
}
