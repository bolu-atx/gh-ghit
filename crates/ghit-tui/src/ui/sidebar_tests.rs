//! Frame tests for the sidebar panes.

use crate::app::{App, poll_sections};
use crate::input::{handle_key, handle_mouse};
use ghit_core::model::{
    Label, PairedRef,
};
use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{frame_text_sized, sample, loaded_app, key, click, draw_buffer, cells, check_pr, check_app, frame_text, sidebar_text};
use ghit_core::model::ItemRef;

fn labels_app() -> App {
    let mut app = loaded_app();
    app.refs.numbers = vec![ItemRef::github(9)];
    app.sidebar.meta_loaded = true;
    app.sidebar.labels = vec!["bug".to_string(), "docs".to_string()];
    app.sidebar.milestones = vec![ghit_core::model::Milestone {
        title: "v1".to_string(),
        due_on: None, ..Default::default()
    }];
    app
}

/// Text of the PRs sidebar only, mirroring `sidebar_text`.
fn pr_sidebar_text(app: &mut App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let area = app.hits.side_pr_area;
    let buf = terminal.backend().buffer();
    let mut text = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            text.push_str(buf[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn history_window_and_click_follow_drawn_rows() {
    let mut app = loaded_app();
    for number in 10..=15 {
        app.issues
            .data
            .insert(ItemRef::github(number), sample(ItemRef::github(number)));
    }
    app.nav.history = vec![
        ItemRef::github(7),
        ItemRef::github(9),
        ItemRef::github(10),
        ItemRef::github(11),
        ItemRef::github(12),
        ItemRef::github(13),
    ];
    app.show_issue(ItemRef::github(15));
    app.refs.numbers = vec![ItemRef::github(9)];
    let text = frame_text_sized(&mut app, 160, 40);
    assert!(text.contains("History"));
    assert_eq!(app.hits.history_rows.len(), 5);
    assert_eq!(app.hits.history_rows[4].0.y, 37);
    assert_eq!(app.hits.refs_area.bottom(), app.hits.history_rows[0].0.y - 1);
    assert!(text.contains("▌ #15"));
    assert!(text.matches('█').count() <= 1, "History adds no list selection marker");
    let row = app.hits.history_rows[2].0;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), row.x, row.y));
    assert_eq!(app.nav.current, Some(ItemRef::github(12)));
    assert_eq!(
        app.nav.future,
        vec![ItemRef::github(15), ItemRef::github(13)]
    );
}

#[test]
fn labels_pane_lists_milestones_then_labels_inside_120x40() {
    let mut app = labels_app();
    let text = frame_text(&mut app);
    assert!(text.contains("Filters"), "{text}");
    let order: Vec<usize> = ["v1", "(no milestone)", "bug", "docs"]
        .iter().map(|name| text.find(name).expect(name)).collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "group order: {text}");
    assert!(!text.contains('\u{f14a}'), "nothing selected yet: {text}");
    assert!(text.contains(" d hide done "), "buttons replace the counts footer: {text}");
    assert_eq!(app.hits.labels_rows.len(), 4);
    let (refs, labels) = (app.hits.refs_area, app.hits.labels_area);
    assert_eq!(refs.bottom(), labels.y, "panes stack without a gap");
    assert!(labels.height >= 8, "no truncation at 120x40: {labels:?}");
}

fn milestone(title: &str, open: u32, closed: u32, state_closed: bool) -> ghit_core::model::Milestone {
    ghit_core::model::Milestone { title: title.to_string(), open_items: open, closed_items: closed,
        closed: state_closed, ..Default::default() }
}

#[test]
fn milestone_percent_and_done() {
    assert_eq!(milestone("a", 0, 0, false).percent(), None);
    assert_eq!(milestone("a", 1, 3, false).percent(), Some(75));
    assert!(!milestone("a", 1, 3, false).done());
    assert!(milestone("a", 0, 3, false).done(), "every item closed");
    assert!(milestone("a", 2, 0, true).done(), "closed on GitHub");
    assert!(!milestone("a", 0, 0, false).done(), "empty is not done");
}

#[test]
fn labels_pane_shows_milestone_progress() {
    let mut app = labels_app();
    app.sidebar.milestones = vec![milestone("v1", 1, 3, false), milestone("v2", 0, 0, false)];
    let text = frame_text(&mut app);
    assert!(text.contains("▰▰▰▰▱  75%"), "{text}");
    assert!(text.contains("v2"), "{text}");
    assert_eq!(text.matches('%').count(), 1, "no bar without items: {text}");
}

