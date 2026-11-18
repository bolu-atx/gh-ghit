//! Tests for focus, panes and history.

use super::*;
use super::sidebar::RECENT_REFRESH_INTERVAL;
use super::test_support::{issue, pending_nav_app, restore_xdg, sidebar_app, temp_xdg};

#[test]
fn toggle_row_expansion_is_sticky_and_keyed_by_number() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    app.nav.visited = vec![7, 9, 11];
    app.nav.current = Some(7);
    app.nav.pane = Pane::Issues;

    app.toggle_row_expansion();
    assert!(app.diff.expanded_rows.contains(&7), "space expands the selected row");
    app.toggle_row_expansion();
    assert!(!app.diff.expanded_rows.contains(&7), "space collapses it again");

    // Expansion sticks across cursor moves and is keyed by number:
    // expanding one row never touches another.
    app.toggle_row_expansion();
    app.show_issue(9);
    assert!(app.diff.expanded_rows.contains(&7), "cursor move keeps expansion");
    app.toggle_row_expansion();
    assert!(app.diff.expanded_rows.contains(&9));
    assert!(app.diff.expanded_rows.contains(&7), "rows expand independently");

    // The refs pane toggles its highlighted row.
    app.nav.pane = Pane::Refs;
    app.refs.highlight = Some(11);
    app.toggle_row_expansion();
    assert!(app.diff.expanded_rows.contains(&11));

    // The body pane has no row selection.
    app.nav.pane = Pane::Main;
    let before = app.diff.expanded_rows.clone();
    app.toggle_row_expansion();
    assert_eq!(app.diff.expanded_rows, before);
}

#[test]
fn bootstrap_starts_in_main_so_auto_refresh_runs() {
    let mut app = App::new(None);
    bootstrap(&mut app, &[]);
    assert!(matches!(app.nav.focus, Focus::Main), "boot must not sit in the command bar");
    // A Main boot leaves the refresh gate open; the Bar guard above
    // still holds once the user opens `:` or `/` on purpose.
    let start = app.sections.last_fetch;
    app.repo_slug = Some("o/r".to_string());
    app.auto_refresh_recent(start + RECENT_REFRESH_INTERVAL);
    assert!(app.sections.rx.is_some(), "startup focus must not block the refresh");
    app.auth.bootstrap_rx = None;
    app.auth.rx = None;
    app.sections.rx = None;
}

#[test]
fn bar_submit_of_current_issue_clears_bar_and_returns_focus() {
    // Boot auto-shows the last number, so `:7 Enter` re-opens the
    // current issue through the already-current branch. The submit
    // tail must still run: without it focus strands in the bar and
    // later keys (`y`, `2`) land as bar text (e2e pr/diff/refs yank).
    let mut app = sidebar_app(&[7]);
    app.nav.focus = Focus::Bar;
    app.bar = "7".to_string();
    app.submit_bar();
    assert!(matches!(app.nav.focus, Focus::Main), "submit leaves the bar");
    assert!(app.bar.is_empty(), "submit clears its text");
}

#[test]
fn history_jump_replays_and_clamps() {
    let mut app = sidebar_app(&[7, 9, 11, 12, 13]);
    app.nav.current = Some(13);
    app.nav.history = vec![7, 9, 11, 12];
    app.go_back_n(3);
    assert_eq!(app.nav.current, Some(9));
    assert_eq!(app.nav.history, vec![7]);
    assert_eq!(app.nav.future, vec![13, 12, 11]);
    for number in [11, 12, 13] {
        app.go_forward();
        assert_eq!(app.nav.current, Some(number));
    }
    app.go_back_n(100);
    assert_eq!(app.nav.current, Some(7));
    assert!(app.nav.history.is_empty());
}

#[test]
fn history_jump_one_matches_back_and_respects_guard() {
    let mut first = sidebar_app(&[7, 9]);
    first.nav.current = Some(9);
    first.nav.history = vec![7];
    let mut second = sidebar_app(&[7, 9]);
    second.nav.current = Some(9);
    second.nav.history = vec![7];
    first.go_back();
    second.go_back_n(1);
    assert_eq!(first.nav.current, second.nav.current);
    assert_eq!(first.nav.history, second.nav.history);
    assert_eq!(first.nav.future, second.nav.future);
    let mut guarded = pending_nav_app();
    guarded.nav.history = vec![7];
    guarded.go_back_n(3);
    assert_eq!(guarded.nav.current, Some(9));
    assert_eq!(guarded.nav.history, vec![7]);
    guarded.confirm_choose(1);
    assert_eq!(guarded.nav.current, Some(7));
}

