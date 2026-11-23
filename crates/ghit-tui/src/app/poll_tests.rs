//! Tests for landing background fetches.

use super::*;
use super::test_support::{issue, land_sections, restore_xdg, section_row, sidebar_app, temp_xdg};
use ghit_core::model::Author;
use ghit_core::model::ItemRef;

#[test]
fn issue_progress_rides_the_issue_channel() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.issues.pending.insert(ItemRef::github(9));
    app.issues
        .tx
        .send((
            ItemRef::github(9),
            github::IssueEvent::Progress { done: 2, total: 7 },
        ))
        .unwrap();
    poll_issues(&mut app);
    assert_eq!(app.issues.progress.get(&ItemRef::github(9)), Some(&(2, 7)));
    assert!(
        app.issues.pending.contains(&ItemRef::github(9)),
        "progress is not completion"
    );
    let fresh = issue(ItemRef::github(9), "2026-01-06T00:00:00Z");
    app.issues
        .tx
        .send((
            ItemRef::github(9),
            github::IssueEvent::Done(Ok(Box::new((fresh, vec![])))),
        ))
        .unwrap();
    poll_issues(&mut app);
    assert!(
        !app.issues.progress.contains_key(&ItemRef::github(9)),
        "terminal result clears progress"
    );
    assert!(!app.issues.pending.contains(&ItemRef::github(9)));
    assert!(app.issues.data.contains_key(&ItemRef::github(9)));
}

#[test]
fn full_fetch_keeps_sub_issue_fields_from_the_sections_row() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let mut row = issue(ItemRef::github(9), "2026-01-05T00:00:00Z");
    row.parent = Some(ItemRef::github(3));
    row.sub_summary = Some(model::SubSummary { total: 2, completed: 1 });
    app.issues.data.insert(ItemRef::github(9), row);
    app.issues.pending.insert(ItemRef::github(9));
    let fresh = issue(ItemRef::github(9), "2026-01-06T00:00:00Z");
    app.issues
        .tx
        .send((
            ItemRef::github(9),
            github::IssueEvent::Done(Ok(Box::new((fresh, vec![])))),
        ))
        .unwrap();
    poll_issues(&mut app);
    let data = &app.issues.data[&ItemRef::github(9)];
    assert_eq!(data.updated_at, "2026-01-06T00:00:00Z", "full fetch wins");
    assert_eq!(data.parent, Some(ItemRef::github(3)), "parent survives the REST fetch");
    assert_eq!(data.sub_summary, Some(model::SubSummary { total: 2, completed: 1 }));
}

#[test]
fn startup_queues_the_requested_issue_before_pins_and_sections() {
    let (_guard, _dir, saved) = temp_xdg();
    github::save_pins("o/r", &[ItemRef::github(3)]);
    let mut app = App::new(Some("o/r".to_string()));
    app.fetches.hold = true;
    app.initial_numbers = vec![ItemRef::github(5)];
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.bootstrap_rx = Some(rx);
    tx.send(Some("o/r".to_string())).unwrap();
    assert!(poll_bootstrap(&mut app));
    let view = fetch_queue::View { current: app.nav.requested, tab_shown: false };
    let order = app.fetches.queue_mut().order(&view);
    assert_eq!(
        order,
        vec![
            (core::Kind::Details, Some(ItemRef::github(5))),
            (core::Kind::Sections, None),
            (core::Kind::Details, Some(ItemRef::github(3))),
            (core::Kind::Other, None),
        ],
        "current issue, sections, then pins and repo meta"
    );
    assert_eq!(
        app.nav.requested,
        Some(ItemRef::github(5)),
        "pins never take over the main pane"
    );
    restore_xdg(saved);
}

#[test]
fn section_landing_warns_at_row_cap() {
    let mut app = sidebar_app(&[ItemRef::github(1)]);
    let rows = (1..=500).map(|number| {
        section_row(issue(ItemRef::github(number), "2026-01-06T00:00:00Z"))
    }).collect();
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Capped).unwrap();
    tx.send(github::SectionEvent::Done(Ok(rows))).unwrap();
    poll_sections(&mut app);
    assert!(app.message.contains("500 row cap"));
    assert!(app.message.contains("narrow"));
    poll_sections(&mut app);
    assert!(app.sections.rx.is_none(), "the capped fetch still lands its rows");
    // Auto-refresh repeats the same setting: the notice shows once.
    app.message.clear();
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Capped).unwrap();
    poll_sections(&mut app);
    assert!(app.message.is_empty(), "no repeat for the same fetch setting");
    // A user window change re-arms the notice.
    app.reload_recent();
    assert!(!app.sections.cap_notified);
}

