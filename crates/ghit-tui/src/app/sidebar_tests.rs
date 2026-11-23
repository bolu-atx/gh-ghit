//! Tests for sidebar rows, sort, filters and auto-refresh.

use super::*;
use ghit_core::github;
use ghit_core::session::{RECENT_REFRESH_INTERVAL, RECENT_REFRESH_MAX_INTERVAL, STATUS_REFRESH_INTERVAL};
use super::test_support::{issue, restore_xdg, section_row, sidebar_app, temp_xdg};
use ghit_core::model::BoardData;
use ghit_core::model::ItemRef;

/// Store rows as a sync that started `age_secs` ago.
fn put_sections(rows: Vec<ghit_core::github::Row>, age_secs: i64) {
    let started = chrono::Utc::now() - chrono::TimeDelta::seconds(age_secs);
    ghit_core::store::open("o/r").unwrap().write(&started.to_rfc3339(), &rows).unwrap();
}

fn cached_row(number: ItemRef, complete: bool) -> ghit_core::github::Row {
    ghit_core::github::Row {
        issue: issue(number, "2026-01-05T00:00:00Z"),
        statuses: vec![],
        detail: ghit_core::model::BoardDetail {
            created: "2026-01-01".to_string(),
            updated: "2026-01-05".to_string(),
            comments: 0,
        },
        complete,
        window_at: None,
    }
}

/// Add a background (recent) row with the fields the pills read.
fn add_recent(app: &mut App, n: ItemRef, updated: &str, labels: &[&str], milestone: Option<&str>) {
    let mut d = issue(n, updated);
    d.labels = labels
        .iter()
        .map(|name| model::Label { name: name.to_string() })
        .collect();
    d.milestone = milestone.map(str::to_string);
    app.session.issues.data.insert(n, d);
    app.session.issues.statuses.insert(n, vec![]);
    app.nav.recent.push(n);
}

#[test]
fn startup_hydrates_sidebar_from_sections_cache() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut newer = cached_row(ItemRef::github(7), false);
    newer.issue.created_at = "2026-01-02T00:00:00Z".to_string();
    put_sections(vec![newer, cached_row(ItemRef::github(9), true)], 0);
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    app.hydrate_cached_sections();
    assert_eq!(
        app.nav.recent,
        vec![ItemRef::github(7), ItemRef::github(9)],
        "stored order (open rows newest created first) becomes sidebar order"
    );
    assert!(
        app.session.issues.data.contains_key(&ItemRef::github(7))
            && app.session.issues.data.contains_key(&ItemRef::github(9))
    );
    assert!(
        app.session.issues.partial.contains(&ItemRef::github(7)),
        "slim rows stay partial so open runs the full fetch"
    );
    assert!(!app.session.issues.partial.contains(&ItemRef::github(9)));
    assert!(
        app.session.issues.statuses.contains_key(&ItemRef::github(7))
            && app.board.details.contains_key(&ItemRef::github(7))
    );
    restore_xdg(saved);
}

#[test]
fn startup_hydration_leaves_present_rows_alone() {
    let (_guard, _dir, saved) = temp_xdg();
    put_sections(
        vec![
            cached_row(ItemRef::github(7), false),
            cached_row(ItemRef::github(9), false),
        ],
        0,
    );
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    let mut kept = issue(ItemRef::github(7), "2026-01-05T00:00:00Z");
    kept.title = "kept".to_string();
    app.session.issues.data.insert(ItemRef::github(7), kept);
    app.hydrate_cached_sections();
    assert_eq!(
        app.session.issues.data[&ItemRef::github(7)].title,
        "kept",
        "CLI-opened rows win over the cache"
    );
    assert!(app.session.issues.data.contains_key(&ItemRef::github(9)));
    assert_eq!(
        app.nav.recent,
        vec![ItemRef::github(9)],
        "present rows are not re-added to recent"
    );
    restore_xdg(saved);
}

#[test]
fn startup_hydration_ignores_a_missing_store_but_paints_an_old_sync() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    app.hydrate_cached_sections();
    assert!(app.nav.recent.is_empty() && app.session.issues.data.is_empty(), "no store means no rows");
    // The store is the shared local copy: a process that does not hold
    // the sync lease reads it whatever its age.
    put_sections(vec![cached_row(ItemRef::github(7), false)], 3601);
    app.hydrate_cached_sections();
    assert_eq!(
        app.nav.recent,
        vec![ItemRef::github(7)],
        "an old sync still paints"
    );
    restore_xdg(saved);
}

