use super::*;

#[test]
fn bar_parses_numbers_and_commands() {
    assert_eq!(
        parse_bar("12 34"),
        BarCommand::Numbers(vec![ItemRef::github(12), ItemRef::github(34)])
    );
    assert_eq!(parse_bar("o"), BarCommand::OpenCurrent);
    assert_eq!(parse_bar("open"), BarCommand::OpenCurrent);
    assert_eq!(parse_bar("o 42"), BarCommand::Open(ItemRef::github(42)));
    assert_eq!(parse_bar("q"), BarCommand::Quit);
    assert_eq!(parse_bar("quit"), BarCommand::Quit);
    assert_eq!(parse_bar("exit"), BarCommand::Quit);
    assert_eq!(parse_bar(""), BarCommand::Dismiss);
    assert_eq!(parse_bar("o abc"), BarCommand::Hint);
    assert_eq!(parse_bar("hello"), BarCommand::Hint);
}

#[test]
fn selection_walks_and_clamps() {
    let order = vec![ItemRef::github(7), ItemRef::github(9)];
    assert_eq!(
        next_in_order(&order, Some(ItemRef::github(7)), 1),
        Some(ItemRef::github(9))
    );
    assert_eq!(
        next_in_order(&order, Some(ItemRef::github(9)), 1),
        Some(ItemRef::github(9))
    );
    assert_eq!(
        next_in_order(&order, Some(ItemRef::github(7)), -1),
        Some(ItemRef::github(7))
    );
    assert_eq!(next_in_order(&order, None, 1), Some(ItemRef::github(7)));
    assert_eq!(next_in_order(&order, None, -1), Some(ItemRef::github(9)));
    assert_eq!(next_in_order(&[], Some(ItemRef::github(7)), 1), None);
    assert_eq!(
        next_in_order(&order, Some(ItemRef::github(999)), 1),
        Some(ItemRef::github(7))
    );
}

#[test]
fn history_push_records_leaves_and_dedupes() {
    assert_eq!(
        history_push(&[], None, ItemRef::github(7)),
        Vec::<ItemRef>::new()
    );
    assert_eq!(
        history_push(&[], Some(ItemRef::github(7)), ItemRef::github(7)),
        Vec::<ItemRef>::new()
    );
    assert_eq!(
        history_push(&[], Some(ItemRef::github(7)), ItemRef::github(9)),
        vec![ItemRef::github(7)]
    );
    assert_eq!(
        history_push(
            &[ItemRef::github(7)],
            Some(ItemRef::github(7)),
            ItemRef::github(9)
        ),
        vec![ItemRef::github(7)]
    );
    assert_eq!(
        history_push(
            &[ItemRef::github(5)],
            Some(ItemRef::github(7)),
            ItemRef::github(9)
        ),
        vec![ItemRef::github(5), ItemRef::github(7)]
    );
}

#[test]
fn history_push_caps_at_limit() {
    let back: Vec<_> = (1..=HISTORY_LIMIT as u64).map(ItemRef::github).collect();
    let out = history_push(&back, Some(ItemRef::github(999)), ItemRef::github(1000));
    assert_eq!(out.len(), HISTORY_LIMIT);
    assert_eq!(out[0], ItemRef::github(2));
    assert_eq!(out[out.len() - 1], ItemRef::github(999));
}

#[test]
fn cursor_clamps_to_grid() {
    assert_eq!(board_cursor(3, 5, 0, 0, 1, 2), (1, 2));
    assert_eq!(board_cursor(3, 5, 0, 0, -1, -1), (0, 0));
    assert_eq!(board_cursor(3, 5, 2, 4, 1, 1), (2, 4));
    assert_eq!(board_cursor(3, 0, 0, 0, 0, 1), (0, 0));
    assert_eq!(board_cursor(0, 0, 0, 0, 1, 1), (0, 0));
}

#[test]
fn window_cycles_and_labels() {
    assert_eq!(cycle_window(1), 24);
    assert_eq!(cycle_window(24), 168);
    assert_eq!(cycle_window(168), 1);
    assert_eq!(cycle_window(99), 1);
    assert_eq!(window_label(1), "open + merged ≤1h");
    assert_eq!(window_label(24), "open + merged ≤24h");
    assert_eq!(window_label(168), "open + merged ≤7d");
    assert_eq!(window_label(0), "open + merged all time");
}

