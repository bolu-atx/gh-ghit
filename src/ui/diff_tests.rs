//! Frame tests for the Diff tab.

use crate::app::{App, Tab};
use crate::input::{handle_key, handle_mouse};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{loaded_app, key, click, frame_text, diff_pr};

fn diff_loaded_app() -> App {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.diff.expanded_files.insert((9, "src/a.rs".to_string()));
    app.set_tab(Tab::Visual);
    app
}

#[test]
fn review_threads_render_at_matching_lines_in_both_modes() {
    use crate::model::{DiffFile, ReviewThread, ReviewComment};
    let mut app = loaded_app();
    let mut pr = diff_pr(9);
    pr.review_threads = vec![
        ReviewThread { id: "a".into(), path: "a.rs".into(), line: Some(1), comments: vec![ReviewComment { id: "c".into(), body: "inline note".into(), ..Default::default() }], ..Default::default() },
        ReviewThread { id: "b".into(), path: "a.rs".into(), line: Some(8), ..Default::default() },
    ];
    app.issues.data.insert(9, pr); app.show_issue(9);
    app.diff.files.insert(9, vec![DiffFile { filename: "a.rs".into(), patch: "@@ -1 +1 @@\n-old\n+new".into(), ..Default::default() }]);
    app.diff.expanded_files.insert((9, "a.rs".into())); app.set_tab(Tab::Visual);
    for split in [false, true] {
        app.diff.split = split;
        let text = frame_text(&mut app);
        assert!(text.find("line 8").unwrap() < text.find("+new").unwrap(), "{text}");
        assert!(text.find("+new").unwrap() < text.find("line 1").unwrap(), "{text}");
        assert!(text.contains('\u{f075}') && text.contains("inline note"), "{text}");
    }
}

#[test]
fn pending_review_threads_show_a_pending_pill() {
    use crate::model::{DiffFile, ReviewThread, ReviewComment};
    let mut app = loaded_app();
    let mut pr = diff_pr(9);
    pr.review_threads = vec![
        ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(1), comments: vec![
            ReviewComment { id: "c".into(), body: "draft note".into(), state: "PENDING".into(), ..Default::default() },
        ], ..Default::default() },
        ReviewThread { id: "t2".into(), path: "a.rs".into(), line: Some(8), comments: vec![
            ReviewComment { id: "d".into(), body: "live note".into(), ..Default::default() },
        ], ..Default::default() },
    ];
    app.issues.data.insert(9, pr);
    app.show_issue(9);
    app.diff.files.insert(9, vec![DiffFile { filename: "a.rs".into(), patch: "@@ -1 +1 @@\n-old\n+new".into(), ..Default::default() }]);
    app.diff.expanded_files.insert((9, "a.rs".into()));
    app.set_tab(Tab::Visual);
    app.confirm_choose(1);
    assert!(!app.confirm_open(), "discard keeps the test on the Diff tab");
    let text = frame_text(&mut app);
    assert!(text.contains("line 1 · unresolved · pending"), "{text}");
    assert!(text.contains("pending") && text.contains("draft note"), "{text}");
    assert!(!text.contains("line 8 · unresolved · pending"), "{text}");
}

#[test]
fn diff_groups_review_threads_by_file() {
    use crate::model::{DiffFile, parse_review_threads};
    let mut app = loaded_app();
    let mut pr = diff_pr(9);
    pr.review_threads = parse_review_threads(&serde_json::json!({"reviewThreads":{"nodes":[
        {"id":"t1","path":"src/a.rs","line":3,"isResolved":true,"comments":{"nodes":[{"id":"c1","author":{"login":"ada"},"body":"resolved body"}]}},
        {"id":"t2","path":"src/a.rs","line":7,"isResolved":false,"comments":{"nodes":[{"id":"c2","author":{"login":"bob"},"body":"**open body**"}]}},
        {"id":"t3","path":"src/b.rs","line":null,"isResolved":false,"comments":{"nodes":[{"id":"c3","author":{"login":"cam"},"body":"other body"}]}}
    ]}}));
    app.issues.data.insert(9, pr);
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), ..Default::default() },
        DiffFile { filename: "src/b.rs".into(), ..Default::default() },
    ]);
    app.set_tab(Tab::Visual);
    let text = frame_text(&mut app);
    let a = text.find("▸ ").unwrap();
    let open = text.find("line 7 · unresolved").unwrap();
    let resolved = text.find("line 3 · resolved").unwrap();
    let b = text.find("src/b.rs").unwrap();
    assert!(a < open && open < resolved && resolved < b, "{text}");
    assert!(text.contains("bob") && text.contains("open body") && text.contains("ada") && text.contains("resolved body"), "{text}");
    assert!(text.contains("line unavailable · unresolved") && text.contains("other body"), "{text}");
}

