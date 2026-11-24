//! Functional business-logic core: pure transitions over plain data.
//!
//! Data-first design: every rule here takes values in and returns values
//! out. No `App`, no channels, no threads, no widgets — so each rule gets a
//! unit test without a TUI. The stateful shells (`app`, `input`) delegate to
//! these functions instead of reimplementing the rules inline.

mod focus;
mod fuzzy;
mod layout;
pub mod map;
pub(crate) mod query;
mod search;
mod tree;
mod urgency;
mod wrap;

pub use focus::{
    Dialog, Layer, Pane, PaneVisibility, TextInput, step_pane,
    visible_panes,
};
pub use fuzzy::score as fuzzy_score;
pub use layout::{MAX_DIFF_FILES, SMALL_DIFF_LINES, RIGHT_WIDTH, SIDEBAR_WIDTH, columns};
#[cfg(any(test, feature = "test-support"))]
pub use layout::MIN_MAIN_WIDTH;
pub use query::{Arms, LocalQuery};
pub use search::find_ranges;
pub use tree::{needs_lazy_children, tree_rows};
pub use urgency::{Kind, Tier, urgency};
pub use wrap::{cursor_pos, index_at, starts_line, text_rows};

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{DiffFile, IssueData};
use crate::model::ItemRef;

/// Default merged/closed window: the last 24 hours.
pub const DEFAULT_WINDOW_HOURS: u64 = 24;

/// What the command bar text means. Parsing never touches state.
#[derive(Debug, PartialEq, Eq)]
pub enum BarCommand {
    /// One or more issue numbers to load.
    Numbers(Vec<ItemRef>),
    /// Open the current issue in the browser.
    OpenCurrent,
    /// Open an explicit issue number in the browser.
    Open(ItemRef),
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
        let numbers: Vec<ItemRef> = parts
            .iter()
            .filter_map(|p| p.parse::<u64>().ok().map(ItemRef::github))
            .collect();
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
                Ok(n) => BarCommand::Open(ItemRef::github(n)),
                Err(_) => BarCommand::Hint,
            }
        }
        _ => BarCommand::Hint,
    }
}

/// Next selection walking issues-then-PRs as one flat list, clamped at the
/// ends. `None` when the list is empty.
pub fn next_in_order(order: &[ItemRef], current: Option<ItemRef>, delta: isize) -> Option<ItemRef> {
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

/// Sidebar subtitle for the merged window. Zero means all time.
pub fn window_label(hours: u64) -> String {
    match hours {
        1 => "open + merged ≤1h".to_string(),
        168 => "open + merged ≤7d".to_string(),
        0 => "open + merged all time".to_string(),
        h => format!("open + merged ≤{h}h"),
    }
}

/// Sidebar loading body: what the pending fetch covers, from the open/done
/// toggles, the window, and the active query. `pr` picks "merged" over
/// "closed". Zero hours is all time.
pub fn pane_loading_label(pr: bool, show_open: bool, show_done: bool, hours: u64, query: &str) -> String {
    let noun = if pr { "pull requests" } else { "issues" };
    let query = query.trim();
    if !query.is_empty() {
        return format!("Loading {noun} for \u{201c}{query}\u{201d}\u{2026}");
    }
    let done = if pr { "merged" } else { "closed" };
    let span = match hours {
        0 => format!("all {done}"),
        168 => format!("last 7d {done}"),
        h => format!("last {h}h {done}"),
    };
    let scope = match (show_open, show_done) {
        (true, true) => format!("open + {span} "),
        (true, false) => "open ".to_string(),
        (false, true) => format!("{span} "),
        (false, false) => String::new(),
    };
    format!("Loading {scope}{noun}\u{2026}")
}

/// `updated:>=` token for a recency window. Zero hours is all time and
/// owns no token. Dates clamp to a decade back so absurd prefs never
/// overflow the date arithmetic.
pub fn window_token(hours: u64, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    if hours == 0 {
        return None;
    }
    let hours = hours.min(24 * 365 * 10) as i64;
    let date = (now - chrono::Duration::hours(hours)).date_naive();
    Some(format!("updated:>={date}"))
}

/// Server query for the search path: the user text plus the window's
/// `updated:` token. All time (zero hours) adds no token. The token is
/// computed when the fetch runs, so the query text itself stays exactly
/// what the user typed.
pub fn search_query(query: &str, hours: u64, now: chrono::DateTime<chrono::Utc>) -> String {
    let query = query.trim();
    match window_token(hours, now) {
        Some(token) if !query.is_empty() => format!("{query} {token}"),
        _ => query.to_string(),
    }
}

/// One in-flight fetch the status bar can name, in bar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSource {
    Issue,
    Diff,
    Details,
    Board,
    Sections,
    Backfill,
}