#[test]
fn pane_loading_label_names_the_fetch_scope() {
    assert_eq!(
        pane_loading_label(false, true, true, 24, ""),
        "Loading open + last 24h closed issues…"
    );
    assert_eq!(
        pane_loading_label(true, true, true, 168, ""),
        "Loading open + last 7d merged pull requests…"
    );
    assert_eq!(
        pane_loading_label(true, true, true, 0, ""),
        "Loading open + all merged pull requests…"
    );
    assert_eq!(
        pane_loading_label(false, true, false, 24, ""),
        "Loading open issues…"
    );
    assert_eq!(
        pane_loading_label(false, false, true, 1, ""),
        "Loading last 1h closed issues…"
    );
    assert_eq!(
        pane_loading_label(false, false, false, 24, ""),
        "Loading issues…"
    );
    assert_eq!(
        pane_loading_label(true, true, true, 24, " is:draft "),
        "Loading pull requests for “is:draft”…"
    );
}

#[test]
fn search_query_adds_the_window_token_only_when_dated() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(window_token(24, now).as_deref(), Some("updated:>=2026-11-09"));
    assert_eq!(window_token(168, now).as_deref(), Some("updated:>=2026-11-03"));
    assert_eq!(window_token(0, now), None, "all time adds no token");
    assert_eq!(search_query("label:bug", 24, now), "label:bug updated:>=2026-11-09");
    assert_eq!(search_query("  label:bug  ", 0, now), "label:bug");
    assert_eq!(search_query("", 24, now), "", "no query means no search");
}

#[test]
fn milestone_groups_order_by_due_then_title_with_none_last() {
    // 1 and 2 share v1, 3 is v2, 4 has no milestone, 5 is undated.
    let milestone = |n: ItemRef| match n {
        ItemRef {
            number: 1,
            source: crate::model::Source::GitHub,
        }
        | ItemRef {
            number: 2,
            source: crate::model::Source::GitHub,
        } => Some("v1".to_string()),
        ItemRef {
            number: 3,
            source: crate::model::Source::GitHub,
        } => Some("v2".to_string()),
        ItemRef {
            number: 5,
            source: crate::model::Source::GitHub,
        } => Some("backlog".to_string()),
        _ => None,
    };
    let due = |title: &str| match title {
        "v1" => Some("2026-02-01T00:00:00Z".to_string()),
        "v2" => Some("2026-01-01T00:00:00Z".to_string()),
        _ => None,
    };
    let groups = milestone_groups(
        &[
            ItemRef::github(1),
            ItemRef::github(2),
            ItemRef::github(3),
            ItemRef::github(4),
            ItemRef::github(5),
        ],
        &milestone,
        &due,
    );
    let labels: Vec<&str> = groups.iter().map(|(label, _)| label.as_str()).collect();
    assert_eq!(labels, vec!["v2", "v1", "backlog", "(no milestone)"]);
    assert_eq!(
        groups[1].1,
        vec![ItemRef::github(1), ItemRef::github(2)],
        "members keep the incoming order"
    );
    assert_eq!(groups[3].1, vec![ItemRef::github(4)]);
    assert!(milestone_groups(&[], &milestone, &due).is_empty());
}

#[test]
fn loading_names_each_pending_source() {
    for (state, source) in [
        (FetchState { issues: 1, ..FetchState::default() }, LoadSource::Issue),
        (FetchState { diffs: 2, ..FetchState::default() }, LoadSource::Diff),
        (FetchState { details: 3, ..FetchState::default() }, LoadSource::Details),
        (FetchState { board: true, ..FetchState::default() }, LoadSource::Board),
        (FetchState { sections_live: true, ..FetchState::default() }, LoadSource::Sections),
        (FetchState { bootstrap_live: true, ..FetchState::default() }, LoadSource::Sections),
    ] {
        let got = loading(&state);
        assert_eq!(got.len(), 1, "{state:?}");
        assert_eq!(got[0].source, source, "{state:?}");
        assert_eq!(got[0].progress, None, "{state:?}");
    }
}

#[test]
fn loading_idle_means_empty() {
    assert_eq!(loading(&FetchState::default()), vec![]);
}