/// A booted app on repo `o/r` with its store link open, as
/// `poll_session` leaves it; fetches stay queued.
fn linked_app() -> App {
    let mut app = sidebar_app(&[]);
    app.session.fetches.hold = true;
    app.session.sections.link = github::StoreLink::open("o/r");
    assert!(app.session.sections.link.is_some(), "store opens");
    app
}

#[test]
fn lease_holder_alone_auto_refreshes_the_listing() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder(), "one lease per repo");
    for app in [&mut holder, &mut other] {
        let start = app.session.sections.last_fetch;
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    }
    assert!(holder.session.sections.rx.is_some(), "the holder syncs");
    assert!(other.session.sections.rx.is_none(), "a non-holder sends no listing request");
    // A query never refreshes on GitHub by itself, in any instance.
    other.sidebar.query_active = "label:bug".to_string();
    let start = other.session.sections.last_fetch;
    other.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(other.session.sections.rx.is_none() && !other.session.sections.replace);
    // A manual refresh takes the lease from any process.
    other.sidebar.query_active.clear();
    other.session.sections.rx = None;
    other.refresh_listing();
    assert!(other.sync_holder() && other.session.sections.rx.is_some(), "r takes the lease and walks");
    assert_eq!(other.session.sections.refresh, github::Refresh::Delta, "the Full plan was consumed by the spawn");
    restore_xdg(saved);
}

#[test]
fn another_process_sync_lands_through_the_store_watch() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = linked_app();
    app.session.sections.merged_window_hours = 168;
    let now = Instant::now();
    assert!(!app.watch_store(now), "nothing new yet");
    let mut merged = section_row(issue(ItemRef::github(4), "2026-01-05T00:00:00Z"));
    merged.issue.is_pr = true;
    merged.issue.state = "MERGED".to_string();
    merged.window_at = Some(chrono::Utc::now().to_rfc3339());
    put_sections(vec![cached_row(ItemRef::github(7), false), merged], 0);
    assert!(app.watch_store(now), "a sync by another connection lands");
    assert!(
        app.nav.recent.contains(&ItemRef::github(7))
            && app.nav.recent.contains(&ItemRef::github(4))
    );
    assert!(!app.watch_store(now), "each sync lands once");
    // List toggles and the window filter the stored rows locally.
    app.toggle_list(1);
    assert!(
        !app.nav.recent.contains(&ItemRef::github(4)),
        "hiding merged PRs drops the row"
    );
    assert!(app.session.sections.rx.is_none(), "a toggle sends no request");
    app.toggle_list(1);
    assert!(app.nav.recent.contains(&ItemRef::github(4)));
    app.set_window(1);
    assert!(app.nav.recent.contains(&ItemRef::github(4)), "merged now is inside 1h");
    assert!(app.session.sections.rx.is_none(), "a window change sends no request");
    restore_xdg(saved);
}

#[test]
fn sort_cycle_orders_rows_and_restores_default() {
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)]);
    app.session.issues
        .data
        .get_mut(&ItemRef::github(7))
        .unwrap()
        .updated_at = "2026-01-05T00:00:00Z".to_string();
    app.session.issues
        .data
        .get_mut(&ItemRef::github(9))
        .unwrap()
        .updated_at = "2026-01-09T00:00:00Z".to_string();
    app.session.issues
        .data
        .get_mut(&ItemRef::github(11))
        .unwrap()
        .updated_at = "2026-01-01T00:00:00Z".to_string();
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)]
    );
    app.cycle_sort();
    assert_eq!(app.sidebar.sort, core::SideSort::Updated);
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(9), ItemRef::github(7), ItemRef::github(11)],
        "newest first"
    );
    app.toggle_sort_dir();
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(11), ItemRef::github(7), ItemRef::github(9)],
        "direction flips"
    );
    for _ in 0..4 {
        app.cycle_sort();
    }
    assert_eq!(app.sidebar.sort, core::SideSort::Default);
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
        "default order is back"
    );
}

