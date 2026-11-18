//! Frame tests for the modal dialogs.

use crate::app::Tab;
use crate::input::{handle_key, handle_mouse};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use super::test_support::{board_app, press_cell, frame_text_sized, sample, loaded_app, key, TempPrefs, click, frame_text, diff_pr};

#[test]
fn format_dialog_clicks_pick_options_and_toggle_chips() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let _ = frame_text(&mut app);
    let left = app.format_left_widgets();
    // A choice option selects that option and focuses its row.
    press_cell(&mut app, left + 2, Some(2));
    assert_eq!(app.sidebar.sort, crate::core::SideSort::Author);
    assert_eq!(app.sidebar.format_focus, left + 2);
    let _ = frame_text(&mut app);
    press_cell(&mut app, left + 3, Some(1));
    assert!(app.sidebar.sort_desc, "descending option");
    // A chip toggles like space.
    let _ = frame_text(&mut app);
    let before = app.sidebar.show_open_prs;
    press_cell(&mut app, 2, None);
    assert_eq!(app.sidebar.show_open_prs, !before);
    // The search row starts the query edit.
    let _ = frame_text(&mut app);
    press_cell(&mut app, 0, None);
    assert!(app.query_edit());
    // Clicks never reach the panes behind the dialog.
    let current = app.nav.current;
    let side = app.hits.side_issue_area;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), side.x + 1, side.y + 2));
    assert_eq!(app.nav.current, current);
    assert!(app.format_open(), "edit swallows outside clicks");
}

#[test]
fn format_dialog_outside_click_closes_and_wheel_is_inert() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let _ = frame_text(&mut app);
    let scroll = app.nav.scroll;
    let p = app.hits.form_popup;
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, p.x + 2, p.y + 2));
    assert_eq!(app.nav.scroll, scroll);
    assert!(app.format_open());
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), 0, 0));
    assert!(!app.format_open(), "outside press closes like Esc");
}

#[test]
fn filter_dialog_toggles_hide_recent_rows() {
    // Toggles persist to prefs.json, so isolate the data dir.
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    app.prefs_loaded = true;
    let mut merged = sample(20);
    merged.is_pr = true;
    merged.state = "MERGED".to_string();
    app.issues.data.insert(20, merged);
    app.issues.statuses.insert(20, vec![]);
    app.nav.recent.push(20);
    use crate::model::SideRow::{Divider, Item};
    assert_eq!(app.pr_rows(), vec![Item(9), Divider("recent".to_string()), Item(20)]);
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(app.format_open());
    let text = frame_text(&mut app);
    assert!(text.contains("Format"));
    assert!(text.contains("1 Query"));
    handle_key(&mut app, key(KeyCode::Char('m')));
    assert!(!app.sidebar.show_merged_prs);
    assert_eq!(app.pr_rows(), vec![Item(9)]);
    // Requested rows ignore the toggle.
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(!app.format_open());
}

#[test]
fn filter_query_draft_commits_and_clears() {
    use crate::app::QueryAction;
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    app.start_query_edit();
    assert_eq!(app.sidebar.query_draft, "");
    app.sidebar.query_draft = "label:bug".to_string();
    assert_eq!(app.commit_query_draft(), QueryAction::Run("label:bug".to_string()));
    assert_eq!(app.sidebar.query_active, "label:bug");
    assert!(!app.format_open());
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    app.start_query_edit();
    assert_eq!(app.sidebar.query_draft, "label:bug");
    // Esc in edit mode reverts the draft but keeps the dialog.
    app.sidebar.query_draft = "other".to_string();
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.format_open());
    assert!(!app.query_edit());
    assert_eq!(app.sidebar.query_draft, "label:bug");
    assert_eq!(app.sidebar.query_active, "label:bug");
    // Empty commit clears back to the window listing.
    app.start_query_edit();
    app.sidebar.query_draft = String::new();
    assert_eq!(app.commit_query_draft(), QueryAction::Refetch);
    assert!(app.sidebar.query_active.is_empty());
}

