//! Functional business-logic core: pure transitions over plain data.
//!
//! Data-first design: every rule here takes values in and returns values
//! out. No `App`, no channels, no threads, no widgets — so each rule gets a
//! unit test without a TUI. The stateful shells (`app`, `input`) delegate to
//! these functions instead of reimplementing the rules inline.

use std::collections::HashSet;

/// What the command bar text means. Parsing never touches state.
#[derive(Debug, PartialEq, Eq)]
pub enum BarCommand {
    /// One or more issue numbers to load.
    Numbers(Vec<u64>),
    /// Open the current issue in the browser.
    OpenCurrent,
    /// Open an explicit issue number in the browser.
    Open(u64),
    Quit,
    /// Empty bar: dismiss back to the main pane.
    Dismiss,
    /// Anything else: show the hint.
    Hint,
}

/// Parse command-bar text. Mirrors the historical `submit_bar` rules:
/// all-digit words are numbers, `o`/`open` opens, `q` quits.
pub fn parse_bar(text: &str) -> BarCommand {
    let trimmed = text.trim().to_string();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if !parts.is_empty() && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        let numbers: Vec<u64> =
            parts.iter().filter_map(|p| p.parse::<u64>().ok()).collect();
        return BarCommand::Numbers(numbers);
    }
    match trimmed.as_str() {
        "o" | "open" => BarCommand::OpenCurrent,
        "q" | "quit" | "exit" => BarCommand::Quit,
        "" => BarCommand::Dismiss,
        _ if parts.len() == 2
            && (parts[0] == "o" || parts[0] == "open")
            && parts[1].chars().all(|c| c.is_ascii_digit()) =>
        {
            match parts[1].parse::<u64>() {
                Ok(n) => BarCommand::Open(n),
                Err(_) => BarCommand::Hint,
            }
        }
        _ => BarCommand::Hint,
    }
}

/// Next selection walking issues-then-PRs as one flat list, clamped at the
/// ends. `None` when the list is empty.
pub fn next_in_order(order: &[u64], current: Option<u64>, delta: isize) -> Option<u64> {
    if order.is_empty() {
        return None;
    }
    let last = order.len() - 1;
    let next = match current.and_then(|n| order.iter().position(|x| *x == n)) {
        Some(pos) => (pos as isize + delta).clamp(0, last as isize) as usize,
        None => {
            if delta > 0 { 0 } else { last }
        }
    };
    Some(order[next])
}

/// Board cursor after a `(dx, dy)` move, clamped to the grid. An empty
/// column holds row 0.
pub fn board_cursor(
    n_cols: usize,
    rows_in_col: usize,
    col: usize,
    row: usize,
    dx: isize,
    dy: isize,
) -> (usize, usize) {
    if n_cols == 0 {
        return (0, 0);
    }
    let col = (col as isize + dx).clamp(0, n_cols as isize - 1) as usize;
    let row = if rows_in_col == 0 {
        0
    } else {
        (row as isize + dy).clamp(0, rows_in_col as isize - 1) as usize
    };
    (col, row)
}

/// Merged-PR window cycle: 1h → 24h → 7d → 1h.
pub fn cycle_window(hours: u64) -> u64 {
    match hours {
        1 => 24,
        24 => 168,
        _ => 1,
    }
}

/// Sidebar subtitle for the merged window.
pub fn window_label(hours: u64) -> String {
    match hours {
        1 => "open + merged ≤1h".to_string(),
        168 => "open + merged ≤7d".to_string(),
        h => format!("open + merged ≤{h}h"),
    }
}

/// Toggle `n` in the pin list, returning the new list.
pub fn toggle_pin(pins: &[u64], n: u64) -> Vec<u64> {
    let mut out = pins.to_vec();
    if out.contains(&n) {
        out.retain(|x| *x != n);
    } else {
        out.push(n);
    }
    out
}

