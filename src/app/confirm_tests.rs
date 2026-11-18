//! Tests for confirm dialogs, undo and auth prompts.

use super::*;
use super::test_support::{pending_nav_app, sidebar_app};

fn draft_app(body: &str) -> App {
    let mut app = sidebar_app(&[7]);
    app.compose.draft = Some(ComposeState {
        number: 7,
        target: ComposeTarget::Thread,
        body: body.to_string(),
        cursor: body.chars().count(),
        sending: false,
        kill: String::new(),
    });
    app
}

fn pending_review_app() -> App {
    let mut app = sidebar_app(&[7]);
    let d = app.issues.data.get_mut(&7).unwrap();
    d.is_pr = true;
    d.review_threads = vec![model::ReviewThread {
        id: "t1".into(),
        path: "a.rs".into(),
        line: Some(1),
        comments: vec![model::ReviewComment { id: "c".into(), body: "x".into(), state: "PENDING".into(), ..Default::default() }],
        ..Default::default()
    }];
    app
}

#[test]
fn esc_on_typed_draft_asks_and_keep_leaves_it() {
    let mut app = draft_app("hello");
    app.request_discard_compose();
    assert!(matches!(app.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::DiscardDraft)));
    app.confirm_choose(0);
    assert!(app.compose_open(), "keep editing leaves the draft");
    app.request_discard_compose();
    app.confirm_choose(1);
    assert!(!app.compose_open(), "discard drops it");
    let mut empty = draft_app("");
    empty.request_discard_compose();
    assert!(!empty.compose_open() && empty.confirm.is_none());
}

#[test]
fn quit_with_draft_asks_first() {
    let mut app = draft_app("hello");
    app.request_quit();
    assert!(!app.quit && app.confirm_open());
    app.confirm_cancel();
    assert!(!app.quit);
    app.request_quit();
    app.confirm_choose(1);
    assert!(app.quit);
    let mut plain = sidebar_app(&[7]);
    plain.request_quit();
    assert!(plain.quit && plain.confirm.is_none(), "nothing at risk quits at once");
}

#[test]
fn quit_with_pending_review_asks_first() {
    let mut app = pending_review_app();
    app.request_quit();
    assert!(!app.quit && app.confirm_open());
    app.confirm_choose(0);
    assert!(!app.quit, "cancel stays");
    app.request_quit();
    app.confirm_choose(1);
    assert!(app.quit);
}

#[test]
fn evict_undo_restores_row_in_place() {
    let mut app = sidebar_app(&[7, 9, 11]);
    app.nav.current = Some(9);
    app.diff.expanded_rows.insert(9);
    app.evict_current();
    assert!(app.message.contains("removed #9 · u undo"), "{}", app.message);
    assert_eq!(app.nav.visited, vec![7, 11]);
    app.undo_last();
    assert_eq!(app.nav.visited, vec![7, 9, 11], "same order");
    assert!(app.issues.data.contains_key(&9) && app.issues.statuses.contains_key(&9));
    assert!(app.diff.expanded_rows.contains(&9));
    assert_eq!(app.nav.current, Some(9));
    assert!(app.undo.is_none(), "one shot");
}