#[test]
fn d_hides_done_milestones_but_keeps_selected_ones() {
    use ghit_core::core::Pane;
    let mut app = labels_app();
    app.sidebar.milestones = vec![
        milestone("v1", 2, 1, false), milestone("shipped", 0, 4, false), milestone("retired", 1, 0, true),
    ];
    let text = frame_text(&mut app);
    assert!(text.contains("hide done") && text.contains("shipped") && text.contains("retired"), "{text}");
    app.nav.pane = Pane::Labels;
    handle_key(&mut app, key(KeyCode::Char('d')));
    assert!(app.sidebar.hide_done_milestones);
    let text = frame_text(&mut app);
    assert!(text.contains("show done") && text.contains("v1"), "{text}");
    assert!(!text.contains("shipped") && !text.contains("retired"), "{text}");
    assert!(text.contains("2 done milestones hidden"), "{text}");
    // A selected done milestone stays visible so it can be cleared.
    app.sidebar.milestone_filter.insert(Some("shipped".to_string()));
    assert!(frame_text(&mut app).contains("shipped"));
    assert!(frame_text(&mut app).contains("1 done milestone hidden"));
    handle_key(&mut app, key(KeyCode::Char('d')));
    let text = frame_text(&mut app);
    assert!(text.contains("retired") && !text.contains("hidden"), "{text}");
}

#[test]
fn o_opens_the_highlighted_filter_not_the_current_item() {
    use ghit_core::core::Pane;
    let mut app = labels_app();
    app.repo_slug = Some("o/r".to_string());
    frame_text(&mut app);
    app.nav.pane = Pane::Labels;
    handle_key(&mut app, key(KeyCode::Char('o')));
    assert_eq!(app.message, "opened milestone:\"v1\" in browser");
    // Rows: v1, (no milestone), bug, docs.
    handle_key(&mut app, key(KeyCode::Char('j')));
    handle_key(&mut app, key(KeyCode::Char('o')));
    assert_eq!(app.message, "opened no:milestone in browser");
    handle_key(&mut app, key(KeyCode::Char('j')));
    handle_key(&mut app, key(KeyCode::Char('o')));
    assert_eq!(app.message, "opened label:\"bug\" in browser");
    assert_eq!(crate::app::encode_query("label:\"a b\""), "label%3A%22a%20b%22");
}

#[test]
fn labels_pane_keys_move_toggle_and_clear() {
    use ghit_core::core::Pane;
    let mut app = labels_app();
    frame_text(&mut app);
    app.nav.pane = Pane::Labels;
    // Rows: v1, (no milestone), bug, docs.
    handle_key(&mut app, key(KeyCode::Char('j')));
    handle_key(&mut app, key(KeyCode::Char('j')));
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.sidebar.label_filter.contains("bug"));
    let text = frame_text(&mut app);
    assert!(text.contains("\u{f14a} bug") && text.contains("█ "), "{text}");
    assert!(text.contains("1 on"), "{text}");
    handle_key(&mut app, key(KeyCode::Char('k')));
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(app.sidebar.milestone_filter.contains(&None));
    handle_key(&mut app, key(KeyCode::Char('X')));
    assert!(app.sidebar.label_filter.is_empty() && app.sidebar.milestone_filter.is_empty());
    // The Format dialog flips the same set the pane reads.
    app.sidebar.label_filter.insert("docs".to_string());
    assert!(frame_text(&mut app).contains("\u{f14a} docs"));
}

#[test]
fn labels_pane_click_toggles_and_wheel_moves() {
    use ghit_core::core::Pane;
    let mut app = labels_app();
    frame_text(&mut app);
    let (rect, index) = app.hits.labels_rows[3];
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), rect.x + 3, rect.y));
    assert_eq!(app.nav.pane, Pane::Labels);
    assert_eq!(index, 3);
    assert!(app.sidebar.label_filter.contains("docs"));
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, rect.x + 3, rect.y));
    assert_eq!(app.labels.cursor, 2);
    assert_eq!(app.nav.pane, Pane::Labels);
}

#[test]
fn labels_pane_shows_loading_until_meta_lands() {
    let mut app = loaded_app();
    app.refs.numbers = vec![ItemRef::github(9)];
    let (_tx, rx) = std::sync::mpsc::channel();
    app.sidebar.meta_rx = Some(rx);
    assert!(frame_text(&mut app).contains("loading …"));
    assert!(app.hits.labels_rows.is_empty());
}

