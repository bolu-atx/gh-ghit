//! Tests for key handling.

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::*;
use crate::app::{Focus, board_open};
use ghit_core::core::Pane;
use super::test_support::sample;
use ghit_core::model::ItemRef;

fn ctrl_key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn alt_key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, KeyModifiers::ALT)
}

#[test]
fn ctrl_o_and_ctrl_i_walk_history() {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    for n in [7, 9] {
        app.session.issues
            .data
            .insert(ItemRef::github(n), sample(ItemRef::github(n)));
        app.session.issues.statuses.insert(ItemRef::github(n), vec![]);
    }
    app.ensure_visited(ItemRef::github(7));
    app.load_issue(ItemRef::github(7), false);
    app.ensure_visited(ItemRef::github(9));
    app.load_issue(ItemRef::github(9), false);
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));

    handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));

    // Ctrl+I arrives as Tab on terminals without the kitty protocol.
    handle_key(&mut app, ctrl_key(KeyCode::Tab));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));

    handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
    handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));

    // Plain Tab (no Ctrl) still cycles panes instead of redoing.
    app.nav.pane = ghit_core::core::Pane::Issues;
    handle_key(
        &mut app,
        crossterm::event::KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()),
    );
    assert_eq!(app.nav.pane, ghit_core::core::Pane::PRs);
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
}

fn plain_key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
}

fn compose_app() -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    app.session.issues
        .data
        .insert(ItemRef::github(7), sample(ItemRef::github(7)));
    app.session.issues.statuses.insert(ItemRef::github(7), vec![]);
    app.nav.visited.push(ItemRef::github(7));
    app.show_issue(ItemRef::github(7));
    app
}

#[test]
fn shift_r_reopens_reauth_only_while_degraded() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('R')));
    assert!(!app.confirm_open(), "healthy key is a no-op");
    app.session.auth.dismissed = true;
    app.session.auth.degraded = true;
    handle_key(&mut app, plain_key(KeyCode::Char('R')));
    assert!(
        matches!(app.confirm().map(|d| &d.kind), Some(crate::app::ConfirmKind::Reauth)),
        "degraded key reopens the dialog",
    );
}

#[test]
fn c_opens_compose_and_esc_discards() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    assert!(app.compose_open());
    handle_key(&mut app, plain_key(KeyCode::Char('x')));
    assert_eq!(app.draft().unwrap().body, "x");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(app.confirm_open(), "typed draft asks before discarding");
    assert!(app.compose_open());
    handle_key(&mut app, plain_key(KeyCode::Char('k')));
    assert!(app.compose_open() && !app.confirm_open(), "k keeps editing");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    handle_key(&mut app, plain_key(KeyCode::Char('d')));
    assert!(!app.compose_open(), "d discards");
}

#[test]
fn esc_on_empty_draft_closes_silently() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.compose_open() && !app.confirm_open());
}

#[test]
fn compose_ctrl_s_with_empty_draft_stays_open() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    handle_key(&mut app, ctrl_key(KeyCode::Char('s')));
    assert!(app.compose_open(), "empty draft never sends");
    assert!(!app.draft().unwrap().sending);
    assert!(app.message.contains("empty"));
}

#[test]
fn compose_ctrl_c_never_quits() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    handle_key(&mut app, ctrl_key(KeyCode::Char('c')));
    assert!(!app.quit);
    assert!(app.compose_open(), "draft survives");
}

fn type_text(app: &mut App, text: &str) {
    for ch in text.chars() {
        handle_key(app, plain_key(KeyCode::Char(ch)));
    }
}

fn compose_cursor(app: &App) -> usize {
    app.draft().expect("compose").cursor
}

#[test]
fn compose_emacs_home_end_and_char_steps() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "hello");
    handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
    assert_eq!(compose_cursor(&app), 0, "ctrl-a jumps to line start");
    handle_key(&mut app, ctrl_key(KeyCode::Char('f')));
    handle_key(&mut app, ctrl_key(KeyCode::Char('f')));
    assert_eq!(compose_cursor(&app), 2, "ctrl-f steps forward");
    handle_key(&mut app, ctrl_key(KeyCode::Char('e')));
    assert_eq!(compose_cursor(&app), 5, "ctrl-e jumps to line end");
    handle_key(&mut app, ctrl_key(KeyCode::Char('b')));
    assert_eq!(compose_cursor(&app), 4, "ctrl-b steps back");
    handle_key(&mut app, ctrl_key(KeyCode::Char('p')));
    assert_eq!(compose_cursor(&app), 4, "ctrl-p on one line stays put");
}

