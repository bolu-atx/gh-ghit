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

fn row(n: ItemRef, updated: &str) -> github::Row {
    let mut data = issue(n, "row");
    data.updated_at = updated.to_string();
    github::Row { issue: data, statuses: vec![], detail: Default::default(), complete: true, window_at: None }
}

fn ctx(recent: Vec<ItemRef>) -> PollCtx {
    PollCtx {
        now: Instant::now(),
        searching: false,
        show: [true; 4],
        idle: false,
        view: View::default(),
        held: Held { recent, keep: HashSet::new() },
    }
}

#[test]
fn a_refresh_lands_new_rows_and_drops_vanished_background_rows() {
    let mut session = Session::new(Some("o/r".to_string()));
    session.fetches.hold = true;
    let (stay, gone, fresh) = (item(1), item(2), item(3));
    session.issues.data.insert(stay, issue(stay, "old"));
    session.issues.data.insert(gone, issue(gone, "gone"));
    let (tx, rx) = std::sync::mpsc::channel();
    session.sections.rx = Some(rx);
    tx.send(github::SectionEvent::Done(Ok(vec![row(stay, "2026-02-01"), row(fresh, "2026-02-01")]))).unwrap();

    let events = session.poll_frame(&ctx(vec![stay, gone]));
    let landed = events.iter().find_map(|e| match e {
        SessionEvent::RowsLanded { landed, auto } => Some((landed, *auto)),
        _ => None,
    });
    let (landed, auto) = landed.expect("rows landed");
    assert!(!auto);
    assert_eq!(landed.filed, vec![fresh]);
    assert_eq!(landed.gone, vec![gone]);
    assert!(session.issues.data.contains_key(&fresh) && !session.issues.data.contains_key(&gone));
    assert!(session.sections.rx.is_none());
}

#[test]
fn a_full_item_fetch_wins_over_a_listing_row_landing_in_the_same_frame() {
    let mut session = Session::new(None);
    let n = item(4);
    session.issues.data.insert(n, issue(n, "listing"));
    let (tx, rx) = std::sync::mpsc::channel();
    session.sections.rx = Some(rx);
    let mut stale = row(n, "2099-01-01");
    stale.issue.title = "stale row".to_string();
    tx.send(github::SectionEvent::Done(Ok(vec![stale]))).unwrap();
    session.issues.tx.send((n, github::IssueEvent::Done(Ok(Box::new((issue(n, "full"), vec![])))))).unwrap();
    session.poll_frame(&ctx(vec![n]));
    assert_eq!(session.issues.data[&n].title, "full");
}

#[test]
fn item_fetches_wait_a_frame_while_the_front_end_lands_replace_rows() {
    let mut session = Session::new(None);
    let n = item(5);
    let (tx, rx) = std::sync::mpsc::channel();
    session.sections.rx = Some(rx);
    session.sections.replace = true;
    tx.send(github::SectionEvent::Done(Ok(vec![row(n, "2099-01-01")]))).unwrap();
    session.issues.tx.send((n, github::IssueEvent::Done(Ok(Box::new((issue(n, "full"), vec![])))))).unwrap();
    let first = session.poll_frame(&ctx(vec![]));
    assert!(first.iter().any(|e| matches!(e, SessionEvent::SectionsDone { replace: true, .. })));
    assert!(!first.iter().any(|e| matches!(e, SessionEvent::IssueLoaded(_))), "items wait for the front end's rows");
    let second = session.poll_frame(&ctx(vec![]));
    assert!(second.iter().any(|e| matches!(e, SessionEvent::IssueLoaded(m) if *m == n)));
    assert_eq!(session.issues.data[&n].title, "full");
}