#[test]
fn diff_tab_folds_files_by_default() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        DiffFile { filename: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: String::new() },
    ]);
    app.set_tab(Tab::Visual);
    app.nav.pane = crate::core::Pane::Main;
    let text = frame_text(&mut app);
    assert!(text.contains("2 Diff"), "{text}");
    assert!(text.contains("2 files"), "{text}");
    assert!(text.contains("src/a.rs"), "{text}");
    assert!(text.contains("▸"), "folded by default");
    assert!(text.contains("2 changed"), "{text}");
    assert!(text.contains("──"), "ruled sections");
    assert!(text.contains("❯ src/a.rs"), "summary names the cursor file");
    assert!(text.contains('\u{f15c}'), "file icon");
    assert!(text.contains('\u{f040}'), "modified icon");
    assert!(!text.contains("-old"), "hunks hidden while folded");
    assert!(!text.contains("binary or large file"), "note hidden while folded");
    // Space unfolds the file under the active line.
    handle_key(&mut app, key(KeyCode::Char(' ')));
    assert!(app.diff.expanded_files.contains(&(9, "src/a.rs".to_string())));
    let open = frame_text(&mut app);
    assert!(open.contains("▾"), "{open}");
    assert!(open.contains("@@ -1"), "{open}");
    assert!(open.contains("1 │ -old"), "old-side gutter on deletions: {open}");
    assert!(open.contains("1 │ +new"), "new-side gutter on additions: {open}");
    assert!(open.contains("│ @@ -1 +1 @@"), "blank gutter on hunk headers: {open}");
    // Down walks line by line: header, hunk, deletion, addition, then
    // the second file's blank, rule, and header rows.
    for _ in 0..6 {
        handle_key(&mut app, key(KeyCode::Down));
    }
    assert_eq!(app.diff.cursor, 0, "still resolving file 0 until render");
    let _ = frame_text(&mut app);
    assert_eq!(app.diff.cursor, 1, "landed on the second header");
    assert!(app.diff.expanded_files.contains(&(9, "img.png".to_string())), "landed-on collapsed file opens");
    // Unfolding the second file shows its binary note.
    let both = frame_text(&mut app);
    assert!(both.contains("binary or large file"), "{both}");
    // Up walks back to the first header; clicking it folds it back.
    for _ in 0..6 {
        handle_key(&mut app, key(KeyCode::Up));
    }
    let row = app.hits.body_rows.iter().position(|l| l.starts_with("▾ ")).unwrap();
    let (bx, by) = (app.hits.body_area.x, app.hits.body_area.y);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), bx + 1, by + row as u16));
    assert!(!app.diff.expanded_files.contains(&(9, "src/a.rs".to_string())));
    assert_eq!(app.diff.cursor, 0);
}

