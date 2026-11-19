//! Tests for sidebar rows, sort, filters and auto-refresh.

use super::*;
use super::sidebar::{FULL_REFRESH_INTERVAL, RECENT_REFRESH_INTERVAL, RECENT_REFRESH_MAX_INTERVAL, STATUS_REFRESH_INTERVAL, refresh_interval_from, refresh_plan};
use super::test_support::{issue, restore_xdg, section_row, sidebar_app, temp_xdg};
use crate::model::BoardData;

/// Store rows as a sync that started `age_secs` ago.
fn put_sections(rows: Vec<crate::github::Row>, age_secs: i64) {
    let started = chrono::Utc::now() - chrono::TimeDelta::seconds(age_secs);
    crate::store::open("o/r").unwrap().write(&started.to_rfc3339(), &rows).unwrap();
}

fn cached_row(number: u64, complete: bool) -> crate::github::Row {
    crate::github::Row {
        issue: issue(number, "2026-01-05T00:00:00Z"),
        statuses: vec![],
        detail: crate::model::BoardDetail {
            created: "2026-01-01".to_string(),
            updated: "2026-01-05".to_string(),
            comments: 0,
        },
        complete,
        window_at: None,
    }
}

/// Add a background (recent) row with the fields the pills read.
fn add_recent(app: &mut App, n: u64, updated: &str, labels: &[&str], milestone: Option<&str>) {
    let mut d = issue(n, updated);
    d.labels = labels
        .iter()
        .map(|name| model::Label { name: name.to_string() })
        .collect();
    d.milestone = milestone.map(str::to_string);
    app.issues.data.insert(n, d);
    app.issues.statuses.insert(n, vec![]);
    app.nav.recent.push(n);
}

#[test]
fn startup_hydrates_sidebar_from_sections_cache() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut newer = cached_row(7, false);
    newer.issue.created_at = "2026-01-02T00:00:00Z".to_string();
    put_sections(vec![newer, cached_row(9, true)], 0);
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    app.hydrate_cached_sections();
    assert_eq!(app.nav.recent, vec![7, 9], "stored order (open rows newest created first) becomes sidebar order");
    assert!(app.issues.data.contains_key(&7) && app.issues.data.contains_key(&9));
    assert!(app.issues.partial.contains(&7), "slim rows stay partial so open runs the full fetch");
    assert!(!app.issues.partial.contains(&9));
    assert!(app.issues.statuses.contains_key(&7) && app.board.details.contains_key(&7));
    restore_xdg(saved);
}

#[test]
fn startup_hydration_leaves_present_rows_alone() {
    let (_guard, _dir, saved) = temp_xdg();
    put_sections(vec![cached_row(7, false), cached_row(9, false)], 0);
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let mut kept = issue(7, "2026-01-05T00:00:00Z");
    kept.title = "kept".to_string();
    app.issues.data.insert(7, kept);
    app.hydrate_cached_sections();
    assert_eq!(app.issues.data[&7].title, "kept", "CLI-opened rows win over the cache");
    assert!(app.issues.data.contains_key(&9));
    assert_eq!(app.nav.recent, vec![9], "present rows are not re-added to recent");
    restore_xdg(saved);
}

#[test]
fn startup_hydration_ignores_a_missing_store_but_paints_an_old_sync() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    app.hydrate_cached_sections();
    assert!(app.nav.recent.is_empty() && app.issues.data.is_empty(), "no store means no rows");
    // The store is the shared local copy: a process that does not hold
    // the sync lease reads it whatever its age.
    put_sections(vec![cached_row(7, false)], 3601);
    app.hydrate_cached_sections();
    assert_eq!(app.nav.recent, vec![7], "an old sync still paints");
    restore_xdg(saved);
}

/// A booted app on repo `o/r` with its store link open, as
/// `poll_bootstrap` leaves it; fetches stay queued.
fn linked_app() -> App {
    let mut app = sidebar_app(&[]);
    app.fetches.hold = true;
    app.sections.link = github::StoreLink::open("o/r");
    assert!(app.sections.link.is_some(), "store opens");
    app
}