#[test]
fn capital_f_opens_the_format_dialog_in_query_edit_mode() {
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('F')));
    assert!(app.format_open(), "F opens the sidebar dialog");
    assert!(app.query_edit(), "the query field starts editing");
    let text = frame_text(&mut app);
    assert!(text.contains("1 Query"), "{text}");
    handle_key(&mut app, key(KeyCode::Char('b')));
    handle_key(&mut app, key(KeyCode::Char('u')));
    handle_key(&mut app, key(KeyCode::Char('g')));
    assert_eq!(app.sidebar.query_draft, "bug");
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.format_open(), "esc keeps the dialog up");
    assert!(!app.query_edit());
    assert!(app.sidebar.query_draft.is_empty(), "esc reverts the draft");
    // `ctrl-s` submits the pending review; it never opens the dialog.
    app.close(crate::app::Slot::Format);
    handle_key(&mut app, crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(!app.format_open());
    assert!(app.message.contains("no pending comments"), "{}", app.message);
}

#[test]
fn sidebar_format_uses_columns_and_wraps_chips() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    app.sidebar.labels = ["bug", "ui", "perf", "docs", "chore"]
        .iter().map(|s| s.to_string()).collect();
    let text = frame_text_sized(&mut app, 120, 40);
    let rows: Vec<String> = text.chars().collect::<Vec<_>>().chunks(120)
        .map(|row| row.iter().collect()).collect();
    let header = rows.iter().find(|row| row.contains("1 Query")).unwrap();
    assert!(header.contains("3 Group"), "{header}");
    assert!(header.find("1 Query") < header.find("3 Group"), "{header}");
    for title in ["2 Filter", "4 Sort", "Display"] {
        assert!(text.contains(title), "missing {title}: {text}");
    }
    assert!(rows.iter().any(|row| row.contains("bug") && row.contains("ui")
        && row.contains("perf")), "chips share a row: {text}");
    assert!(app.sidebar.format_columns);
}

#[test]
fn sidebar_format_narrow_scrolls_to_focus() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    app.sidebar.labels = (0..40).map(|n| format!("label{n}")).collect();
    app.sidebar.format_focus = app.format_widgets() - 1;
    let text = frame_text_sized(&mut app, 80, 24);
    assert!(!app.sidebar.format_columns);
    assert!(text.contains("ci checks") && text.contains("▌"), "{text}");
    assert!(text.contains("esc close"), "footer remains visible: {text}");
}

#[test]
fn saved_queries_save_overwrite_apply_delete_and_render() {
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    app.prefs_loaded = true;
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    assert!(frame_text(&mut app).contains("none (s saves the query)"));
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert!(!app.name_edit());
    assert_eq!(app.query_tag(), None);
    app.sidebar.query_active = "label:bug".into();
    assert_eq!(app.query_tag(), Some("query"));
    handle_key(&mut app, key(KeyCode::Char('s')));
    app.sidebar.name_draft = "bugs".into();
    assert!(frame_text(&mut app).contains("bugs▌"));
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.query_tag(), Some("bugs"));
    let text = frame_text(&mut app);
    assert!(text.contains("[bugs]") && text.contains("label:bug"), "{text}");
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert_eq!(app.sidebar.name_draft, "bugs");
    handle_key(&mut app, key(KeyCode::Esc));
    app.sidebar.query_active = "is:open".into();
    handle_key(&mut app, key(KeyCode::Char('s')));
    app.sidebar.name_draft = "bugs".into();
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.sidebar.saved_queries.len(), 1);
    assert_eq!(app.sidebar.saved_queries[0].query, "is:open");
    assert_eq!(crate::github::load_prefs().saved_queries, app.sidebar.saved_queries);
    app.sidebar.labels = vec!["bug".into()];
    app.sidebar.format_focus = 7;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.sidebar.label_filter.contains("bug"));
    app.sidebar.query_active = "other".into();
    app.sidebar.format_focus = 1;
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.sidebar.query_active, "is:open");
    assert_eq!(app.query_tag(), Some("bugs"));
    handle_key(&mut app, key(KeyCode::Char('x')));
    assert!(app.sidebar.saved_queries.is_empty());
    assert_eq!(app.query_tag(), Some("query"));
}

