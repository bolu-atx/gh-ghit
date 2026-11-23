//! Tests for confirm dialogs, undo and auth prompts.

use ghit_core::github;
use super::*;
use super::test_support::{pending_nav_app, sidebar_app};
use ghit_core::model::ItemRef;

fn draft_app(body: &str) -> App {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    app.open_draft(ComposeState {
        number: ItemRef::github(7),
        target: ComposeTarget::Thread,
        body: body.to_string(),
        cursor: body.chars().count(),
        sending: false,
        kill: String::new(), view: Default::default(),
    });
    app
}

fn pending_review_app() -> App {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
    let d = app.session.issues.data.get_mut(&ItemRef::github(7)).unwrap();
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
    assert!(matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::DiscardDraft)));
    app.confirm_choose(0);
    assert!(app.compose_open(), "keep editing leaves the draft");
    app.request_discard_compose();
    app.confirm_choose(1);
    assert!(!app.compose_open(), "discard drops it");
    let mut empty = draft_app("");
    empty.request_discard_compose();
    assert!(!empty.compose_open() && !empty.confirm_open());
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
    let mut plain = sidebar_app(&[ItemRef::github(7)]);
    plain.request_quit();
    assert!(plain.quit && !plain.confirm_open(), "nothing at risk quits at once");
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
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)]);
    app.nav.current = Some(ItemRef::github(9));
    app.diff.expanded_rows.insert(ItemRef::github(9));
    app.evict_current();
    assert!(app.message.contains("removed #9 · u undo"), "{}", app.message);
    assert_eq!(app.nav.visited, vec![ItemRef::github(7), ItemRef::github(11)]);
    app.undo_last();
    assert_eq!(
        app.nav.visited,
        vec![ItemRef::github(7), ItemRef::github(9), ItemRef::github(11)],
        "same order"
    );
    assert!(
        app.session.issues.data.contains_key(&ItemRef::github(9))
            && app.session.issues.statuses.contains_key(&ItemRef::github(9))
    );
    assert!(app.diff.expanded_rows.contains(&ItemRef::github(9)));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    assert!(app.undo.is_none(), "one shot");
}

#[test]
fn saved_query_delete_undo_restores_index() {
    let mut app = sidebar_app(&[ItemRef::github(7)]);
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
    let mut app = sidebar_app(&[ItemRef::github(7), ItemRef::github(9)]);
    app.nav.current = Some(ItemRef::github(7));
    app.evict_current();
    assert!(app.undo_live());
    app.undo.as_mut().unwrap().at = Instant::now() - Duration::from_secs(10);
    assert!(!app.undo_live());
    app.undo_last();
    assert_eq!(
        app.nav.visited,
        vec![ItemRef::github(9)],
        "expired undo restores nothing"
    );
    app.nav.current = Some(ItemRef::github(9));
    app.evict_current();
    app.notify("other news".to_string());
    assert!(!app.undo_live(), "a later toast hides the undo offer");
}

#[test]
fn pending_nav_guard_trips_and_cancel_stays() {
    let mut app = pending_nav_app();
    app.request_issue(ItemRef::github(7));
    let dialog = app.confirm().expect("guard trips");
    assert_eq!(dialog.title, "Unsubmitted comments");
    assert!(dialog.body.iter().any(|l| l.text.contains("#9")), "names the issue: {:?}", dialog.body);
    assert_eq!(dialog.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Submit them", "Discard", "Cancel"]);
    assert_eq!(app.nav.current, Some(ItemRef::github(9)), "no navigation while the dialog is up");
    app.confirm_cancel();
    assert!(!app.confirm_open());
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(9)),
        "cancel stays put"
    );
}

#[test]
fn pending_nav_discard_navigates_without_submitting() {
    let mut app = pending_nav_app();
    app.request_issue(ItemRef::github(7));
    app.confirm_choose(1);
    assert!(!app.confirm_open());
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.compose.submitting.is_empty(), "discard never submits");
}

#[test]
fn pending_nav_submit_navigates_and_submits() {
    let mut app = pending_nav_app();
    app.request_issue(ItemRef::github(7));
    app.confirm_choose(0);
    assert!(!app.confirm_open());
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(7)),
        "submit still navigates"
    );
    assert!(
        app.compose.submitting.contains(&ItemRef::github(9)),
        "submit starts the review submit"
    );
}