#[test]
fn compose_word_jumps() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "foo bar");
    handle_key(&mut app, alt_key(KeyCode::Char('b')));
    assert_eq!(compose_cursor(&app), 4, "alt-b jumps to the word start");
    handle_key(&mut app, alt_key(KeyCode::Char('b')));
    assert_eq!(compose_cursor(&app), 0, "alt-b again jumps to the first word");
    handle_key(&mut app, alt_key(KeyCode::Char('f')));
    assert_eq!(compose_cursor(&app), 3, "alt-f jumps to the word end");
    handle_key(&mut app, alt_key(KeyCode::Char('f')));
    assert_eq!(compose_cursor(&app), 7, "alt-f again jumps past the last word");
}

#[test]
fn compose_kill_ring_kills_and_yanks() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "hello world");
    handle_key(&mut app, ctrl_key(KeyCode::Char('w')));
    assert_eq!(app.draft().unwrap().body, "hello ");
    handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
    assert_eq!(app.draft().unwrap().body, "hello world");
    assert_eq!(compose_cursor(&app), 11);
    handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
    handle_key(&mut app, ctrl_key(KeyCode::Char('k')));
    assert_eq!(app.draft().unwrap().body, "");
    handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
    assert_eq!(app.draft().unwrap().body, "hello world");
}

#[test]
fn compose_alt_d_deletes_the_word_ahead() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "foo bar");
    handle_key(&mut app, ctrl_key(KeyCode::Char('a')));
    handle_key(&mut app, alt_key(KeyCode::Char('d')));
    assert_eq!(app.draft().unwrap().body, " bar");
    assert_eq!(compose_cursor(&app), 0, "cursor stays put");
    handle_key(&mut app, ctrl_key(KeyCode::Char('y')));
    assert_eq!(app.draft().unwrap().body, "foo bar");
}

#[test]
fn compose_ctrl_u_kills_to_line_start() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "hello");
    handle_key(&mut app, plain_key(KeyCode::Left));
    handle_key(&mut app, plain_key(KeyCode::Left));
    handle_key(&mut app, ctrl_key(KeyCode::Char('u')));
    assert_eq!(app.draft().unwrap().body, "lo");
    assert_eq!(compose_cursor(&app), 0);
}

#[test]
fn compose_ctrl_d_deletes_the_char_ahead() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "abc");
    handle_key(&mut app, plain_key(KeyCode::Left));
    handle_key(&mut app, plain_key(KeyCode::Left));
    handle_key(&mut app, ctrl_key(KeyCode::Char('d')));
    assert_eq!(app.draft().unwrap().body, "ac");
    assert_eq!(compose_cursor(&app), 1);
}

#[test]
fn compose_ctrl_t_transposes_chars() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "ab");
    handle_key(&mut app, ctrl_key(KeyCode::Char('t')));
    assert_eq!(app.draft().unwrap().body, "ba");
}

#[test]
fn compose_home_end_and_delete_keys() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "hi");
    handle_key(&mut app, plain_key(KeyCode::Home));
    assert_eq!(compose_cursor(&app), 0, "home jumps to line start");
    handle_key(&mut app, plain_key(KeyCode::End));
    assert_eq!(compose_cursor(&app), 2, "end jumps to line end");
    handle_key(&mut app, plain_key(KeyCode::Left));
    handle_key(&mut app, plain_key(KeyCode::Delete));
    assert_eq!(app.draft().unwrap().body, "h");
}

#[test]
fn compose_ctrl_i_inserts_tab_and_never_walks_history() {
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "a");
    handle_key(&mut app, ctrl_key(KeyCode::Char('i')));
    type_text(&mut app, "b");
    assert_eq!(app.draft().unwrap().body, "a\tb");
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "tab never leaves the modal"
    );
    assert!(app.compose_open());
}