/// Split the sidebar into pinned | requested | recent. Pins win: a number in
/// the pin order shows only in the pinned group, and a requested number
/// never repeats in recent.
pub fn section_split(
    pin_order: &[u64],
    visited: &[u64],
    recent: &[u64],
    pinned: &HashSet<u64>,
) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let pin_set: HashSet<u64> = pin_order.iter().copied().collect();
    let pinned_sec: Vec<u64> = pin_order.to_vec();
    let visited_sec: Vec<u64> = visited
        .iter()
        .filter(|n| !pin_set.contains(n) && !pinned.contains(n))
        .copied()
        .collect();
    let seen: HashSet<u64> = pin_set.union(pinned).copied().collect::<HashSet<_>>()
        .union(&visited_sec.iter().copied().collect::<HashSet<_>>())
        .copied()
        .collect();
    let recent_sec: Vec<u64> = recent.iter().filter(|n| !seen.contains(n)).copied().collect();
    (pinned_sec, visited_sec, recent_sec)
}

/// Display rows for one pane (Issues or PRs): pinned, then requested, then
/// recent, with a labeled divider slot between non-empty groups. Empty
/// groups leave no divider behind.
pub fn pane_order(pinned: &[u64], requested: &[u64], recent: &[u64]) -> Vec<crate::model::SideRow> {
    use crate::model::SideRow;
    let mut out: Vec<SideRow> = vec![];
    for (label, group) in [("pinned", pinned), ("requested", requested), ("recent", recent)] {
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(SideRow::Divider(label));
        }
        out.extend(group.iter().copied().map(SideRow::Item));
    }
    out
}

/// True when `query` matches the row: a case-insensitive substring of
/// the title, a label name, or the author login, or a digit substring of
/// the number (`#` prefix optional). Empty query matches every row, so a
/// cleared `/` filter restores the full sidebar.
pub fn matches_search(number: u64, title: &str, labels: &[&str], author: &str, query: &str) -> bool {
    let q = query.trim().trim_start_matches('#').to_lowercase();
    if q.is_empty() {
        return true;
    }
    if number.to_string().contains(&q) {
        return true;
    }
    if title.to_lowercase().contains(&q) {
        return true;
    }
    if author.to_lowercase().contains(&q) {
        return true;
    }
    labels.iter().any(|l| l.to_lowercase().contains(&q))
}