#[test]
fn pane_loading_maps_fetches_to_panes() {
    assert_eq!(pane_loading(&PaneInputs::default()), PaneLoading::default());
    let sidebar = PaneInputs { bootstrap_live: true, ..PaneInputs::default() };
    assert_eq!(
        pane_loading(&sidebar),
        PaneLoading { sidebar: true, sidebar_spinner: true, ..PaneLoading::default() }
    );
    let both = PaneInputs {
        bootstrap_live: true,
        requested_pending: true,
        ..PaneInputs::default()
    };
    let quiet = pane_loading(&both);
    assert!(quiet.sidebar_spinner && !quiet.sidebar, "lower tier: spinner only");
    assert!(quiet.main && quiet.main_spinner, "current tier keeps its loading body");
    let fresh = PaneInputs { requested_pending: true, ..PaneInputs::default() };
    assert!(pane_loading(&fresh).main && pane_loading(&fresh).main_spinner);
    let cached = PaneInputs {
        requested_pending: true,
        shown_cached: true,
        ..PaneInputs::default()
    };
    assert!(
        !pane_loading(&cached).main,
        "cached text stays during refresh"
    );
    assert!(pane_loading(&cached).main_spinner, "spinner still shows");
    let refs = PaneInputs { refs_pending: true, ..PaneInputs::default() };
    assert!(pane_loading(&refs).refs);
}

#[test]
fn loading_message_wins_over_spinner() {
    let state = FetchState {
        issues: 1,
        diffs: 1,
        board: true,
        sections_live: true,
        message_visible: true,
        ..FetchState::default()
    };
    assert_eq!(loading(&state), vec![]);
}

#[test]
fn backfill_label_shows_progress_while_running_only() {
    let running = FetchState {
        backfill: true,
        backfill_progress: Some((300, 5000)),
        ..FetchState::default()
    };
    assert_eq!(loading_label(&loading(&running)), "backfill 300/5000");
    let unknown = FetchState { backfill: true, ..FetchState::default() };
    assert_eq!(loading_label(&loading(&unknown)), "backfill");
    let done = FetchState { backfill_progress: Some((300, 5000)), ..FetchState::default() };
    assert_eq!(loading(&done), vec![], "hidden when not running");
}

#[test]
fn progress_label_bounds_unknown_and_complete() {
    assert_eq!(progress_label(0, 0), None, "unknown total stays indeterminate");
    assert_eq!(progress_label(0, 5), Some("0/5".to_string()));
    assert_eq!(progress_label(2, 5), Some("2/5".to_string()));
    assert_eq!(progress_label(5, 5), None, "complete hides the numbers");
    assert_eq!(progress_label(7, 5), None, "over-complete hides too");
}

#[test]
fn loading_carries_determinate_progress_only() {
    let state = FetchState {
        issues: 1,
        issue_progress: Some((2, 7)),
        sections_live: true,
        section_progress: Some((120, 500)),
        ..FetchState::default()
    };
    let got = loading(&state);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].progress, Some((2, 7)));
    assert_eq!(got[1].progress, Some((120, 500)));
    assert_eq!(loading_label(&got), "issue 2/7 · sections 120/500");
    // Unknown (0/0) and complete totals collapse to the plain label.
    let flat = FetchState { issues: 1, issue_progress: Some((0, 0)), ..FetchState::default() };
    assert_eq!(loading(&flat)[0].progress, None);
    assert_eq!(loading_label(&loading(&flat)), "issue");
    let done = FetchState { issues: 1, issue_progress: Some((7, 7)), ..FetchState::default() };
    assert_eq!(loading_label(&loading(&done)), "issue");
}

#[test]
fn pins_toggle() {
    assert_eq!(
        toggle_pin(&[ItemRef::github(7)], ItemRef::github(9)),
        vec![ItemRef::github(7), ItemRef::github(9)]
    );
    assert_eq!(
        toggle_pin(
            &[ItemRef::github(7), ItemRef::github(9)],
            ItemRef::github(9)
        ),
        vec![ItemRef::github(7)]
    );
}

#[test]
fn sections_split_and_dedup() {
    let pinned: HashSet<ItemRef> = [9].map(ItemRef::github).into_iter().collect();
    let (p, v, r) = section_split(
        &[ItemRef::github(9)],
        &[ItemRef::github(9), ItemRef::github(7)],
        &[ItemRef::github(7), ItemRef::github(5)],
        &pinned,
    );
    assert_eq!(p, vec![ItemRef::github(9)]);
    assert_eq!(v, vec![ItemRef::github(7)]);
    assert_eq!(r, vec![ItemRef::github(5)]);
}

