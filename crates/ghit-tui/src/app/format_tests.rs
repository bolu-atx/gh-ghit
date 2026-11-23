//! Tests for persisted view prefs.

use ghit_core::github;
use super::*;
use super::test_support::{restore_xdg, temp_xdg};
use ghit_core::model::BoardData;
use ghit_core::model::ItemRef;

#[test]
fn format_prefs_survive_across_sessions() {
    let (_guard, _dir, saved) = temp_xdg();
    // First session: flip toggles, fields, window, query, and the
    // board filter, then persist like the key handlers do.
    let mut first = App::new(Some("o/r".to_string()));
    first.session.repo_slug = Some("o/r".to_string());
    first.prefs_loaded = true;
    first.sidebar.show_merged_prs = false;
    first.session.sections.merged_window_hours = 1;
    first.sidebar.row_issue.author = true;
    first.sidebar.row_pr.checks = false;
    first.board.fields.milestone = true;
    first.sidebar.query_active = "label:bug".to_string();
    first.sidebar.label_filter.insert("bug".to_string());
    first.sidebar.milestone_filter.insert(Some("v1".to_string()));
    first.sidebar.milestone_filter.insert(None);
    first.sidebar.group_by = model::GroupBy::Milestone;
    first.sidebar.sort = core::SideSort::Updated;
    first.sidebar.sort_desc = false;
    first.board.filter = "auth".to_string();
    first.board.milestones.insert(Some("v1".to_string()));
    first.session.board.loading = true;
    first.open(crate::app::Entry::Board);
    first.persist_prefs();
    // Second session: startup applies everything except the board
    // filter, which returns when its owner board opens.
    let mut second = App::new(Some("o/r".to_string()));
    second.session.repo_slug = Some("o/r".to_string());
    second.apply_prefs();
    assert!(!second.sidebar.show_merged_prs);
    assert_eq!(second.session.sections.merged_window_hours, 1);
    assert!(second.sidebar.row_issue.author);
    assert!(!second.sidebar.row_pr.checks);
    assert!(second.board.fields.milestone);
    assert_eq!(second.sidebar.query_active, "label:bug");
    assert!(second.sidebar.label_filter.contains("bug"));
    assert!(second.sidebar.milestone_filter.contains(&Some("v1".to_string())));
    assert!(second.sidebar.milestone_filter.contains(&None));
    assert_eq!(second.sidebar.group_by, model::GroupBy::Milestone);
    assert_eq!(second.sidebar.sort, core::SideSort::Updated);
    assert!(!second.sidebar.sort_desc, "direction restores too");
    assert!(second.board.filter.is_empty(), "board filter waits for its owner");
    second.restore_board_filter("o");
    assert_eq!(second.board.filter, "auth");
    assert!(second.board.milestones.contains(&Some("v1".to_string())));
    second.restore_board_filter("other");
    assert!(second.board.filter.is_empty(), "unknown owners start unfiltered");
    restore_xdg(saved);
}

#[test]
fn stale_saved_milestones_prune_when_board_lands() {
    use ghit_core::model::BoardItem;
    let (_guard, _dir, saved) = temp_xdg();
    let mut prefs = github::load_prefs();
    prefs.board_filters.insert(
        github::prefs_owner_key("o"),
        github::SavedBoardFilter {
            filter: String::new(),
            milestones: vec![Some("v1".to_string())],
        },
    );
    github::save_prefs(&prefs);
    let mut app = App::new(Some("o/r".to_string()));
    app.prefs_loaded = true;
    // Restoring before the board lands keeps the saved pills: there
    // is nothing to check them against yet.
    app.restore_board_filter("o");
    assert!(app.board.milestones.contains(&Some("v1".to_string())));
    // The board lands without v1: the stale pill drops, the focus
    // clamps to the widget count, and the pruned set persists.
    let card = |milestone: Option<&str>| BoardItem {
        title: "card".to_string(),
        number: Some(ItemRef::github(1)),
        status: Some("Todo".to_string()),
        labels: vec![],
        repo: None,
        milestone: milestone.map(str::to_string),
        content: serde_json::Value::Null,
    };
    app.session.board.data = Some(BoardData {
        title: "b".to_string(),
        order: vec!["Todo".to_string()],
        items: vec![card(Some("v2")), card(None)],
        url: String::new(),
    });
    app.open(crate::app::Entry::Board);
    app.board.filter_focus = 99;
    app.prune_board_milestones();
    assert!(app.board.milestones.is_empty(), "stale v1 drops");
    let widgets = app.board_pills().len() + 1 + model::CardFields::ORDER.len();
    assert!(app.board.filter_focus < widgets, "focus clamps to the widget count");
    let back = github::load_prefs();
    let saved_filter = &back.board_filters[&github::prefs_owner_key("o")];
    assert!(saved_filter.milestones.is_empty(), "pruned set persists");
    let board = app.session.board.data.as_ref().unwrap();
    assert!(
        board_columns(board, &app.board.filter, &app.board.milestones)[0]
            .1
            .iter()
            .any(|c| c.number == Some(ItemRef::github(1))),
        "lanes match again"
    );
    restore_xdg(saved);
}