#[test]
fn compose_ctrl_o_arms_editor_or_names_a_missing_editor() {
    // Serialized with every other env-mutating test process-wide.
    let _env = ghit_core::cache::test_env_lock();
    let prior = std::env::var("EDITOR").ok();
    std::env::remove_var("EDITOR");
    let mut app = compose_app();
    handle_key(&mut app, plain_key(KeyCode::Char('c')));
    type_text(&mut app, "draft");
    handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
    assert!(!app.compose.editor_requested, "nothing to run without an editor");
    assert!(app.message.contains("EDITOR"), "toast names the fix: {}", app.message);
    assert!(app.compose_open(), "draft survives");

    std::env::set_var("EDITOR", "true");
    handle_key(&mut app, ctrl_key(KeyCode::Char('o')));
    assert!(app.compose.editor_requested, "ctrl-o arms the editor run");
    assert!(app.compose_open(), "draft survives until the run lands");
    match prior {
        Some(v) => std::env::set_var("EDITOR", v),
        None => std::env::remove_var("EDITOR"),
    }
}

fn pending_dialog_app() -> App {
    let mut app = compose_app();
    app.session.issues
        .data
        .insert(ItemRef::github(9), sample(ItemRef::github(9)));
    app.session.issues.statuses.insert(ItemRef::github(9), vec![]);
    app.nav.visited.push(ItemRef::github(9));
    let d = app.session.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.is_pr = true;
    d.review_threads = vec![ghit_core::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(1),
        comments: vec![ghit_core::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app.request_issue(ItemRef::github(9));
    assert!(app.confirm_open(), "guard trips");
    app
}

#[test]
fn confirm_dialog_keys_submit_discard_and_cancel() {
    // Discard navigates without submitting.
    let mut app = pending_dialog_app();
    handle_key(&mut app, plain_key(KeyCode::Char('d')));
    assert!(!app.confirm_open());
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    assert!(app.session.sends.submitting.is_empty());
    // Submit navigates and submits.
    let mut app = pending_dialog_app();
    handle_key(&mut app, plain_key(KeyCode::Char('s')));
    assert!(!app.confirm_open());
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    assert!(app.session.sends.submitting.contains(&ItemRef::github(7)));
    // Enter picks the first choice (Submit).
    let mut app = pending_dialog_app();
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert!(!app.confirm_open());
    assert!(app.session.sends.submitting.contains(&ItemRef::github(7)));
    // Esc cancels: no navigation, nothing sent.
    let mut app = pending_dialog_app();
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.confirm_open());
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.session.sends.submitting.is_empty());
    // Quit never escapes the dialog.
    let mut app = pending_dialog_app();
    handle_key(&mut app, plain_key(KeyCode::Char('q')));
    assert!(!app.quit);
    assert!(app.confirm_open());
}

#[test]
fn ctrl_s_submits_pending_review() {
    let mut app = compose_app();
    let d = app.session.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.is_pr = true;
    d.review_threads = vec![ghit_core::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(1),
        comments: vec![ghit_core::model::ReviewComment { id: "c".into(), body: "draft".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    handle_key(&mut app, crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.session.sends.submitting.contains(&ItemRef::github(7)), "ctrl-s starts the submit");
}

fn step_app() -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    for n in [7u64, 8] {
        app.session.issues
            .data
            .insert(ItemRef::github(n), sample(ItemRef::github(n)));
        app.session.issues.statuses.insert(ItemRef::github(n), vec![]);
        app.nav.visited.push(ItemRef::github(n));
    }
    app.show_issue(ItemRef::github(7));
    app.nav.pane = Pane::Main;
    app
}

/// Which handler owns keys, per open layer. Written against the flag
/// storage and `handle_key` only, so it pins behavior across the
/// focus-model migration.
#[test]
fn top_layer_owns_keys_and_esc_pops_exactly_one() {
    use ghit_core::core::{Dialog, Layer, TextInput};
    let mut app = step_app();
    app.nav.pane = Pane::Issues;
    assert_eq!(app.focus().layer, Layer::Base);

    // Shortcuts swallow navigation; Esc closes.
    app.open(crate::app::Entry::Shortcuts);
    handle_key(&mut app, plain_key(KeyCode::Char('l')));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "shortcuts swallow l"
    );
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Shortcuts));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.shortcuts_open());

    // Confirm swallows everything, quit included; Esc cancels.
    app.open_pending_nav(crate::app::NavIntent::Back(1));
    handle_key(&mut app, plain_key(KeyCode::Char('q')));
    assert!(!app.quit, "quit never escapes the confirm");
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Confirm));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.confirm_open());

    // Compose takes typing; Esc on a typed draft asks before it
    // discards the whole modal.
    app.open_compose();
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Compose));
    handle_key(&mut app, plain_key(KeyCode::Char('l')));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "compose types, never steps"
    );
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Confirm));
    handle_key(&mut app, plain_key(KeyCode::Char('d')));
    assert!(!app.compose_open());

    // Format dialog: Esc closes from the dialog, but a text row pops
    // only itself and leaves the dialog open.
    app.open_sidebar_format();
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Format));
    app.start_query_edit();
    assert_eq!(app.focus().layer, Layer::Input(TextInput::Query));
    handle_key(&mut app, plain_key(KeyCode::Char('x')));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(app.format_open() && !app.query_edit(), "Esc pops the text row");
    assert_eq!(app.sidebar.query_draft, app.sidebar.query_active, "draft reverts");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.format_open());

    // Bar: Esc leaves it and clears the text.
    handle_key(&mut app, plain_key(KeyCode::Char(':')));
    assert_eq!(app.focus().layer, Layer::Input(TextInput::Bar));
    handle_key(&mut app, plain_key(KeyCode::Char('1')));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.focus().layer, Layer::Base);
    assert!(app.bar.is_empty());
}