#[test]
fn pane_order_dividers_between_groups_only() {
    use crate::model::SideRow::{Divider, Item};
    assert_eq!(
        pane_order(
            &[ItemRef::github(9)],
            &[ItemRef::github(7)],
            &[ItemRef::github(5)]
        ),
        vec![
            Item(ItemRef::github(9)),
            Divider("requested".to_string()),
            Item(ItemRef::github(7)),
            Divider("recent".to_string()),
            Item(ItemRef::github(5))
        ]
    );
    assert_eq!(pane_order(&[], &[ItemRef::github(7)], &[]), vec![Item(ItemRef::github(7))]);
    assert_eq!(pane_order(&[], &[], &[]), Vec::<crate::model::SideRow>::new());
    assert_eq!(
        pane_order(&[ItemRef::github(9)], &[], &[ItemRef::github(5)]),
        vec![
            Item(ItemRef::github(9)),
            Divider("recent".to_string()),
            Item(ItemRef::github(5))
        ]
    );
}

#[test]
fn search_matches_title_or_number() {
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "fix"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "FIX"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "#23"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "12"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "zzz"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        ""
    ));
}

#[test]
fn search_matches_label_name() {
    let labels = ["bug", "needs-triage"];
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &labels,
        "alice",
        "bug"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &labels,
        "alice",
        "BUG"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &labels,
        "alice",
        "triage"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &labels,
        "alice",
        "feature"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "alice",
        "bug"
    ));
}

#[test]
fn search_matches_author_login() {
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "octocat",
        "octo"
    ));
    assert!(matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "octocat",
        "OCTOCAT"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "octocat",
        "monalisa"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &[],
        "",
        "octo"
    ));
}

#[test]
fn search_empty_query_matches_all() {
    assert!(matches_search(
        ItemRef::github(7),
        "Anything",
        &["bug"],
        "alice",
        ""
    ));
    assert!(matches_search(
        ItemRef::github(7),
        "Anything",
        &["bug"],
        "alice",
        "   "
    ));
    assert!(matches_search(ItemRef::github(7), "Anything", &[], "", ""));
}

#[test]
fn board_filter_matches_label_title_and_exact_number_but_not_milestone() {
    let labels = ["feature"];
    assert!(matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "board"
    ));
    assert!(matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "FEATURE"
    ));
    // Milestones filter through the dialog pills, never the text.
    assert!(!matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "v0.3"
    ));
    assert!(matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "#103"
    ));
    assert!(!matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "#10"
    ));
    assert!(!matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "103"
    ));
    assert!(!matches_board_filter(
        Some(ItemRef::github(103)),
        "Filter board cards",
        &labels,
        "zzz"
    ));
}

#[test]
fn board_filter_empty_matches_all_and_drafts_never_match_numbers() {
    assert!(matches_board_filter(
        Some(ItemRef::github(103)),
        "Card",
        &[],
        ""
    ));
    assert!(matches_board_filter(None, "Draft", &[], "   "));
    assert!(!matches_board_filter(None, "Draft", &[], "#1"));
    assert!(!matches_board_filter(None, "Draft", &[], "#"));
}

fn sort_issue(
    number: ItemRef,
    updated: &str,
    author: &str,
    state: &str,
    labels: &[&str],
) -> IssueData {
    IssueData {
        number,
        title: format!("issue {number}", number = number.number),
        state: state.to_string(),
        issue_type: String::new(),
        priority: String::new(),
        author: crate::model::Author { login: author.to_string() },
        labels: labels.iter().map(|n| crate::model::Label { name: n.to_string() }).collect(),
        assignees: vec![],
        created_at: String::new(),
        updated_at: updated.to_string(),
        url: String::new(),
        body: String::new(),
        comments: vec![],
        is_pr: false,
        review_threads: vec![],
        paired: vec![],
        checks: None,
        milestone: None,
        parent: None,
        sub_summary: None,
        links: Default::default(),
        head_ref_name: String::new(),
        jira_refs: vec![],
    }
}