#[test]
fn section_landing_refreshes_known_row_display_fields() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    {
        let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
        d.title = "stale title".to_string();
        d.state = "OPEN".to_string();
        d.labels = vec![model::Label { name: "stale".to_string() }];
    }
    let mut fresh = issue(ItemRef::github(7), "2026-01-06T00:00:00Z");
    fresh.title = "fresh title".to_string();
    fresh.state = "CLOSED".to_string();
    fresh.labels = vec![model::Label { name: "bug".to_string() }];
    let mut row = section_row(fresh);
    row.statuses = vec![("Roadmap".to_string(), Some("Done".to_string()))];
    row.detail = model::BoardDetail {
        created: "2025-12-01".to_string(),
        updated: "2026-01-06".to_string(),
        comments: 4,
    };
    land_sections(&mut app, vec![row]);

    let d = app.issues.data.get(&ItemRef::github(7)).unwrap();
    assert_eq!(d.title, "fresh title", "stale title refreshes");
    assert_eq!(d.state, "CLOSED", "stale state refreshes");
    assert_eq!(
        d.labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        vec!["bug"]
    );
    assert_eq!(
        app.issues.statuses.get(&ItemRef::github(7)),
        Some(&vec![("Roadmap".to_string(), Some("Done".to_string()))])
    );
    assert_eq!(
        app.board
            .details
            .get(&ItemRef::github(7))
            .map(|d| d.comments),
        Some(4)
    );
    assert!(
        app.nav.visited.contains(&ItemRef::github(7)),
        "group membership is untouched"
    );
    assert!(
        !app.nav.recent.contains(&ItemRef::github(7)),
        "known rows never re-file as recent"
    );
}

#[test]
fn section_landing_keeps_visited_body_comments_and_threads() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    {
        let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
        d.body = "full body".to_string();
        d.comments = vec![model::Comment {
            author: Author { login: "reviewer".to_string() },
            body: "full comment".to_string(),
            created_at: "2026-01-02T00:00:00Z".to_string(),
        }];
        d.review_threads = vec![model::ReviewThread {
            id: "t1".to_string(),
            path: "a.rs".to_string(),
            line: Some(2),
            ..Default::default()
        }];
    }
    let mut fresh = issue(ItemRef::github(7), "2026-01-06T00:00:00Z");
    fresh.title = "renamed".to_string();
    fresh.body = "listing body".to_string();
    fresh.review_threads = vec![];
    land_sections(&mut app, vec![section_row(fresh)]);

    let d = app.issues.data.get(&ItemRef::github(7)).unwrap();
    assert_eq!(d.title, "renamed", "display fields still refresh");
    assert_eq!(d.body, "full body", "full fetch body wins");
    assert_eq!(d.comments.len(), 1);
    assert_eq!(d.comments[0].body, "full comment");
    assert_eq!(d.review_threads.len(), 1);
    assert_eq!(d.review_threads[0].id, "t1");
}

#[test]
fn an_older_listing_row_never_overwrites_a_fresher_fetch() {
    // The full fetch saw the close; the listing (search index lag or a
    // cached page) still says open with an older updated_at.
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    {
        let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
        d.state = "CLOSED".to_string();
        d.updated_at = "2026-01-06T00:00:00Z".to_string();
    }
    land_sections(
        &mut app,
        vec![section_row(issue(
            ItemRef::github(7),
            "2026-01-05T00:00:00Z",
        ))],
    );
    assert_eq!(
        app.issues.data[&ItemRef::github(7)].state,
        "CLOSED",
        "stale listing loses"
    );

    let mut reopened = issue(ItemRef::github(7), "2026-01-07T00:00:00Z");
    reopened.state = "OPEN".to_string();
    land_sections(&mut app, vec![section_row(reopened)]);
    assert_eq!(
        app.issues.data[&ItemRef::github(7)].state,
        "OPEN",
        "a newer listing still wins"
    );
}

#[test]
fn section_landing_evicts_unvisited_rows_missing_from_the_window() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.issues.data.insert(
        ItemRef::github(9),
        issue(ItemRef::github(9), "2026-01-05T00:00:00Z"),
    );
    app.issues.statuses.insert(ItemRef::github(9), vec![]);
    app.board
        .details
        .insert(ItemRef::github(9), model::BoardDetail::default());
    app.issues.partial.insert(ItemRef::github(9));
    app.nav.recent.push(ItemRef::github(9));
    // The fresh window carries a new row and the visited row; 9 is gone.
    land_sections(
        &mut app,
        vec![section_row(issue(
            ItemRef::github(11),
            "2026-01-06T00:00:00Z",
        ))],
    );

    assert!(
        !app.issues.data.contains_key(&ItemRef::github(9)),
        "vanished row data dropped"
    );
    assert!(!app.issues.statuses.contains_key(&ItemRef::github(9)));
    assert!(!app.board.details.contains_key(&ItemRef::github(9)));
    assert!(!app.issues.partial.contains(&ItemRef::github(9)));
    assert!(!app.nav.recent.contains(&ItemRef::github(9)));
    assert!(
        app.issues.data.contains_key(&ItemRef::github(7)),
        "visited row survives its absence"
    );
    assert!(app.nav.visited.contains(&ItemRef::github(7)));
    assert!(
        app.nav.recent.contains(&ItemRef::github(11)),
        "new window rows list as recent"
    );
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(7), ItemRef::github(11)]
    );
}