#[test]
fn sort_applies_to_the_prs_pane_too() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    for (n, updated) in [(13, "2026-01-03T00:00:00Z"), (15, "2026-01-08T00:00:00Z")] {
        let mut pr = issue(ItemRef::github(n), updated);
        pr.is_pr = true;
        app.session.issues.data.insert(ItemRef::github(n), pr);
        app.session.issues.statuses.insert(ItemRef::github(n), vec![]);
        app.nav.visited.push(ItemRef::github(n));
    }
    assert_eq!(
        app.pr_numbers(),
        vec![ItemRef::github(13), ItemRef::github(15)]
    );
    app.cycle_sort();
    assert_eq!(
        app.pr_numbers(),
        vec![ItemRef::github(15), ItemRef::github(13)],
        "PRs sort newest first"
    );
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(7)],
        "panes stay separate"
    );
}

#[test]
fn label_and_milestone_pills_narrow_recent_rows_only() {
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(
        &mut app,
        ItemRef::github(1),
        "2026-01-05T00:00:00Z",
        &["bug"],
        Some("v1"),
    );
    add_recent(
        &mut app,
        ItemRef::github(2),
        "2026-01-04T00:00:00Z",
        &["docs"],
        Some("v2"),
    );
    add_recent(
        &mut app,
        ItemRef::github(3),
        "2026-01-03T00:00:00Z",
        &[],
        None,
    );
    let query = "label:remote".to_string();
    app.sidebar.query_active = query.clone();
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(1), ItemRef::github(2), ItemRef::github(3)]
    );
    app.sidebar.label_filter.insert("bug".to_string());
    assert_eq!(app.issue_numbers(), vec![ItemRef::github(1)], "label pill narrows");
    assert_eq!(app.sidebar.query_active, query, "filter code never edits the query");
    app.sidebar.label_filter.clear();
    app.sidebar.milestone_filter.insert(Some("v2".to_string()));
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(2)],
        "milestone pill narrows"
    );
    assert_eq!(app.sidebar.query_active, query, "query still untouched");
    app.sidebar.milestone_filter.clear();
    app.sidebar.milestone_filter.insert(None);
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(3)],
        "(no milestone) pill narrows"
    );
    app.sidebar.milestone_filter.insert(Some("v1".to_string()));
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(1), ItemRef::github(3)],
        "milestone pills match any selected"
    );
    // Kinds intersect: label AND milestone.
    app.sidebar.label_filter.insert("docs".to_string());
    assert!(app.issue_numbers().is_empty());
}

#[test]
fn pinned_and_requested_rows_skip_filters_and_grouping() {
    use ghit_core::model::{GroupBy, SideRow::{Divider, Item}};
    let mut app = sidebar_app(&[ItemRef::github(1), ItemRef::github(2)]);
    app.session.pins.order = vec![ItemRef::github(1)];
    app.session.pins.set = [1].map(ItemRef::github).into_iter().collect();
    add_recent(
        &mut app,
        ItemRef::github(3),
        "2026-01-05T00:00:00Z",
        &["bug"],
        Some("v1"),
    );
    add_recent(
        &mut app,
        ItemRef::github(4),
        "2026-01-04T00:00:00Z",
        &["bug"],
        Some("v1"),
    );
    app.sidebar.label_filter.insert("bug".to_string());
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones =
        vec![model::Milestone { title: "v1".to_string(), due_on: None, ..Default::default() }];
    assert_eq!(
        app.issue_rows(),
        vec![
            Item(ItemRef::github(1)),
            Divider("requested".to_string()),
            Item(ItemRef::github(2)),
            Divider("v1 (2)".to_string()),
            Item(ItemRef::github(3)),
            Item(ItemRef::github(4)),
        ],
        "pins and requested rows stay on top, ungrouped and unfiltered"
    );
}

#[test]
fn sort_runs_inside_each_milestone_group() {
    use ghit_core::model::{GroupBy, SideRow::{Divider, Item}};
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(
        &mut app,
        ItemRef::github(3),
        "2026-01-03T00:00:00Z",
        &[],
        Some("v1"),
    );
    add_recent(
        &mut app,
        ItemRef::github(4),
        "2026-01-06T00:00:00Z",
        &[],
        Some("v1"),
    );
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones =
        vec![model::Milestone { title: "v1".to_string(), due_on: None, ..Default::default() }];
    app.cycle_sort();
    assert_eq!(
        app.issue_rows(),
        vec![
            Divider("v1 (2)".to_string()),
            Item(ItemRef::github(4)),
            Item(ItemRef::github(3))
        ],
        "updated sort reorders inside the group"
    );
    assert_eq!(
        app.issue_numbers(),
        vec![ItemRef::github(4), ItemRef::github(3)],
        "j/k follows the grouped display"
    );
}