#[test]
fn tree_orders_siblings_promotes_orphans_and_caps_depth() {
    let mut data = HashMap::new();
    for n in 1..=7 {
        let mut issue = sort_issue(ItemRef::github(n), "", "", "OPEN", &[]);
        issue.parent = match n {
            1 => None,
            2 | 3 => Some(ItemRef::github(1)),
            _ => Some(ItemRef::github(n - 1)),
        };
        data.insert(ItemRef::github(n), issue);
    }
    let rows = tree_rows(
        &[
            ItemRef::github(1),
            ItemRef::github(3),
            ItemRef::github(2),
            ItemRef::github(4),
            ItemRef::github(5),
            ItemRef::github(6),
            ItemRef::github(7),
        ],
        &data,
        &HashSet::new(),
        &HashMap::new(),
    );
    assert_eq!(
        rows.iter().filter_map(|r| r.number()).collect::<Vec<_>>(),
        vec![
            ItemRef::github(1),
            ItemRef::github(3),
            ItemRef::github(4),
            ItemRef::github(5),
            ItemRef::github(6),
            ItemRef::github(7),
            ItemRef::github(2)
        ]
    );
    assert!(matches!(
        rows[5],
        crate::model::SideRow::Tree { depth: 4, .. }
    ));
    let orphan = tree_rows(
        &[ItemRef::github(2)],
        &data,
        &HashSet::new(),
        &HashMap::new(),
    );
    assert!(matches!(
        orphan[0],
        crate::model::SideRow::Tree { depth: 0, .. }
    ));
    let folded = tree_rows(
        &[ItemRef::github(1), ItemRef::github(2), ItemRef::github(3)],
        &data,
        &HashSet::from([ItemRef::github(1)]),
        &HashMap::new(),
    );
    assert_eq!(folded.len(), 1);
}

#[test]
fn unloaded_parent_reads_folded_until_its_children_load() {
    use crate::model::{LazyChildren, SideRow, SubSummary};
    let mut data = HashMap::new();
    let mut parent = sort_issue(ItemRef::github(1), "", "", "OPEN", &[]);
    parent.sub_summary = Some(SubSummary { total: 1, completed: 0 });
    data.insert(ItemRef::github(1), parent);
    let folded = |lazy: &HashMap<ItemRef, LazyChildren>| match &tree_rows(
        &[ItemRef::github(1)],
        &data,
        &HashSet::new(),
        lazy,
    )[0]
    {
        SideRow::Tree {
            folded,
            has_children,
            ..
        } => {
            assert!(has_children);
            *folded
        }
        other => panic!("{other:?}"),
    };
    assert!(folded(&HashMap::new()), "no children yet: show folded");
    assert!(
        !folded(&HashMap::from([(
            ItemRef::github(1),
            LazyChildren::Loading
        )])),
        "loading opens it"
    );
}

#[test]
fn tree_guides_continue_past_open_siblings() {
    // 1 ─┬ 2 ── 4
    //    └ 3
    let mut data = HashMap::new();
    for (n, parent) in [(1, None), (2, Some(1)), (3, Some(1)), (4, Some(2))] {
        let mut issue = sort_issue(ItemRef::github(n), "", "", "OPEN", &[]);
        issue.parent = parent.map(ItemRef::github);
        data.insert(ItemRef::github(n), issue);
    }
    let guides: Vec<String> = tree_rows(
        &[
            ItemRef::github(1),
            ItemRef::github(2),
            ItemRef::github(4),
            ItemRef::github(3),
        ],
        &data,
        &HashSet::new(),
        &HashMap::new(),
    )
    .into_iter()
    .map(|row| match row {
        crate::model::SideRow::Tree { guide, .. } => guide,
        other => panic!("{other:?}"),
    })
    .collect();
    assert_eq!(guides, vec!["", "├─", "│ └─", "└─"]);
}