#[test]
fn sidebar_shows_spinner_and_loading_body_while_sections_load() {
    let mut app = App::new(Some("o/r".to_string()));
    app.tick = 0;
    let (_tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    let text = sidebar_text(&mut app);
    assert!(text.contains("Issues") && text.contains("⠋"), "title spinner: {text}");
    assert!(text.contains("Loading open + last 24h closed issues…"), "loading body: {text}");
    app.sections.rx = None;
    let text = sidebar_text(&mut app);
    assert!(!text.contains("⠋") && !text.contains("Loading"), "idle sidebar: {text}");
}

#[test]
fn sidebar_loading_goes_quiet_while_the_current_issue_loads() {
    let mut app = App::new(Some("o/r".to_string()));
    app.tick = 0;
    let (_tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    app.nav.requested = Some(ItemRef::github(7));
    app.issues.pending.insert(ItemRef::github(7));
    let text = sidebar_text(&mut app);
    assert!(text.contains("⠋"), "title spinner stays: {text}");
    assert!(!text.contains("Loading open + last 24h closed issues…"), "no sidebar loading body under the main loading body: {text}");
    assert!(!text.contains("No issues"), "no empty hint while loading: {text}");
    app.issues.pending.clear();
    assert!(sidebar_text(&mut app).contains("Loading open + last 24h closed issues…"), "loading body returns once the main pane is done");
}

#[test]
fn refs_title_spins_while_mentions_fetch() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "see #9".to_string();
    app.show_issue(ItemRef::github(7));
    app.tick = 0;
    app.issues.pending.insert(ItemRef::github(9));
    let text = frame_text(&mut app);
    assert!(text.contains("Refs") && text.contains("mentioned ⠋"), "refs spinner: {text}");
}

#[test]
fn cached_click_still_joins_its_section() {
    // Data can arrive before the sidebar knows the number (warm cache,
    // board prefetch): an explicit open files it under requested.
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let mut pr = sample(ItemRef::github(9));
    pr.is_pr = true;
    pr.url = "https://github.com/o/r/pull/9".to_string();
    app.issues.data.insert(ItemRef::github(9), pr);
    assert!(app.nav.visited.is_empty());
    app.ensure_visited(ItemRef::github(9));
    app.load_issue(ItemRef::github(9), false);
    assert_eq!(app.nav.visited, vec![ItemRef::github(9)]);
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    // A second click files nothing twice.
    app.ensure_visited(ItemRef::github(9));
    app.load_issue(ItemRef::github(9), false);
    assert_eq!(app.nav.visited, vec![ItemRef::github(9)]);
}

#[test]
fn viewing_a_recent_row_does_not_relocate_it() {
    // Regression: cursoring onto a recent row completes its fetch;
    // the row must stay under recent, not move to requested.
    use crate::app::poll_issues;
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let mut d = sample(ItemRef::github(42));
    d.is_pr = true;
    app.issues.data.insert(ItemRef::github(42), d.clone());
    app.issues.statuses.insert(ItemRef::github(42), vec![]);
    app.nav.recent.push(ItemRef::github(42));
    app.show_issue(ItemRef::github(42));
    assert_eq!(app.nav.visited, Vec::<ItemRef>::new());
    // Background fetch lands.
    app.issues
        .tx
        .send((
            ItemRef::github(42),
            ghit_core::github::IssueEvent::Done(Ok(Box::new((d, vec![])))),
        ))
        .unwrap();
    poll_issues(&mut app);
    assert_eq!(app.nav.visited, Vec::<ItemRef>::new());
    let (_, _, recent) = app.pane_groups(true, false);
    assert_eq!(recent, vec![ItemRef::github(42)]);
    assert_eq!(app.nav.current, Some(ItemRef::github(42)));
}

#[test]
fn merged_window_label_reads_plain() {
    let mut app = App::new(Some("o/r".to_string()));
    assert_eq!(app.window_label(), "open + merged ≤24h");
    app.sections.merged_window_hours = 1;
    assert_eq!(app.window_label(), "open + merged ≤1h");
    app.sections.merged_window_hours = 168;
    assert_eq!(app.window_label(), "open + merged ≤7d");
}

#[test]
fn tab_cycles_issues_prs_refs() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    assert_eq!(app.nav.pane, Pane::Issues);
    // No refs yet: Tab walks Issues, PRs, Main.
    for want in [Pane::PRs, Pane::Main, Pane::Issues] {
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.nav.pane, want);
    }
    // With mentions, Refs joins the cycle after Main.
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "blocked by #9".to_string();
    app.show_issue(ItemRef::github(7));
    for want in [Pane::PRs, Pane::Main, Pane::Refs, Pane::Issues] {
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.nav.pane, want);
    }
    // Shift-Tab walks the same order backwards.
    for want in [Pane::Refs, Pane::Main, Pane::PRs, Pane::Issues] {
        handle_key(&mut app, key(KeyCode::BackTab));
        assert_eq!(app.nav.pane, want);
    }
}

#[test]
fn active_row_is_amber_shaded_when_focused_and_muted_bare_when_not() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    app.nav.pane = Pane::Issues;
    let buf = draw_buffer(&mut app);
    let marker = cells(&buf, "█")[0];
    assert_eq!((marker.fg, marker.bg), (crate::theme::amber(), crate::theme::code_bg()));
    app.nav.pane = Pane::PRs;
    let buf = draw_buffer(&mut app);
    let marker = cells(&buf, "█")[0];
    assert_eq!(marker.fg, crate::theme::muted(), "unfocused marker is quiet");
    assert_eq!(marker.bg, crate::theme::ink(), "unfocused row has no shade");
}

#[test]
fn hover_row_never_looks_like_the_selection() {
    let mut app = loaded_app();
    app.nav.hover = Some(ItemRef::github(9));
    let buf = draw_buffer(&mut app);
    assert_eq!(cells(&buf, "█").len(), 1, "one selection marker");
    let hover = cells(&buf, "\u{203a}");
    assert_eq!(hover.len(), 1, "hover has its own marker");
    assert_eq!(hover[0].bg, crate::theme::ink(), "hover has no selection shade");
    assert_ne!(hover[0].fg, crate::theme::amber());
}

