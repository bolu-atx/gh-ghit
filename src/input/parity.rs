//! Key parity: drives a wide key set through every focus context and
//! compares the state changes with a golden file. The golden file pins
//! what `handle_key` did before the keymap registry took over, so the
//! registry must reproduce it. Set `GHIT_BLESS=1` to rewrite the file.

use std::fmt::Write as _;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Focus, Tab};
use crate::core::Pane;
use crate::input::handle_key;
use crate::model::{
    Author, GraphEdgeKind, GraphNode, IssueData, SubSummary, build_graph,
};

const GOLDEN: &str = include_str!("key_parity.golden");

/// Keys that touch the network, the clipboard or disk. The keymap
/// tests check their routing instead. `t` cycles the process-wide
/// theme, which would race the frame tests.
const SKIP: [char; 5] = ['y', 'p', 'r', 'w', 't'];

fn issue(number: u64, is_pr: bool) -> IssueData {
    IssueData {
        number,
        title: format!("Item {number}"),
        state: "OPEN".to_string(),
        author: Author { login: "bo".to_string() },
        labels: vec![],
        assignees: vec![],
        created_at: "2026-01-02T00:00:00Z".to_string(),
        updated_at: "2026-01-05T00:00:00Z".to_string(),
        url: format!("https://github.com/o/r/issues/{number}"),
        body: format!("see #{}", number + 100),
        comments: vec![],
        review_threads: vec![],
        paired: vec![],
        checks: None,
        milestone: None,
        parent: None,
        sub_summary: None,
        is_pr,
    }
}

fn base() -> App {
    let mut app = App::new(Some("o/r".to_string()));
    app.repo_slug = Some("o/r".to_string());
    for (n, pr) in [(7u64, false), (8, false), (9, true), (10, true)] {
        app.issues.data.insert(n, issue(n, pr));
        app.issues.statuses.insert(n, vec![]);
        app.nav.visited.push(n);
    }
    app.show_issue(8);
    app
}

fn pane(pane: Pane) -> App {
    let mut app = base();
    app.nav.pane = pane;
    app
}

fn tree() -> App {
    let mut app = pane(Pane::Issues);
    app.issues.data.get_mut(&7).unwrap().sub_summary = Some(SubSummary { total: 2, completed: 0 });
    app.issues.data.get_mut(&8).unwrap().parent = Some(7);
    app.nav.recent = vec![7, 8];
    app.nav.visited.clear();
    app.sidebar.sub_issue_tree = true;
    app.show_issue(7);
    app
}

fn diff() -> App {
    let mut app = base();
    app.show_issue(9);
    app.nav.pane = Pane::Main;
    app.nav.tab = Tab::Visual;
    app.hits.body_area = ratatui::layout::Rect::new(30, 0, 60, 10);
    app.hits.body_rows = (0..10).map(|i| format!("line {i}")).collect();
    app.diff.headers = vec![0];
    app
}

fn graph() -> App {
    let mut app = base();
    app.show_issue(7);
    app.nav.pane = Pane::Main;
    app.nav.tab = Tab::Visual;
    let node = |number: u64| GraphNode {
        number,
        title: format!("Node {number}"),
        state: "OPEN".to_string(),
        is_pr: false,
        labels: vec![],
        project_status: None,
        linked_prs: vec![],
    };
    let data = build_graph(
        node(7),
        vec![(node(8), GraphEdgeKind::Blocks), (node(11), GraphEdgeKind::BlockedBy)],
    );
    app.graph.data.insert(7, data);
    app
}

fn refs() -> App {
    let mut app = pane(Pane::Refs);
    app.refs.numbers = vec![7, 9];
    app
}

fn labels() -> App {
    let mut app = pane(Pane::Labels);
    app.sidebar.labels = vec!["bug".to_string(), "ui".to_string()];
    app
}

fn history() -> App {
    let mut app = pane(Pane::History);
    app.ensure_visited(7);
    app.load_issue(7, false);
    app.ensure_visited(9);
    app.load_issue(9, false);
    app.nav.pane = Pane::History;
    app
}

fn shortcuts() -> App {
    let mut app = base();
    app.open(crate::app::Entry::Shortcuts);
    app
}

fn compose() -> App {
    let mut app = base();
    app.open_compose();
    for c in "hello world".chars() {
        app.compose_insert(c);
    }
    app.compose_insert('\n');
    app.compose_insert('x');
    app.compose_step(-3);
    app
}

fn confirm() -> App {
    let mut app = compose();
    app.request_discard_compose();
    app
}

fn format() -> App {
    let mut app = base();
    app.open_sidebar_format();
    app
}

fn query() -> App {
    let mut app = base();
    app.open_sidebar_format();
    app.start_query_edit();
    app
}