#[test]
fn sidebar_format_navigation_and_sort_match_shortcuts() {
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    let mut older = sample(11);
    older.updated_at = "2020-01-01T00:00:00Z".into();
    app.issues.data.insert(11, older);
    app.nav.visited.push(11);
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    frame_text(&mut app);
    let left = app.format_left_widgets();
    handle_key(&mut app, key(KeyCode::Char('l')));
    assert!(app.sidebar.format_focus >= left);
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert!(app.sidebar.format_focus >= left);
    handle_key(&mut app, key(KeyCode::Char('h')));
    assert!(app.sidebar.format_focus < left);
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert!(app.sidebar.format_focus < left);
    app.sidebar.format_focus = left + 2;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert_eq!(app.sidebar.sort, crate::core::SideSort::Updated);
    assert!(app.sidebar.sort_desc);
    let descending = app.issue_rows();
    app.sidebar.format_focus = left + 3;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.sidebar.sort_desc);
    assert_ne!(app.issue_rows(), descending);
    let mut via_shortcuts = loaded_app();
    handle_key(&mut via_shortcuts, key(KeyCode::Char('s')));
    handle_key(&mut via_shortcuts, key(KeyCode::Char('S')));
    assert_eq!((app.sidebar.sort, app.sidebar.sort_desc),
        (via_shortcuts.sidebar.sort, via_shortcuts.sidebar.sort_desc));
}

#[test]
fn format_dialog_lists_pills_and_space_toggles_them() {
    use crate::model::GroupBy;
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string(), "docs".to_string()];
    app.sidebar.milestones = vec![crate::model::Milestone {
        title: "v1".to_string(),
        due_on: None,
    }];
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    app.sidebar.format_focus = 6;
    let text = frame_text(&mut app);
    assert!(text.contains("bug") && text.contains("docs"), "label pills: {text}");
    assert!(text.contains("v1") && text.contains("(no milestone)"), "milestone pills: {text}");
    let fetching = app.sections.rx.is_some();
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.sidebar.label_filter.contains("bug"), "space flips the focused label");
    assert!(!app.sidebar.label_filter.contains("docs"));
    assert_eq!(app.sections.rx.is_some(), fetching, "pills never refetch");
    // Milestone pills follow the labels: v1, then "(no milestone)".
    app.sidebar.format_focus = 8;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.sidebar.milestone_filter.contains(&Some("v1".to_string())));
    app.sidebar.format_focus = 9;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.sidebar.milestone_filter.contains(&None), "(no milestone) pill");
    // The grouping row sits below the pills.
    app.sidebar.format_focus = 10;
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert_eq!(app.sidebar.group_by, GroupBy::Milestone);
    let text = frame_text(&mut app);
    assert!(text.contains("filtered"), "pane title marks active filters: {text}");
}

#[test]
fn row_fields_dialog_toggles_with_jk_and_space() {
    // Row flips persist, so isolate the data dir.
    let _prefs = TempPrefs::new();
    let mut app = loaded_app();
    app.prefs_loaded = true;
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(app.format_open());
    let text = frame_text(&mut app);
    assert!(text.contains("Format"));
    assert!(text.contains("Display") && text.contains("issues"));
    assert!(text.contains("PRs"));
    assert!(app.sidebar.row_issue.labels);
    app.sidebar.format_focus = app.format_left_widgets() + 4;
    handle_key(&mut app, key(KeyCode::Char('j')));
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.sidebar.row_issue.updated, "space flips the focused row");
    // The existing single keys still work beside the new navigation.
    handle_key(&mut app, key(KeyCode::Char('m')));
    assert!(!app.sidebar.show_merged_prs);
    // Both flips persisted across sessions.
    let prefs = crate::cache::load_prefs();
    assert!(!prefs.row_issue.updated);
    assert!(!prefs.show_merged_prs);
}

#[test]
fn board_format_dialog_toggles_card_fields_with_space() {
    // Card flips persist, so isolate the data dir.
    let _prefs = TempPrefs::new();
    let mut app = board_app(1);
    app.prefs_loaded = true;
    handle_key(&mut app, key(KeyCode::Char('f')));
    assert!(app.format_open());
    // One pill sits between the text row and the card fields.
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(!app.board.fields.number, "space flips the focused field");
    let prefs = crate::cache::load_prefs();
    assert!(!prefs.board_cards.number, "the flip persisted");
}