#[test]
fn lease_holder_alone_auto_refreshes_the_listing() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder(), "one lease per repo");
    for app in [&mut holder, &mut other] {
        let start = app.sections.last_fetch;
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    }
    assert!(holder.sections.rx.is_some(), "the holder syncs");
    assert!(other.sections.rx.is_none(), "a non-holder sends no listing request");
    // Search mode stays per instance: a non-holder still refreshes its search.
    other.sidebar.query_active = "label:bug".to_string();
    let start = other.sections.last_fetch;
    other.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(other.sections.rx.is_some() && other.sections.replace);
    // A manual refresh takes the lease from any process.
    other.sidebar.query_active.clear();
    other.sections.rx = None;
    other.refresh_listing();
    assert!(other.sync_holder() && other.sections.rx.is_some(), "r takes the lease and walks");
    assert_eq!(other.sections.refresh, github::Refresh::Delta, "the Full plan was consumed by the spawn");
    restore_xdg(saved);
}

#[test]
fn another_process_sync_lands_through_the_store_watch() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = linked_app();
    app.sections.merged_window_hours = 168;
    let now = Instant::now();
    assert!(!app.watch_store(now), "nothing new yet");
    let mut merged = section_row(issue(4, "2026-01-05T00:00:00Z"));
    merged.issue.is_pr = true;
    merged.issue.state = "MERGED".to_string();
    merged.window_at = Some(chrono::Utc::now().to_rfc3339());
    put_sections(vec![cached_row(7, false), merged], 0);
    assert!(app.watch_store(now), "a sync by another connection lands");
    assert!(app.nav.recent.contains(&7) && app.nav.recent.contains(&4));
    assert!(!app.watch_store(now), "each sync lands once");
    // List toggles and the window filter the stored rows locally.
    app.toggle_list(1);
    assert!(!app.nav.recent.contains(&4), "hiding merged PRs drops the row");
    assert!(app.sections.rx.is_none(), "a toggle sends no request");
    app.toggle_list(1);
    assert!(app.nav.recent.contains(&4));
    app.set_window(1);
    assert!(app.nav.recent.contains(&4), "merged now is inside 1h");
    assert!(app.sections.rx.is_none(), "a window change sends no request");
    restore_xdg(saved);
}

#[test]
fn sort_cycle_orders_rows_and_restores_default() {
    let mut app = sidebar_app(&[7, 9, 11]);
    app.issues.data.get_mut(&7).unwrap().updated_at = "2026-01-05T00:00:00Z".to_string();
    app.issues.data.get_mut(&9).unwrap().updated_at = "2026-01-09T00:00:00Z".to_string();
    app.issues.data.get_mut(&11).unwrap().updated_at = "2026-01-01T00:00:00Z".to_string();
    assert_eq!(app.issue_numbers(), vec![7, 9, 11]);
    app.cycle_sort();
    assert_eq!(app.sidebar.sort, core::SideSort::Updated);
    assert_eq!(app.issue_numbers(), vec![9, 7, 11], "newest first");
    app.toggle_sort_dir();
    assert_eq!(app.issue_numbers(), vec![11, 7, 9], "direction flips");
    for _ in 0..4 {
        app.cycle_sort();
    }
    assert_eq!(app.sidebar.sort, core::SideSort::Default);
    assert_eq!(app.issue_numbers(), vec![7, 9, 11], "default order is back");
}