#[test]
fn milestone_groups_order_dated_then_undated_then_none() {
    use ghit_core::model::{GroupBy, SideRow::Divider};
    let mut app = sidebar_app(&[]);
    app.nav.visited.clear();
    add_recent(
        &mut app,
        ItemRef::github(1),
        "2026-01-05T00:00:00Z",
        &[],
        Some("later"),
    );
    add_recent(
        &mut app,
        ItemRef::github(2),
        "2026-01-05T00:00:00Z",
        &[],
        Some("sooner"),
    );
    add_recent(
        &mut app,
        ItemRef::github(3),
        "2026-01-05T00:00:00Z",
        &[],
        Some("backlog"),
    );
    add_recent(
        &mut app,
        ItemRef::github(4),
        "2026-01-05T00:00:00Z",
        &[],
        None,
    );
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones = vec![
        model::Milestone { title: "later".to_string(), due_on: Some("2026-06-01T00:00:00Z".to_string()), ..Default::default() },
        model::Milestone { title: "sooner".to_string(), due_on: Some("2026-02-01T00:00:00Z".to_string()), ..Default::default() },
        model::Milestone { title: "backlog".to_string(), due_on: None, ..Default::default() },
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
    app.session.repo_slug = Some("o/r".to_string());
    app.prefs_loaded = true;
    app.sidebar.label_filter = ["bug".to_string(), "gone".to_string()].into_iter().collect();
    app.sidebar.milestone_filter =
        [Some("v1".to_string()), Some("gone".to_string()), None].into_iter().collect();
    app.sidebar.format_focus = 99;
    app.land_repo_meta(Ok((
        vec!["bug".to_string()],
        vec![model::Milestone { title: "v1".to_string(), due_on: None, ..Default::default() }],
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
    app.session.sections.merged_window_hours = 168;
    app.set_window(0);
    assert_eq!(app.session.sections.merged_window_hours, 0);
    assert_eq!(app.window_label(), "open + merged all time");
    assert_eq!(app.sidebar.query_active, "label:bug", "window keeps the query");
    app.reset_recent_default();
    assert_eq!(app.session.sections.merged_window_hours, core::DEFAULT_WINDOW_HOURS);
    assert!(app.sidebar.query_active.is_empty(), "zero clears the query");
    assert!(app.sidebar.query_draft.is_empty());
}

#[test]
fn startup_restored_query_runs_locally_instead_of_window_fetch() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut prefs = github::load_prefs();
    prefs.queries.insert(github::prefs_repo_key("o/r"), "label:bug".to_string());
    github::save_prefs(&prefs);
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.auth.bootstrap_rx = Some(rx);
    tx.send(Some("o/r".to_string())).unwrap();
    assert!(poll_session(&mut app));
    assert_eq!(app.sidebar.query_active, "label:bug");
    assert!(app.session.sections.rx.is_none() && !app.session.sections.replace, "restored query sends no search");
    assert_eq!(queued(&mut app), 0);
    restore_xdg(saved);
}

#[test]
fn startup_without_saved_query_fetches_window_listing() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.auth.bootstrap_rx = Some(rx);
    tx.send(Some("o/r".to_string())).unwrap();
    assert!(poll_session(&mut app));
    assert!(app.sidebar.query_active.is_empty());
    assert!(app.session.sections.rx.is_some(), "window listing still fetches");
    assert!(!app.session.sections.replace, "window listing appends");
    restore_xdg(saved);
}

#[test]
fn auto_refresh_fires_at_interval() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let start = app.session.sections.last_fetch;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL - Duration::from_millis(1));
    assert!(app.session.sections.rx.is_none());
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.session.sections.rx.is_some());
    assert!(app.session.sections.auto_refresh);
}

