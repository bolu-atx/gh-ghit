//! Frame tests for the main pane and header.

use crate::app::{App, Tab};
use crate::input::{handle_key, handle_mouse};
use crate::model::PairedRef;
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use super::test_support::{sample, loaded_app, key, click, check_pr, check_app, frame_text};
use crate::model::ItemRef;

/// Masthead rows only (brand line plus the issue line), so masthead
/// assertions cannot pass off sidebar rows as the title source.
fn masthead_text(app: &mut App) -> String {
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, app)).expect("draw");
    let buf = terminal.backend().buffer();
    (0..2)
        .map(|y| (crate::core::SIDEBAR_WIDTH..120).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn style_links_colors_refs_across_span_edges() {
    use crate::theme::{issue_ref, pr_ref, teal};
    use ratatui::text::{Line, Span};
    let mut lines = vec![Line::from(vec![Span::raw("fixes #1"), Span::raw("2 and #5 via https://x.io, #8")])];
    super::main_pane::style_links(&mut lines, None, |n| match n {
        ItemRef {
            number: 12,
            source: crate::model::Source::GitHub,
        } => Some(false),
        ItemRef {
            number: 5,
            source: crate::model::Source::GitHub,
        } => Some(true),
        _ => None,
    });
    let linked: Vec<(&str, _)> = lines[0]
        .spans
        .iter()
        .filter(|s| s.style.fg.is_some())
        .map(|s| (s.content.as_ref(), s.style.fg.unwrap()))
        .collect();
    assert_eq!(
        linked,
        [("#1", issue_ref()), ("2", issue_ref()), ("#5", pr_ref()), ("https://x.io", teal()), ("#8", teal())],
        "issue, PR, URL, and unknown ref each take their color"
    );
    let all: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(all, "fixes #12 and #5 via https://x.io, #8", "text survives the split");
}

#[test]
fn main_shows_loading_while_first_issue_loads() {
    let mut app = App::new(Some("o/r".to_string()));
    app.nav.requested = Some(ItemRef::github(7));
    app.issues.pending.insert(ItemRef::github(7));
    let text = frame_text(&mut app);
    assert!(text.contains("Loading #7…") && text.contains("⣾") && !text.contains("No issue loaded"), "main loading: {text}");
    app.issues.pending.clear();
    assert!(frame_text(&mut app).contains("No issue loaded"), "idle main");
    // A cached issue keeps its text while another one is fetching.
    let mut app = loaded_app();
    app.tick = 0;
    app.nav.requested = Some(ItemRef::github(12));
    app.issues.pending.insert(ItemRef::github(12));
    let text = frame_text(&mut app);
    assert!(text.contains("Fix the thing ⠋"), "main spinner: {text}");
    assert!(!text.contains("Loading #"), "no loading body over cached text: {text}");
}

#[test]
fn jk_in_main_scrolls_instead_of_moving_selection() {
    use crate::core::Pane;
    let mut app = loaded_app();
    app.nav.pane = Pane::Main;
    handle_key(&mut app, key(KeyCode::Char('j')));
    assert_eq!(app.nav.scroll, 3);
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    handle_key(&mut app, key(KeyCode::Char('k')));
    assert_eq!(app.nav.scroll, 0);
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn details_header_shows_clickable_paired_line() {
    let mut app = loaded_app();
    let text = frame_text(&mut app);
    assert!(!text.contains("Paired"), "no line without pairs: {text}");
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().paired = vec![PairedRef {
        number: ItemRef::github(9),
        state: "MERGED".to_string(),
    }];
    app.show_issue(ItemRef::github(7));
    let text = frame_text(&mut app);
    assert!(text.contains("Paired"), "{text}");
    assert!(text.contains("#9 MERGED"), "{text}");
    // Clickable: the line joins header click resolution so `#9`
    // navigates to the counterpart.
    assert!(app.hits.header_rows.iter().any(|l| l.contains("Paired")), "{:?}", app.hits.header_rows);
}

#[test]
fn paired_header_click_opens_counterpart_in_main() {
    use crate::core::Pane;
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().paired = vec![PairedRef {
        number: ItemRef::github(9),
        state: "MERGED".to_string(),
    }];
    app.show_issue(ItemRef::github(7));
    let _ = frame_text(&mut app);
    let idx = app.hits.header_rows.iter().position(|l| l.contains("Paired")).expect("paired row");
    let col = app.hits.header_area.x + app.hits.header_rows[idx].find("#9").expect("paired ref col") as u16;
    let row_y = app.hits.header_area.y + idx as u16;
    handle_mouse(
        &mut app,
        click(MouseEventKind::Down(MouseButton::Left), col, row_y),
    );
    assert_eq!(
        app.nav.current,
        Some(ItemRef::github(9)),
        "clicking the paired header opens the counterpart"
    );
    assert_eq!(
        app.nav.pane,
        Pane::Main,
        "counterpart opens in the main tab"
    );
    assert!(matches!(app.nav.focus, crate::app::Focus::Main));
}

#[test]
fn mouse_click_on_refs_row_highlights_source_instead_of_opening() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body =
        "intro\n\nblocked by #9, see details".to_string();
    app.show_issue(ItemRef::github(7));
    let _ = frame_text(&mut app); // records hit areas
    // +2 skips the border and the breathing-room spacer row.
    let (x, y) = (app.hits.refs_area.x + 2, app.hits.refs_area.y + 2);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)), "stays on the issue");
    assert_eq!(app.refs.highlight, Some(ItemRef::github(9)));
    // Scrolled so the mentioning row sits at the top.
    let row = app.hits.body_rows.iter().position(|l| l.contains("#9")).unwrap();
    assert_eq!(app.nav.scroll, row as u16);
    // Clicking the same row again clears the highlight.
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.refs.highlight, None);
    // Switching issues clears it too.
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.refs.highlight, Some(ItemRef::github(9)));
    // The mentioning line renders highlighted at the top of the pane.
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let buf = terminal.backend().buffer().clone();
    let cell = &buf[(app.hits.body_area.x + 1, app.hits.body_area.y)];
    assert_eq!(cell.bg, crate::theme::amber(), "source line highlighted");
    app.show_issue(ItemRef::github(9));
    assert_eq!(app.refs.highlight, None);
}

