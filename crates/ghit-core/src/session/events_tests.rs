//! Headless session tests: results land through `poll` as events, no terminal.

use super::*;
use crate::cache::test_env_lock;
use crate::model::IssueData;

fn item(n: u64) -> ItemRef {
    ItemRef::github(n)
}

fn issue(n: ItemRef, title: &str) -> IssueData {
    IssueData { number: n, title: title.to_string(), ..Default::default() }
}

fn with_xdg<T>(f: impl FnOnce() -> T) -> T {
    let _guard = test_env_lock();
    let dir = tempfile::TempDir::new().unwrap();
    let saved = std::env::var("XDG_DATA_HOME").ok();
    std::env::set_var("XDG_DATA_HOME", dir.path());
    let out = f();
    match saved {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
    out
}

#[test]
fn load_marks_the_item_pending_and_a_landed_fetch_reports_it() {
    let mut session = Session::new(Some("o/r".to_string()));
    let n = item(7);
    session.fetches.hold = true;
    session.load(n, false, &View::default());
    assert!(session.issues.pending.contains(&n));
    session.load(n, false, &View::default());
    assert_eq!(session.fetches.queue_mut().len(), 1, "a second load while pending queues nothing");

    session.issues.tx.send((n, github::IssueEvent::Progress { done: 1, total: 3 })).unwrap();
    session.issues.tx.send((n, github::IssueEvent::Done(Ok(Box::new((issue(n, "landed"), vec![])))))).unwrap();
    let events = session.poll();
    assert!(matches!(events[0], SessionEvent::IssueProgress(m) if m == n));
    assert!(matches!(events[1], SessionEvent::IssueLoaded(m) if m == n));
    assert_eq!(session.issues.data[&n].title, "landed");
    assert!(session.issues.pending.is_empty() && session.issues.progress.is_empty());
    assert!(session.poll().is_empty(), "events drain once");
}

#[test]
fn a_failed_fetch_reports_the_error_and_clears_pending() {
    let mut session = Session::new(None);
    let n = item(9);
    session.issues.pending.insert(n);
    session.issues.tx.send((n, github::IssueEvent::Done(Err("boom".to_string())))).unwrap();
    let events = session.poll();
    assert!(matches!(&events[0], SessionEvent::IssueFailed { number, error } if *number == n && error == "boom"));
    assert!(session.issues.pending.is_empty());
    assert!(!session.issues.data.contains_key(&n));
}

#[test]
fn a_full_fetch_keeps_the_parent_the_listing_carried() {
    let mut session = Session::new(None);
    let (n, parent) = (item(2), item(1));
    let mut row = issue(n, "row");
    row.parent = Some(parent);
    session.issues.data.insert(n, row);
    session.issues.partial.insert(n);
    session.issues.tx.send((n, github::IssueEvent::Done(Ok(Box::new((issue(n, "full"), vec![])))))).unwrap();
    session.poll();
    assert_eq!(session.issues.data[&n].parent, Some(parent));
    assert!(!session.issues.partial.contains(&n));
}

#[test]
fn pins_toggle_on_and_off_and_persist() {
    with_xdg(|| {
        let mut session = Session::new(Some("o/r".to_string()));
        let n = item(5);
        assert_eq!(session.toggle_pin(n), Ok(true));
        assert!(session.pins.set.contains(&n));
        assert_eq!(session.pins.order, vec![n]);
        assert_eq!(github::load_pins("o/r"), vec![n]);

        let mut fresh = Session::new(Some("o/r".to_string()));
        fresh.load_pins("o/r", &[]);
        assert_eq!(fresh.pins.order, vec![n]);

        assert_eq!(session.toggle_pin(n), Ok(false));
        assert!(session.pins.set.is_empty() && session.pins.order.is_empty());
        assert!(github::load_pins("o/r").is_empty());
    });
}

#[test]
fn pinning_needs_a_repo() {
    let mut session = Session::new(None);
    assert!(session.toggle_pin(item(1)).is_err());
    assert!(session.pins.set.is_empty());
}

#[test]
fn startup_pins_skip_the_items_asked_for() {
    with_xdg(|| {
        github::save_pins("o/r", &[item(1), item(2)]);
        let mut session = Session::new(Some("o/r".to_string()));
        session.load_pins("o/r", &[item(1)]);
        assert_eq!(session.pins.order, vec![item(2)]);
    });
}

#[test]
fn a_section_listing_lands_as_an_event_and_resets_the_feed() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut session = Session::new(None);
    session.sections.rx = Some(rx);
    session.sections.replace = true;
    tx.send(github::SectionEvent::Progress { done: 1, total: 2 }).unwrap();
    assert!(matches!(session.poll()[0], SessionEvent::SectionsProgress));
    assert_eq!(session.sections.progress, Some((1, 2)));
    tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
    let events = session.poll();
    assert!(matches!(events[0], SessionEvent::SectionsDone { replace: true, auto: false, .. }));
    assert!(session.sections.rx.is_none() && session.sections.progress.is_none() && !session.sections.replace);
}

#[test]
fn the_cap_notice_fires_once() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut session = Session::new(None);
    session.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Capped).unwrap();
    assert!(matches!(session.poll()[0], SessionEvent::SectionsCapped));
    tx.send(github::SectionEvent::Capped).unwrap();
    assert!(session.poll().is_empty());
}

#[test]
fn sub_issue_children_land_with_their_parent_set() {
    let mut session = Session::new(None);
    let (parent, child) = (item(1), item(2));
    session.subs.tx.send((parent, Ok(vec![issue(child, "kid")]))).unwrap();
    assert!(matches!(session.poll()[0], SessionEvent::SubIssuesLoaded(p) if p == parent));
    assert_eq!(session.issues.data[&child].parent, Some(parent));
    assert!(session.issues.partial.contains(&child));
    assert!(matches!(session.subs.state[&parent], LazyChildren::Loaded(ref v) if v == &[child]));
}