#[test]
fn arrows_walk_diff_lines() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        DiffFile { filename: "src/b.rs".into(), status: "modified".into(), additions: 2, deletions: 0, patch: "@@ -1 +1 @@\n+new".into() },
    ]);
    app.set_tab(Tab::Visual);
    app.nav.pane = crate::core::Pane::Main;
    let _ = frame_text(&mut app);
    // The cursor anchors to the first header without expanding it.
    let h0 = app.diff.headers[0];
    assert_eq!(app.diff.line, Some(h0));
    assert!(app.diff.expanded_files.is_empty());
    // Down steps one row at a time and never touches the selection.
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.diff.line, Some(h0 + 1));
    assert_eq!(app.nav.current, Some(9), "selection untouched");
    // Walking onto the second header and rendering resolves the file
    // and unfolds it: two blank/rule rows then the header.
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Down));
    let _ = frame_text(&mut app);
    assert_eq!(app.diff.cursor, 1);
    assert!(app.diff.expanded_files.contains(&(9, "src/b.rs".to_string())));
    // Up steps back a single row.
    let back = app.diff.line.unwrap();
    handle_key(&mut app, key(KeyCode::Up));
    assert_eq!(app.diff.line, Some(back - 1));
    // j/k stays with the side panes: in Issues it walks issues only.
    app.show_issue(7);
    app.nav.pane = crate::core::Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.diff.cursor, 0);
    assert_eq!(app.nav.current, Some(7));
}

#[test]
fn shift_jk_jumps_between_diff_files() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    // Long expanded first file: the second header starts off-screen,
    // so the jump must scroll, not just hop visible headers.
    let patch_a =
        format!("@@ -1 +1 @@\n{}", (0..60).map(|i| format!("+line{i}")).collect::<Vec<_>>().join("\n"));
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 60, deletions: 0, patch: patch_a },
        DiffFile { filename: "src/b.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: "@@ -1 +1 @@\n+new".into() },
    ]);
    app.diff.expanded_files.insert((9, "src/a.rs".into()));
    app.set_tab(Tab::Visual);
    app.nav.pane = crate::core::Pane::Main;
    let _ = frame_text(&mut app);
    assert_eq!(app.diff.cursor, 0);
    let top = app.nav.scroll;
    // J lands the second header at the top and opens it.
    handle_key(&mut app, key(KeyCode::Char('J')));
    assert_eq!(app.diff.cursor, 1, "J moves the file cursor before render");
    let _ = frame_text(&mut app);
    assert!(app.nav.scroll > top, "off-screen header scrolls into view");
    assert_eq!(app.diff.cursor, 1, "render keeps the jumped file");
    assert!(app.diff.expanded_files.contains(&(9, "src/b.rs".to_string())), "jumped-on file opens");
    assert!(
        app.hits.body_rows.first().is_some_and(|l| l.contains("src/b.rs")),
        "header lands at top: {:?}",
        app.hits.body_rows.first()
    );
    // J at the last file is a no-op.
    let held = app.nav.scroll;
    handle_key(&mut app, key(KeyCode::Char('J')));
    let _ = frame_text(&mut app);
    assert_eq!((app.diff.cursor, app.nav.scroll), (1, held), "clamped at the last file");
    // K jumps back to the first header.
    handle_key(&mut app, key(KeyCode::Char('K')));
    let _ = frame_text(&mut app);
    assert_eq!(app.diff.cursor, 0);
    assert!(
        app.hits.body_rows.first().is_some_and(|l| l.contains("src/a.rs")),
        "first header back at top: {:?}",
        app.hits.body_rows.first()
    );
}

#[test]
fn diff_active_line_highlights_and_yanks() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.set_tab(Tab::Visual);
    app.nav.pane = crate::core::Pane::Main;
    let _ = frame_text(&mut app);
    // Unfold, then step onto the deletion line.
    handle_key(&mut app, key(KeyCode::Char(' ')));
    let _ = frame_text(&mut app);
    let h0 = app.diff.headers[0];
    handle_key(&mut app, key(KeyCode::Down));
    handle_key(&mut app, key(KeyCode::Down));
    assert_eq!(app.diff.line, Some(h0 + 2));
    let _ = frame_text(&mut app);
    // The active line yanks with its file and PR identity.
    assert_eq!(
        app.yank_text().unwrap(),
        "#9 Fix the thing — src/a.rs: \"-old\" — https://github.com/o/r/issues/9"
    );
    // The active row paints amber across the body width.
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let vis = h0 + 2;
    let cell = &buf[(app.hits.body_area.x + 1, app.hits.body_area.y + vis as u16)];
    assert_eq!(cell.bg, crate::theme::line_hl(), "active line tinted");
    let row_y = app.hits.body_area.y + vis as u16;
    let code = (app.hits.body_area.x..app.hits.body_area.x + app.hits.body_area.width)
        .map(|x| &buf[(x, row_y)])
        .find(|c| c.symbol() == "-")
        .expect("deletion marker on the active row");
    assert_eq!(code.fg, crate::theme::red(), "deletion keeps its color");
    assert_eq!(code.bg, crate::theme::line_hl(), "code cells share the tint");
}