#[test]
fn pr_masthead_shows_check_rollup_glyph_per_state() {
    use crate::model::CheckState;
    for (checks, glyph) in [
        (Some(CheckState::Pass), "\u{f00c}"),
        (Some(CheckState::Fail), "\u{f00d}"),
        (Some(CheckState::Pending), "\u{f017}"),
    ] {
        // The masthead title carries the glyph now that the header
        // block starts at the meta lines …
        let mut app = check_app(ItemRef::github(9), checks);
        let mast = masthead_text(&mut app);
        assert!(mast.contains(glyph), "{checks:?} glyph in masthead: {mast}");
        assert!(mast.contains("#9"), "masthead keeps the number: {mast}");
        // … and the header block itself stays clean.
        let d = check_pr(ItemRef::github(9), checks);
        let header: String = super::main_pane::header_lines(&d, &[])
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(
            !header.contains("\u{f00c}") && !header.contains("\u{f00d}") && !header.contains("\u{f017}"),
            "{checks:?} header stays clean: {header}"
        );
        // … and the full frame renders it.
        assert!(frame_text(&mut app).contains(glyph), "{checks:?} glyph in frame");
    }
    // No rollup, an unknown state, or a non-PR renders no masthead glyph.
    for checks in [None, Some(CheckState::Unknown)] {
        let mut app = check_app(ItemRef::github(9), checks);
        let mast = masthead_text(&mut app);
        assert!(
            !mast.contains("\u{f00c}") && !mast.contains("\u{f00d}") && !mast.contains("\u{f017}"),
            "{checks:?} masthead stays clean: {mast}"
        );
    }
    let mut issue = sample(ItemRef::github(7));
    issue.checks = Some(CheckState::Pass);
    let header: String = super::main_pane::header_lines(&issue, &[])
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
        .collect::<Vec<_>>()
        .join("");
    assert!(
        !header.contains("\u{f00c}"),
        "issues never render the rollup: {header}"
    );
}

#[test]
fn mouse_click_switches_tabs() {
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    let (tx, y) = (app.hits.tabs_area.x, app.hits.tabs_area.y);
    handle_mouse(
        &mut app,
        click(MouseEventKind::Down(MouseButton::Left), tx + 14, y),
    );
    assert_eq!(app.nav.tab, Tab::Visual);
    handle_mouse(
        &mut app,
        click(MouseEventKind::Down(MouseButton::Left), tx + 1, y),
    );
    assert_eq!(app.nav.tab, Tab::Details);
}

#[test]
fn mouse_click_on_ref_opens_issue() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body =
        "blocked by #9, see details".to_string();
    let _ = frame_text(&mut app);
    let row = app.hits
        .body_rows
        .iter()
        .position(|l| l.contains("#9"))
        .expect("ref visible");
    let (x, y) = (app.hits.body_area.x + 2, app.hits.body_area.y + row as u16);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
}