#[test]
fn format_dialogs_fit_short_terminal_and_keep_focus_visible() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    // Deepest focused widget in the sidebar dialog.
    app.sidebar.format_focus = app.format_widgets().saturating_sub(1);
    let text = frame_text_sized(&mut app, 100, 24);
    assert!(text.contains("PRs"), "section header fits: {text}");
    assert!(text.contains("ci checks"), "focused row stays visible: {text}");
    assert!(text.contains("▌"), "focus marker stays visible: {text}");
    // Ten pills push the board dialog past 24 rows: the popup caps
    // at the area and scrolls the focused card field into view.
    let mut board = board_app(10);
    for (i, item) in board.board.data.as_mut().unwrap().items.iter_mut().enumerate() {
        item.milestone = Some(format!("m{i}"));
    }
    board.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let widgets = board.board_pills().len() + crate::model::CardFields::ORDER.len();
    board.board.filter_focus = widgets;
    let text = frame_text_sized(&mut board, 100, 24);
    assert!(text.contains("card fields"), "card fields header fits: {text}");
    assert!(text.contains("comments"), "focused field stays visible: {text}");
    assert!(text.contains("▌"), "board focus marker stays visible: {text}");
}

#[test]
fn compose_modal_shows_thread_target_and_draft() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert!(app.compose_open());
    for ch in "# hi".chars() {
        handle_key(&mut app, key(KeyCode::Char(ch)));
    }
    let text = frame_text(&mut app);
    assert!(text.contains("Comment on #7"), "{text}");
    assert!(text.contains("# hi"), "draft: {text}");
    assert!(text.contains("ctrl-s send · ctrl-r preview"), "{text}");
}

#[test]
fn auth_modal_names_scopes_and_command() {
    let mut app = loaded_app();
    app.open_reauth(
        "token lacks scopes ghit needs".to_string(),
        vec!["project".to_string()],
        true,
    );
    let text = frame_text(&mut app);
    assert!(text.contains("GitHub auth"), "{text}");
    assert!(text.contains("missing scopes:"), "{text}");
    assert!(text.contains("project"), "{text}");
    assert!(text.contains("gh auth refresh -s repo -s read:org -s project"), "{text}");
    assert!(text.contains("y re-auth now"), "{text}");
}

// No second frame test for the dead-token state: same modal,
// same layout, and the live drive covers it.

#[test]
fn compose_on_diff_line_targets_file_and_line() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.diff.expanded_files.insert((9, "a.rs".into()));
    app.set_tab(Tab::Visual);
    let _ = frame_text(&mut app);
    // Header, hunk, deletion, then the addition the comment lands on.
    for _ in 0..3 {
        handle_key(&mut app, key(KeyCode::Down));
    }
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert_eq!(
        app.draft().map(|c| c.target.clone()),
        Some(crate::app::ComposeTarget::Line { path: "a.rs".to_string(), line: 1 }),
    );
    let text = frame_text(&mut app);
    assert!(text.contains("a.rs:1"), "{text}");
}

#[test]
fn compose_on_thread_block_targets_the_reply() {
    use crate::model::{DiffFile, ReviewThread, ReviewComment};
    let mut app = loaded_app();
    let mut pr = diff_pr(9);
    pr.review_threads = vec![
        ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(1), comments: vec![ReviewComment { id: "c".into(), body: "inline note".into(), ..Default::default() }], ..Default::default() },
    ];
    app.issues.data.insert(9, pr);
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.diff.expanded_files.insert((9, "a.rs".into()));
    app.set_tab(Tab::Visual);
    let _ = frame_text(&mut app);
    // Header, hunk, deletion, addition, then the thread header the reply lands in.
    for _ in 0..4 {
        handle_key(&mut app, key(KeyCode::Down));
    }
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert_eq!(
        app.draft().map(|c| c.target.clone()),
        Some(crate::app::ComposeTarget::Reply { thread_id: "t1".into() }),
    );
    let text = frame_text(&mut app);
    assert!(text.contains("Reply on #9"), "{text}");
}