#[test]
fn pane_visibility_survives_prefs_reload() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut first = App::new(None);
    first.prefs_loaded = true;
    first.toggle_sidebar();
    first.toggle_refs();
    let mut second = App::new(None);
    second.apply_prefs();
    assert!(!second.sidebar.show);
    assert!(!second.refs.show);
    assert_eq!(second.nav.pane, Pane::Main);
    restore_xdg(saved);
}

#[test]
fn history_back_and_forward_walk_explicit_visits() {
    let mut app = sidebar_app(&[7, 9, 11]);

    app.ensure_visited(7);
    app.load_issue(7, false);
    app.ensure_visited(9);
    app.load_issue(9, false);
    app.ensure_visited(11);
    app.load_issue(11, false);
    assert_eq!(app.nav.current, Some(11));
    assert_eq!(app.nav.history, vec![7, 9]);

    app.go_back();
    assert_eq!(app.nav.current, Some(9));
    assert_eq!(app.nav.history, vec![7]);
    assert_eq!(app.nav.future, vec![11]);

    app.go_back();
    assert_eq!(app.nav.current, Some(7));
    assert_eq!(app.nav.history, Vec::<u64>::new());
    assert_eq!(app.nav.future, vec![11, 9]);

    // An empty back stack toasts instead of moving.
    app.go_back();
    assert_eq!(app.nav.current, Some(7));
    assert_eq!(app.message, "no older issue");

    app.go_forward();
    assert_eq!(app.nav.current, Some(9));
    assert_eq!(app.nav.history, vec![7]);
    assert_eq!(app.nav.future, vec![11]);

    // A new jump drops the forward chain.
    app.ensure_visited(11);
    app.load_issue(11, false);
    assert_eq!(app.nav.current, Some(11));
    assert_eq!(app.nav.history, vec![7, 9]);
    assert_eq!(app.nav.future, Vec::<u64>::new());

    // An empty forward stack toasts instead of moving.
    app.go_forward();
    assert_eq!(app.nav.current, Some(11));
    assert_eq!(app.message, "no newer issue");
}

#[test]
fn selection_tracks_current_not_hover() {
    let mut app = sidebar_app(&[7, 9, 11]);
    // Click/Enter/j/k/h/l all funnel through `show_issue`.
    app.show_issue(9);
    assert_eq!(app.nav.current, Some(9));
    assert_eq!(app.sidebar.issue_state.selected(), Some(2));
    assert_eq!(app.sidebar.pr_state.selected(), None);
    // A wheel hover never moves the tracked row.
    app.nav.hover = Some(11);
    assert_eq!(app.effective(&app.issue_rows()), Some(9));
    // Committing from the hover lands on it and clears it.
    app.ensure_visited(11);
    app.load_issue(11, false);
    assert_eq!(app.nav.current, Some(11));
    assert_eq!(app.nav.hover, None, "a commit clears the hover");
    assert_eq!(app.sidebar.issue_state.selected(), Some(3));
    assert_eq!(app.sidebar.pr_state.selected(), None);
}

#[test]
fn step_issue_walks_cached_neighbors() {
    let mut app = sidebar_app(&[589, 590, 591]);
    app.show_issue(590);
    app.step_issue(1);
    assert_eq!(app.nav.current, Some(591), "l opens #591");
    assert_eq!(app.nav.hover, None);
    assert_eq!(app.nav.step_pending, None, "cached landing clears the step");
    assert!(app.nav.visited.contains(&591), "explicit visit files the row");
    assert_eq!(app.sidebar.issue_state.selected(), Some(3));
    app.step_issue(-1);
    assert_eq!(app.nav.current, Some(590), "h opens #590");
    assert_eq!(app.sidebar.issue_state.selected(), Some(2));
    app.step_issue(-1);
    assert_eq!(app.nav.current, Some(589), "h opens #589");
    assert_eq!(app.sidebar.issue_state.selected(), Some(1));
}