#[test]
fn sort_applies_to_the_prs_pane_too() {
    let mut app = sidebar_app(&[7]);
    for (n, updated) in [(13, "2026-01-03T00:00:00Z"), (15, "2026-01-08T00:00:00Z")] {
        let mut pr = issue(n, updated);
        pr.is_pr = true;
        app.issues.data.insert(n, pr);
        app.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    assert_eq!(app.pr_numbers(), vec![13, 15]);
    app.cycle_sort();
    assert_eq!(app.pr_numbers(), vec![15, 13], "PRs sort newest first");
    assert_eq!(app.issue_numbers(), vec![7], "panes stay separate");
}

#[test]
fn label_and_milestone_pills_narrow_recent_rows_only() {
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(&mut app, 1, "2026-01-05T00:00:00Z", &["bug"], Some("v1"));
    add_recent(&mut app, 2, "2026-01-04T00:00:00Z", &["docs"], Some("v2"));
    add_recent(&mut app, 3, "2026-01-03T00:00:00Z", &[], None);
    let query = "label:remote".to_string();
    app.sidebar.query_active = query.clone();
    assert_eq!(app.issue_numbers(), vec![1, 2, 3]);
    app.sidebar.label_filter.insert("bug".to_string());
    assert_eq!(app.issue_numbers(), vec![1], "label pill narrows");
    assert_eq!(app.sidebar.query_active, query, "filter code never edits the query");
    app.sidebar.label_filter.clear();
    app.sidebar.milestone_filter.insert(Some("v2".to_string()));
    assert_eq!(app.issue_numbers(), vec![2], "milestone pill narrows");
    assert_eq!(app.sidebar.query_active, query, "query still untouched");
    app.sidebar.milestone_filter.clear();
    app.sidebar.milestone_filter.insert(None);
    assert_eq!(app.issue_numbers(), vec![3], "(no milestone) pill narrows");
    app.sidebar.milestone_filter.insert(Some("v1".to_string()));
    assert_eq!(app.issue_numbers(), vec![1, 3], "milestone pills match any selected");
    // Kinds intersect: label AND milestone.
    app.sidebar.label_filter.insert("docs".to_string());
    assert!(app.issue_numbers().is_empty());
}

#[test]
fn pinned_and_requested_rows_skip_filters_and_grouping() {
    use crate::model::{GroupBy, SideRow::{Divider, Item}};
    let mut app = sidebar_app(&[1, 2]);
    app.nav.pin_order = vec![1];
    app.nav.pinned = [1].into_iter().collect();
    add_recent(&mut app, 3, "2026-01-05T00:00:00Z", &["bug"], Some("v1"));
    add_recent(&mut app, 4, "2026-01-04T00:00:00Z", &["bug"], Some("v1"));
    app.sidebar.label_filter.insert("bug".to_string());
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones =
        vec![model::Milestone { title: "v1".to_string(), due_on: None }];
    assert_eq!(
        app.issue_rows(),
        vec![
            Item(1),
            Divider("requested".to_string()),
            Item(2),
            Divider("v1 (2)".to_string()),
            Item(3),
            Item(4),
        ],
        "pins and requested rows stay on top, ungrouped and unfiltered"
    );
}

#[test]
fn sort_runs_inside_each_milestone_group() {
    use crate::model::{GroupBy, SideRow::{Divider, Item}};
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(&mut app, 3, "2026-01-03T00:00:00Z", &[], Some("v1"));
    add_recent(&mut app, 4, "2026-01-06T00:00:00Z", &[], Some("v1"));
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones =
        vec![model::Milestone { title: "v1".to_string(), due_on: None }];
    app.cycle_sort();
    assert_eq!(
        app.issue_rows(),
        vec![Divider("v1 (2)".to_string()), Item(4), Item(3)],
        "updated sort reorders inside the group"
    );
    assert_eq!(app.issue_numbers(), vec![4, 3], "j/k follows the grouped display");
}

#[test]
fn milestone_groups_order_dated_then_undated_then_none() {
    use crate::model::{GroupBy, SideRow::Divider};
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(&mut app, 1, "2026-01-05T00:00:00Z", &[], Some("later"));
    add_recent(&mut app, 2, "2026-01-05T00:00:00Z", &[], Some("sooner"));
    add_recent(&mut app, 3, "2026-01-05T00:00:00Z", &[], Some("backlog"));
    add_recent(&mut app, 4, "2026-01-05T00:00:00Z", &[], None);
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones = vec![
        model::Milestone { title: "later".to_string(), due_on: Some("2026-06-01T00:00:00Z".to_string()) },
        model::Milestone { title: "sooner".to_string(), due_on: Some("2026-02-01T00:00:00Z".to_string()) },
        model::Milestone { title: "backlog".to_string(), due_on: None },
    ];
    let dividers: Vec<String> = app.issue_rows().into_iter()
        .filter_map(|row| match row {
            Divider(label) => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(
        dividers,
        vec![
            "sooner (1)".to_string(),
            "later (1)".to_string(),
            "backlog (1)".to_string(),
            "(no milestone) (1)".to_string()
        ]
    );
}

#[test]
fn stale_saved_pills_prune_when_picker_lists_land() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    app.prefs_loaded = true;
    app.sidebar.label_filter = ["bug".to_string(), "gone".to_string()].into_iter().collect();
    app.sidebar.milestone_filter =
        [Some("v1".to_string()), Some("gone".to_string()), None].into_iter().collect();
    app.sidebar.format_focus = 99;
    app.land_repo_meta(Ok((
        vec!["bug".to_string()],
        vec![model::Milestone { title: "v1".to_string(), due_on: None }],
    )));
    assert!(app.sidebar.meta_loaded);
    assert_eq!(app.sidebar.label_filter, ["bug".to_string()].into_iter().collect());
    assert!(app.sidebar.milestone_filter.contains(&Some("v1".to_string())));
    assert!(app.sidebar.milestone_filter.contains(&None), "(no milestone) stays live");
    assert!(!app.sidebar.milestone_filter.contains(&Some("gone".to_string())));
    assert!(app.sidebar.format_focus < app.format_widgets(), "focus clamps");
    let back = github::load_prefs();
    let saved_filter = &back.sidebar_filters[&github::prefs_repo_key("o/r")];
    assert_eq!(saved_filter.labels, vec!["bug".to_string()], "pruned labels persist");
    assert!(saved_filter.milestones.contains(&None));
    restore_xdg(saved);
}

#[test]
fn four_selects_all_time_and_zero_resets_to_recent() {
    let mut app = sidebar_app(&[]);
    app.sidebar.query_active = "label:bug".to_string();
    app.sections.merged_window_hours = 168;
    app.set_window(0);
    assert_eq!(app.sections.merged_window_hours, 0);
    assert_eq!(app.window_label(), "open + merged all time");
    assert_eq!(app.sidebar.query_active, "label:bug", "window keeps the query");
    app.reset_recent_default();
    assert_eq!(app.sections.merged_window_hours, core::DEFAULT_WINDOW_HOURS);
    assert!(app.sidebar.query_active.is_empty(), "zero clears the query");
    assert!(app.sidebar.query_draft.is_empty());
}

#[test]
fn startup_restored_query_runs_search_instead_of_window_fetch() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut prefs = github::load_prefs();
    prefs.queries.insert(github::prefs_repo_key("o/r"), "label:bug".to_string());
    github::save_prefs(&prefs);
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.bootstrap_rx = Some(rx);
    tx.send(Some("o/r".to_string())).unwrap();
    assert!(poll_bootstrap(&mut app));
    assert_eq!(app.sidebar.query_active, "label:bug");
    assert!(app.sections.rx.is_some(), "restored query fetches from the server");
    assert!(app.sections.replace, "search results replace the listing");
    restore_xdg(saved);
}

#[test]
fn startup_without_saved_query_fetches_window_listing() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.bootstrap_rx = Some(rx);
    tx.send(Some("o/r".to_string())).unwrap();
    assert!(poll_bootstrap(&mut app));
    assert!(app.sidebar.query_active.is_empty());
    assert!(app.sections.rx.is_some(), "window listing still fetches");
    assert!(!app.sections.replace, "window listing appends");
    restore_xdg(saved);
}