#[test]
fn mouse_click_highlights_the_row_it_expands() {
    use crate::core::Pane;
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.set_tab(Tab::Visual);
    let _ = frame_text(&mut app);
    let row = app.hits.body_rows.iter().position(|l| l.starts_with("▸ ")).unwrap();
    let (bx, by) = (app.hits.body_area.x, app.hits.body_area.y);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), bx + 1, by + row as u16));
    assert_eq!(app.nav.pane, Pane::Main, "click takes body focus");
    assert_eq!(app.diff.cursor, 0);
    // The click parks the line; render resolves the file and expands it.
    let _ = frame_text(&mut app);
    assert!(app.diff.expanded_files.contains(&(9, "src/a.rs".to_string())));
    // The expanded header renders with the accent background.
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let hrow = app.diff.headers[0];
    let cell = &buf[(app.hits.body_area.x + 1, app.hits.body_area.y + hrow as u16)];
    assert_eq!(cell.bg, crate::theme::line_hl(), "expanded row highlighted");
}

#[test]
fn diff_summary_shows_file_icon() {
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.pending.remove(&9);
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        DiffFile { filename: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: String::new() },
    ]);
    app.set_tab(Tab::Visual);
    let text = frame_text(&mut app);
    assert!(text.contains("\u{f15c} 2 files"), "{text}");
}

#[test]
fn diff_tab_notes_issues_have_no_diff() {
    let mut app = loaded_app();
    app.set_tab(Tab::Visual);
    let text = frame_text(&mut app);
    assert!(text.contains("Issues have no diff."), "{text}");
    assert!(text.contains("Map tab (3)"), "the note points at the map: {text}");
    assert!(!text.contains("Reading the local copy"), "the map stays on its own tab: {text}");
}

#[test]
fn diff_tab_shows_loading_while_pending() {
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    app.diff.files.remove(&9);
    app.diff.pending.insert(9);
    app.set_tab(Tab::Visual);
    assert!(frame_text(&mut app).contains("Fetching diff"), "loading state");
}

#[test]
fn diff_fetch_lands_without_touching_selection() {
    use crate::app::poll_diffs;
    use crate::model::DiffFile;
    let mut app = loaded_app();
    app.issues.data.insert(9, diff_pr(9));
    app.show_issue(9);
    let files = vec![DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: "@@ -1 +1 @@\n+new".into() }];
    app.diff.tx.send((9, Ok(files))).unwrap();
    poll_diffs(&mut app);
    assert!(!app.diff.pending.contains(&9));
    assert_eq!(app.diff.files.get(&9).unwrap().len(), 1);
    assert_eq!(app.nav.current, Some(9));
}

#[test]
fn mouse_click_reaches_diff_tab() {
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    let (tx, y) = (app.hits.tabs_area.x, app.hits.tabs_area.y);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 18, y));
    assert_eq!(app.nav.tab, Tab::Visual);
}

#[test]
fn split_mode_pairs_old_and_new_side_by_side() {
    let mut app = diff_loaded_app();
    app.diff.split = true;
    let text = frame_text(&mut app);
    assert!(text.contains("split (s: unified)"), "{text}");
    assert!(text.contains("│"), "{text}");
    assert!(text.contains("-old"), "{text}");
    assert!(text.contains("+new"), "{text}");
    assert!(text.contains("1 -old"), "old-side gutter in split left cell: {text}");
    assert!(text.contains("1 +new"), "new-side gutter in split right cell: {text}");
}