#[test]
fn board_esc_closes_one_layer_and_keeps_the_filter() {
    use ghit_core::core::{Dialog, Layer};
    let mut app = step_app();
    app.session.board.loading = true;
    app.open(crate::app::Entry::Board);
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::Board));
    app.board.filter = "bug".to_string();
    app.open_board_filter();
    assert_eq!(app.focus().layer, Layer::Dialog(Dialog::BoardFilter));
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.format_open(), "form Esc closes the form");
    assert_eq!(app.board.filter, "bug", "and keeps the filter");
    assert!(board_open(&app), "the board stays up");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!board_open(&app), "board Esc closes the board");
    assert_eq!(app.board.filter, "bug", "and keeps the filter");
}

#[test]
fn history_pane_keys_walk_and_go_back() {
    let mut app = step_app();
    app.nav.history = vec![ItemRef::github(5), ItemRef::github(6), ItemRef::github(8)];
    app.refs.show = true;
    assert!(app.history_visible());
    app.nav.pane = Pane::History;
    // k moves up toward older rows, j back down; both clamp.
    handle_key(&mut app, plain_key(KeyCode::Char('j')));
    assert_eq!(app.history_steps(), 0);
    handle_key(&mut app, plain_key(KeyCode::Char('k')));
    handle_key(&mut app, plain_key(KeyCode::Char('k')));
    assert_eq!(app.history_steps(), 2);
    for _ in 0..9 {
        handle_key(&mut app, plain_key(KeyCode::Char('k')));
    }
    assert_eq!(app.history_steps(), 3, "clamped to the rows drawn");
    // Enter goes back that many steps, like clicking the row.
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.history, Vec::<ItemRef>::new());
    assert_eq!(app.nav.history_cursor, 0);
}

#[test]
fn brackets_steps_between_neighbors_on_main() {
    let mut app = step_app();
    handle_key(&mut app, plain_key(KeyCode::Char(']')));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(8)),
        "] opens the next issue"
    );
    assert_eq!(app.sidebar.issue_state.selected(), Some(2));
    handle_key(&mut app, plain_key(KeyCode::Char('[')));
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "[ opens the previous issue"
    );
    assert_eq!(app.sidebar.issue_state.selected(), Some(1));
    assert_eq!(app.nav.step_pending, None, "cached steps never linger");
}

#[test]
fn brackets_steps_from_every_pane() {
    // The neighbor step works from every pane: nothing else claims
    // [/] outside Graph.
    let mut app = step_app();
    for pane in [Pane::Main, Pane::Issues, Pane::PRs, Pane::Refs] {
        app.nav.pane = pane;
        app.message.clear();
        handle_key(&mut app, plain_key(KeyCode::Char(']')));
        assert_eq!(
            app.nav.current,
            Some(ItemRef::github(8)),
            "] steps from {pane:?}"
        );
        assert_eq!(app.nav.step_pending, None, "cached steps never linger");
        handle_key(&mut app, plain_key(KeyCode::Char('[')));
        assert_eq!(
            app.nav.current,
            Some(ItemRef::github(7)),
            "[ steps back from {pane:?}"
        );
        assert!(app.message.is_empty(), "no toast on a live neighbor");
    }
}