#[test]
fn auto_refresh_guards_each_active_view() {
    for guard in 0..7 {
        let mut app = sidebar_app(&[ItemRef::github(7)]);
        let (_tx, rx) = std::sync::mpsc::channel();
        match guard {
            0 => app.session.sections.rx = Some(rx),
            1 => {
                app.board.loading = true;
                app.open(crate::app::Entry::Board);
            }
            2 => {
                app.board.data = Some(BoardData {
                    title: String::new(),
                    order: vec![],
                    items: vec![],
                    url: String::new(),
                });
                app.open(crate::app::Entry::Board);
            }
            3 => app.open_draft(ComposeState {
                number: ItemRef::github(7),
                target: ComposeTarget::Thread,
                body: String::new(),
                cursor: 0,
                sending: false,
                kill: String::new(),
                view: Default::default(),
            }),
            4 => app.open(crate::app::Entry::Format(crate::app::FormatEdit::None)),
            5 => app.open(crate::app::Entry::Shortcuts),
            6 => app.nav.focus = Focus::Bar,
            _ => unreachable!(),
        }
        let start = app.session.sections.last_fetch;
        app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
        assert_eq!(app.session.sections.last_fetch, start, "guard {guard}");
        assert!(!app.session.sections.auto_refresh, "guard {guard}");
    }
}

#[test]
fn auto_refresh_failure_backs_off_and_success_resets() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    for expected in [60, 60] {
        let (tx, rx) = std::sync::mpsc::channel();
        app.session.sections.rx = Some(rx);
        app.session.sections.auto_refresh = true;
        tx.send(github::SectionEvent::Done(Err("offline".into()))).unwrap();
        poll_session(&mut app);
        assert_eq!(app.session.sections.auto_refresh_interval, Duration::from_secs(expected));
    }
    let start = app.session.sections.last_fetch;
    app.auto_refresh_recent(start + Duration::from_secs(59));
    assert!(app.session.sections.rx.is_none());
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.sections.rx = Some(rx);
    app.session.sections.auto_refresh = true;
    tx.send(github::SectionEvent::Done(Ok(vec![]))).unwrap();
    poll_session(&mut app);
    assert_eq!(app.session.sections.auto_refresh_interval, RECENT_REFRESH_INTERVAL);
}

#[test]
fn auto_refresh_probes_until_a_status_refresh_is_due() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.session.repo_slug = Some("o/r".to_string());
    let start = app.session.sections.last_fetch;
    let status_clock = app.session.sections.last_status;
    let full_clock = app.session.sections.last_full;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.session.sections.rx.is_some());
    assert!(!app.session.sections.probe, "probe flag is consumed by the spawn");
    assert_eq!(app.session.sections.last_status, status_clock, "a change-only pass keeps the status clock");
    // Past the status span, the refresh runs without the probe.
    app.session.sections.rx = None;
    app.session.sections.last_status = status_clock - STATUS_REFRESH_INTERVAL;
    let late = start + RECENT_REFRESH_INTERVAL * 2;
    app.session.sections.last_fetch = late - RECENT_REFRESH_INTERVAL;
    app.auto_refresh_recent(late);
    assert!(app.session.sections.rx.is_some());
    assert!(app.session.sections.last_status > status_clock, "status clock restarts");
    assert_eq!(app.session.sections.last_full, full_clock, "a status pass is not a full walk");
    app.session.sections.rx = None;
}

#[test]
fn unchanged_probe_keeps_rows_and_resets_backoff() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let before = app.nav.recent.clone();
    let full = app.session.sections.last_full;
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.sections.rx = Some(rx);
    app.session.sections.auto_refresh = true;
    app.session.sections.auto_refresh_interval = RECENT_REFRESH_MAX_INTERVAL;
    tx.send(github::SectionEvent::Unchanged).unwrap();
    poll_session(&mut app);
    assert!(app.session.sections.rx.is_none());
    assert!(!app.session.sections.auto_refresh);
    assert_eq!(app.session.sections.auto_refresh_interval, RECENT_REFRESH_INTERVAL);
    assert_eq!(app.nav.recent, before, "no fetch ran, rows stay put");
    assert_eq!(app.session.sections.last_full, full, "a probe is not a full fetch");
}

#[test]
fn auto_refresh_landing_preserves_view() {
    for current in [Some(ItemRef::github(7)), None] {
        let mut app = sidebar_app(&[ItemRef::github(7)]);
        app.nav.current = current;
        app.nav.scroll = 12;
        app.nav.hover = Some(ItemRef::github(7));
        let (tx, rx) = std::sync::mpsc::channel();
        app.session.sections.rx = Some(rx);
        app.session.sections.auto_refresh = true;
        let row = ghit_core::github::Row {
            issue: issue(ItemRef::github(9), "2026-01-06T00:00:00Z"),
            statuses: vec![],
            detail: model::BoardDetail::default(),
            complete: true,
            window_at: None,
        };
        tx.send(github::SectionEvent::Done(Ok(vec![row]))).unwrap();
        poll_session(&mut app);
        assert!(app.nav.recent.contains(&ItemRef::github(9)));
        assert_eq!(app.nav.current, current);
        assert_eq!(app.nav.scroll, 12);
        assert_eq!(app.nav.hover, Some(ItemRef::github(7)));
    }
}