#[test]
fn a_diff_load_marks_pending_and_lands_files() {
    let mut session = Session::new(Some("o/r".to_string()));
    session.fetches.hold = true;
    let n = item(8);
    let mut pr = issue(n, "pr");
    pr.is_pr = true;
    session.issues.data.insert(n, pr);
    session.load_diff(n, false, &View::default());
    assert!(session.diff.pending.contains(&n));
    session.load_diff(n, false, &View::default());
    assert_eq!(session.fetches.queue_mut().len(), 1, "a pending diff queues nothing more");

    session.diff.tx.send((n, Ok(vec![]))).unwrap();
    let events = session.poll();
    assert!(matches!(events[0], SessionEvent::DiffLoaded(m) if m == n));
    assert!(session.diff.files.contains_key(&n) && session.diff.raw.contains_key(&n));
    assert!(session.diff.pending.is_empty());
}

#[test]
fn a_diff_load_skips_plain_issues() {
    let mut session = Session::new(None);
    let n = item(9);
    session.issues.data.insert(n, issue(n, "issue"));
    session.load_diff(n, false, &View::default());
    assert!(session.diff.pending.is_empty());
}

#[test]
fn a_state_change_result_lands_as_an_event() {
    let mut session = Session::new(None);
    let n = item(6);
    session.sends.state_tx.send((n, true, Ok(()))).unwrap();
    session.sends.state_tx.send((n, false, Err("nope".to_string()))).unwrap();
    let events = session.poll();
    assert!(matches!(events[0], SessionEvent::StateChanged { number, open: true, result: Ok(()) } if number == n));
    assert!(matches!(&events[1], SessionEvent::StateChanged { open: false, result: Err(e), .. } if e == "nope"));
}

#[test]
fn a_review_submit_locks_its_pr_until_the_result_lands() {
    let mut session = Session::new(None);
    let n = item(5);
    assert!(session.submit_review(n));
    assert!(!session.submit_review(n), "one submit per PR");
    session.sends.review_tx.send((n, Ok(()))).unwrap();
    let events = session.poll();
    assert!(matches!(events[0], SessionEvent::ReviewSubmitted { number, .. } if number == n));
    assert!(session.sends.submitting.is_empty());
}

#[test]
fn a_sent_draft_reports_its_result() {
    let mut session = Session::new(None);
    let n = item(3);
    session.sends.tx.send((n, Err("down".to_string()))).unwrap();
    let events = session.poll();
    assert!(matches!(&events[0], SessionEvent::DraftSent { number, result: Err(e) } if *number == n && e == "down"));
}

#[test]
fn board_results_land_with_their_details() {
    let mut session = Session::new(None);
    let n = item(7);
    session.board.loading = true;
    let (tx, rx) = std::sync::mpsc::channel();
    session.board.rx = Some(rx);
    tx.send(Err("no board".to_string())).unwrap();
    let events = session.poll();
    assert!(matches!(&events[0], SessionEvent::BoardFailed(e) if e == "no board"));
    assert!(!session.board.loading && session.board.rx.is_none());
    session.board.detail_pending.insert(n);
    session.board.detail_tx.send((n, Ok(Default::default()))).unwrap();
    assert!(matches!(session.poll()[0], SessionEvent::BoardDetail(m) if m == n));
    assert!(session.board.details.contains_key(&n) && session.board.detail_pending.is_empty());
}

#[test]
fn only_known_github_prs_bypass_the_issue_backend() {
    let mut session = Session::new(None);
    assert!(session.issue_backend.shares_github_sections());
    let number = item(1);
    assert!(!session.is_github_pr(number), "unknown numbers use issue-or-PR discovery");
    session.issues.data.insert(number, issue(number, "issue"));
    assert!(!session.is_github_pr(number));
    session.issues.data.get_mut(&number).unwrap().is_pr = true;
    assert!(session.is_github_pr(number));
    let jira = ItemRef { source: Source::Jira, number: 1 };
    let mut data = issue(jira, "jira");
    data.is_pr = true;
    session.issues.data.insert(jira, data);
    assert!(!session.is_github_pr(jira), "source takes precedence over a PR flag");
}