#[test]
fn saved_query_delete_undo_restores_index() {
    let mut app = sidebar_app(&[7]);
    for (name, query) in [("a", "is:open"), ("b", "label:bug"), ("c", "is:pr")] {
        app.sidebar.saved_queries.push(github::SavedQuery { name: name.into(), query: query.into() });
    }
    app.sidebar.format_focus = 2;
    app.delete_named_query();
    assert!(app.message.contains("deleted query b · u undo"), "{}", app.message);
    assert_eq!(app.sidebar.saved_queries.len(), 2);
    app.undo_last();
    let names: Vec<&str> = app.sidebar.saved_queries.iter().map(|q| q.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(app.sidebar.saved_queries[1].query, "label:bug");
}

#[test]
fn undo_expires_with_its_window() {
    let mut app = sidebar_app(&[7, 9]);
    app.nav.current = Some(7);
    app.evict_current();
    assert!(app.undo_live());
    app.undo.as_mut().unwrap().at = Instant::now() - Duration::from_secs(10);
    assert!(!app.undo_live());
    app.undo_last();
    assert_eq!(app.nav.visited, vec![9], "expired undo restores nothing");
    app.nav.current = Some(9);
    app.evict_current();
    app.notify("other news".to_string());
    assert!(!app.undo_live(), "a later toast hides the undo offer");
}

#[test]
fn pending_nav_guard_trips_and_cancel_stays() {
    let mut app = pending_nav_app();
    app.request_issue(7);
    let dialog = app.confirm.as_ref().expect("guard trips");
    assert_eq!(dialog.title, "Unsubmitted comments");
    assert!(dialog.body.iter().any(|l| l.text.contains("#9")), "names the issue: {:?}", dialog.body);
    assert_eq!(dialog.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Submit them", "Discard", "Cancel"]);
    assert_eq!(app.nav.current, Some(9), "no navigation while the dialog is up");
    app.confirm_cancel();
    assert!(app.confirm.is_none());
    assert_eq!(app.nav.current, Some(9), "cancel stays put");
}

#[test]
fn pending_nav_discard_navigates_without_submitting() {
    let mut app = pending_nav_app();
    app.request_issue(7);
    app.confirm_choose(1);
    assert!(app.confirm.is_none());
    assert_eq!(app.nav.current, Some(7));
    assert!(app.compose.submitting.is_empty(), "discard never submits");
}

#[test]
fn pending_nav_submit_navigates_and_submits() {
    let mut app = pending_nav_app();
    app.request_issue(7);
    app.confirm_choose(0);
    assert!(app.confirm.is_none());
    assert_eq!(app.nav.current, Some(7), "submit still navigates");
    assert!(app.compose.submitting.contains(&9), "submit starts the review submit");
}

#[test]
fn pending_nav_tab_switch_guards_and_same_target_passes() {
    let mut app = pending_nav_app();
    app.request_tab(crate::app::Tab::Visual);
    assert!(app.confirm.is_some(), "tab switch trips the guard");
    app.confirm_choose(1);
    assert_eq!(app.nav.tab, crate::app::Tab::Visual);
    // Same-target requests never open the dialog.
    let mut plain = sidebar_app(&[7, 9]);
    plain.show_issue(9);
    plain.request_issue(9);
    assert!(plain.confirm.is_none());
    plain.request_tab(plain.nav.tab);
    assert!(plain.confirm.is_none());
    // Without pending threads everything navigates directly.
    plain.request_issue(7);
    assert!(plain.confirm.is_none());
    assert_eq!(plain.nav.current, Some(7));
}

#[test]
fn auth_probe_landing_drives_the_prompt() {
    // Missing scopes raise the modal; full coverage stays silent;
    // a dead token names its cause with no scopes to list.
    let mut app = App::new(Some("o/r".to_string()));
    for (result, expect_prompt) in [
        (Ok(vec!["project".to_string()]), true),
        (Ok(vec![]), false),
        (Err("GitHub auth check failed: 401 Unauthorized".to_string()), true),
    ] {
        let (tx, rx) = std::sync::mpsc::channel();
        app.auth.rx = Some(rx);
        tx.send(result).unwrap();
        poll_auth(&mut app);
        assert!(app.auth.rx.is_none(), "probe lands once");
        assert_eq!(app.confirm.is_some(), expect_prompt);
        app.confirm = None;
    }
}

#[test]
fn auth_failure_prompts_once_then_stays_quiet() {
    let mut app = App::new(Some("o/r".to_string()));
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    let dialog = app.confirm.as_ref().expect("first auth failure prompts");
    assert!(dialog.body.iter().any(|l| l.text.contains("scopes could not be read")), "fetch failures cannot name the gap");
    app.confirm = None;
    app.auth.dismissed = true;
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(app.confirm.is_none(), "dismissed stays dismissed");
    app.auth.dismissed = false;
    auth_failure(&mut app, "bad search payload");
    assert!(app.confirm.is_none(), "non-auth never prompts");
}

#[test]
fn auth_degraded_holds_until_a_reauth_succeeds() {
    let mut app = App::new(Some("o/r".to_string()));
    assert!(!app.auth.degraded, "healthy boot has no badge");
    let denied = "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad credentials";
    auth_failure(&mut app, denied);
    assert!(app.confirm.is_some(), "first failure prompts");
    assert!(app.auth.degraded, "the problem is recorded with the prompt");
    app.confirm = None;
    app.auth.dismissed = true;
    assert!(app.auth.degraded, "dismissing keeps the badge");
    auth_failure(&mut app, denied);
    assert!(app.confirm.is_none(), "dismissed never re-modals");
    auth_failure(&mut app, "bad search payload");
    assert!(app.auth.degraded, "non-auth leaves the badge alone");
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.rx = Some(rx);
    tx.send(Ok(vec![])).unwrap();
    poll_auth(&mut app);
    assert!(app.auth.degraded, "a clean probe cannot prove every permission");
}

#[test]
fn auth_question_blocked_by_another_dialog_still_degrades() {
    let mut app = pending_nav_app();
    app.request_issue(7);
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad credentials");
    assert!(
        matches!(app.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "the open question stays",
    );
    assert!(app.auth.degraded, "the badge records the blocked auth question");
}

#[test]
fn reopen_reauth_only_reopens_while_degraded() {
    let mut app = App::new(Some("o/r".to_string()));
    app.reopen_reauth();
    assert!(app.confirm.is_none(), "healthy key is a no-op");
    app.auth.dismissed = true;
    app.auth.degraded = true;
    app.reopen_reauth();
    assert!(
        matches!(app.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::Reauth)),
        "degraded key reopens the dialog",
    );
    assert!(app.auth.degraded, "reopening keeps the badge until a re-auth succeeds");
    let mut guarded = pending_nav_app();
    guarded.request_issue(7);
    guarded.auth.degraded = true;
    guarded.reopen_reauth();
    assert!(
        matches!(guarded.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "never stacks over another question",
    );
}

#[test]
fn auth_failure_waits_for_an_open_dialog() {
    let mut app = pending_nav_app();
    app.request_issue(7);
    assert!(app.confirm.is_some(), "nav guard holds the dialog");
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(
        matches!(app.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "auth never replaces another question",
    );
    app.confirm_cancel();
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(
        matches!(app.confirm.as_ref().map(|d| &d.kind), Some(ConfirmKind::Reauth)),
        "next failure re-raises it",
    );
}

#[test]
fn auth_probe_non_auth_failure_notifies_without_modal() {
    // A rate limit is not a scope problem: status line, never the
    // re-auth modal.
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.auth.rx = Some(rx);
    tx.send(Err("GitHub auth check failed: 403 Forbidden for https://api.github.com/user: rate limit".to_string())).unwrap();
    poll_auth(&mut app);
    assert!(app.confirm.is_none(), "no modal for quota errors");
    assert!(app.message.contains("403"), "still surfaces: {}", app.message);
}