#[test]
fn auto_refresh_interval_env_override() {
    assert_eq!(refresh_interval_from(None), RECENT_REFRESH_INTERVAL);
    assert_eq!(refresh_interval_from(Some("3600")), Duration::from_secs(3600));
    for bad in ["0", "nope", ""] {
        assert_eq!(refresh_interval_from(Some(bad)), RECENT_REFRESH_INTERVAL, "reject {bad}");
    }
}

#[test]
fn auto_refresh_fires_at_interval() {
    let mut app = sidebar_app(&[7]);
    let start = app.sections.last_fetch;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL - Duration::from_millis(1));
    assert!(app.sections.rx.is_none());
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.sections.rx.is_some());
    assert!(app.sections.auto_refresh);
}

#[test]
fn auto_refresh_guards_each_active_view() {
    for guard in 0..7 {
        let mut app = sidebar_app(&[7]);
        let (_tx, rx) = std::sync::mpsc::channel();
        match guard {
            0 => app.sections.rx = Some(rx),
            1 => { app.board.loading = true; app.open(crate::app::Entry::Board); }
            2 => { app.board.data = Some(BoardData { title: String::new(), order: vec![], items: vec![], url: String::new() }); app.open(crate::app::Entry::Board); }
            3 => app.open_draft(ComposeState { number: 7, target: ComposeTarget::Thread, body: String::new(), cursor: 0, sending: false, kill: String::new(), view: Default::default() }),
            4 => app.open(crate::app::Entry::Format(crate::app::FormatEdit::None)),
            5 => app.open(crate::app::Entry::Shortcuts),
            6 => app.nav.focus = Focus::Bar,
            _ => unreachable!(),
        }
        let start = app.sections.last_fetch;
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
        assert_eq!(app.sections.last_fetch, start, "guard {guard}");
        assert!(!app.sections.auto_refresh, "guard {guard}");
    }
}