/// Collapse whitespace runs to single spaces so yanked text pastes as one
/// line for agents.
pub fn collapse_single_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trim `s` to `limit` chars, appending `…` on cut. Char-boundary safe;
/// matches the `render_toast` width clamp so toasts stay testable.
pub fn toast_snippet(s: &str, limit: usize) -> String {
    let limit = limit.max(1);
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let cut: String = s.chars().take(limit.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Agent-ready issue/PR identity: `…` only enters via the toast, never here.
pub fn yank_issue_text(number: u64, title: &str, url: &str) -> String {
    format!("#{number} {} — {url}", collapse_single_line(title))
}

/// Agent-ready diff context: PR identity plus the file under `diff_cursor`.
/// `first_hunk` is the `@@ … @@` header when the file is expanded.
pub fn yank_diff_text(
    number: u64,
    title: &str,
    url: &str,
    filename: &str,
    status: &str,
    adds: usize,
    dels: usize,
    first_hunk: Option<&str>,
) -> String {
    let hunk = first_hunk
        .map(|h| format!(" {}", collapse_single_line(h)))
        .unwrap_or_default();
    format!(
        "#{number} {} — {filename} ({status} +{adds} -{dels}{hunk}) — {url}",
        collapse_single_line(title)
    )
}

/// Agent-ready diff line: PR identity plus the active patch line.
pub fn yank_diff_line_text(
    number: u64,
    title: &str,
    url: &str,
    filename: &str,
    line: &str,
) -> String {
    format!(
        "#{number} {} — {filename}: \"{}\" — {url}",
        collapse_single_line(title),
        collapse_single_line(line)
    )
}

/// First `@@` hunk header in a unified patch, if any.
pub fn first_hunk(patch: &str) -> Option<String> {
    patch.lines().find(|l| l.starts_with("@@")).map(|l| l.to_string())
}

/// Agent-ready refs context: the raw-body line mentioning `target`.
/// Returns the full yank text, or `None` when no line mentions it.
/// Line numbers are 1-based raw `body` lines, not wrapped render rows.
pub fn yank_ref_text(
    current: u64,
    target: u64,
    body: &str,
    repo: Option<&str>,
    target_title: Option<&str>,
    target_url: Option<&str>,
) -> Option<String> {
    for (idx, line) in body.lines().enumerate() {
        let hits = crate::model::links_in_line(line, repo);
        if hits
            .iter()
            .any(|(_, t)| *t == crate::model::LinkTarget::Issue(target))
        {
            let snippet = toast_snippet(&collapse_single_line(line), 120);
            let mut out = format!("#{current} refs #{target} (body line {}): \"{snippet}\"", idx + 1);
            match (target_title, target_url) {
                (Some(t), Some(u)) if !t.is_empty() && !u.is_empty() => {
                    out.push_str(&format!(" — {} {u}", collapse_single_line(t)));
                }
                (_, Some(u)) if !u.is_empty() => {
                    out.push_str(&format!(" — {u}"));
                }
                _ => {}
            }
            return Some(out);
        }
    }
    None
}

/// Byte ranges of every case-insensitive non-overlapping hit of `query` in
/// `text`, for highlight rendering. Empty query matches nothing.
pub fn match_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let q = query.trim().trim_start_matches('#');
    if q.is_empty() {
        return vec![];
    }
    let lower = text.to_lowercase();
    let needle = q.to_lowercase();
    let mut out = vec![];
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&needle) {
        let start = from + rel;
        let end = start + needle.len();
        // Lowercasing is ASCII-safe for our titles; guard the slice anyway.
        if text.get(start..end).is_some() {
            out.push((start, end));
        }
        from = end;
        if from >= lower.len() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_parses_numbers_and_commands() {
        assert_eq!(parse_bar("12 34"), BarCommand::Numbers(vec![12, 34]));
        assert_eq!(parse_bar("o"), BarCommand::OpenCurrent);
        assert_eq!(parse_bar("open"), BarCommand::OpenCurrent);
        assert_eq!(parse_bar("o 42"), BarCommand::Open(42));
        assert_eq!(parse_bar("q"), BarCommand::Quit);
        assert_eq!(parse_bar("quit"), BarCommand::Quit);
        assert_eq!(parse_bar("exit"), BarCommand::Quit);
        assert_eq!(parse_bar(""), BarCommand::Dismiss);
        assert_eq!(parse_bar("o abc"), BarCommand::Hint);
        assert_eq!(parse_bar("hello"), BarCommand::Hint);
    }

    #[test]
    fn selection_walks_and_clamps() {
        let order = vec![7, 9];
        assert_eq!(next_in_order(&order, Some(7), 1), Some(9));
        assert_eq!(next_in_order(&order, Some(9), 1), Some(9));
        assert_eq!(next_in_order(&order, Some(7), -1), Some(7));
        assert_eq!(next_in_order(&order, None, 1), Some(7));
        assert_eq!(next_in_order(&order, None, -1), Some(9));
        assert_eq!(next_in_order(&[], Some(7), 1), None);
        assert_eq!(next_in_order(&order, Some(999), 1), Some(7));
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
    }

    #[test]
    fn pins_toggle() {
        assert_eq!(toggle_pin(&[7], 9), vec![7, 9]);
        assert_eq!(toggle_pin(&[7, 9], 9), vec![7]);
    }

    #[test]
    fn sections_split_and_dedup() {
        let pinned: HashSet<u64> = [9].into_iter().collect();
        let (p, v, r) = section_split(&[9], &[9, 7], &[7, 5], &pinned);
        assert_eq!(p, vec![9]);
        assert_eq!(v, vec![7]);
        assert_eq!(r, vec![5]);
    }

    #[test]
    fn pane_order_dividers_between_groups_only() {
        use crate::model::SideRow::{Divider, Item};
        assert_eq!(
            pane_order(&[9], &[7], &[5]),
            vec![Item(9), Divider("requested"), Item(7), Divider("recent"), Item(5)]
        );
        assert_eq!(pane_order(&[], &[7], &[]), vec![Item(7)]);
        assert_eq!(pane_order(&[], &[], &[]), Vec::<crate::model::SideRow>::new());
        assert_eq!(pane_order(&[9], &[], &[5]), vec![Item(9), Divider("recent"), Item(5)]);
    }

    #[test]
    fn search_matches_title_or_number() {
        assert!(matches_search(123, "Fix login", &[], "alice", "fix"));
        assert!(matches_search(123, "Fix login", &[], "alice", "FIX"));
        assert!(matches_search(123, "Fix login", &[], "alice", "#23"));
        assert!(matches_search(123, "Fix login", &[], "alice", "12"));
        assert!(!matches_search(123, "Fix login", &[], "alice", "zzz"));
        assert!(matches_search(123, "Fix login", &[], "alice", ""));
    }

    #[test]
    fn search_matches_label_name() {
        let labels = ["bug", "needs-triage"];
        assert!(matches_search(123, "Fix login", &labels, "alice", "bug"));
        assert!(matches_search(123, "Fix login", &labels, "alice", "BUG"));
        assert!(matches_search(123, "Fix login", &labels, "alice", "triage"));
        assert!(!matches_search(123, "Fix login", &labels, "alice", "feature"));
        assert!(!matches_search(123, "Fix login", &[], "alice", "bug"));
    }

    #[test]
    fn search_matches_author_login() {
        assert!(matches_search(123, "Fix login", &[], "octocat", "octo"));
        assert!(matches_search(123, "Fix login", &[], "octocat", "OCTOCAT"));
        assert!(!matches_search(123, "Fix login", &[], "octocat", "monalisa"));
        assert!(!matches_search(123, "Fix login", &[], "", "octo"));
    }

    #[test]
    fn search_empty_query_matches_all() {
        assert!(matches_search(7, "Anything", &["bug"], "alice", ""));
        assert!(matches_search(7, "Anything", &["bug"], "alice", "   "));
        assert!(matches_search(7, "Anything", &[], "", ""));
    }

    #[test]
    fn search_no_match_on_unrelated_query() {
        assert!(!matches_search(123, "Fix login", &["bug"], "alice", "zzz"));
        assert!(!matches_search(123, "Fix login", &["bug"], "alice", "999"));
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
            yank_issue_text(7, "Fix  login\nbug", "https://example.com/7"),
            "#7 Fix login bug — https://example.com/7"
        );
    }

    #[test]
    fn yank_diff_names_file_and_hunk() {
        let text = yank_diff_text(
            9,
            "Add thing",
            "https://example.com/pull/9",
            "src/a.rs",
            "modified",
            10,
            2,
            Some("@@ -1 +1 @@"),
        );
        assert_eq!(
            text,
            "#9 Add thing — src/a.rs (modified +10 -2 @@ -1 +1 @@) — https://example.com/pull/9"
        );
        let folded = yank_diff_text(9, "Add thing", "u", "src/a.rs", "added", 5, 0, None);
        assert_eq!(folded, "#9 Add thing — src/a.rs (added +5 -0) — u");
    }

    #[test]
    fn yank_ref_reports_raw_body_line_and_snippet() {
        let body = "intro line\nsee #9 for details\nlast";
        let text = yank_ref_text(7, 9, body, None, Some("Nine"), Some("u9")).unwrap();
        assert_eq!(text, "#7 refs #9 (body line 2): \"see #9 for details\" — Nine u9");
    }

    #[test]
    fn yank_ref_returns_none_without_mention() {
        assert!(yank_ref_text(7, 9, "no refs here", None, None, None).is_none());
    }

    #[test]
    fn yank_diff_line_quotes_active_line() {
        assert_eq!(
            yank_diff_line_text(9, "Add thing", "u", "src/a.rs", "  +new"),
            "#9 Add thing — src/a.rs: \"+new\" — u"
        );
    }
}