#[test]
fn only_the_lease_holder_queues_a_backfill_page() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder());
    for app in [&mut holder, &mut other] {
        app.session.sections.backfill.months = 12;
        app.drive_backfill(Instant::now() + Duration::from_secs(10));
    }
    assert!(holder.session.sections.backfill.rx.is_some(), "the holder backfills");
    assert!(other.session.sections.backfill.rx.is_none(), "a non-holder never does");
    // No store link, no backfill, even though such an app syncs on its own.
    let mut bare = sidebar_app(&[]);
    bare.drive_backfill(Instant::now() + Duration::from_secs(10));
    assert!(bare.session.sections.backfill.rx.is_none());
    restore_xdg(saved);
}

/// Stored rows as one finished sync plus backfill walks marked `done`.
fn seed_backfilled(rows: Vec<ghit_core::github::Row>, done: bool) {
    put_sections(rows, 0);
    let mut store = ghit_core::store::open("o/r").unwrap();
    for conn in ghit_core::api::Conn::ALL {
        store.backfill_save(conn.name(), None, &[], None, done, 0).unwrap();
    }
}

fn stored(number: ItemRef, state: &str, labels: &[&str], title: &str) -> ghit_core::github::Row {
    let mut row = cached_row(number, true);
    row.issue.state = state.to_string();
    row.issue.title = title.to_string();
    row.issue.updated_at = chrono::Utc::now().to_rfc3339();
    row.issue.labels = labels.iter().map(|name| model::Label { name: name.to_string() }).collect();
    row
}

fn queued(app: &mut App) -> usize {
    app.session.fetches.queue_mut().len()
}

#[test]
fn local_query_answers_from_the_store_with_zero_requests() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(vec![
        stored(ItemRef::github(1), "OPEN", &["bug"], "crash on start"),
        stored(ItemRef::github(2), "OPEN", &["feature"], "dark mode"),
        stored(ItemRef::github(3), "CLOSED", &["bug"], "old crash"),
    ], true);
    let mut app = linked_app();
    app.session.sections.merged_window_hours = 0;
    app.sidebar.query_active = "is:open label:bug".to_string();
    app.run_gh_search("is:open label:bug".to_string());
    assert_eq!(app.nav.recent, vec![ItemRef::github(1)], "only the matching stored row lands");
    assert!(app.session.sections.rx.is_none() && !app.session.sections.replace, "no fetch was spawned");
    assert_eq!(queued(&mut app), 0, "zero requests queued");
    assert_eq!(app.sidebar.query_active, "is:open label:bug", "the query tag keeps its text");

    // A store change re-filters with no request.
    put_sections(
        vec![
            stored(ItemRef::github(1), "OPEN", &["bug"], "crash on start"),
            stored(ItemRef::github(2), "OPEN", &["bug"], "dark mode"),
        ],
        0,
    );
    assert!(
        app.watch_store(Instant::now()),
        "the changed store re-filters"
    );
    assert!(
        app.nav.recent.contains(&ItemRef::github(1))
            && app.nav.recent.contains(&ItemRef::github(2))
    );
    assert!(app.session.sections.rx.is_none());
    assert_eq!(queued(&mut app), 0);

    // Auto-refresh in local mode runs no search; only the holder's quiet sync queues.
    let start = app.session.sections.last_fetch;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.session.sections.rx.is_none() && !app.session.sections.replace);
    assert_eq!(queued(&mut app), 1, "just the sync");
    restore_xdg(saved);
}

#[test]
fn local_query_matches_title_words_through_a_saved_query() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![
            stored(ItemRef::github(1), "OPEN", &[], "Crash on start"),
            stored(ItemRef::github(2), "OPEN", &[], "dark mode"),
        ],
        true,
    );
    let mut app = linked_app();
    app.session.sections.merged_window_hours = 0;
    app.sidebar.saved_queries = vec![github::SavedQuery { name: "crashes".to_string(), query: "is:open crash START".to_string() }];
    app.sidebar.format_focus = 1;
    app.toggle_format_widget();
    assert_eq!(app.sidebar.query_active, "is:open crash START");
    assert_eq!(app.nav.recent, vec![ItemRef::github(1)]);
    assert_eq!(queued(&mut app), 0);
    restore_xdg(saved);
}