#[test]
fn mouse_click_opens_nearest_ref_on_row() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "fix #7 then #9 ok".to_string();
    let _ = frame_text(&mut app);
    let row = app.hits
        .body_rows
        .iter()
        .position(|l| l.contains("#7 then #9"))
        .expect("refs visible");
    let line = app.hits.body_rows[row].clone();
    let (bax, bay) = (app.hits.body_area.x, app.hits.body_area.y);
    let x9 = bax + line.find("#9").unwrap() as u16;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x9, bay + row as u16));
    assert_eq!(app.nav.current, Some(ItemRef::github(9)));
    let x7 = bax + line.find("#7").unwrap() as u16;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x7, bay + row as u16));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn mouse_click_on_url_opens_browser_not_issue() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body =
        "see https://example.com/a ok".to_string();
    let _ = frame_text(&mut app);
    let row = app.hits
        .body_rows
        .iter()
        .position(|l| l.contains("example.com"))
        .expect("url visible");
    let (x, y) = (app.hits.body_area.x + 2, app.hits.body_area.y + row as u16);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
    assert!(app.message.contains("opened https://example.com/a"));
}

#[test]
fn mouse_click_on_plain_text_keeps_issue() {
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    let row = app.hits
        .body_rows
        .iter()
        .position(|l| l.contains("hello"))
        .expect("body visible");
    let (x, y) = (app.hits.body_area.x + 2, app.hits.body_area.y + row as u16);
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
    assert_eq!(app.nav.current, Some(ItemRef::github(7)));
}

#[test]
fn mouse_wheel_scrolls() {
    let mut app = loaded_app();
    handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 60, 20));
    assert_eq!(app.nav.scroll, 3);
    handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 60, 20));
    assert_eq!(app.nav.scroll, 0);
}

#[test]
fn desk_renders_issue() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let d = sample(ItemRef::github(7));
    app.issues.data.insert(ItemRef::github(7), d);
    app.issues.statuses.insert(ItemRef::github(7), vec![]);
    app.nav.visited.push(ItemRef::github(7));
    app.show_issue(ItemRef::github(7));
    let text = frame_text(&mut app);
    assert!(text.contains("ghit"), "masthead wordmark");
    assert!(text.contains("#7"), "issue number");
    assert!(text.contains("Body"), "tab label");
    assert!(text.contains("hello"), "markdown body");
}

#[test]
fn masthead_splits_brand_and_issue_lines() {
    let mut app = loaded_app();
    let mast = masthead_text(&mut app);
    let mut lines = mast.lines();
    let brand = lines.next().unwrap_or_default();
    let issue = lines.next().unwrap_or_default();
    assert!(brand.contains("ghit"), "brand line keeps the wordmark: {brand}");
    assert!(!brand.contains("#7"), "number leaves the brand line: {brand}");
    assert!(issue.contains("#7"), "issue line keeps the number: {issue}");
}

#[test]
fn main_pane_keeps_a_single_title_line_with_hint() {
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().title = "Quasar title nine".to_string();
    let text = frame_text(&mut app);
    // Masthead plus the one sidebar row; the header block starts at
    // the meta lines and no longer repeats the title.
    assert_eq!(text.matches("Quasar title nine").count(), 2, "single main-pane title: {text}");
    assert!(
        !app.hits.header_rows.iter().any(|l| l.contains("Quasar title nine")),
        "header starts at meta: {:?}",
        app.hits.header_rows
    );
    assert!(text.contains("x evict row") && text.contains("? shortcuts · q quit"), "hints: {text}");
}

#[test]
fn desk_renders_empty() {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    let text = frame_text(&mut app);
    assert!(text.contains("ghit"));
    assert!(text.contains("No issue loaded"));
}

#[test]
fn details_contains_body_before_comments() {
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.body = "body evidence".into();
    d.comments.push(crate::model::Comment { body: "comment evidence".into(), ..Default::default() });
    let text = frame_text(&mut app);
    assert!(text.find("Body").unwrap() < text.find("body evidence").unwrap());
    assert!(text.find("body evidence").unwrap() < text.find("Comments").unwrap());
    assert!(text.find("Comments").unwrap() < text.find("comment evidence").unwrap());
}

#[test]
fn content_tabs_show_nerd_icons() {
    let mut app = loaded_app();
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f15c}'), "{text}");
    assert!(text.contains('\u{f121}'), "{text}");
    assert!(text.contains('\u{f279}'), "{text}");
}