#[test]
fn jk_in_refs_moves_highlight_instead_of_opening() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "see #9 then #12".to_string();
    app.show_issue(ItemRef::github(7));
    app.nav.pane = Pane::Refs;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.refs.highlight, Some(ItemRef::github(9)));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.refs.highlight, Some(ItemRef::github(12)));
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.refs.highlight, Some(ItemRef::github(9)));
}

#[test]
fn panes_group_pins_requested_and_recent_with_dividers() {
    let mut app = loaded_app();
    app.nav.pin_order = vec![ItemRef::github(9)];
    app.nav.pinned = [9].map(ItemRef::github).into_iter().collect();
    app.nav.recent = vec![ItemRef::github(9), ItemRef::github(7), ItemRef::github(42)];
    let (ip, iv, ir) = app.pane_groups(false, false);
    assert_eq!(
        (ip, iv, ir),
        (vec![], vec![ItemRef::github(7)], vec![ItemRef::github(42)])
    );
    let (pp, pv, pr) = app.pane_groups(true, false);
    assert_eq!((pp, pv, pr), (vec![ItemRef::github(9)], vec![], vec![]));
    use ghit_core::model::SideRow::{Divider, Item};
    assert_eq!(
        app.issue_rows(),
        vec![
            Item(ItemRef::github(7)),
            Divider("recent".to_string()),
            Item(ItemRef::github(42))
        ]
    );
    assert_eq!(app.pr_rows(), vec![Item(ItemRef::github(9))]);
    let text = frame_text(&mut app);
    assert!(text.contains("Issues"));
    assert!(text.contains("Pull requests"));
}

#[test]
fn pane_titles_carry_count_and_sort_with_group_footers() {
    let mut app = loaded_app();
    // loaded_app files one issue and one PR under requested.
    assert_eq!(app.side_subtitle(false), "1 · recent · default");
    assert_eq!(app.side_subtitle(true), "1 · recent · default");
    assert_eq!(app.side_footer(false), "pinned (0) · requested (1) · recent (0)");
    assert_eq!(app.side_footer(true), "pinned (0) · requested (1) · recent (0)");
    let text = frame_text(&mut app);
    assert!(text.contains("1 · recent · default"), "count, context, sort in subtitle: {text}");
    assert!(
        text.contains("pinned (0) · requested (1) · recent (0)"),
        "group counts in footer: {text}"
    );
    // Sort state rides the subtitle; reverse flips the arrow.
    app.sidebar.sort = ghit_core::core::SideSort::Updated;
    app.sidebar.sort_desc = true;
    let text = frame_text(&mut app);
    assert!(text.contains("1 · recent · updated ↓"), "sort indicator: {text}");
    app.toggle_sort_dir();
    let text = frame_text(&mut app);
    assert!(text.contains("1 · recent · updated ↑"), "reverse indicator: {text}");
    // An active query names itself and any filter marks the pane.
    app.sidebar.query_active = "is:open".to_string();
    assert_eq!(app.query_tag(), Some("query"));
    assert!(!app.side_subtitle(true).contains("filtered"), "no marker without filters");
    app.sidebar.show_open_issues = false;
    assert!(app.side_subtitle(true).contains("filtered"), "filter marker in title");
    assert!(app.side_footer(true).contains("is:open"), "query in footer");
}

#[test]
fn refs_pane_shares_title_and_footer_shape() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "see #9".to_string();
    app.show_issue(ItemRef::github(7));
    assert!(app.refs_visible());
    assert_eq!(app.refs_subtitle(), "1 mentioned");
    assert_eq!(app.refs_footer(), "mentioned (1) · paired (0)");
    let text = frame_text(&mut app);
    assert!(text.contains("1 mentioned"), "refs subtitle: {text}");
    assert!(text.contains("mentioned (1) · paired (0)"), "refs footer: {text}");
}

#[test]
fn refs_pane_lists_paired_rows_and_enter_opens() {
    use ghit_core::core::Pane;
    use ghit_core::model::SideRow::{Divider, Item};
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.body = String::new();
    d.paired = vec![PairedRef {
        number: ItemRef::github(9),
        state: "MERGED".to_string(),
    }];
    app.show_issue(ItemRef::github(7));
    assert!(app.refs_visible());
    assert_eq!(
        app.refs_rows(),
        vec![Divider("Paired".to_string()), Item(ItemRef::github(9))]
    );
    let text = frame_text(&mut app);
    assert!(text.contains("Paired"), "{text}");
    app.nav.pane = Pane::Refs;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.refs.highlight, Some(ItemRef::github(9)));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "j only moves the highlight"
    );
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(9)),
        "Enter opens the paired counterpart"
    );
}

#[test]
fn paired_refs_row_click_opens_instead_of_highlighting() {
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.body = String::new();
    d.paired = vec![PairedRef {
        number: ItemRef::github(9),
        state: "MERGED".to_string(),
    }];
    app.show_issue(ItemRef::github(7));
    let _ = frame_text(&mut app); // records hit areas
    // Divider sits on the first list row; the paired item below it.
    let (x, y) = (app.hits.refs_area.x + 2, app.hits.refs_area.y + 3);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    assert_eq!(app.refs.highlight, None);
}