/// Plain fetch state the status bar derives from. Counts are pending-set
/// sizes, the bools name live receivers, and progress is `(done, total)`.
/// Plain values only: no `App`, no channels, no widgets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchState {
    pub issues: usize,
    pub diffs: usize,
    pub details: usize,
    pub sections_live: bool,
    pub bootstrap_live: bool,
    pub board: bool,
    pub message_visible: bool,
    pub issue_progress: Option<(usize, usize)>,
    pub section_progress: Option<(usize, usize)>,
    /// The store backfill runs, with `(fetched, total)` once known.
    pub backfill: bool,
    pub backfill_progress: Option<(usize, usize)>,
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
    if s.backfill {
        out.push(LoadingItem {
            source: LoadSource::Backfill,
            progress: s.backfill_progress.filter(|(d, t)| progress_label(*d, *t).is_some()),
        });
    }
    out
}

/// Plain inputs for per-pane loading: what is in flight and whether the
/// pane already has something to show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneInputs {
    pub sections_live: bool,
    pub bootstrap_live: bool,
    pub requested_pending: bool,
    pub shown_cached: bool,
    pub refs_pending: bool,
}

/// Which panes show their own spinner or loading body right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaneLoading {
    /// Sidebar title spinner.
    pub sidebar_spinner: bool,
    /// Sidebar loading body.
    pub sidebar: bool,
    pub main: bool,
    pub main_spinner: bool,
    pub refs: bool,
}

/// Pane-level loading. The main loading body only shows while nothing is
/// cached for the pane; a cached issue keeps its text during a refresh.
/// Only the most urgent loading tier shows its full cue: while the
/// current issue loads, the sidebar and refs show a title spinner only.
pub fn pane_loading(i: &PaneInputs) -> PaneLoading {
    PaneLoading {
        sidebar_spinner: i.sections_live || i.bootstrap_live,
        sidebar: (i.sections_live || i.bootstrap_live) && !i.requested_pending,
        main: i.requested_pending && !i.shown_cached,
        main_spinner: i.requested_pending,
        refs: i.refs_pending,
    }
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
                LoadSource::Details => "details",
                LoadSource::Board => "board",
                LoadSource::Sections => "sections",
                LoadSource::Backfill => "backfill",
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
pub fn toggle_pin(pins: &[ItemRef], n: ItemRef) -> Vec<ItemRef> {
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
pub fn history_push(back: &[ItemRef], current: Option<ItemRef>, next: ItemRef) -> Vec<ItemRef> {
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
    pin_order: &[ItemRef],
    visited: &[ItemRef],
    recent: &[ItemRef],
    pinned: &HashSet<ItemRef>,
) -> (Vec<ItemRef>, Vec<ItemRef>, Vec<ItemRef>) {
    let pin_set: HashSet<ItemRef> = pin_order.iter().copied().collect();
    let pinned_sec: Vec<ItemRef> = pin_order.to_vec();
    let visited_sec: Vec<ItemRef> = visited
        .iter()
        .filter(|n| !pin_set.contains(n) && !pinned.contains(n))
        .copied()
        .collect();
    let seen: HashSet<ItemRef> = pin_set
        .union(pinned)
        .copied()
        .collect::<HashSet<_>>()
        .union(&visited_sec.iter().copied().collect::<HashSet<_>>())
        .copied()
        .collect();
    let recent_sec: Vec<ItemRef> = recent
        .iter()
        .filter(|n| !seen.contains(n))
        .copied()
        .collect();
    (pinned_sec, visited_sec, recent_sec)
}

/// Display rows for one pane (Issues or PRs): pinned, then requested, then
/// recent, with a labeled divider slot between non-empty groups. Empty
/// groups leave no divider behind.
pub fn pane_order(
    pinned: &[ItemRef],
    requested: &[ItemRef],
    recent: &[ItemRef],
) -> Vec<crate::model::SideRow> {
    use crate::model::SideRow;
    let mut out: Vec<SideRow> = vec![];
    for (label, group) in [("pinned", pinned), ("requested", requested), ("recent", recent)] {
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(SideRow::Divider(label.to_string()));
        }
        out.extend(group.iter().copied().map(SideRow::Item));
    }
    out
}

/// Display rows with the recent group split by milestone: pinned and
/// requested groups on top, then one divider per milestone. Group labels
/// carry the member count. `(no milestone)` always sorts last.
pub fn grouped_pane_order(
    pinned: &[ItemRef],
    requested: &[ItemRef],
    groups: &[(String, Vec<ItemRef>)],
) -> Vec<crate::model::SideRow> {
    use crate::model::SideRow;
    let mut out = pane_order(pinned, requested, &[]);
    for (label, rows) in groups {
        if rows.is_empty() {
            continue;
        }
        out.push(SideRow::Divider(format!("{label} ({})", rows.len())));
        out.extend(rows.iter().copied().map(SideRow::Item));
    }
    out
}

/// Bucket recent rows by milestone title for display. Group order is due
/// date ascending, then title; milestones without a due date follow the
/// dated ones, and `(no milestone)` stays last. Members keep the incoming
/// order.
pub fn milestone_groups(
    rows: &[ItemRef],
    milestone_of: &dyn Fn(ItemRef) -> Option<String>,
    due_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, Vec<ItemRef>)> {
    use std::collections::HashMap;
    let mut buckets: HashMap<Option<String>, Vec<ItemRef>> = HashMap::new();
    for &n in rows {
        buckets.entry(milestone_of(n)).or_default().push(n);
    }
    let mut groups: Vec<(Option<String>, Vec<ItemRef>)> = buckets.into_iter().collect();
    groups.sort_by(|(a, _), (b, _)| match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(a), Some(b)) => {
            let due_order = match (due_of(a), due_of(b)) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            due_order.then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        }
    });
    groups
        .into_iter()
        .map(|(title, rows)| {
            (title.unwrap_or_else(|| "(no milestone)".to_string()), rows)
        })
        .collect()
}