#[test]
fn plain_mode_frame_has_no_nerd_glyphs() {
    crate::theme::set_nerd(false);
    let mut app = loaded_app();
    let text = frame_text(&mut app);
    let pua = text.chars().find(|c| ('\u{e000}'..='\u{f8ff}').contains(c));
    assert_eq!(pua, None, "{text}");
    assert!(text.contains("1 Details") && text.contains("3 Map"), "{text}");
    assert!(text.contains("Issues"), "{text}");
    assert!(!text.contains("  Issues"), "no stray double space: {text}");
}

#[test]
fn content_tabs_show_shortcut_numbers() {
    // The titles stay the same for issues, PRs and no open item.
    let mut app = loaded_app();
    for number in [7, 9] {
        app.show_issue(ItemRef::github(number));
        let text = frame_text(&mut app);
        assert!(text.contains("1 Details"), "details tab names its key: {text}");
        assert!(text.contains("2 Diff"), "diff tab names its key: {text}");
        assert!(text.contains("3 Map"), "map tab names its key: {text}");
    }
    let mut empty = App::new(Some("o/r".to_string()));
    let text = frame_text(&mut empty);
    assert!(text.contains("1 Details") && text.contains("3 Map"), "{text}");
}

#[test]
fn tab_click_zones_match_wider_labels() {
    // Glyph plus two spaces plus `1 Details` spans rel 0..12, then a
    // one-cell divider: rel 12 still selects Details, rel 13 lands on
    // `2 Diff` (13..22), rel 23 on `3 Map`. The split follows `tab_at`.
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    let (tx, y) = (app.hits.tabs_area.x, app.hits.tabs_area.y);
    for (rel, tab) in [
        (8, Tab::Details), (12, Tab::Details), (13, Tab::Visual), (21, Tab::Visual),
        (22, Tab::Visual), (23, Tab::Map), (30, Tab::Map), (8, Tab::Details),
    ] {
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + rel, y));
        assert_eq!(app.nav.tab, tab, "click at rel {rel}");
    }
}

#[test]
fn masthead_wordmark_shows_git_icon() {
    let mut app = loaded_app();
    let text = frame_text(&mut app);
    assert!(text.contains('\u{f1d3}'), "{text}");
    let brand = format!("ghit v{}", env!("CARGO_PKG_VERSION"));
    assert!(text.contains(&brand), "version follows the wordmark: {text}");
    assert!(text.contains("#7"));
}

#[test]
fn issue_header_renders_above_tabs_not_in_body() {
    let mut app = loaded_app();
    let _ = frame_text(&mut app);
    assert!(!app.hits.header_rows.is_empty());
    // The header block starts at the meta lines: state and author
    // render there, while the `#N title` lives only in the masthead.
    assert!(app.hits.header_rows.iter().any(|l| l.contains("OPEN")));
    assert!(!app.hits.header_rows.iter().any(|l| l.contains("#7")));
    assert!(app.hits.header_area.y + app.hits.header_area.height <= app.hits.tabs_area.y);
    // The meta/label block left the scrollable body: meta text like
    // the state and assignees no longer appear in body rows.
    assert!(!app.hits.body_rows.iter().any(|l| l.contains("assignees:")), "{:?}", app.hits.body_rows);
    assert!(!app.hits.body_rows.iter().any(|l| l.contains("OPEN")), "{:?}", app.hits.body_rows);
}

#[test]
fn header_url_click_opens() {
    // The title left the header block, so header clicks resolve
    // through the meta lines now: the issue URL line opens its link.
    // (The test `open_url` stub notifies instead of opening a browser.)
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().url =
        "https://example.com/note".to_string();
    let _ = frame_text(&mut app);
    let idx = app.hits.header_rows.iter().position(|l| l.contains("example.com")).expect("url row");
    let col = app.hits.header_area.x + app.hits.header_rows[idx].find("example.com").expect("url col") as u16;
    let row_y = app.hits.header_area.y + idx as u16;
    handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), col, row_y));
    assert!(app.message.contains("opened https://example.com/note"), "{}", app.message);
}

#[test]
fn narrow_header_wraps_without_clipping_labels() {
    use crate::model::Label;
    let mut app = loaded_app();
    let d = app.issues.data.get_mut(&ItemRef::github(7)).unwrap();
    d.labels = vec![Label { name: "modernization".to_string() }, Label { name: "new-modernization".to_string() }];
    let backend = TestBackend::new(70, 40);
    let mut terminal = Terminal::new(backend).expect("test backend");
    terminal.draw(|f| crate::app::draw_frame(f, &mut app)).expect("draw");
    let text: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(text.contains("modernization"), "{text}");
    assert!(text.contains("new-modernization"), "{text}");
    // The long URL hard-splits across wrapped rows; both halves render.
    assert!(text.contains("https://github.com/o/"), "{text}");
    assert!(text.contains("r/issues/7"), "{text}");
    // One logical meta line became several wrapped rows.
    assert!(app.hits.header_rows.len() > 3, "{:?}", app.hits.header_rows);
}