#[test]
fn auto_refresh_failure_backs_off_and_success_resets() {
    let mut app = sidebar_app(&[7]);
    for expected in [60, 60] {
        let (tx, rx) = std::sync::mpsc::channel();
        app.sections.rx = Some(rx);
        app.sections.auto_refresh = true;
        tx.send(github::SectionEvent::Done(Err("offline".into()))).unwrap();
        poll_sections(&mut app);
        assert_eq!(app.sections.auto_refresh_interval, Duration::from_secs(expected));
    }
    let start = app.sections.last_fetch;
    app.auto_refresh_recent(start + Duration::from_secs(59));
    assert!(app.sections.rx.is_none());
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    app.sections.auto_refresh = true;
    tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
    poll_sections(&mut app);
    assert_eq!(app.sections.auto_refresh_interval, RECENT_REFRESH_INTERVAL);
}

#[test]
fn refresh_plan_picks_delta_status_or_full() {
    let secs = Duration::from_secs;
    assert_eq!(refresh_plan(secs(30), secs(30)), github::Refresh::Delta);
    assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL - secs(1), secs(60)), github::Refresh::Delta);
    assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL, secs(600)), github::Refresh::Status);
    assert_eq!(refresh_plan(secs(10), FULL_REFRESH_INTERVAL - secs(1)), github::Refresh::Delta);
    assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL, FULL_REFRESH_INTERVAL), github::Refresh::Full);
    assert_eq!(refresh_plan(secs(0), FULL_REFRESH_INTERVAL), github::Refresh::Full);
}

#[test]
fn auto_refresh_probes_until_a_status_refresh_is_due() {
    let mut app = sidebar_app(&[7]);
    app.repo_slug = Some("o/r".to_string());
    let start = app.sections.last_fetch;
    let status_clock = app.sections.last_status;
    let full_clock = app.sections.last_full;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.sections.rx.is_some());
    assert!(!app.sections.probe, "probe flag is consumed by the spawn");
    assert_eq!(app.sections.last_status, status_clock, "a change-only pass keeps the status clock");
    // Past the status span, the refresh runs without the probe.
    app.sections.rx = None;
    app.sections.last_status = status_clock - STATUS_REFRESH_INTERVAL;
    let late = start + RECENT_REFRESH_INTERVAL * 2;
    app.sections.last_fetch = late - RECENT_REFRESH_INTERVAL;
    app.auto_refresh_recent(late);
    assert!(app.sections.rx.is_some());
    assert!(app.sections.last_status > status_clock, "status clock restarts");
    assert_eq!(app.sections.last_full, full_clock, "a status pass is not a full walk");
    app.sections.rx = None;
}

