//! Functional business-logic core: pure transitions over plain data.
//!
//! Data-first design: every rule here takes values in and returns values
//! out. No `App`, no channels, no threads, no widgets — so each rule gets a
//! unit test without a TUI. The stateful shells (`app`, `input`) delegate to
//! these functions instead of reimplementing the rules inline.

use std::collections::{HashMap, HashSet};

use crate::model::IssueData;

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

/// Graph cursor step over the flat node order (center first, then
/// sections), clamped at the ends. `None` when the graph is empty; an
/// out-of-range cursor clamps into the fresh order.
pub fn graph_select(count: usize, current: Option<usize>, delta: isize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let next = match current {
        Some(pos) => (pos as isize + delta).clamp(0, last as isize) as usize,
        None => {
            if delta > 0 { 0 } else { last }
        }
    };
    Some(next.min(last))
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

/// One in-flight fetch the status bar can name, in bar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSource {
    Issue,
    Diff,
    Graph,
    Details,
    Board,
    Sections,
}

/// Plain fetch state the status bar derives from. Counts are pending-set
/// sizes, the bools name live receivers, and progress is `(done, total)`.
/// Plain values only: no `App`, no channels, no widgets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchState {
    pub issues: usize,
    pub diffs: usize,
    pub graphs: usize,
    pub details: usize,
    pub sections_live: bool,
    pub bootstrap_live: bool,
    pub board: bool,
    pub message_visible: bool,
    pub issue_progress: Option<(usize, usize)>,
    pub section_progress: Option<(usize, usize)>,
}

/// One bar segment: what is fetching, plus determinate progress if known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadingItem {
    pub source: LoadSource,
    pub progress: Option<(usize, usize)>,
}

/// Determinate `done/total` text. `None` when the total is unknown
/// (`0/0`) or the work is complete (`done >= total`): the spinner then
/// stays indeterminate, or hides once the poll lands.
pub fn progress_label(done: usize, total: usize) -> Option<String> {
    if total == 0 || done >= total {
        return None;
    }
    Some(format!("{done}/{total}"))
}

/// Spinner input: what is in flight right now. Empty means idle, so the
/// segment collapses. Explicit messages win: a visible message hides the
/// spinner. Progress rides along only while determinate; unknown or
/// complete totals collapse to the plain label.
pub fn loading(s: &FetchState) -> Vec<LoadingItem> {
    if s.message_visible {
        return vec![];
    }
    let mut out = Vec::new();
    if s.issues > 0 {
        out.push(LoadingItem {
            source: LoadSource::Issue,
            progress: s.issue_progress.filter(|(d, t)| progress_label(*d, *t).is_some()),
        });
    }
    if s.diffs > 0 {
        out.push(LoadingItem { source: LoadSource::Diff, progress: None });
    }
    if s.graphs > 0 {
        out.push(LoadingItem { source: LoadSource::Graph, progress: None });
    }
    if s.details > 0 {
        out.push(LoadingItem { source: LoadSource::Details, progress: None });
    }
    if s.board {
        out.push(LoadingItem { source: LoadSource::Board, progress: None });
    }
    if s.sections_live || s.bootstrap_live {
        out.push(LoadingItem {
            source: LoadSource::Sections,
            progress: s.section_progress.filter(|(d, t)| progress_label(*d, *t).is_some()),
        });
    }
    out
}