#[test]
fn the_map_filter_is_saved_per_repo() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut first = App::new(Some("o/r".to_string()));
    first.session.repo_slug = Some("o/r".to_string());
    first.prefs_loaded = true;
    first.map_filter_flip(crate::app::FilterRow::Closed);
    first.map_filter_flip(crate::app::FilterRow::Label("bug".to_string()));
    first.map_toggle_scope();
    let want = first.map.filter.clone();
    assert!(want.closed && want.labels.contains("bug") && want.scope == model::MapScope::Repo);
    let mut second = App::new(Some("o/r".to_string()));
    second.session.repo_slug = Some("o/r".to_string());
    second.apply_prefs();
    assert_eq!(second.map.filter, want, "every change saved, scope included");
    let mut other = App::new(Some("o/other".to_string()));
    other.session.repo_slug = Some("o/other".to_string());
    other.apply_prefs();
    assert_eq!(other.map.filter, model::MapFilter::default(), "another repo keeps its own filter");
    restore_xdg(saved);
}

#[test]
fn prefs_saved_under_another_host_stay_unrestored() {
    let _guard = ghit_core::cache::test_env_lock();
    let dir = tempfile::TempDir::new().unwrap();
    let saved_xdg = std::env::var("XDG_DATA_HOME").ok();
    let saved_host = std::env::var("GH_HOST").ok();
    std::env::set_var("XDG_DATA_HOME", dir.path());
    std::env::set_var("GH_HOST", "ghe.corp");
    let mut first = App::new(Some("acme/r".to_string()));
    first.session.repo_slug = Some("acme/r".to_string());
    first.prefs_loaded = true;
    first.sidebar.query_active = "label:bug".to_string();
    first.board.filter = "auth".to_string();
    first.board.milestones.insert(Some("v1".to_string()));
    first.session.board.loading = true;
    first.open(crate::app::Entry::Board);
    first.persist_prefs();
    // Same owner and repo on github.com restore nothing: Format
    // prefs never bleed across hosts.
    std::env::set_var("GH_HOST", "github.com");
    let mut second = App::new(Some("acme/r".to_string()));
    second.session.repo_slug = Some("acme/r".to_string());
    second.apply_prefs();
    assert!(second.sidebar.query_active.is_empty(), "query stays on its host");
    second.restore_board_filter("acme");
    assert!(second.board.filter.is_empty(), "board filter stays on its host");
    assert!(second.board.milestones.is_empty(), "milestone pills stay on its host");
    match saved_host {
        Some(v) => std::env::set_var("GH_HOST", v),
        None => std::env::remove_var("GH_HOST"),
    }
    match saved_xdg {
        Some(v) => std::env::set_var("XDG_DATA_HOME", v),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

#[test]
fn persist_before_startup_load_writes_nothing() {
    let (_guard, _dir, saved) = temp_xdg();
    let mut prefs = github::load_prefs();
    prefs.queries.insert(github::prefs_repo_key("o/r"), "label:bug".to_string());
    prefs.board_cards.milestone = true;
    github::save_prefs(&prefs);
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    // Pre-bootstrap input mutates defaults and tries to persist, as
    // `f` then `m` would before the bootstrap lands.
    app.sidebar.show_merged_prs = false;
    app.board.fields.milestone = false;
    app.sidebar.query_active.clear();
    app.persist_prefs();
    let back = github::load_prefs();
    assert_eq!(
        back.queries.get(&github::prefs_repo_key("o/r")),
        Some(&"label:bug".to_string()),
        "unsaved query survives the early write"
    );
    assert!(back.board_cards.milestone, "unsaved card field survives the early write");
    assert!(back.show_merged_prs, "unsaved toggle survives the early write");
    // The bootstrap applies the saved state and re-arms persisting.
    app.apply_prefs();
    assert!(app.prefs_loaded);
    assert_eq!(app.sidebar.query_active, "label:bug");
    assert!(app.board.fields.milestone);
    app.persist_prefs();
    let live = github::load_prefs();
    assert_eq!(
        live.queries.get(&github::prefs_repo_key("o/r")),
        Some(&"label:bug".to_string()),
        "post-load persist keeps the query"
    );
    restore_xdg(saved);
}