fn name() -> App {
    let mut app = base();
    app.open_sidebar_format();
    app.set_format_edit(crate::app::FormatEdit::Name);
    app
}

fn board() -> App {
    let mut app = base();
    let data: crate::model::BoardData = serde_json::from_value(serde_json::json!({
        "title": "B",
        "order": ["Todo", "Done"],
        "items": [
            {"title": "a", "number": 7, "status": "Todo"},
            {"title": "b", "number": 8, "status": "Todo"},
            {"title": "c", "number": 9, "status": "Done"},
        ],
        "url": "https://github.com/orgs/o/projects/1",
    }))
    .expect("board json");
    app.board.data = Some(data);
    app.open(crate::app::Entry::Board);
    app.board.states = vec![Default::default(), Default::default()];
    app
}

fn board_filter() -> App {
    let mut app = board();
    app.open_board_filter();
    app
}

fn board_filter_row() -> App {
    let mut app = board_filter();
    app.board.filter_focus = 1;
    app
}

fn board_query() -> App {
    let mut app = board_filter();
    app.set_format_edit(crate::app::FormatEdit::Query);
    app
}

fn bar() -> App {
    let mut app = base();
    app.nav.focus = Focus::Bar;
    app.bar = "ab".to_string();
    app
}

fn search_bar() -> App {
    let mut app = bar();
    app.sidebar.searching = true;
    app.sidebar.search = "ab".to_string();
    app
}

type Setup = (&'static str, fn() -> App);

fn contexts() -> Vec<Setup> {
    vec![
        ("issues", || pane(Pane::Issues)),
        ("issues-tree", tree),
        ("prs", || pane(Pane::PRs)),
        ("main-details", || pane(Pane::Main)),
        ("main-diff", diff),
        ("main-graph", graph),
        ("refs", refs),
        ("labels", labels),
        ("history", history),
        ("shortcuts", shortcuts),
        ("compose", compose),
        ("confirm", confirm),
        ("format", format),
        ("format-query", query),
        ("format-name", name),
        ("board", board),
        ("board-filter", board_filter),
        ("board-filter-row", board_filter_row),
        ("board-query", board_query),
        ("bar", bar),
        ("search-bar", search_bar),
    ]
}

fn keys() -> Vec<(String, KeyEvent)> {
    let mut out = vec![];
    for c in ' '..='~' {
        if SKIP.contains(&c) {
            continue;
        }
        let mods = if c.is_ascii_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
        out.push((format!("{c:?}"), KeyEvent::new(KeyCode::Char(c), mods)));
    }
    for c in 'a'..='z' {
        if SKIP.contains(&c) {
            continue;
        }
        out.push((format!("ctrl-{c}"), KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
        out.push((format!("alt-{c}"), KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)));
    }
    out.push(("ctrl-S".into(), KeyEvent::new(KeyCode::Char('S'), KeyModifiers::CONTROL)));
    out.push(("alt-F".into(), KeyEvent::new(KeyCode::Char('F'), KeyModifiers::ALT)));
    out.push(("ctrl-alt-o".into(), KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL | KeyModifiers::ALT)));
    out.push(("ctrl-alt-s".into(), KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL | KeyModifiers::ALT)));
    let specials = [
        ("enter", KeyCode::Enter), ("esc", KeyCode::Esc), ("tab", KeyCode::Tab),
        ("backtab", KeyCode::BackTab), ("backspace", KeyCode::Backspace),
        ("delete", KeyCode::Delete), ("home", KeyCode::Home), ("end", KeyCode::End),
        ("up", KeyCode::Up), ("down", KeyCode::Down), ("left", KeyCode::Left),
        ("right", KeyCode::Right), ("pageup", KeyCode::PageUp), ("pagedown", KeyCode::PageDown),
        ("f1", KeyCode::F(1)),
    ];
    for (name, code) in specials {
        out.push((name.to_string(), KeyEvent::new(code, KeyModifiers::NONE)));
    }
    for (name, code) in [("tab", KeyCode::Tab), ("enter", KeyCode::Enter), ("backspace", KeyCode::Backspace)] {
        out.push((format!("ctrl-{name}"), KeyEvent::new(code, KeyModifiers::CONTROL)));
        out.push((format!("alt-{name}"), KeyEvent::new(code, KeyModifiers::ALT)));
    }
    out
}