#[test]
fn brackets_on_main_without_an_issue_toasts() {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.repo_slug = Some("o/r".to_string());
    app.nav.pane = Pane::Main;
    handle_key(&mut app, plain_key(KeyCode::Char('[')));
    assert_eq!(app.message, "view an issue first");
    assert!(app.session.issues.pending.is_empty(), "no fetch without an issue");
}

#[test]
fn bar_filter_and_board_keep_their_h_and_l() {
    let mut app = step_app();
    // The command bar types the keys as text.
    app.nav.focus = crate::app::Focus::Bar;
    handle_key(&mut app, plain_key(KeyCode::Char('h')));
    handle_key(&mut app, plain_key(KeyCode::Char('l')));
    assert_eq!(app.bar, "hl");
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    // The filter dialog swallows them.
    app.nav.focus = crate::app::Focus::Main;
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    handle_key(&mut app, plain_key(KeyCode::Char('h')));
    handle_key(&mut app, plain_key(KeyCode::Char('l')));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert_eq!(app.nav.step_pending, None);
    // The board modal moves columns instead of stepping issues.
    app.close(crate::app::Slot::Format);
    app.session.board.loading = true;
    app.open(crate::app::Entry::Board);
    handle_key(&mut app, plain_key(KeyCode::Char('h')));
    handle_key(&mut app, plain_key(KeyCode::Char('l')));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)), "board keeps h/l");
    assert_eq!(app.nav.step_pending, None);
    assert!(app.session.board.loading, "board stays open");
}

#[test]
fn auth_prompt_y_requests_reauth_and_n_dismisses() {
    let mut app = step_app();
    app.open_reauth(
        "token lacks scopes ghit needs".to_string(),
        vec!["project".to_string()],
        true,
    );
    handle_key(&mut app, plain_key(KeyCode::Char('y')));
    assert!(app.session.auth.reauth_requested, "y arms the refresh");
    assert!(app.confirm_open(), "loop clears the modal after running");
    let mut app = step_app();
    app.open_reauth(
        "GitHub auth check failed: 401 Unauthorized".to_string(),
        vec![],
        false,
    );
    handle_key(&mut app, plain_key(KeyCode::Char('n')));
    assert!(!app.session.auth.reauth_requested, "n never refreshes");
    assert!(!app.confirm_open(), "n dismisses");
    assert!(matches!(app.nav.focus, crate::app::Focus::Main), "hint must render");
    assert!(app.message.contains("gh auth refresh"), "hint: {}", app.message);
}

#[test]
fn auth_esc_dismisses_like_n() {
    let mut app = step_app();
    app.open_reauth(
        "GitHub auth check failed: 401 Unauthorized".to_string(),
        vec![],
        false,
    );
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.session.auth.reauth_requested, "esc never refreshes");
    assert!(!app.confirm_open(), "esc dismisses");
    assert!(app.session.auth.dismissed, "esc counts as dismissed");
    assert!(app.message.contains("gh auth refresh"), "hint: {}", app.message);
}
fn press(app: &mut App, c: char) {
    handle_key(app, plain_key(KeyCode::Char(c)));
}

#[test]
fn gg_jumps_and_a_stray_key_cancels_the_prefix() {
    let mut app = step_app();
    app.nav.pane = Pane::Issues;
    app.show_issue(ItemRef::github(8));
    press(&mut app, 'g');
    assert_eq!(app.pending_prefix, Some('g'), "g waits");
    assert_eq!(app.nav.current, Some(ItemRef::github(8)));
    press(&mut app, 'g');
    assert_eq!(app.pending_prefix, None);
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "gg selects the first row"
    );
    press(&mut app, 'G');
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(8)),
        "G selects the last row"
    );
    press(&mut app, 'g');
    press(&mut app, 'k');
    assert_eq!(app.pending_prefix, None, "any other key cancels");
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "and runs as usual"
    );
}

#[test]
fn a_mouse_press_drops_a_waiting_prefix() {
    let mut app = step_app();
    press(&mut app, 'g');
    handle_mouse(&mut app, MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.pending_prefix, None);
}