#[test]
fn queries_the_store_cannot_answer_show_a_notice_and_send_nothing() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![stored(ItemRef::github(1), "OPEN", &["bug"], "crash")],
        true,
    );
    let mut app = linked_app();
    app.session.sections.merged_window_hours = 0;
    // An unknown term, and a closed query reaching past the backfill horizon.
    for query in ["in:body crash", "is:closed"] {
        app.sidebar.query_active = query.to_string();
        app.run_gh_search(query.to_string());
        assert!(app.session.sections.rx.is_none() && !app.session.sections.replace, "{query} sends no search");
        assert_eq!(queued(&mut app), 0, "{query} queues no request");
        assert!(app.message.contains(&app.github_key()) && app.message.contains("GitHub"), "{}", app.message);
        assert_eq!(app.query_source(), "local");
    }
    restore_xdg(saved);
}

#[test]
fn an_unfinished_backfill_sends_no_search() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![stored(ItemRef::github(1), "OPEN", &["bug"], "crash")],
        false,
    );
    let mut app = linked_app();
    app.run_gh_search("is:open label:bug".to_string());
    assert!(app.session.sections.rx.is_none() && queued(&mut app) == 0, "the store is incomplete, the notice names the action");
    restore_xdg(saved);
}

#[test]
fn holder_syncs_behind_a_local_query_and_the_view_refilters() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![
            stored(ItemRef::github(1), "OPEN", &["bug"], "crash"),
            stored(ItemRef::github(2), "OPEN", &[], "dark mode"),
        ],
        true,
    );
    let mut holder = linked_app();
    let mut other = linked_app();
    assert!(holder.sync_holder() && !other.sync_holder());
    for app in [&mut holder, &mut other] {
        app.session.sections.merged_window_hours = 0;
        app.sidebar.query_active = "is:open label:bug".to_string();
        app.run_gh_search("is:open label:bug".to_string());
        assert_eq!(app.nav.recent, vec![ItemRef::github(1)]);
    }
    let start = holder.session.sections.last_sync;
    holder.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    other.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(holder.session.sections.sync_rx.is_some(), "the holder syncs behind its query");
    assert_eq!(queued(&mut holder), 1);
    assert!(holder.session.sections.rx.is_none(), "the query result has no fetch of its own");
    assert!(other.session.sections.sync_rx.is_none() && queued(&mut other) == 0, "a non-holder never syncs");

    // The sync's Done writes the store but lands nothing in the sidebar.
    let (tx, rx) = std::sync::mpsc::channel();
    holder.session.sections.sync_rx = Some(rx);
    tx.send(github::SectionEvent::Done(Ok(vec![stored(
        ItemRef::github(9),
        "OPEN",
        &["bug"],
        "listing row",
    )])))
    .unwrap();
    poll_session(&mut holder);
    assert!(holder.session.sections.sync_rx.is_none());
    assert_eq!(
        holder.nav.recent,
        vec![ItemRef::github(1)],
        "the query result stays"
    );
    assert!(!holder.session.issues.data.contains_key(&ItemRef::github(9)));

    // The store changed: the re-filter picks the row up with no request.
    put_sections(
        vec![
            stored(ItemRef::github(1), "OPEN", &["bug"], "crash"),
            stored(ItemRef::github(2), "OPEN", &["bug"], "dark mode"),
        ],
        0,
    );
    assert!(holder.watch_store(Instant::now()));
    assert!(holder.nav.recent.contains(&ItemRef::github(2)));
    assert_eq!(queued(&mut holder), 1, "no new request");
    restore_xdg(saved);
}

#[test]
fn holder_still_syncs_behind_an_unanswerable_query_and_sends_no_search() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut holder = linked_app();
    holder.sidebar.query_active = "in:body crash".to_string();
    let start = holder.session.sections.last_sync;
    holder.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(holder.session.sections.sync_rx.is_some(), "the store still syncs");
    assert!(holder.session.sections.rx.is_none() && !holder.session.sections.replace, "no search of its own");
    restore_xdg(saved);
}

fn gh_hit(number: ItemRef, updated: &str, title: &str) -> ghit_core::github::Row {
    let mut row = cached_row(number, false);
    row.issue.updated_at = updated.to_string();
    row.issue.title = title.to_string();
    row
}