#[test]
fn unchanged_probe_keeps_rows_and_resets_backoff() {
    let mut app = sidebar_app(&[7]);
    let before = app.nav.recent.clone();
    let full = app.sections.last_full;
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    app.sections.auto_refresh = true;
    app.sections.auto_refresh_interval = RECENT_REFRESH_MAX_INTERVAL;
    tx.send(github::SectionEvent::Unchanged).unwrap();
    poll_sections(&mut app);
    assert!(app.sections.rx.is_none());
    assert!(!app.sections.auto_refresh);
    assert_eq!(app.sections.auto_refresh_interval, RECENT_REFRESH_INTERVAL);
    assert_eq!(app.nav.recent, before, "no fetch ran, rows stay put");
    assert_eq!(app.sections.last_full, full, "a probe is not a full fetch");
}

#[test]
fn auto_refresh_landing_preserves_view() {
    for current in [Some(7), None] {
        let mut app = sidebar_app(&[7]);
        app.nav.current = current;
        app.nav.scroll = 12;
        app.nav.hover = Some(7);
        let (tx, rx) = std::sync::mpsc::channel();
        app.sections.rx = Some(rx);
        app.sections.auto_refresh = true;
        let row = crate::github::Row { issue: issue(9, "2026-01-06T00:00:00Z"), statuses: vec![], detail: model::BoardDetail::default(), complete: true, window_at: None };
        tx.send(github::SectionEvent::Done(Ok(vec![row]))).unwrap();
        poll_sections(&mut app);
        assert!(app.nav.recent.contains(&9));
        assert_eq!(app.nav.current, current);
        assert_eq!(app.nav.scroll, 12);
        assert_eq!(app.nav.hover, Some(7));
    }
}

#[test]
fn only_the_lease_holder_queues_a_backfill_page() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder());
    for app in [&mut holder, &mut other] {
        app.sections.backfill.months = 12;
        app.drive_backfill(Instant::now() + Duration::from_secs(10));
    }
    assert!(holder.sections.backfill.rx.is_some(), "the holder backfills");
    assert!(other.sections.backfill.rx.is_none(), "a non-holder never does");
    // No store link, no backfill, even though such an app syncs on its own.
    let mut bare = sidebar_app(&[]);
    bare.drive_backfill(Instant::now() + Duration::from_secs(10));
    assert!(bare.sections.backfill.rx.is_none());
    restore_xdg(saved);
}

/// Stored rows as one finished sync plus backfill walks marked `done`.
fn seed_backfilled(rows: Vec<crate::github::Row>, done: bool) {
    put_sections(rows, 0);
    let mut store = crate::store::open("o/r").unwrap();
    for conn in crate::api::Conn::ALL {
        store.backfill_save(conn.name(), None, &[], None, done, 0).unwrap();
    }
}

fn stored(number: u64, state: &str, labels: &[&str], title: &str) -> crate::github::Row {
    let mut row = cached_row(number, true);
    row.issue.state = state.to_string();
    row.issue.title = title.to_string();
    row.issue.updated_at = chrono::Utc::now().to_rfc3339();
    row.issue.labels = labels.iter().map(|name| model::Label { name: name.to_string() }).collect();
    row
}

fn queued(app: &mut App) -> usize {
    app.fetches.queue_mut().len()
}

#[test]
fn local_query_answers_from_the_store_with_zero_requests() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![
        stored(1, "OPEN", &["bug"], "crash on start"),
        stored(2, "OPEN", &["feature"], "dark mode"),
        stored(3, "CLOSED", &["bug"], "old crash"),
    ], true);
    let mut app = linked_app();
    app.sections.merged_window_hours = 0;
    app.sidebar.query_active = "is:open label:bug".to_string();
    app.run_gh_search("is:open label:bug".to_string());
    assert_eq!(app.nav.recent, vec![1], "only the matching stored row lands");
    assert!(app.sections.rx.is_none() && !app.sections.replace, "no fetch was spawned");
    assert_eq!(queued(&mut app), 0, "zero requests queued");
    assert_eq!(app.sidebar.query_active, "is:open label:bug", "the query tag keeps its text");

    // A store change re-filters with no request.
    put_sections(vec![stored(1, "OPEN", &["bug"], "crash on start"), stored(2, "OPEN", &["bug"], "dark mode")], 0);
    assert!(app.watch_store(Instant::now()), "the changed store re-filters");
    assert!(app.nav.recent.contains(&1) && app.nav.recent.contains(&2));
    assert!(app.sections.rx.is_none());
    assert_eq!(queued(&mut app), 0);

    // Auto-refresh in local mode runs no search; only the holder's quiet sync queues.
    let start = app.sections.last_fetch;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.sections.rx.is_none() && !app.sections.replace);
    assert_eq!(queued(&mut app), 1, "just the sync");
    restore_xdg(saved);
}