#[test]
fn h_and_l_fold_and_unfold_the_diff_file() {
    let mut app = diff_loaded_app();
    app.nav.pane = crate::core::Pane::Main;
    let key9 = (9, "src/a.rs".to_string());
    assert!(app.diff.expanded_files.contains(&key9));
    handle_key(&mut app, key(KeyCode::Char('h')));
    assert!(!app.diff.expanded_files.contains(&key9), "h folds");
    handle_key(&mut app, key(KeyCode::Char('h')));
    assert!(!app.diff.expanded_files.contains(&key9), "h on a folded file stays folded");
    handle_key(&mut app, key(KeyCode::Char('l')));
    assert!(app.diff.expanded_files.contains(&key9), "l unfolds");
    assert_eq!(app.nav.current, Some(9), "h/l no longer step issues here");
}

#[test]
fn unified_mode_hints_at_split() {
    let mut app = diff_loaded_app();
    let text = frame_text(&mut app);
    assert!(text.contains("unified (s: split)"), "{text}");
}

#[test]
fn s_toggles_split_only_on_diff_tab() {
    use crate::core::Pane;
    let mut app = diff_loaded_app();
    app.nav.pane = Pane::Main;
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert!(app.diff.split);
    assert!(app.message.contains("split"));
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert!(!app.diff.split);
    assert!(app.message.contains("unified"));
    app.set_tab(Tab::Details);
    handle_key(&mut app, key(KeyCode::Char('s')));
    assert!(!app.diff.split, "s outside Diff does nothing");
}

fn expand_app() -> App {
    use crate::model::DiffFile;
    let mut app = diff_loaded_app();
    app.diff.expanded_files.clear();
    let big: String = (0..320).map(|i| format!("+big{i}\n")).collect();
    app.diff.files.insert(9, vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        DiffFile { filename: "src/big.rs".into(), status: "modified".into(), additions: 320, deletions: 0, patch: format!("@@ -0,0 +1,320 @@\n{big}") },
        DiffFile { filename: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: String::new() },
        DiffFile { filename: "src/c.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: "@@ -1 +1 @@\n+see".into() },
    ]);
    app.nav.pane = crate::core::Pane::Main;
    app
}

fn open_files(app: &App) -> Vec<String> {
    let mut v: Vec<String> = app.diff.expanded_files.iter().map(|(_, f)| f.clone()).collect();
    v.sort();
    v
}

#[test]
fn expand_all_opens_small_files_then_folds_everything() {
    let mut app = expand_app();
    let _ = frame_text(&mut app);
    handle_key(&mut app, key(KeyCode::Char('a')));
    let text = frame_text(&mut app);
    assert_eq!(open_files(&app), ["src/a.rs", "src/c.rs"], "large and patchless stay folded");
    assert_eq!(app.message, "expanded 2 files · 2 left folded");
    assert!(text.contains("+new") && text.contains("+see"), "{text}");
    assert!(text.contains("large diff · space"), "{text}");
    assert!(!text.contains("+big0"), "{text}");
    // Space still opens the large file by hand; expand-all stays "fold".
    app.diff.expanded_files.remove(&(9, "src/c.rs".into()));
    assert!(app.diff_will_expand());
    app.diff.expanded_files.insert((9, "src/c.rs".into()));
    assert!(!app.diff_will_expand());
    app.diff.expanded_files.insert((9, "src/big.rs".into()));
    handle_key(&mut app, key(KeyCode::Char('a')));
    let _ = frame_text(&mut app);
    assert!(open_files(&app).is_empty(), "second press folds all, large included");
    assert_eq!(app.message, "folded all files");
}