#[test]
fn yank_text_covers_issue_diff_and_refs() {
    use crate::core::Pane;
    use crate::model::DiffFile;
    // Issue identity.
    let app = loaded_app();
    assert_eq!(
        app.yank_text().unwrap(),
        "#7 Fix the thing — https://github.com/o/r/issues/7"
    );
    // Diff file context follows the file cursor, folded by default.
    let mut app = loaded_app();
    app.show_issue(ItemRef::github(9));
    app.diff.files.insert(ItemRef::github(9), vec![
        DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
    ]);
    app.set_tab(crate::app::Tab::Visual);
    assert_eq!(
        app.yank_text().unwrap(),
        "#9 Fix the thing — src/a.rs (modified +1 -1) — https://github.com/o/r/issues/9"
    );
    // Expanded files add the hunk header for agents.
    app.diff.expanded_files.insert((ItemRef::github(9), "src/a.rs".to_string()));
    assert!(app.yank_text().unwrap().contains("@@ -1 +1 @@"), "{}", app.yank_text().unwrap());
    // Refs context names the raw body line and the ticket.
    let mut app = loaded_app();
    app.issues.data.get_mut(&ItemRef::github(7)).unwrap().body = "intro\nsee #9 here\n".to_string();
    app.refs.numbers = vec![ItemRef::github(9)];
    app.nav.pane = Pane::Refs;
    app.refs.highlight = Some(ItemRef::github(9));
    let text = app.yank_text().unwrap();
    assert!(text.contains("#7 refs #9 (body line 2)"), "{text}");
    assert!(text.contains("https://github.com/o/r/issues/9"), "{text}");
    // No selection to yank.
    let mut app = loaded_app();
    app.nav.pane = Pane::Refs;
    app.refs.highlight = None;
    assert!(app.yank_text().is_none());
}

#[test]
fn yank_key_and_empty_state_toast() {
    // `y` reaches `yank`: an empty viewer explains instead of copying.
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    handle_key(&mut app, key(KeyCode::Char('y')));
    assert_eq!(app.message, "view an issue first");
    // A long yank toast still renders with the short hints intact.
    let mut app = loaded_app();
    app.notify(format!(
        "yanked {}",
        crate::core::toast_snippet("#7 refs #9 (body line 2): \"see #9 here\" — https://github.com/o/r/issues/9", 80)
    ));
    let text = frame_text(&mut app);
    assert!(text.contains("yanked #7"), "{text}");
    assert!(text.contains("? shortcuts · q quit"), "status keeps the hint");
    assert!(!text.contains("open board"), "context hints drop under a long toast: {text}");
}

#[test]
fn number_keys_switch_content_tabs() {
    let mut app = loaded_app();
    handle_key(&mut app, key(KeyCode::Char('4')));
    assert_eq!(app.nav.tab, Tab::Details);
    handle_key(&mut app, key(KeyCode::Char('1')));
    assert_eq!(app.nav.tab, Tab::Details);
    handle_key(&mut app, key(KeyCode::Char('2')));
    assert_eq!(app.nav.tab, Tab::Visual);
    handle_key(&mut app, key(KeyCode::Char('3')));
    assert_eq!(app.nav.tab, Tab::Map);
    // The map key works from a sidebar pane and with no item open.
    let mut empty = App::new(Some("o/r".to_string()));
    assert_ne!(empty.nav.pane, crate::core::Pane::Main);
    handle_key(&mut empty, key(KeyCode::Char('3')));
    assert_eq!(empty.nav.tab, Tab::Map);
}

#[test]
fn loading_body_spinner_advances_with_the_tick() {
    let mut app = App::new(Some("o/r".to_string()));
    app.nav.requested = Some(ItemRef::github(7));
    app.issues.pending.insert(ItemRef::github(7));
    app.tick = 0;
    let first = frame_text(&mut app);
    app.tick = 1;
    let second = frame_text(&mut app);
    assert!(first.contains("⣾ Loading #7…"), "tick 0: {first}");
    assert!(second.contains("⣽ Loading #7…"), "tick 1: {second}");
}
