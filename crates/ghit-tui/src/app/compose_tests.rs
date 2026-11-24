//! Tests for compose drafts.

use super::*;
use super::test_support::sidebar_app;
use ghit_core::model::ItemRef;

/// PR with one file, one thread on its second line, and the cursor
/// parked by absolute body row. Rows mirror `ui` rendering: header,
/// dented patch rows, then the thread block.
fn thread_pr_app(diff_line: usize) -> App {
    let mut app = sidebar_app(&[ItemRef::github(9)]);
    let d = app.session.item_mut(ItemRef::github(9)).unwrap();
    d.is_pr = true;
    d.review_threads = vec![ghit_core::model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(2),
        ..Default::default()
    }];
    app.session.diff_test().files.insert(
        ItemRef::github(9),
        vec![ghit_core::model::DiffFile {
            filename: "a.rs".into(),
            patch: "@@ -1,2 +1,2 @@\n a\n+b".into(),
            ..Default::default()
        }],
    );
    app.nav.tab = Tab::Visual;
    app.hits.body_rows = vec![
        "▸ a.rs",
        "  @@ -1,2 +1,2 @@",
        "   a",
        "  +b",
        "      \u{f075} line 2 · unresolved",
        "      reviewer",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect();
    app.hits.body_base = 0;
    app.diff.line = Some(diff_line);
    app.diff.row_files = vec![None, None, None, Some(0), Some(0), Some(0)];
    app
}

#[test]
fn compose_needs_a_loaded_issue() {
    let mut app = App::new(Some("o/r".to_string()));
    app.session.set_repo_slug("o/r");
    app.open_compose();
    assert!(app.draft().is_none());
    assert_eq!(app.message, "view an issue first");

    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    let c = app.draft().expect("compose");
    assert_eq!(
        (c.number, c.body.as_str(), c.cursor),
        (ItemRef::github(7), "", 0)
    );
    assert_eq!(c.target, ComposeTarget::Thread);
}

#[test]
fn compose_on_a_thread_row_targets_the_reply() {
    let mut app = thread_pr_app(4);
    app.open_compose();
    let c = app.draft().expect("compose");
    assert_eq!(c.target, ComposeTarget::Reply { thread_id: "t1".into() });
}

#[test]
fn compose_on_a_code_row_still_targets_the_line() {
    let mut app = thread_pr_app(3);
    app.open_compose();
    let c = app.draft().expect("compose");
    assert_eq!(c.target, ComposeTarget::Line { path: "a.rs".into(), line: 2 });
}

#[test]
fn compose_edits_at_the_cursor() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    for ch in "hi\nok".chars() {
        app.compose_insert(ch);
    }
    assert_eq!(app.draft().unwrap().body, "hi\nok");
    app.compose_step(-3);
    app.compose_insert('!');
    assert_eq!(app.draft().unwrap().body, "hi!\nok");
    app.compose_backspace();
    assert_eq!(app.draft().unwrap().body, "hi\nok");
    // Down keeps the column across lines.
    app.compose_line(1);
    app.compose_insert('?');
    assert_eq!(app.draft().unwrap().body, "hi\nok?");
}

#[test]
fn compose_kills_stop_at_newlines() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    for ch in "one\ntwo".chars() {
        app.compose_insert(ch);
    }
    app.compose_step(-3);
    assert_eq!(app.draft().unwrap().cursor, 4);
    app.compose_kill_to_end();
    assert_eq!(app.draft().unwrap().body, "one\n");
    // On the newline itself the kill eats it, joining the lines.
    app.compose_step(-1);
    app.compose_kill_to_end();
    assert_eq!(app.draft().unwrap().body, "one");
    assert_eq!(app.draft().unwrap().kill, "\n");
    app.compose_yank();
    assert_eq!(app.draft().unwrap().body, "one\n");
}