#[test]
fn compose_modal_shows_reply_target_and_discards() {
    let mut app = loaded_app();
    app.open_draft(crate::app::ComposeState {
        number: 7,
        target: crate::app::ComposeTarget::Reply { thread_id: "t".into() },
        body: "reply".into(),
        cursor: 5,
        sending: false,
        kill: String::new(), view: Default::default(),
    });
    let text = frame_text(&mut app);
    assert!(text.contains("Reply"), "{text}");
    assert!(text.contains("reply"), "{text}");
    handle_key(&mut app, key(KeyCode::Esc));
    assert!(app.confirm_open(), "typed draft asks first");
    handle_key(&mut app, key(KeyCode::Char('d')));
    assert!(!app.compose_open());
}

#[test]
fn confirm_frame_shows_choices_and_click_picks() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('c')));
    handle_key(&mut app, key(KeyCode::Char('h')));
    handle_key(&mut app, key(KeyCode::Esc));
    let text = frame_text(&mut app);
    assert!(text.contains("Discard draft"), "{text}");
    assert!(text.contains("k keep editing"), "{text}");
    assert!(text.contains("d discard"), "{text}");
    assert_eq!(app.hits.confirm_choices.len(), 2);
    let rect = app.hits.confirm_choices[1].0;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x, rect.y));
    assert!(!app.confirm_open() && !app.compose_open(), "click on d discards");
}

#[test]
fn undo_toast_frame_and_click_restores() {
    let mut app = loaded_app();
    app.nav.pane = crate::core::Pane::Issues;
    app.nav.current = Some(7);
    handle_key(&mut app, key(KeyCode::Char('x')));
    let text = frame_text(&mut app);
    assert!(text.contains("removed #7 · u undo"), "{text}");
    let toast = app.hits.undo_toast;
    assert!(toast.width > 0, "toast rect recorded");
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), toast.x, toast.y));
    assert!(app.nav.visited.contains(&7), "click undoes");
    assert!(app.undo.is_none());
}

#[test]
fn compose_footer_names_the_editor_key() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('c')));
    let text = frame_text(&mut app);
    assert!(text.contains("ctrl-o"), "footer offers the external editor: {text}");
    assert!(text.contains("ctrl-s"), "footer keeps the send key: {text}");
}

#[test]
fn compose_modal_steals_mouse_focus() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&7).unwrap().body = "blocked by #9, see details".to_string();
    handle_key(&mut app, key(KeyCode::Char('c')));
    assert!(app.compose_open());
    let _ = frame_text(&mut app);
    let row = app.hits
        .body_rows
        .iter()
        .position(|l| l.contains("#9"))
        .expect("ref visible");
    let (x, y) = (app.hits.body_area.x + 2, app.hits.body_area.y + row as u16);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(7), "clicks never leak past the modal");
    assert!(app.compose_open(), "the draft survives the click");
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 60, 20));
    assert_eq!(app.nav.scroll, 0, "wheel never scrolls behind the modal");
}

#[test]
fn pending_nav_dialog_renders_all_three_actions() {
    let mut app = loaded_app();
    app.nav.current = Some(9);
    let d = app.issues.data.get_mut(&9).unwrap();
    d.is_pr = true;
    d.review_threads = vec![crate::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(1),
        comments: vec![crate::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app.open_pending_nav(crate::app::NavIntent::Issue(7));
    let text = frame_text(&mut app);
    assert!(text.contains("Unsubmitted comments"), "{text}");
    assert!(text.contains("Submit them") && text.contains("Discard") && text.contains("Cancel"), "{text}");
    assert!(text.contains("#9"), "{text}");
}

#[test]
fn filter_dialog_shows_nerd_icons() {
    let mut app = loaded_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    let text = frame_text(&mut app);
    // Font Awesome: fa-filter title, fa-clock-o window row. All four
    // toggles default on, so the checked box renders.
    assert!(text.contains('\u{f0b0}'), "title: {text}");
    assert!(text.contains("window"), "window: {text}");
    assert!(text.contains('\u{f14a}'), "checked: {text}");
    app.sidebar.show_closed_issues = false;
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f096}'), "unchecked: {text}");
}