/// True when `query` matches the row: a case-insensitive substring of
/// the title, a label name, or the author login, or a digit substring of
/// the number (`#` prefix optional). Empty query matches every row, so a
/// cleared `/` filter restores the full sidebar.
pub fn matches_search(
    number: ItemRef,
    title: &str,
    labels: &[&str],
    author: &str,
    query: &str,
) -> bool {
    let q = query.trim().trim_start_matches('#').to_lowercase();
    if q.is_empty() {
        return true;
    }
    if number.number.to_string().contains(&q) {
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

/// True when `query` matches a board card: a case-insensitive substring of
/// the card title or a label name, or the card number written exactly as
/// `#123`. Milestones filter through the dialog pills, never this text.
/// Empty query matches every card, so clearing the filter restores the
/// full board.
pub fn matches_board_filter(
    number: Option<ItemRef>,
    title: &str,
    labels: &[&str],
    query: &str,
) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    if let Some(rest) = q.strip_prefix('#') {
        let parsed = rest.parse::<u64>().ok();
        return parsed.is_some() && parsed == number.map(|item| item.number);
    }
    title.to_lowercase().contains(&q)
        || labels.iter().any(|l| l.to_lowercase().contains(&q))
}

/// Sidebar sort key. `Default` restores the incoming pinned/requested/
/// recent order; `Updated` sorts by the raw timestamp (newest first by
/// default), `Author`/`State` by login/state, and `Label` by the first
/// label name alphabetically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    rows: Vec<ItemRef>,
    sort: SideSort,
    desc: bool,
    data: &HashMap<ItemRef, IssueData>,
) -> Vec<ItemRef> {
    if sort == SideSort::Default {
        return rows;
    }
    let key = |n: ItemRef| -> Option<String> {
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
    let mut keyed: Vec<(Option<String>, ItemRef)> = rows.into_iter().map(|n| (key(n), n)).collect();
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
pub fn yank_issue_text(number: ItemRef, title: &str, url: &str) -> String {
    format!(
        "{number} {} — {url}",
        collapse_single_line(title),
        number = number.label()
    )
}

/// Agent-ready diff context: PR identity plus the file under `diff_cursor`.
/// `first_hunk` is the `@@ … @@` header when the file is expanded.
pub fn yank_diff_text(
    number: ItemRef,
    title: &str,
    url: &str,
    file: &DiffFile,
    first_hunk: Option<&str>,
) -> String {
    let hunk = first_hunk
        .map(|h| format!(" {}", collapse_single_line(h)))
        .unwrap_or_default();
    format!(
        "{number} {} — {} ({} +{} -{}{hunk}) — {url}",
        collapse_single_line(title),
        file.filename,
        file.status,
        file.additions,
        file.deletions,
        number = number.label()
    )
}

/// Agent-ready diff line: PR identity plus the active patch line.
pub fn yank_diff_line_text(
    number: ItemRef,
    title: &str,
    url: &str,
    filename: &str,
    line: &str,
) -> String {
    format!(
        "{number} {} — {filename}: \"{}\" — {url}",
        collapse_single_line(title),
        collapse_single_line(line),
        number = number.label()
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
    current: ItemRef,
    target: ItemRef,
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
            let mut out = format!(
                "{current} refs {target} (body line {}): \"{snippet}\"",
                idx + 1,
                current = current.label(),
                target = target.label()
            );
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
mod tests;