#[test]
fn search_landing_keeps_replace_semantics() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "full body".to_string();
    let mut stale = issue(ItemRef::github(9), "2026-01-05T00:00:00Z");
    stale.body = "stale body".to_string();
    app.issues.data.insert(ItemRef::github(9), stale);
    app.issues.statuses.insert(ItemRef::github(9), vec![]);
    app.issues.partial.insert(ItemRef::github(9));
    app.nav.recent.push(ItemRef::github(9));
    app.sections.replace = true;

    let mut hit = issue(ItemRef::github(9), "2026-01-06T00:00:00Z");
    hit.title = "search hit".to_string();
    hit.body = "search body".to_string();
    land_sections(&mut app, vec![section_row(hit)]);

    assert!(!app.sections.replace, "replace flag is consumed");
    assert_eq!(
        app.issues.data.get(&ItemRef::github(9)).unwrap().body,
        "search body",
        "search rows replace cached data instead of refreshing display fields"
    );
    assert_eq!(
        app.nav
            .recent
            .iter()
            .filter(|n| **n == ItemRef::github(9))
            .count(),
        1,
        "dedup holds"
    );
    let visited = app
        .issues
        .data
        .get(&ItemRef::github(7))
        .expect("visited row survives an absent search result");
    assert_eq!(visited.body, "full body", "visited body is never clobbered");
}

#[test]
fn section_landing_keeps_current_and_scroll_when_rows_vanish() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    // The open row was reached by cursor walk, so it is current but
    // never filed as visited.
    app.nav.current = Some(ItemRef::github(9));
    app.nav.scroll = 12;
    app.nav.hover = Some(ItemRef::github(9));
    app.issues.data.insert(
        ItemRef::github(9),
        issue(ItemRef::github(9), "2026-01-05T00:00:00Z"),
    );
    app.issues.statuses.insert(ItemRef::github(9), vec![]);
    app.issues.partial.insert(ItemRef::github(9));
    app.nav.recent.push(ItemRef::github(9));
    land_sections(
        &mut app,
        vec![section_row(issue(
            ItemRef::github(7),
            "2026-01-06T00:00:00Z",
        ))],
    );

    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(9)),
        "current survives leaving the window"
    );
    assert!(
        app.issues.data.contains_key(&ItemRef::github(9)),
        "open row keeps its data"
    );
    assert!(app.nav.recent.contains(&ItemRef::github(9)));
    assert_eq!(app.nav.scroll, 12);
    assert_eq!(app.nav.hover, Some(ItemRef::github(9)));
}

#[test]
fn section_progress_rides_the_section_channel() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Progress { done: 120, total: 500 }).unwrap();
    poll_sections(&mut app);
    assert_eq!(app.sections.progress, Some((120, 500)));
    assert!(app.sections.rx.is_some(), "progress keeps the receiver live");
    tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
    poll_sections(&mut app);
    assert_eq!(app.sections.progress, None);
    assert!(app.sections.rx.is_none(), "terminal result releases the receiver");
}

#[test]
fn auto_refresh_failures_notify_instead_of_going_silent() {
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    app.sections.auto_refresh = true;
    tx.send(github::SectionEvent::Done(Err("GitHub request failed: 403".to_string()))).unwrap();
    poll_sections(&mut app);
    assert!(app.sections.rx.is_none(), "terminal result clears the channel");
    assert!(app.message.contains("403"), "auto error surfaces: {}", app.message);
}

#[test]
fn toggling_a_list_filters_locally_without_a_fetch() {
    let mut app = sidebar_app(&[ItemRef::github(1)]);
    app.fetches.hold = true;
    app.toggle_list(1);
    assert!(!app.sidebar.show_merged_prs);
    assert_eq!(app.show_mask(), [true, false, true, true]);
    assert!(app.sections.rx.is_none(), "toggle sends no request");
    let view = fetch_queue::View { current: app.nav.requested, tab_shown: false };
    assert!(app.fetches.queue_mut().order(&view).is_empty());
}