fn land_search(app: &mut App, rows: Vec<ghit_core::github::Row>) {
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.sections.rx = Some(rx);
    app.session.sections.replace = true;
    tx.send(github::SectionEvent::Done(Ok(rows))).unwrap();
    poll_session(app);
}

#[test]
fn the_search_action_runs_one_github_search_and_lands_github_rows() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![stored(ItemRef::github(1), "OPEN", &["bug"], "crash")],
        true,
    );
    let mut app = linked_app();
    app.sidebar.query_active = "in:body crash".to_string();
    app.run_gh_search("in:body crash".to_string());
    assert_eq!(app.query_source(), "local");
    assert_eq!(queued(&mut app), 0);
    app.search_github();
    assert_eq!(queued(&mut app), 1, "exactly one search");
    assert!(app.session.sections.rx.is_some() && app.session.sections.replace);
    land_search(
        &mut app,
        vec![gh_hit(
            ItemRef::github(40),
            "2030-01-01T00:00:00Z",
            "body hit",
        )],
    );
    assert_eq!(app.query_source(), "GitHub");
    assert_eq!(app.nav.recent, vec![ItemRef::github(40)]);
    let stored = ghit_core::store::open("o/r")
        .unwrap()
        .rows_by_number(&[ItemRef::github(40)]);
    assert_eq!(stored.len(), 1, "the hit is in the store");
    restore_xdg(saved);
}

#[test]
fn github_results_upsert_newer_only_and_keep_statuses() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut old = stored(ItemRef::github(5), "OPEN", &[], "stored title");
    old.issue.updated_at = "2030-01-01T00:00:00Z".to_string();
    old.issue.checks = Some(model::CheckState::Pass);
    old.statuses = vec![("Board".to_string(), Some("Doing".to_string()))];
    let mut same = stored(ItemRef::github(6), "OPEN", &[], "same time");
    same.issue.updated_at = "2030-01-01T00:00:00Z".to_string();
    seed_backfilled(vec![old, same], true);
    let mut app = linked_app();
    app.sidebar.query_active = "in:body x".to_string();
    land_search(&mut app, vec![
        gh_hit(ItemRef::github(5), "2030-02-01T00:00:00Z", "newer title"),
        gh_hit(ItemRef::github(6), "2030-01-01T00:00:00Z", "ignored title"),
    ]);
    let store = ghit_core::store::open("o/r").unwrap();
    let five = &store.rows_by_number(&[ItemRef::github(5)])[0];
    assert_eq!(five.issue.title, "newer title");
    assert!(five.issue.checks.is_some(), "checks survive the REST row");
    assert_eq!(five.statuses.len(), 1, "statuses survive the REST row");
    assert_eq!(
        store.rows_by_number(&[ItemRef::github(6)])[0].issue.title,
        "same time",
        "equal updated_at keeps the stored row"
    );
    restore_xdg(saved);
}

#[test]
fn a_github_result_set_stays_visible_and_never_auto_refreshes() {
    let (_guard, _dir, saved) = temp_xdg();
    seed_backfilled(
        vec![stored(ItemRef::github(1), "OPEN", &["bug"], "crash")],
        true,
    );
    let mut app = linked_app();
    app.sidebar.query_active = "in:body crash".to_string();
    land_search(
        &mut app,
        vec![gh_hit(
            ItemRef::github(40),
            "2030-01-01T00:00:00Z",
            "body hit",
        )],
    );
    let queued_before = queued(&mut app);
    // Auto-refresh sends no search.
    let start = app.session.sections.last_fetch;
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.session.sections.rx.is_none() && !app.session.sections.replace);
    assert!(queued(&mut app) <= queued_before + 1, "at most the holder's quiet sync");
    // A store change re-reads the same numbers.
    put_sections(
        vec![
            stored(ItemRef::github(1), "OPEN", &["bug"], "crash"),
            gh_hit(ItemRef::github(40), "2030-01-02T00:00:00Z", "body hit v2"),
        ],
        0,
    );
    assert!(app.watch_store(Instant::now()));
    assert_eq!(
        app.nav.recent,
        vec![ItemRef::github(40)],
        "the GitHub result set stays"
    );
    assert_eq!(app.query_source(), "GitHub");
    // A new local query run resets the source.
    app.run_gh_search("in:body crash".to_string());
    assert_eq!(app.query_source(), "local");
    restore_xdg(saved);
}