#[test]
fn refs_pane_lists_mentioned_rows_and_toggles() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body =
        "blocked by #9, see details".to_string();
    app.show_issue(ItemRef::github(7));
    assert_eq!(app.refs.numbers, vec![ItemRef::github(9)]);
    assert!(app.refs_visible());
    let text = frame_text(&mut app);
    assert!(text.contains("Refs"), "{text}");
    handle_key(&mut app, key(KeyCode::Char('e')));
    assert!(!app.refs.show);
    assert!(!app.refs_visible());
    let hidden = frame_text(&mut app);
    assert!(!hidden.contains("Refs"), "{hidden}");
    // Recompute on arrival: a fresh body fetch repopulates the pane.
    handle_key(&mut app, key(KeyCode::Char('e')));
    let mut d = sample(ItemRef::github(7));
    d.body = "see #9".to_string();
    app.issues
        .tx
        .send((
            ItemRef::github(7),
            ghit_core::github::IssueEvent::Done(Ok(Box::new((d, vec![])))),
        ))
        .unwrap();
    crate::app::poll_issues(&mut app);
    assert_eq!(app.refs.numbers, vec![ItemRef::github(9)]);
}

#[test]
fn overflowing_pane_keeps_selected_row_visible() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for n in 1..=30 {
        let d = sample(ItemRef::github(n));
        app.issues.data.insert(ItemRef::github(n), d);
        app.issues.statuses.insert(ItemRef::github(n), vec![]);
        app.nav.visited.push(ItemRef::github(n));
    }
    app.show_issue(ItemRef::github(30));
    let _ = frame_text(&mut app);
    assert!(app.sidebar.issue_state.offset() > 0, "pane scrolled");
}

#[test]
fn wheel_over_sidebar_hovers_without_visiting() {
    let mut app = loaded_app();
    // A second issue row so the wheel has somewhere to go.
    let d = sample(ItemRef::github(8));
    app.issues.data.insert(ItemRef::github(8), d);
    app.issues.statuses.insert(ItemRef::github(8), vec![]);
    app.nav.visited.push(ItemRef::github(8));
    let _ = frame_text(&mut app); // records hit areas
    let (x, y) = (app.hits.side_issue_area.x + 2, app.hits.side_issue_area.y + 2);
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, x, y));
    assert_eq!(app.nav.hover, Some(ItemRef::github(8)));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "wheel never visits"
    );
    let _ = frame_text(&mut app); // selection stays on the active issue
    assert_eq!(app.sidebar.issue_state.selected(), Some(1), "hover never outranks current");
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert_eq!(app.nav.scroll, 0, "body scroll untouched");
}

#[test]
fn slash_search_filters_and_enters_first_hit() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(9)).unwrap().title = "Zebra migration".to_string();
    handle_key(&mut app, key(KeyCode::Char('/')));
    assert!(app.sidebar.searching);
    for c in "zebra".chars() {
        handle_key(&mut app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
    }
    assert!(app.issue_rows().is_empty());
    assert_eq!(
        app.pr_rows(),
        vec![ghit_core::model::SideRow::Item(ItemRef::github(9))]
    );
    let text = frame_text(&mut app);
    assert!(text.contains("zebra"));
    handle_key(&mut app, key(KeyCode::Enter));
    assert!(!app.sidebar.searching);
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    // Highlight persists after commit.
    assert_eq!(app.sidebar.search, "zebra");
}

#[test]
fn search_n_jumps_between_hits() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().title = "Fix login".to_string();
    app.issues.data.get_mut(&ItemRef::github(9)).unwrap().title = "Fix logout".to_string();
    app.sidebar.search = "fix".to_string();
    assert_eq!(
        app.search_hits(),
        vec![ItemRef::github(7), ItemRef::github(9)]
    );
    handle_key(&mut app, key(KeyCode::Char('n')));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    handle_key(&mut app, key(KeyCode::Char('N')));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn sub_issue_tree_renders_folds_and_promotes_filtered_children() {
    let mut app = App::new(Some("o/r".to_string()));
    let mut parent = sample(ItemRef::github(1));
    parent.milestone = Some("parent".into());
    parent.sub_summary = Some(ghit_core::model::SubSummary { total: 2, completed: 1 });
    let mut child = sample(ItemRef::github(2));
    child.parent = Some(ItemRef::github(1));
    child.milestone = Some("child".into());
    app.issues.data.insert(ItemRef::github(1), parent);
    app.issues.data.insert(ItemRef::github(2), child);
    app.nav.recent = vec![ItemRef::github(1), ItemRef::github(2)];
    let off = sidebar_text(&mut app);
    assert!(!off.contains("▾") && !off.contains("└─"));
    // A leaf's `{0, 0}` summary (GraphQL default) never badges.
    app.issues.data.get_mut(&ItemRef::github(2)).unwrap().sub_summary =
        Some(ghit_core::model::SubSummary { total: 0, completed: 0 });
    app.sidebar.sub_issue_tree = true;
    app.sidebar.group_by = ghit_core::model::GroupBy::Milestone;
    let on = sidebar_text(&mut app);
    assert!(on.contains("▾ #1") && on.contains("└─  #2") && on.contains("1/2"), "{on}");
    assert!(!on.contains("0/0"), "leaf rows carry no badge: {on}");
    assert_eq!(app.issue_rows().iter().filter(|r| matches!(r,
        ghit_core::model::SideRow::Divider(_))).count(), 1);
    app.sidebar.folded_issues.insert(ItemRef::github(1));
    let folded = sidebar_text(&mut app);
    assert!(folded.contains("▸ #1") && !folded.contains("#2"), "{folded}");
    app.sidebar.milestone_filter.insert(Some("child".into()));
    let filtered = sidebar_text(&mut app);
    assert!(filtered.contains("#2") && !filtered.contains("└─"), "{filtered}");
}