/// Observable state, one `name=value` pair per concern.
fn signature(app: &App) -> Vec<(&'static str, String)> {
    fn sorted<T: Ord + std::fmt::Debug + Clone>(set: &std::collections::HashSet<T>) -> String {
        let mut items: Vec<T> = set.iter().cloned().collect();
        items.sort();
        format!("{items:?}")
    }
    let message = if app.message.starts_with("theme: ") { "theme: *".to_string() } else { app.message.clone() };
    let draft = app.draft().map(|d| (d.body.clone(), d.cursor, d.kill.clone()));
    let confirm = app.confirm().map(|d| (d.title.clone(), d.choices.len()));
    vec![
        ("pane", format!("{:?}", app.nav.pane)),
        ("tab", format!("{:?}", app.nav.tab)),
        ("focus", (app.nav.focus == Focus::Bar).to_string()),
        ("current", format!("{:?}", app.nav.current)),
        ("requested", format!("{:?}", app.nav.requested)),
        ("scroll", app.nav.scroll.to_string()),
        ("history", format!("{:?}/{:?}/{}", app.nav.history, app.nav.future, app.nav.history_cursor)),
        ("shortcuts", app.shortcuts_open().to_string()),
        ("quit", app.quit.to_string()),
        ("message", message),
        ("bar", format!("{:?}", app.bar)),
        ("search", format!("{:?}/{}", app.sidebar.search, app.sidebar.searching)),
        ("format", format!("{}/{}/{}", app.format_open(), app.query_edit(), app.name_edit())),
        ("drafts", format!("{:?}/{:?}", app.sidebar.query_draft, app.sidebar.name_draft)),
        ("format-focus", app.sidebar.format_focus.to_string()),
        ("show", format!("{}{}{}{}{}", app.sidebar.show_open_prs, app.sidebar.show_merged_prs,
            app.sidebar.show_open_issues, app.sidebar.show_closed_issues, app.sidebar.show)),
        ("sort", format!("{:?}/{}/{:?}", app.sidebar.sort, app.sidebar.sort_desc, app.sidebar.group_by)),
        ("folded", sorted(&app.sidebar.folded_issues)),
        ("filters", format!("{}/{}", sorted(&app.sidebar.label_filter), app.sidebar.milestone_filter.len())),
        ("window", app.sections.merged_window_hours.to_string()),
        ("refs", format!("{}/{:?}", app.refs.show, app.refs.highlight)),
        ("labels-cursor", app.labels.cursor.to_string()),
        ("compose", format!("{draft:?}/{}", app.compose.editor_requested)),
        ("confirm", format!("{confirm:?}")),
        ("board", format!("{}/{}/{}/{:?}/{}", app.board.data.is_some(), app.board.col, app.board.row,
            app.board.expanded, app.board.filter_focus)),
        ("board-filter", format!("{:?}/{}", app.board.filter, sorted(&app.board.milestone_draft.iter().cloned().collect()))),
        ("diff", format!("{:?}/{}/{}/{}", app.diff.line, app.diff.touched, app.diff.split, sorted(&app.diff.expanded_rows))),
        ("graph", app.graph.cursor.to_string()),
        ("pending", format!("{:?}", app.pending_prefix)),
        ("undo", app.undo.is_some().to_string()),
        ("reauth", app.auth.reauth_requested.to_string()),
    ]
}

/// Two-key sequences, sent one after the other.
fn sequences() -> Vec<(String, Vec<KeyEvent>)> {
    let plain = |c: char| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
    vec![
        ("g g".to_string(), vec![plain('g'), plain('g')]),
        ("g j".to_string(), vec![plain('g'), plain('j')]),
    ]
}

fn render() -> String {
    let mut out = String::new();
    let singles = keys().into_iter().map(|(name, key)| (name, vec![key]));
    let all: Vec<(String, Vec<KeyEvent>)> = singles.chain(sequences()).collect();
    for (context, setup) in contexts() {
        for (name, presses) in &all {
            let mut app = setup();
            let before = signature(&app);
            for key in presses {
                handle_key(&mut app, *key);
            }
            let after = signature(&app);
            let changes: Vec<String> = before
                .iter()
                .zip(&after)
                .filter(|(b, a)| b.1 != a.1)
                .map(|(b, a)| format!("{}: {} -> {}", b.0, b.1, a.1))
                .collect();
            if !changes.is_empty() {
                let _ = writeln!(out, "{context} | {name} | {}", changes.join(" ; "));
            }
        }
    }
    out
}

#[test]
fn every_context_matches_the_golden_key_log() {
    let actual = render();
    if std::env::var_os("GHIT_BLESS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/input/key_parity.golden");
        std::fs::write(path, &actual).expect("write golden");
        return;
    }
    if actual != GOLDEN {
        let diff: Vec<String> = actual
            .lines()
            .filter(|line| !GOLDEN.lines().any(|g| g == *line))
            .map(|line| format!("+ {line}"))
            .chain(GOLDEN.lines().filter(|g| !actual.lines().any(|line| line == *g)).map(|g| format!("- {g}")))
            .take(60)
            .collect();
        panic!("key parity drifted ({} lines):\n{}", diff.len(), diff.join("\n"));
    }
}