/// Bar text for the in-flight list, without the spinner glyph: plain
/// labels joined with ` · `, determinate `n/m` where known.
pub fn loading_label(items: &[LoadingItem]) -> String {
    items
        .iter()
        .map(|item| {
            let name = match item.source {
                LoadSource::Issue => "issue",
                LoadSource::Diff => "diff",
                LoadSource::Graph => "graph",
                LoadSource::Details => "details",
                LoadSource::Board => "board",
                LoadSource::Sections => "sections",
            };
            match item.progress.and_then(|(d, t)| progress_label(d, t)) {
                Some(p) => format!("{name} {p}"),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
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

/// Cap for the back/forward stacks so long sessions stay bounded; the
/// oldest entry drops when a new jump lands on a full stack.
pub const HISTORY_LIMIT: usize = 100;

/// Record leaving `current` for `next` on the back stack. No-op when
/// there is nothing to leave (`None`), when `next` is the current
/// issue, or when `current` already tops the stack (async completions
/// must not double-record one jump).
pub fn history_push(back: &[u64], current: Option<u64>, next: u64) -> Vec<u64> {
    let mut out = back.to_vec();
    let Some(cur) = current else { return out };
    if cur == next || out.last() == Some(&cur) {
        return out;
    }
    out.push(cur);
    if out.len() > HISTORY_LIMIT {
        out.remove(0);
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

/// Sidebar sort key. `Default` restores the incoming pinned/requested/
/// recent order; `Updated` sorts by the raw timestamp (newest first by
/// default), `Author`/`State` by login/state, and `Label` by the first
/// label name alphabetically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideSort {
    #[default]
    Default,
    Updated,
    Author,
    State,
    Label,
}

impl SideSort {
    /// One key cycles Default → Updated → Author → State → Label → Default.
    pub fn next(self) -> Self {
        match self {
            SideSort::Default => SideSort::Updated,
            SideSort::Updated => SideSort::Author,
            SideSort::Author => SideSort::State,
            SideSort::State => SideSort::Label,
            SideSort::Label => SideSort::Default,
        }
    }

    /// Direction a key starts in: dates newest first, text A→Z.
    pub fn natural_desc(self) -> bool {
        self == SideSort::Updated
    }

    /// Bar-hint label, e.g. `updated ↓`; `default` has no direction.
    pub fn label(self, desc: bool) -> String {
        match self {
            SideSort::Default => "default".to_string(),
            SideSort::Updated => format!("updated {}", if desc { "↓" } else { "↑" }),
            SideSort::Author => format!("author {}", if desc { "↓" } else { "↑" }),
            SideSort::State => format!("state {}", if desc { "↓" } else { "↑" }),
            SideSort::Label => format!("label {}", if desc { "↓" } else { "↑" }),
        }
    }
}

/// Sort one pane group by the active key. `Default` returns the rows
/// untouched; every other key compares case-folded text with the row
/// number as the tie-break so the order is deterministic. Loading rows
/// (no data yet) keep their relative place at the end.
pub fn sort_rows(
    rows: Vec<u64>,
    sort: SideSort,
    desc: bool,
    data: &HashMap<u64, IssueData>,
) -> Vec<u64> {
    if sort == SideSort::Default {
        return rows;
    }
    let key = |n: u64| -> Option<String> {
        let d = data.get(&n)?;
        Some(match sort {
            SideSort::Updated => d.updated_at.clone(),
            SideSort::Author => d.author.login.to_lowercase(),
            SideSort::State => d.state.to_lowercase(),
            SideSort::Label => d
                .labels
                .iter()
                .map(|l| l.name.to_lowercase())
                .min()
                .unwrap_or_default(),
            SideSort::Default => String::new(),
        })
    };
    let mut keyed: Vec<(Option<String>, u64)> = rows.into_iter().map(|n| (key(n), n)).collect();
    keyed.sort_by(|(ka, a), (kb, b)| match (ka, kb) {
        (Some(a_key), Some(b_key)) => {
            let ord = a_key.cmp(b_key);
            let ord = if desc { ord.reverse() } else { ord };
            ord.then(a.cmp(b))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    });
    keyed.into_iter().map(|(_, n)| n).collect()
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
    fn graph_cursor_steps_and_clamps() {
        assert_eq!(graph_select(4, Some(1), 1), Some(2));
        assert_eq!(graph_select(4, Some(0), -1), Some(0));
        assert_eq!(graph_select(4, Some(3), 1), Some(3));
        assert_eq!(graph_select(4, None, 1), Some(0));
        assert_eq!(graph_select(4, None, -1), Some(3));
        assert_eq!(graph_select(0, Some(0), 1), None);
        // A cursor past a shrunken order clamps instead of panicking.
        assert_eq!(graph_select(2, Some(9), 1), Some(1));
        assert_eq!(graph_select(2, Some(9), -1), Some(1));
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
    fn history_push_records_leaves_and_dedupes() {
        assert_eq!(history_push(&[], None, 7), Vec::<u64>::new());
        assert_eq!(history_push(&[], Some(7), 7), Vec::<u64>::new());
        assert_eq!(history_push(&[], Some(7), 9), vec![7]);
        assert_eq!(history_push(&[7], Some(7), 9), vec![7]);
        assert_eq!(history_push(&[5], Some(7), 9), vec![5, 7]);
    }

    #[test]
    fn history_push_caps_at_limit() {
        let back: Vec<u64> = (1..=HISTORY_LIMIT as u64).collect();
        let out = history_push(&back, Some(999), 1000);
        assert_eq!(out.len(), HISTORY_LIMIT);
        assert_eq!(out[0], 2);
        assert_eq!(out[out.len() - 1], 999);
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
    fn loading_names_each_pending_source() {
        for (state, source) in [
            (FetchState { issues: 1, ..FetchState::default() }, LoadSource::Issue),
            (FetchState { diffs: 2, ..FetchState::default() }, LoadSource::Diff),
            (FetchState { graphs: 1, ..FetchState::default() }, LoadSource::Graph),
            (FetchState { details: 3, ..FetchState::default() }, LoadSource::Details),
            (FetchState { board: true, ..FetchState::default() }, LoadSource::Board),
            (FetchState { sections_live: true, ..FetchState::default() }, LoadSource::Sections),
            (FetchState { bootstrap_live: true, ..FetchState::default() }, LoadSource::Sections),
        ] {
            let got = loading(&state);
            assert_eq!(got.len(), 1, "{state:?}");
            assert_eq!(got[0].source, source, "{state:?}");
            assert_eq!(got[0].progress, None, "{state:?}");
        }
    }

    #[test]
    fn loading_idle_means_empty() {
        assert_eq!(loading(&FetchState::default()), vec![]);
    }

    #[test]
    fn loading_message_wins_over_spinner() {
        let state = FetchState {
            issues: 1,
            diffs: 1,
            board: true,
            sections_live: true,
            message_visible: true,
            ..FetchState::default()
        };
        assert_eq!(loading(&state), vec![]);
    }

    #[test]
    fn progress_label_bounds_unknown_and_complete() {
        assert_eq!(progress_label(0, 0), None, "unknown total stays indeterminate");
        assert_eq!(progress_label(0, 5), Some("0/5".to_string()));
        assert_eq!(progress_label(2, 5), Some("2/5".to_string()));
        assert_eq!(progress_label(5, 5), None, "complete hides the numbers");
        assert_eq!(progress_label(7, 5), None, "over-complete hides too");
    }

    #[test]
    fn loading_carries_determinate_progress_only() {
        let state = FetchState {
            issues: 1,
            issue_progress: Some((2, 7)),
            sections_live: true,
            section_progress: Some((120, 500)),
            ..FetchState::default()
        };
        let got = loading(&state);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].progress, Some((2, 7)));
        assert_eq!(got[1].progress, Some((120, 500)));
        assert_eq!(loading_label(&got), "issue 2/7 · sections 120/500");
        // Unknown (0/0) and complete totals collapse to the plain label.
        let flat = FetchState { issues: 1, issue_progress: Some((0, 0)), ..FetchState::default() };
        assert_eq!(loading(&flat)[0].progress, None);
        assert_eq!(loading_label(&loading(&flat)), "issue");
        let done = FetchState { issues: 1, issue_progress: Some((7, 7)), ..FetchState::default() };
        assert_eq!(loading_label(&loading(&done)), "issue");
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

    fn sort_issue(number: u64, updated: &str, author: &str, state: &str, labels: &[&str]) -> IssueData {
        IssueData {
            number,
            title: format!("issue {number}"),
            state: state.to_string(),
            author: crate::model::Author { login: author.to_string() },
            labels: labels.iter().map(|n| crate::model::Label { name: n.to_string() }).collect(),
            assignees: vec![],
            created_at: String::new(),
            updated_at: updated.to_string(),
            url: String::new(),
            body: String::new(),
            comments: vec![],
            is_pr: false,
            review_threads: vec![],
            paired: vec![],
            checks: None,
        }
    }

    fn sort_data() -> HashMap<u64, IssueData> {
        [
            sort_issue(7, "2026-01-05T00:00:00Z", "carol", "OPEN", &["ui"]),
            sort_issue(9, "2026-01-09T00:00:00Z", "alice", "CLOSED", &["bug"]),
            sort_issue(11, "2026-01-01T00:00:00Z", "bob", "MERGED", &["docs"]),
        ]
        .into_iter()
        .map(|d| (d.number, d))
        .collect()
    }

    #[test]
    fn default_sort_restores_the_incoming_order() {
        let rows = vec![11, 7, 9];
        assert_eq!(sort_rows(rows.clone(), SideSort::Default, false, &sort_data()), rows);
        assert_eq!(sort_rows(rows.clone(), SideSort::Default, true, &sort_data()), rows);
    }

    #[test]
    fn updated_sort_puts_newest_first_and_reverses() {
        let data = sort_data();
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Updated, true, &data), vec![9, 7, 11]);
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Updated, false, &data), vec![11, 7, 9]);
    }

    #[test]
    fn author_sort_orders_by_login() {
        let data = sort_data();
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Author, false, &data), vec![9, 11, 7]);
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Author, true, &data), vec![7, 11, 9]);
    }

    #[test]
    fn state_sort_orders_lexically() {
        let data = sort_data();
        // closed < merged < open once case-folded.
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::State, false, &data), vec![9, 11, 7]);
    }

    #[test]
    fn label_sort_orders_by_first_label_name() {
        let data = sort_data();
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Label, false, &data), vec![9, 11, 7]);
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Label, true, &data), vec![7, 11, 9]);
    }

    #[test]
    fn sort_cycle_ends_back_at_default() {
        let mut sort = SideSort::Default;
        let mut seen = vec![sort];
        for _ in 0..4 {
            sort = sort.next();
            seen.push(sort);
        }
        assert_eq!(
            seen,
            vec![SideSort::Default, SideSort::Updated, SideSort::Author, SideSort::State, SideSort::Label]
        );
        assert_eq!(sort.next(), SideSort::Default);
    }

    #[test]
    fn sort_direction_toggle_flips_key_order() {
        let data = sort_data();
        let asc = sort_rows(vec![7, 9, 11], SideSort::Updated, false, &data);
        let desc = sort_rows(vec![7, 9, 11], SideSort::Updated, true, &data);
        assert_eq!(asc.iter().rev().copied().collect::<Vec<_>>(), desc);
    }

    #[test]
    fn sort_keeps_loading_rows_last_and_labels_cover_directions() {
        let mut data = sort_data();
        data.remove(&11);
        assert_eq!(sort_rows(vec![7, 9, 11], SideSort::Updated, false, &data), vec![7, 9, 11]);
        assert_eq!(sort_rows(vec![11, 9, 7], SideSort::Author, true, &data), vec![7, 9, 11]);
        assert_eq!(SideSort::Default.label(true), "default");
        assert_eq!(SideSort::Updated.label(true), "updated ↓");
        assert_eq!(SideSort::Author.label(false), "author ↑");
        assert_eq!(SideSort::State.label(false), "state ↑");
        assert_eq!(SideSort::Label.label(false), "label ↑");
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