#[test]
fn lazy_sub_issue_rows_render_dim_with_outside_marker() {
    let mut app = App::new(Some("o/r".into()));
    let mut parent = sample(ItemRef::github(1));
    parent.title = "parent title".into();
    parent.sub_summary = Some(ghit_core::model::SubSummary { total: 2, completed: 0 });
    app.issues.data.insert(ItemRef::github(1), parent);
    let mut fetched = sample(ItemRef::github(2));
    fetched.title = "fetched child".into();
    app.issues.data.insert(ItemRef::github(2), fetched);
    let mut outside = sample(ItemRef::github(3));
    outside.title = "outside child".into();
    app.issues.data.insert(ItemRef::github(3), outside);
    app.issues.partial.insert(ItemRef::github(3));
    app.nav.recent = vec![ItemRef::github(1), ItemRef::github(2)];
    app.sidebar.sub_issue_tree = true;
    app.subs
        .state
        .insert(ItemRef::github(1), ghit_core::model::LazyChildren::Loading);
    let loading = sidebar_text(&mut app);
    assert!(
        loading.contains("▾ #1") && loading.contains("loading …"),
        "{loading}"
    );
    app.subs.state.insert(
        ItemRef::github(1),
        ghit_core::model::LazyChildren::Loaded(vec![ItemRef::github(2), ItemRef::github(3)]),
    );
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let area = app.hits.side_issue_area;
    let buf = terminal.backend().buffer();
    let cell = |number: char| -> (u16, u16) {
        (area.y..area.y + area.height)
            .find_map(|y| (area.x..area.x + area.width.saturating_sub(1)).find_map(|x| {
                (buf[(x, y)].symbol() == "#"
                    && buf[(x + 1, y)].symbol().starts_with(number))
                    .then_some((x, y))
            }))
            .expect("child row renders")
    };
    let text: String = (area.y..area.y + area.height).map(|y| {
        (area.x..area.x + area.width).map(|x| buf[(x, y)].symbol()).collect::<String>()
    }).collect::<Vec<_>>().join("\n");
    assert!(text.contains("outside query"), "{text}");
    assert_eq!(buf[cell('3')].style().fg, Some(crate::theme::muted()), "lazy child dim");
    assert_ne!(buf[cell('2')].style().fg, Some(crate::theme::muted()), "query child normal");
    assert_eq!(app.issue_rows().iter().filter_map(|r| r.number()).collect::<Vec<_>>(),
        vec![ItemRef::github(1), ItemRef::github(2), ItemRef::github(3)]);
}

#[test]
fn sub_issue_tree_keys_and_glyph_click_toggle_folds() {
    let mut app = App::new(Some("o/r".into()));
    let parent = sample(ItemRef::github(1));
    let mut child = sample(ItemRef::github(2));
    child.parent = Some(ItemRef::github(1));
    app.issues.data.insert(ItemRef::github(1), parent);
    app.issues.data.insert(ItemRef::github(2), child);
    app.nav.recent = vec![ItemRef::github(1), ItemRef::github(2)];
    app.sidebar.sub_issue_tree = true;
    app.nav.pane = ghit_core::core::Pane::Issues;
    app.show_issue(ItemRef::github(1));
    handle_key(&mut app, key(KeyCode::Left));
    assert!(app.sidebar.folded_issues.contains(&ItemRef::github(1)));
    handle_key(&mut app, key(KeyCode::Right));
    assert!(!app.sidebar.folded_issues.contains(&ItemRef::github(1)));
    handle_key(&mut app, key(KeyCode::Char('z')));
    assert!(app.sidebar.folded_issues.contains(&ItemRef::github(1)));
    handle_key(&mut app, key(KeyCode::Char('Z')));
    assert!(app.sidebar.folded_issues.is_empty());
    frame_text(&mut app);
    let area = app.hits.side_issue_area;
    handle_mouse(&mut app, MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 3,
        row: area.y + 2, modifiers: KeyModifiers::empty(),
    });
    assert!(app.sidebar.folded_issues.contains(&ItemRef::github(1)));
}