#[test]
fn compose_kill_to_start_keeps_the_tail() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    for ch in "ab\ncd".chars() {
        app.compose_insert(ch);
    }
    app.compose_step(-1);
    app.compose_kill_to_start();
    assert_eq!(app.draft().unwrap().body, "ab\nd");
    assert_eq!(app.draft().unwrap().kill, "c");
}

#[test]
fn compose_editor_run_roundtrips_through_a_real_subprocess() {
    // cargo test owns no TTY: raw-mode and alt-screen calls fail
    // and are ignored, so this runs the whole suspend-spawn-land
    // path headless. `true` edits nothing and exits 0, so the draft
    // lands byte-identical through the temp file.
    let _env = ghit_core::cache::test_env_lock();
    let prior = std::env::var("EDITOR").ok();
    std::env::set_var("EDITOR", "true");
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    for ch in "draft ✓".chars() {
        app.compose_insert(ch);
    }
    app.compose.editor_requested = true;
    // A fixed viewport skips the terminal size query, which fails
    // without a TTY (CI runners).
    let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
    let options = ratatui::TerminalOptions { viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, 80, 24)) };
    let mut terminal = ratatui::Terminal::with_options(backend, options).expect("headless terminal");
    run_compose_editor(&mut app, &mut terminal);
    assert!(!app.compose.editor_requested, "the run disarms itself");
    assert_eq!(app.draft().unwrap().body, "draft ✓");
    assert!(app.message.contains("edited in true"), "toast: {}", app.message);
    // A failing editor keeps the draft.
    std::env::set_var("EDITOR", "false");
    app.compose.editor_requested = true;
    run_compose_editor(&mut app, &mut terminal);
    assert_eq!(app.draft().unwrap().body, "draft ✓");
    assert!(app.message.contains("draft kept"), "toast: {}", app.message);
    // A missing binary names the failure and keeps the draft.
    std::env::set_var("EDITOR", "/nonexistent-ghit-editor-xyz");
    app.compose.editor_requested = true;
    run_compose_editor(&mut app, &mut terminal);
    assert_eq!(app.draft().unwrap().body, "draft ✓");
    assert!(app.message.contains("failed to start"), "toast: {}", app.message);
    match prior {
        Some(v) => std::env::set_var("EDITOR", v),
        None => std::env::remove_var("EDITOR"),
    }
}

#[test]
fn compose_lands_editor_text_at_the_end() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    app.land_editor_text("edited\ntext".to_string(), "true");
    let c = app.draft().unwrap();
    assert_eq!((c.body.as_str(), c.cursor), ("edited\ntext", 11));
    assert!(app.message.contains("true"), "toast names the editor");
}

#[test]
fn compose_empty_submit_stays_open() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    app.submit_compose();
    let c = app.draft().expect("draft kept");
    assert!(!c.sending);
    assert!(app.message.contains("empty"), "status names the problem");
}

#[test]
fn compose_poll_success_closes_and_refreshes() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    app.draft_mut().unwrap().body = "looks good".to_string();
    app.draft_mut().unwrap().sending = true;
    app.session.sends_test().tx.send((ItemRef::github(7), Ok(()))).unwrap();
    poll_session(&mut app);
    assert!(app.draft().is_none());
    assert_eq!(app.message, "comment posted to #7");
    assert!(
        app.session.issues_test().pending.contains(&ItemRef::github(7)),
        "refetch shows the new comment"
    );
}

#[test]
fn compose_poll_failure_keeps_the_draft() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_compose();
    app.draft_mut().unwrap().body = "looks good".to_string();
    app.draft_mut().unwrap().sending = true;
    app.session
        .sends_test()
        .tx
        .send((
            ItemRef::github(7),
            Err("token scopes deny comments".to_string()),
        ))
        .unwrap();
    poll_session(&mut app);
    let c = app.draft().expect("draft kept");
    assert!(!c.sending);
    assert_eq!(c.body, "looks good");
    assert_eq!(app.message, "token scopes deny comments");
}