#[test]
fn pending_nav_tab_switch_guards_and_same_target_passes() {
    let mut app = pending_nav_app();
    app.request_tab(crate::app::Tab::Visual);
    assert!(app.confirm_open(), "tab switch trips the guard");
    app.confirm_choose(1);
    assert_eq!(app.nav.tab, crate::app::Tab::Visual);
    // Same-target requests never open the dialog.
    let mut plain = sidebar_app(&[ItemRef::github(7), ItemRef::github(9)]);
    plain.show_issue(ItemRef::github(9));
    plain.request_issue(ItemRef::github(9));
    assert!(!plain.confirm_open());
    plain.request_tab(plain.nav.tab);
    assert!(!plain.confirm_open());
    // Without pending threads everything navigates directly.
    plain.request_issue(ItemRef::github(7));
    assert!(!plain.confirm_open());
    assert_eq!(plain.nav.current, Some(ItemRef::github(7)));
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
        app.session.auth.rx = Some(rx);
        tx.send(result).unwrap();
        poll_session(&mut app);
        assert!(app.session.auth.rx.is_none(), "probe lands once");
        assert_eq!(app.confirm_open(), expect_prompt);
        app.take_confirm();
    }
}

#[test]
fn auth_failure_prompts_once_then_stays_quiet() {
    let mut app = App::new(Some("o/r".to_string()));
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    let dialog = app.confirm().expect("first auth failure prompts");
    assert!(dialog.body.iter().any(|l| l.text.contains("scopes could not be read")), "fetch failures cannot name the gap");
    app.take_confirm();
    app.session.auth.dismissed = true;
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(!app.confirm_open(), "dismissed stays dismissed");
    app.session.auth.dismissed = false;
    auth_failure(&mut app, "bad search payload");
    assert!(!app.confirm_open(), "non-auth never prompts");
}

#[test]
fn auth_degraded_holds_until_a_reauth_succeeds() {
    let mut app = App::new(Some("o/r".to_string()));
    assert!(!app.session.auth.degraded, "healthy boot has no badge");
    let denied = "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad credentials";
    auth_failure(&mut app, denied);
    assert!(app.confirm_open(), "first failure prompts");
    assert!(app.session.auth.degraded, "the problem is recorded with the prompt");
    app.take_confirm();
    app.session.auth.dismissed = true;
    assert!(app.session.auth.degraded, "dismissing keeps the badge");
    auth_failure(&mut app, denied);
    assert!(!app.confirm_open(), "dismissed never re-modals");
    auth_failure(&mut app, "bad search payload");
    assert!(app.session.auth.degraded, "non-auth leaves the badge alone");
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.auth.rx = Some(rx);
    tx.send(Ok(vec![])).unwrap();
    poll_session(&mut app);
    assert!(app.session.auth.degraded, "a clean probe cannot prove every permission");
}

#[test]
fn auth_question_blocked_by_another_dialog_still_degrades() {
    let mut app = pending_nav_app();
    app.request_issue(ItemRef::github(7));
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad credentials");
    assert!(
        matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "the open question stays",
    );
    assert!(app.session.auth.degraded, "the badge records the blocked auth question");
}

#[test]
fn reopen_reauth_only_reopens_while_degraded() {
    let mut app = App::new(Some("o/r".to_string()));
    app.reopen_reauth();
    assert!(!app.confirm_open(), "healthy key is a no-op");
    app.session.auth.dismissed = true;
    app.session.auth.degraded = true;
    app.reopen_reauth();
    assert!(
        matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::Reauth)),
        "degraded key reopens the dialog",
    );
    assert!(app.session.auth.degraded, "reopening keeps the badge until a re-auth succeeds");
    let mut guarded = pending_nav_app();
    guarded.request_issue(ItemRef::github(7));
    guarded.session.auth.degraded = true;
    guarded.reopen_reauth();
    assert!(
        matches!(guarded.confirm().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "never stacks over another question",
    );
}

#[test]
fn auth_failure_waits_for_an_open_dialog() {
    let mut app = pending_nav_app();
    app.request_issue(ItemRef::github(7));
    assert!(app.confirm_open(), "nav guard holds the dialog");
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(
        matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::PendingNav { .. })),
        "auth never replaces another question",
    );
    app.confirm_cancel();
    auth_failure(&mut app, "GitHub request failed: 401 Unauthorized for https://api.github.com/x: Bad");
    assert!(
        matches!(app.confirm().map(|d| &d.kind), Some(ConfirmKind::Reauth)),
        "next failure re-raises it",
    );
}

#[test]
fn auth_probe_non_auth_failure_notifies_without_modal() {
    // A rate limit is not a scope problem: status line, never the
    // re-auth modal.
    let mut app = App::new(Some("o/r".to_string()));
    let (tx, rx) = std::sync::mpsc::channel();
    app.session.auth.rx = Some(rx);
    tx.send(Err("GitHub auth check failed: 403 Forbidden for https://api.github.com/user: rate limit".to_string())).unwrap();
    poll_session(&mut app);
    assert!(!app.confirm_open(), "no modal for quota errors");
    assert!(app.message.contains("403"), "still surfaces: {}", app.message);
}