#[test]
fn milestone_groups_render_in_order_and_clicks_match_rows() {
    use ghit_core::model::{GroupBy, SideRow::{Divider, Item}};
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for (n, milestone) in [(7, Some("v1")), (9, Some("v0.6")), (11, None)] {
        let mut d = sample(ItemRef::github(n));
        d.milestone = milestone.map(str::to_string);
        app.issues.data.insert(ItemRef::github(n), d);
        app.issues.statuses.insert(ItemRef::github(n), vec![]);
        app.nav.recent.push(ItemRef::github(n));
    }
    app.sidebar.group_by = GroupBy::Milestone;
    app.sidebar.meta_loaded = true;
    app.sidebar.milestones = vec![
        ghit_core::model::Milestone {
            title: "v1".to_string(),
            due_on: Some("2026-03-01T00:00:00Z".to_string()), ..Default::default()
        },
        ghit_core::model::Milestone {
            title: "v0.6".to_string(),
            due_on: Some("2026-02-01T00:00:00Z".to_string()), ..Default::default()
        },
    ];
    assert_eq!(
        app.issue_rows(),
        vec![
            Divider("v0.6 (1)".to_string()),
            Item(ItemRef::github(9)),
            Divider("v1 (1)".to_string()),
            Item(ItemRef::github(7)),
            Divider("(no milestone) (1)".to_string()),
            Item(ItemRef::github(11)),
        ],
        "due date, then title, then no milestone"
    );
    let text = sidebar_text(&mut app);
    let order = ["v0.6 (1)", "v1 (1)", "(no milestone) (1)"]
        .map(|label| text.find(label).unwrap_or(usize::MAX));
    assert!(order.windows(2).all(|w| w[0] < w[1]), "dividers render in order: {text}");
    // A click on #7 must land on #7: walk the same row heights the
    // hit test uses (3 lines per item, 1 per divider, plus border
    // and spacer).
    let area = app.hits.side_issue_area;
    let mut rel = 2usize;
    for row in app.issue_rows() {
        if row.number() == Some(ItemRef::github(7)) {
            break;
        }
        rel += if row.number().is_some() { 3 } else { 1 };
    }
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), area.x + 2, area.y + rel as u16 + 1));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)), "click lands on the grouped row");
}

#[test]
fn arrows_move_like_j_and_k() {
    use ghit_core::core::Pane;
    let mut app = loaded_app();
    app.nav.pane = Pane::Main;
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.nav.scroll, 3);
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!(app.nav.scroll, 0);
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.nav.scroll, 0, "in a list the arrow selects");
}