#[test]
fn expand_all_keeps_the_cursor_file_in_view() {
    let mut app = expand_app();
    let _ = frame_text(&mut app);
    // Put the cursor on the last small file, then open everything above it.
    app.diff.cursor = 3;
    handle_key(&mut app, key(KeyCode::Char('J')));
    handle_key(&mut app, key(KeyCode::Char('K')));
    handle_key(&mut app, key(KeyCode::Char('J')));
    let _ = frame_text(&mut app);
    handle_key(&mut app, key(KeyCode::Char('a')));
    let _ = frame_text(&mut app);
    let line = app.diff.line.and_then(|abs| app.hits.body_rows.get(abs - app.hits.body_base as usize));
    assert_eq!(app.diff.cursor, app.diff_file_at_cursor().unwrap(), "line sits in the cursor file");
    assert!(line.is_some_and(|l| l.contains("src/")), "line on the file header: {line:?}");
}

#[test]
fn diff_keys_yield_to_a_focused_side_pane() {
    let mut app = expand_app();
    let _ = frame_text(&mut app);
    app.nav.pane = crate::core::Pane::Issues;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_ne!(app.nav.current, Some(9), "j walks the sidebar, not the diff");
}

#[test]
fn expand_all_mode_is_sticky_across_prs() {
    use crate::model::DiffFile;
    let mut app = expand_app();
    let _ = frame_text(&mut app);
    handle_key(&mut app, key(KeyCode::Char('a')));
    app.issues.data.insert(10, diff_pr(10));
    app.show_issue(10);
    app.diff.pending.remove(&10);
    app.diff.files.insert(10, vec![
        DiffFile { filename: "x.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: "@@ -1 +1 @@\n+x".into() },
    ]);
    app.set_tab(Tab::Visual);
    assert!(app.sync_diff());
    assert!(app.diff.expanded_files.contains(&(10, "x.rs".to_string())), "the next PR opens expanded");
    // A file folded by hand stays folded: the mode applies once per view.
    app.diff.expanded_files.clear();
    assert!(!app.sync_diff());
    assert!(app.diff.expanded_files.is_empty());
    // Folding all here folds PR 9 on its next view too.
    app.diff.expanded_files.insert((10, "x.rs".to_string()));
    app.diff.expanded_files.insert((9, "src/a.rs".to_string()));
    app.nav.pane = crate::core::Pane::Main;
    handle_key(&mut app, key(KeyCode::Char('a')));
    app.show_issue(9);
    app.set_tab(Tab::Visual);
    app.sync_diff();
    assert!(app.diff.expanded_files.is_empty());
}

#[test]
fn context_toggle_widens_open_files_from_the_head_side() {
    use crate::model::DiffFile;
    let mut app = expand_app();
    let raw = vec![DiffFile {
        filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1,
        patch: "@@ -4,7 +4,7 @@\n l4\n l5\n l6\n-old\n+new\n l8\n l9\n l10".into(),
    }];
    app.diff.raw.insert(9, raw.clone());
    app.diff.files.insert(9, raw);
    app.diff.expanded_files.insert((9, "src/a.rs".to_string()));
    let _ = frame_text(&mut app);
    // 3 lines is what GitHub sent: no fetch.
    app.sync_diff();
    assert!(app.diff.heads_pending.is_empty());
    handle_key(&mut app, key(KeyCode::Char('x')));
    assert_eq!(app.message, "diff context: 5 lines");
    app.sync_diff();
    assert!(app.diff.heads_pending.contains(&(9, "src/a.rs".to_string())));
    let head: String = (1..=20).map(|i| if i == 7 { "new\n".to_string() } else { format!("l{i}\n") }).collect();
    app.diff.heads_tx.send((9, vec!["src/a.rs".into()], Ok(vec![("src/a.rs".into(), Ok(head))]))).unwrap();
    assert!(crate::app::poll_head_files(&mut app));
    let patch = &app.diff.files[&9][0].patch;
    assert!(patch.starts_with("@@ -2,11 +2,11 @@\n l2\n"), "{patch}");
    let text = frame_text(&mut app);
    assert!(text.contains("l12"), "{text}");
    // Back around to 3 restores the raw patch.
    for _ in 0..4 {
        handle_key(&mut app, key(KeyCode::Char('x')));
    }
    assert_eq!(app.diff.files[&9][0].patch, app.diff.raw[&9][0].patch);
}