#[test]
fn q_closes_the_top_dialog_and_quits_only_at_base() {
    let mut app = step_app();
    app.open(crate::app::Entry::Format(crate::app::FormatEdit::None));
    press(&mut app, 'q');
    assert!(!app.format_open() && !app.quit, "q closes the Format dialog");
    app.open(crate::app::Entry::Shortcuts);
    press(&mut app, 'q');
    assert!(!app.shortcuts_open() && !app.quit, "q closes help");
    app.session.board.loading = true;
    app.open(crate::app::Entry::Board);
    press(&mut app, 'q');
    assert!(!board_open(&app) && !app.quit, "q closes the board");
    press(&mut app, 'q');
    assert!(app.quit, "q at base quits");
}

#[test]
fn q_types_in_text_inputs_and_esc_never_quits() {
    let mut app = step_app();
    app.nav.focus = Focus::Bar;
    press(&mut app, 'q');
    assert_eq!(app.bar, "q");
    assert!(!app.quit);
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(app.bar.is_empty() && app.nav.focus == Focus::Main);
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(!app.quit, "Esc at base does nothing");
}

#[test]
fn help_opens_from_the_board_and_its_filter_form() {
    let mut app = step_app();
    app.session.board.loading = true;
    app.open(crate::app::Entry::Board);
    press(&mut app, '?');
    assert!(app.shortcuts_open());
    app.close(crate::app::Slot::Shortcuts);
    app.open_board_filter();
    app.board.filter_focus = 1;
    press(&mut app, '?');
    assert!(app.shortcuts_open(), "help opens on a form row");
    app.close(crate::app::Slot::Shortcuts);
    app.board.filter_focus = 0;
    press(&mut app, '?');
    assert!(!app.shortcuts_open() && app.sidebar.query_draft == "?", "the text row types ?");
}

#[test]
fn enter_focuses_the_content_and_esc_returns_to_the_list() {
    let mut app = step_app();
    app.nav.pane = Pane::Issues;
    app.nav.visited.clear();
    app.nav.recent = vec![ItemRef::github(7), ItemRef::github(8)];
    app.show_issue(ItemRef::github(8));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.current, Some(ItemRef::github(8)));
    assert_eq!(app.nav.pane, Pane::Main, "Enter focuses the content");
    assert!(
        !app.nav.visited.contains(&ItemRef::github(8)),
        "the row stays in its group"
    );
    // j scrolls the content now; it does not switch items.
    handle_key(&mut app, plain_key(KeyCode::Char('j')));
    assert_eq!(app.nav.current, Some(ItemRef::github(8)));
    assert_eq!(app.nav.scroll, 3);
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Issues, "Esc returns to the list");
    assert!(app.nav.back.is_empty());
    handle_key(&mut app, plain_key(KeyCode::Char('k')));
    assert!(
        app.nav.current == Some(ItemRef::github(7))
            || app.nav.requested == Some(ItemRef::github(7)),
        "k walks the list again"
    );
}

#[test]
fn enter_in_refs_and_history_focuses_the_content_too() {
    let mut app = step_app();
    app.show_issue(ItemRef::github(8));
    app.nav.pane = Pane::Refs;
    app.refs.highlight = Some(ItemRef::github(7));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.pane, Pane::Main);
    app.session.refs.numbers = vec![ItemRef::github(8)];
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Refs, "Esc returns to Refs");
    app.nav.pane = Pane::History;
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.pane, Pane::Main);
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::History, "Esc returns to History");
}

#[test]
fn esc_walks_the_back_stack_one_frame_at_a_time() {
    let mut app = step_app();
    app.nav.pane = Pane::Issues;
    app.show_issue(ItemRef::github(8));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    // A lateral move to Refs keeps the stack; Enter there pushes Refs
    // with the item it leaves.
    app.nav.pane = Pane::Refs;
    app.refs.highlight = Some(ItemRef::github(7));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(
        (app.nav.pane, app.nav.current),
        (Pane::Main, Some(ItemRef::github(7)))
    );
    assert!(app.back_target().is_some(), "Esc can go back");
    // Esc restores #8 and its Refs, the cursor on the opened reference.
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!((app.nav.pane, app.nav.current), (Pane::Refs, Some(ItemRef::github(8))));
    assert!(app.menu().is_none(), "a back-stack entry beats the main menu");
    assert_eq!(app.refs.highlight, Some(ItemRef::github(7)), "the Refs cursor stays on #7");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Issues, "the next Esc reaches the list");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Issues, "an empty stack stays put");
    assert!(app.menu().is_some(), "an empty stack opens the main menu");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert!(app.menu().is_none());
    // Entering the same pane twice keeps one frame for it.
    handle_key(&mut app, plain_key(KeyCode::Enter));
    app.nav.pane = Pane::Issues;
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.back.len(), 1);
}