#[test]
fn mouse_click_selects_sidebar_row() {
    let mut app = loaded_app();
    app.show_issue(ItemRef::github(9));
    let _ = frame_text(&mut app); // records hit areas
    // +2 skips the border and the breathing-room spacer row.
    let (x, y) = (app.hits.side_issue_area.x + 2, app.hits.side_issue_area.y + 2);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn space_expands_sidebar_row_detail_and_collapse_restores_it() {
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.title = "Fix the thing that keeps breaking tailmarker".to_string();
    d.labels = ["bug", "ui", "docs", "chore", "ci-pipeline"]
        .iter()
        .map(|name| Label { name: name.to_string() })
        .collect();
    app.show_issue(ItemRef::github(7));
    let collapsed = sidebar_text(&mut app);
    assert!(collapsed.contains("#7 Fix the thing"), "{collapsed}");
    assert!(!collapsed.contains("tailmarker"), "collapsed title is a slug");
    assert!(!collapsed.contains("ci-pipeline"), "summary label chips overflow");

    handle_key(&mut app, key(KeyCode::Char(' ')));
    let expanded = sidebar_text(&mut app);
    assert!(expanded.contains("tailmarker"), "full title wraps in: {expanded}");
    assert!(expanded.contains("● bug"), "every label: {expanded}");
    assert!(expanded.contains("ci-pipeline"), "every label: {expanded}");
    assert!(expanded.contains("by bo · OPEN"), "author and state: {expanded}");
    assert!(expanded.contains("created 2026-01-02"), "created date: {expanded}");
    assert!(expanded.contains("updated 2026-01-05"), "updated date: {expanded}");
    // Only the selected row's number expands.
    assert_eq!(expanded.matches("created 2026-01-02").count(), 1);

    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert_eq!(sidebar_text(&mut app), collapsed, "collapse restores the row");
}

#[test]
fn sidebar_rows_show_labels_updated_and_status_and_sort_hint() {
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.labels = vec![Label { name: "bug".to_string() }];
    d.updated_at = "2020-01-05T00:00:00Z".to_string();
    app.issues.statuses.insert(
        ItemRef::github(7),
        vec![("Roadmap".to_string(), Some("Todo".to_string()))],
    );
    let mut newer = sample(ItemRef::github(5));
    newer.updated_at = "2020-02-05T00:00:00Z".to_string();
    app.issues.data.insert(ItemRef::github(5), newer);
    app.issues.statuses.insert(ItemRef::github(5), vec![]);
    app.nav.visited.push(ItemRef::github(5));
    app.show_issue(ItemRef::github(7));

    let text = sidebar_text(&mut app);
    assert!(text.contains("● bug · 01/05/20"), "labels and date: {text}");
    assert!(text.contains("01/05/20 · \u{f10c} Todo"), "date and status: {text}");

    // One key cycles the sort; the `/` hint names key and direction.
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert_eq!(app.sidebar.sort, ghit_core::core::SideSort::Updated);
    let sorted = sidebar_text(&mut app);
    assert!(
        sorted.find("#5").unwrap_or(usize::MAX) < sorted.find("#7").unwrap_or(usize::MAX),
        "newest row first: {sorted}"
    );
    handle_key(&mut app, key(KeyCode::Char('/')));
    let text = frame_text(&mut app);
    assert!(text.contains("sort updated ↓"), "sort hint: {text}");
}

#[test]
fn sidebar_row_hides_labels_when_off_but_keeps_date_and_status() {
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.labels = vec![Label { name: "bug".to_string() }];
    d.updated_at = "2020-01-05T00:00:00Z".to_string();
    app.issues.statuses.insert(
        ItemRef::github(7),
        vec![("Roadmap".to_string(), Some("Todo".to_string()))],
    );
    app.show_issue(ItemRef::github(7));
    let text = sidebar_text(&mut app);
    assert!(text.contains("● bug · 01/05/20"), "labels and date: {text}");
    app.sidebar.row_issue.labels = false;
    let text = sidebar_text(&mut app);
    assert!(!text.contains("● bug"), "labels hide: {text}");
    assert!(text.contains("01/05/20"), "date stays: {text}");
    assert!(text.contains("Todo"), "status stays: {text}");
}

#[test]
fn pr_rows_show_check_rollup_glyph_per_state() {
    use ghit_core::model::CheckState;
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for (n, checks) in [
        (21, Some(CheckState::Pass)),
        (22, Some(CheckState::Fail)),
        (23, Some(CheckState::Pending)),
    ] {
        app.issues
            .data
            .insert(ItemRef::github(n), check_pr(ItemRef::github(n), checks));
        app.issues.statuses.insert(ItemRef::github(n), vec![]);
        app.nav.visited.push(ItemRef::github(n));
    }
    app.show_issue(ItemRef::github(21));
    let text = pr_sidebar_text(&mut app);
    assert!(text.contains("\u{f00c}"), "pass glyph: {text}");
    assert!(text.contains("\u{f00d}"), "fail glyph: {text}");
    assert!(text.contains("\u{f017}"), "pending glyph, never pass: {text}");
}

#[test]
fn pr_row_without_checks_shows_no_glyph() {
    use ghit_core::model::CheckState;
    // No rollup, or an unknown future state, renders nothing.
    for checks in [None, Some(CheckState::Unknown)] {
        let mut app = check_app(ItemRef::github(21), checks);
        let text = pr_sidebar_text(&mut app);
        assert!(
            !text.contains("\u{f00c}") && !text.contains("\u{f00d}") && !text.contains("\u{f017}"),
            "{checks:?} renders no glyph: {text}"
        );
    }
}

#[test]
fn background_section_rows_render_without_changing_selection() {
    let mut app = loaded_app();
    let (tx, rx) = std::sync::mpsc::channel();
    app.sections.rx = Some(rx);
    let mut issue = sample(ItemRef::github(42));
    issue.is_pr = true;
    issue.title = "background pull request".to_string();
    tx.send(ghit_core::github::SectionEvent::Done(Ok(vec![ghit_core::github::Row { issue, statuses: vec![], detail: ghit_core::model::BoardDetail::default(), complete: false, window_at: None }]))).unwrap();
    poll_sections(&mut app);
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.issues.partial.contains(&ItemRef::github(42)));
    assert!(frame_text(&mut app).contains("background pull request"));
}

#[test]
fn section_headers_show_nerd_icons() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "blocked by #9".to_string();
    app.show_issue(ItemRef::github(7));
    let text = frame_text(&mut app);
    // Font Awesome: fa-exclamation-circle, fa-code-fork, fa-link.
    assert!(text.contains('\u{f06a}'), "issues: {text}");
    assert!(text.contains('\u{f126}'), "pull requests: {text}");
    assert!(text.contains('\u{f0c1}'), "refs: {text}");
    assert!(text.contains("Issues"));
    assert!(text.contains("Pull requests"));
    assert!(text.contains("Refs"));
}

#[test]
fn pinned_row_uses_tack_icon() {
    let mut app = loaded_app();
    app.nav.pin_order = vec![ItemRef::github(7)];
    app.nav.pinned = [7].map(ItemRef::github).into_iter().collect();
    app.nav.recent = vec![ItemRef::github(7), ItemRef::github(9)];
    assert!(app
        .issue_rows()
        .iter()
        .any(|r| r.number() == Some(ItemRef::github(7))));
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f08d}'), "{text}");
}