#[test]
fn tree_merges_lazy_children_without_duplicates() {
    use crate::model::{LazyChildren, SideRow};
    let mut data = HashMap::new();
    let mut parent = sort_issue(ItemRef::github(1), "", "", "OPEN", &[]);
    parent.sub_summary = Some(crate::model::SubSummary { total: 3, completed: 1 });
    let mut fetched = sort_issue(ItemRef::github(2), "", "", "OPEN", &[]);
    fetched.parent = Some(ItemRef::github(1));
    data.insert(ItemRef::github(1), parent);
    data.insert(ItemRef::github(2), fetched);
    data.insert(
        ItemRef::github(3),
        sort_issue(ItemRef::github(3), "", "", "CLOSED", &[]),
    );
    let lazy = HashMap::from([(
        ItemRef::github(1),
        LazyChildren::Loaded(vec![ItemRef::github(2), ItemRef::github(3)]),
    )]);
    let rows = tree_rows(
        &[ItemRef::github(1), ItemRef::github(2)],
        &data,
        &HashSet::new(),
        &lazy,
    );
    let numbers: Vec<ItemRef> = rows.iter().filter_map(|r| r.number()).collect();
    assert_eq!(
        numbers,
        vec![ItemRef::github(1), ItemRef::github(2), ItemRef::github(3)]
    );
    assert_eq!(
        numbers.iter().filter(|n| **n == ItemRef::github(2)).count(),
        1,
        "no duplicate row"
    );
    assert!(matches!(
        rows[1],
        SideRow::Tree {
            number: ItemRef {
                number: 2,
                source: crate::model::Source::GitHub
            },
            depth: 1,
            outside: false,
            ..
        }
    ));
    assert!(matches!(
        rows[2],
        SideRow::Tree {
            number: ItemRef {
                number: 3,
                source: crate::model::Source::GitHub
            },
            depth: 1,
            outside: true,
            ..
        }
    ));
}

#[test]
fn tree_summary_marks_parent_and_loading_child() {
    use crate::model::{LazyChildren, SideRow, SubSummary};
    let mut data = HashMap::new();
    let mut parent = sort_issue(ItemRef::github(1), "", "", "OPEN", &[]);
    parent.sub_summary = Some(SubSummary { total: 2, completed: 0 });
    data.insert(ItemRef::github(1), parent);
    let lazy = HashMap::from([(ItemRef::github(1), LazyChildren::Loading)]);
    let rows = tree_rows(&[ItemRef::github(1)], &data, &HashSet::new(), &lazy);
    assert!(matches!(
        rows[0],
        SideRow::Tree {
            number: ItemRef {
                number: 1,
                source: crate::model::Source::GitHub
            },
            has_children: true,
            ..
        }
    ));
    assert!(matches!(rows[1], SideRow::Loading { .. }));
    let folded = tree_rows(
        &[ItemRef::github(1)],
        &data,
        &HashSet::from([ItemRef::github(1)]),
        &lazy,
    );
    assert_eq!(folded.len(), 1, "a folded parent hides its loading child");
}

#[test]
fn tree_nests_a_query_hit_under_its_lazy_parent_once() {
    use crate::model::{LazyChildren, SideRow};
    let mut data = HashMap::new();
    data.insert(
        ItemRef::github(1),
        sort_issue(ItemRef::github(1), "", "", "OPEN", &[]),
    );
    data.insert(
        ItemRef::github(2),
        sort_issue(ItemRef::github(2), "", "", "OPEN", &[]),
    );
    data.insert(
        ItemRef::github(9),
        sort_issue(ItemRef::github(9), "", "", "OPEN", &[]),
    );
    let lazy = HashMap::from([(
        ItemRef::github(1),
        LazyChildren::Loaded(vec![ItemRef::github(2), ItemRef::github(9)]),
    )]);
    // Search rows carry no parent link: #2 arrives top-level and must
    // move under #1 at normal brightness.
    let rows = tree_rows(
        &[ItemRef::github(1), ItemRef::github(2)],
        &data,
        &HashSet::new(),
        &lazy,
    );
    let numbers: Vec<ItemRef> = rows.iter().filter_map(|r| r.number()).collect();
    assert_eq!(
        numbers,
        vec![ItemRef::github(1), ItemRef::github(2), ItemRef::github(9)]
    );
    assert!(matches!(
        rows[1],
        SideRow::Tree {
            number: ItemRef {
                number: 2,
                source: crate::model::Source::GitHub
            },
            depth: 1,
            outside: false,
            ..
        }
    ));
    assert!(matches!(
        rows[2],
        SideRow::Tree {
            number: ItemRef {
                number: 9,
                source: crate::model::Source::GitHub
            },
            depth: 1,
            outside: true,
            ..
        }
    ));
}