#[test]
fn esc_keeps_the_item_once_you_move_on() {
    let mut app = step_app();
    app.show_issue(ItemRef::github(8));
    app.nav.pane = Pane::Refs;
    app.refs.highlight = Some(ItemRef::github(7));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    // Moving on to another item drops the restore; Refs is hidden for
    // it, so Esc has nowhere to go.
    press(&mut app, ']');
    assert_eq!(app.nav.current, Some(ItemRef::github(8)));
    app.session.refs.numbers.clear();
    app.session.refs.paired.clear();
    assert!(app.back_target().is_none(), "hidden Refs is skipped");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(
        (app.nav.pane, app.nav.current),
        (Pane::Main, Some(ItemRef::github(8)))
    );
    handle_key(&mut app, plain_key(KeyCode::Esc));
    // Coming back to #7 does not revive the restore.
    press(&mut app, '[');
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.nav.back.iter().all(|f| f.opened.is_none()));
}

#[test]
fn esc_clears_a_content_search_before_going_back() {
    let mut app = step_app();
    app.nav.pane = Pane::Issues;
    app.show_issue(ItemRef::github(8));
    handle_key(&mut app, plain_key(KeyCode::Enter));
    app.search.query = "x".to_string();
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Main, "the first Esc clears the search");
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(app.nav.pane, Pane::Issues);
}

#[test]
fn esc_after_a_history_jump_restores_the_item() {
    let mut app = step_app();
    app.ensure_visited(ItemRef::github(7));
    app.load_issue(ItemRef::github(7), false);
    app.ensure_visited(ItemRef::github(8));
    app.load_issue(ItemRef::github(8), false);
    app.nav.pane = Pane::History;
    app.history_step(-1);
    handle_key(&mut app, plain_key(KeyCode::Enter));
    assert_eq!(
        (app.nav.pane, app.nav.current),
        (Pane::Main, Some(ItemRef::github(7)))
    );
    handle_key(&mut app, plain_key(KeyCode::Esc));
    assert_eq!(
        (app.nav.pane, app.nav.current),
        (Pane::History, Some(ItemRef::github(8)))
    );
    assert_eq!(
        (app.nav.history.clone(), app.nav.future.clone()),
        (vec![ItemRef::github(7)], vec![]),
        "the chain is as before the jump"
    );
}

#[test]
fn o_opens_the_history_row_under_the_cursor() {
    let mut app = step_app();
    app.ensure_visited(ItemRef::github(7));
    app.load_issue(ItemRef::github(7), false);
    app.ensure_visited(ItemRef::github(8));
    app.load_issue(ItemRef::github(8), false);
    app.nav.pane = Pane::History;
    press(&mut app, 'o');
    assert_eq!(app.message, "opened #8 in browser");
    app.history_step(-1);
    press(&mut app, 'o');
    assert_eq!(app.message, "opened #7 in browser");
}

#[test]
fn brackets_step_issues_from_any_pane() {
    let mut app = step_app();
    for pane in [Pane::Main, Pane::Issues, Pane::Refs] {
        app.nav.pane = pane;
        press(&mut app, ']');
        assert_eq!(app.nav.current, Some(ItemRef::github(8)), "{pane:?}");
        press(&mut app, '[');
        assert_eq!(app.nav.current, Some(ItemRef::github(7)), "{pane:?}");
    }
}

#[test]
fn page_and_half_page_follow_the_visible_height() {
    let mut app = step_app();
    app.hits.body_area = Rect::new(0, 0, 40, 20);
    app.hits.body_total = 200;
    handle_key(&mut app, plain_key(KeyCode::PageDown));
    assert_eq!(app.nav.scroll, 20, "a page is the visible height");
    handle_key(&mut app, ctrl_key(KeyCode::Char('d')));
    assert_eq!(app.nav.scroll, 30, "half a page");
    handle_key(&mut app, ctrl_key(KeyCode::Char('u')));
    handle_key(&mut app, plain_key(KeyCode::PageUp));
    assert_eq!(app.nav.scroll, 0);
    press(&mut app, 'G');
    assert_eq!(app.nav.scroll, 180, "G ends on the last full view");
    press(&mut app, 'g');
    press(&mut app, 'g');
    assert_eq!(app.nav.scroll, 0);
}