#[test]
fn local_query_matches_title_words_through_a_saved_query() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![stored(1, "OPEN", &[], "Crash on start"), stored(2, "OPEN", &[], "dark mode")], true);
    let mut app = linked_app();
    app.sections.merged_window_hours = 0;
    app.sidebar.saved_queries = vec![github::SavedQuery { name: "crashes".to_string(), query: "is:open crash START".to_string() }];
    app.sidebar.format_focus = 1;
    app.toggle_format_widget();
    assert_eq!(app.sidebar.query_active, "is:open crash START");
    assert_eq!(app.nav.recent, vec![1]);
    assert_eq!(queued(&mut app), 0);
    restore_xdg(saved);
}

#[test]
fn queries_the_store_cannot_answer_go_to_the_server() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![stored(1, "OPEN", &["bug"], "crash")], true);
    let mut app = linked_app();
    app.sections.merged_window_hours = 0;
    // An unknown term, and a closed query reaching past the backfill horizon.
    for query in ["in:body crash", "is:closed"] {
        app.sections.rx = None;
        app.run_gh_search(query.to_string());
        assert!(app.sections.rx.is_some() && app.sections.replace, "{query} runs the server search");
    }
    restore_xdg(saved);
}

#[test]
fn an_unfinished_backfill_sends_queries_to_the_server() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![stored(1, "OPEN", &["bug"], "crash")], false);
    let mut app = linked_app();
    app.run_gh_search("is:open label:bug".to_string());
    assert!(app.sections.rx.is_some(), "the store is incomplete");
    restore_xdg(saved);
}

#[test]
fn holder_syncs_behind_a_local_query_and_the_view_refilters() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![stored(1, "OPEN", &["bug"], "crash"), stored(2, "OPEN", &[], "dark mode")], true);
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder());
    for app in [&mut holder, &mut other] {
        app.sections.merged_window_hours = 0;
        app.sidebar.query_active = "is:open label:bug".to_string();
        app.run_gh_search("is:open label:bug".to_string());
        assert_eq!(app.nav.recent, vec![1]);
    }
    let start = holder.sections.last_sync;
    holder.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    other.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(holder.sections.sync_rx.is_some(), "the holder syncs behind its query");
    assert_eq!(queued(&mut holder), 1);
    assert!(holder.sections.rx.is_none(), "the query result has no fetch of its own");
    assert!(other.sections.sync_rx.is_none() && queued(&mut other) == 0, "a non-holder never syncs");

    // The sync's Done writes the store but lands nothing in the sidebar.
    let (tx, rx) = std::sync::mpsc::channel();
    holder.sections.sync_rx = Some(rx);
    tx.send(github::SectionEvent::Done(Ok(vec![stored(9, "OPEN", &["bug"], "listing row")]))).unwrap();
    poll_sections(&mut holder);
    assert!(holder.sections.sync_rx.is_none());
    assert_eq!(holder.nav.recent, vec![1], "the query result stays");
    assert!(!holder.issues.data.contains_key(&9));

    // The store changed: the re-filter picks the row up with no request.
    put_sections(vec![stored(1, "OPEN", &["bug"], "crash"), stored(2, "OPEN", &["bug"], "dark mode")], 0);
    assert!(holder.watch_store(Instant::now()));
    assert!(holder.nav.recent.contains(&2));
    assert_eq!(queued(&mut holder), 1, "no new request");
    restore_xdg(saved);
}

#[test]
fn holder_still_syncs_behind_a_server_query() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    holder.sidebar.query_active = "in:body crash".to_string();
    let start = holder.sections.last_sync;
    holder.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(holder.sections.sync_rx.is_some(), "the store still syncs");
    assert!(holder.sections.rx.is_some() && holder.sections.replace, "the search keeps its own refresh");
    restore_xdg(saved);
}