#[test]
fn lazy_fetch_waits_for_missing_children() {
    use crate::model::{LazyChildren, SubSummary};
    let mut data = HashMap::new();
    let mut parent = sort_issue(ItemRef::github(1), "", "", "OPEN", &[]);
    parent.sub_summary = Some(SubSummary { total: 2, completed: 0 });
    let mut child = sort_issue(ItemRef::github(2), "", "", "OPEN", &[]);
    child.parent = Some(ItemRef::github(1));
    data.insert(ItemRef::github(1), parent);
    data.insert(ItemRef::github(2), child);
    let mut lazy = HashMap::new();
    assert!(
        needs_lazy_children(ItemRef::github(1), &data, &lazy),
        "one of two children fetched"
    );
    lazy.insert(ItemRef::github(1), LazyChildren::Loading);
    assert!(
        !needs_lazy_children(ItemRef::github(1), &data, &lazy),
        "one fetch per parent per session"
    );
    lazy.insert(ItemRef::github(1), LazyChildren::Loaded(vec![]));
    assert!(
        !needs_lazy_children(ItemRef::github(1), &data, &lazy),
        "session cache never refetches"
    );
    lazy.clear();
    data.get_mut(&ItemRef::github(1)).unwrap().sub_summary = Some(SubSummary {
        total: 1,
        completed: 0,
    });
    assert!(
        !needs_lazy_children(ItemRef::github(1), &data, &lazy),
        "every child already fetched"
    );
    data.get_mut(&ItemRef::github(1)).unwrap().sub_summary = None;
    assert!(
        !needs_lazy_children(ItemRef::github(1), &data, &lazy),
        "no summary means no known children"
    );
}

fn sort_data() -> HashMap<ItemRef, IssueData> {
    [
        sort_issue(
            ItemRef::github(7),
            "2026-01-05T00:00:00Z",
            "carol",
            "OPEN",
            &["ui"],
        ),
        sort_issue(
            ItemRef::github(9),
            "2026-01-09T00:00:00Z",
            "alice",
            "CLOSED",
            &["bug"],
        ),
        sort_issue(
            ItemRef::github(11),
            "2026-01-01T00:00:00Z",
            "bob",
            "MERGED",
            &["docs"],
        ),
    ]
    .into_iter()
    .map(|d| (d.number, d))
    .collect()
}

#[test]
fn default_sort_restores_the_incoming_order() {
    let rows = vec![ItemRef::github(11), ItemRef::github(7), ItemRef::github(9)];
    assert_eq!(sort_rows(rows.clone(), SideSort::Default, false, &sort_data()), rows);
    assert_eq!(sort_rows(rows.clone(), SideSort::Default, true, &sort_data()), rows);
}

#[test]
fn updated_sort_puts_newest_first_and_reverses() {
    let data = sort_data();
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Updated,
            true,
            &data
        ),
        vec![ItemRef::github(9), ItemRef::github(7), ItemRef::github(11)]
    );
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Updated,
            false,
            &data
        ),
        vec![ItemRef::github(11), ItemRef::github(7), ItemRef::github(9)]
    );
}

#[test]
fn author_sort_orders_by_login() {
    let data = sort_data();
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Author,
            false,
            &data
        ),
        vec![ItemRef::github(9), ItemRef::github(11), ItemRef::github(7)]
    );
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Author,
            true,
            &data
        ),
        vec![ItemRef::github(7), ItemRef::github(11), ItemRef::github(9)]
    );
}

#[test]
fn state_sort_orders_lexically() {
    let data = sort_data();
    // closed < merged < open once case-folded.
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::State,
            false,
            &data
        ),
        vec![ItemRef::github(9), ItemRef::github(11), ItemRef::github(7)]
    );
}

#[test]
fn label_sort_orders_by_first_label_name() {
    let data = sort_data();
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Label,
            false,
            &data
        ),
        vec![ItemRef::github(9), ItemRef::github(11), ItemRef::github(7)]
    );
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Label,
            true,
            &data
        ),
        vec![ItemRef::github(7), ItemRef::github(11), ItemRef::github(9)]
    );
}

#[test]
fn sort_cycle_ends_back_at_default() {
    let mut sort = SideSort::Default;
    let mut seen = vec![sort];
    for _ in 0..4 {
        sort = sort.next();
        seen.push(sort);
    }
    assert_eq!(
        seen,
        vec![
            SideSort::Default,
            SideSort::Updated,
            SideSort::Author,
            SideSort::State,
            SideSort::Label
        ]
    );
    assert_eq!(sort.next(), SideSort::Default);
}