#[test]
fn step_issue_without_current_toasts() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    app.step_issue(1);
    assert_eq!(app.message, "view an issue first");
    assert!(app.issues.pending.is_empty(), "no fetch without an issue");
    assert_eq!(app.nav.step_pending, None);
}

#[test]
fn step_issue_below_one_toasts_without_fetch() {
    let mut app = sidebar_app(&[1, 2]);
    app.show_issue(1);
    app.step_issue(-1);
    assert_eq!(app.message, "no #0");
    assert_eq!(app.nav.current, Some(1), "nothing moves");
    assert!(app.issues.pending.is_empty(), "h on #1 issues no fetch");
    assert_eq!(app.nav.requested, Some(1));
    assert_eq!(app.nav.step_pending, None);
}

#[test]
fn step_issue_records_pending_until_the_fetch_lands() {
    let mut app = sidebar_app(&[590]);
    app.show_issue(590);
    app.step_issue(1);
    assert_eq!(app.nav.current, Some(590), "current waits for the fetch");
    assert_eq!(app.nav.requested, Some(591));
    assert_eq!(app.nav.step_pending, Some(591));
    assert!(app.issues.pending.contains(&591), "neighbor fetch in flight");
    assert!(app.nav.visited.contains(&591), "explicit visit files the row");
    // The landing clears the step and moves the highlight.
    let fresh = issue(591, "2026-01-06T00:00:00Z");
    app.issues.tx.send((591, github::IssueEvent::Done(Ok(Box::new((fresh, vec![])))))).unwrap();
    poll_issues(&mut app);
    assert_eq!(app.nav.current, Some(591));
    assert_eq!(app.nav.step_pending, None);
    assert_eq!(app.sidebar.issue_state.selected(), Some(2));
}

#[test]
fn step_404_toasts_no_number_and_stays_put() {
    let mut app = sidebar_app(&[590]);
    app.show_issue(590);
    app.nav.step_pending = Some(589);
    app.nav.requested = Some(589);
    app.issues.tx
        .send((589, github::IssueEvent::Done(Err(
            "GitHub request failed: HTTP status client error (404 Not Found) for url (https://api.github.com/repos/o/r/issues/589)".to_string(),
        ))))
        .unwrap();
    poll_issues(&mut app);
    assert_eq!(app.message, "no #589");
    assert_eq!(app.nav.current, Some(590), "nothing moves past the end");
    assert_eq!(app.nav.step_pending, None);
}

#[test]
fn step_other_error_keeps_the_existing_error_path() {
    let mut app = sidebar_app(&[590]);
    app.show_issue(590);
    app.nav.step_pending = Some(591);
    app.nav.requested = Some(591);
    let raw = "GitHub request failed: error sending request for url (https://api.github.com/repos/o/r/issues/591)".to_string();
    app.issues.tx.send((591, github::IssueEvent::Done(Err(raw.clone())))).unwrap();
    poll_issues(&mut app);
    assert_eq!(app.message, raw, "network errors pass through unchanged");
    assert_eq!(app.nav.current, Some(590), "nothing moves on error");
    assert_eq!(app.nav.step_pending, None);
}

#[test]
fn labels_pane_joins_tab_cycle_and_toggle_shares_dialog_state() {
    let mut app = sidebar_app(&[]);
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string()];
    app.nav.history = vec![1];
    app.nav.pane = Pane::PRs;
    app.cycle_pane(1);
    assert_eq!(app.nav.pane, Pane::Main, "Main joins the Tab cycle");
    app.cycle_pane(1);
    assert_eq!(app.nav.pane, Pane::Labels, "history alone shows the right column");
    app.cycle_pane(1);
    assert_eq!(app.nav.pane, Pane::Issues);
    // Rows: (no milestone), bug. Out-of-range toggles do nothing.
    app.toggle_filter_item(1);
    app.toggle_filter_item(9);
    assert!(app.sidebar.label_filter.contains("bug"));
    app.toggle_filter_item(0);
    assert!(app.sidebar.milestone_filter.contains(&None));
    app.toggle_filter_item(1);
    assert!(app.sidebar.label_filter.is_empty());
    app.move_filter_cursor(5);
    assert_eq!(app.labels.cursor, 1, "cursor clamps to the last row");
    app.nav.pane = Pane::Labels;
    app.toggle_refs();
    assert_eq!(app.nav.pane, Pane::Main, "hiding the column drops focus");
}