#[test]
fn sort_direction_toggle_flips_key_order() {
    let data = sort_data();
    let asc = sort_rows(
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
        SideSort::Updated,
        false,
        &data,
    );
    let desc = sort_rows(
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
        SideSort::Updated,
        true,
        &data,
    );
    assert_eq!(asc.iter().rev().copied().collect::<Vec<_>>(), desc);
}

#[test]
fn sort_keeps_loading_rows_last_and_labels_cover_directions() {
    let mut data = sort_data();
    data.remove(&ItemRef::github(11));
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
            SideSort::Updated,
            false,
            &data
        ),
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)]
    );
    assert_eq!(
        sort_rows(
            vec![ItemRef::github(11), ItemRef::github(9), ItemRef::github(7)],
            SideSort::Author,
            true,
            &data
        ),
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)]
    );
    assert_eq!(SideSort::Default.label(true), "default");
    assert_eq!(SideSort::Updated.label(true), "updated ↓");
    assert_eq!(SideSort::Author.label(false), "author ↑");
    assert_eq!(SideSort::State.label(false), "state ↑");
    assert_eq!(SideSort::Label.label(false), "label ↑");
}

#[test]
fn search_no_match_on_unrelated_query() {
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &["bug"],
        "alice",
        "zzz"
    ));
    assert!(!matches_search(
        ItemRef::github(123),
        "Fix login",
        &["bug"],
        "alice",
        "999"
    ));
}

#[test]
fn search_ranges_find_all_hits() {
    assert_eq!(match_ranges("Fix the thing", "th"), vec![(4, 6), (8, 10)]);
    assert_eq!(match_ranges("Fix the thing", ""), Vec::new());
    assert_eq!(match_ranges("abc", "z"), Vec::new());
}

#[test]
fn toast_snippet_trims_with_ellipsis() {
    assert_eq!(toast_snippet("short", 80), "short");
    assert_eq!(toast_snippet("12345", 5), "12345");
    assert_eq!(toast_snippet("123456", 5), "1234…");
    // Multibyte-safe: counts chars, never splits one.
    assert_eq!(toast_snippet("héllo wörld", 6), "héllo…");
    assert_eq!(toast_snippet("anything", 1), "…");
}

#[test]
fn yank_issue_collapses_title_to_one_line() {
    assert_eq!(
        yank_issue_text(
            ItemRef::github(7),
            "Fix  login\nbug",
            "https://example.com/7"
        ),
        "#7 Fix login bug — https://example.com/7"
    );
}

#[test]
fn yank_diff_names_file_and_hunk() {
    let file = DiffFile {
        filename: "src/a.rs".to_string(),
        status: "modified".to_string(),
        additions: 10,
        deletions: 2,
        patch: String::new(),
    };
    let text = yank_diff_text(
        ItemRef::github(9),
        "Add thing",
        "https://example.com/pull/9",
        &file,
        Some("@@ -1 +1 @@"),
    );
    assert_eq!(
        text,
        "#9 Add thing — src/a.rs (modified +10 -2 @@ -1 +1 @@) — https://example.com/pull/9"
    );
    let folded_file = DiffFile {
        filename: "src/a.rs".to_string(),
        status: "added".to_string(),
        additions: 5,
        deletions: 0,
        patch: String::new(),
    };
    let folded = yank_diff_text(ItemRef::github(9), "Add thing", "u", &folded_file, None);
    assert_eq!(folded, "#9 Add thing — src/a.rs (added +5 -0) — u");
}

#[test]
fn yank_ref_reports_raw_body_line_and_snippet() {
    let body = "intro line\nsee #9 for details\nlast";
    let text = yank_ref_text(
        ItemRef::github(7),
        ItemRef::github(9),
        body,
        None,
        Some("Nine"),
        Some("u9"),
    )
    .unwrap();
    assert_eq!(
        text,
        "#7 refs #9 (body line 2): \"see #9 for details\" — Nine u9"
    );
}

#[test]
fn yank_ref_returns_none_without_mention() {
    assert!(yank_ref_text(
        ItemRef::github(7),
        ItemRef::github(9),
        "no refs here",
        None,
        None,
        None
    )
    .is_none());
}

#[test]
fn yank_diff_line_quotes_active_line() {
    assert_eq!(
        yank_diff_line_text(ItemRef::github(9), "Add thing", "u", "src/a.rs", "  +new"),
        "#9 Add thing — src/a.rs: \"+new\" — u"
    );
}
