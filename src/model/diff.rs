//! Diff files, patch line helpers, thread cursor mapping and split rows.

use super::*;

/// One changed file in a PR diff, as returned by the REST files API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
    #[serde(default)]
    pub patch: String,
}

impl DiffFile {
    /// Lines the header reports as changed.
    pub fn changed(&self) -> usize {
        self.additions + self.deletions
    }

    /// Hunk text exists (binary and API-truncated files have none).
    pub fn has_patch(&self) -> bool {
        !diff_lines(&self.patch).is_empty()
    }

    /// Past the size that expand-all opens.
    pub fn is_large(&self) -> bool {
        self.changed() > crate::core::SMALL_DIFF_LINES
    }
}

/// Semantic role of one patch line; the UI maps each to a theme color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Hunk,
    Add,
    Del,
    Context,
}

/// Split a unified `patch` into tagged lines. Hunk headers (`@@`) tag as
/// `Hunk`, `+` as `Add`, `-` as `Del`, everything else as `Context`.
/// Empty or missing patches yield no lines: the UI shows the file header
/// with a binary/large-file note instead.
pub fn diff_lines(patch: &str) -> Vec<(DiffKind, String)> {
    patch
        .lines()
        .map(|l| {
            let kind = if l.starts_with("@@") {
                DiffKind::Hunk
            } else if l.starts_with('+') {
                DiffKind::Add
            } else if l.starts_with('-') {
                DiffKind::Del
            } else {
                DiffKind::Context
            };
            (kind, l.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualContent { Diff, Graph }

pub fn visual_content(is_pr: bool) -> VisualContent {
    if is_pr { VisualContent::Diff } else { VisualContent::Graph }
}

/// Map each patch row to its `(old, new)` file lines. Hunk headers and
/// `\ No newline` markers map to `(None, None)`; deletions carry only
/// old, additions only new, context both. Each counter seeds from its
/// side of the hunk header and steps past rows on that side.
pub fn patch_line_numbers(patch: &str) -> Vec<(Option<u64>, Option<u64>)> {
    fn hunk_start(header: &str, marker: char) -> Option<u64> {
        header.split_whitespace().find_map(|part| part.strip_prefix(marker))
            .and_then(|part| part.split(',').next())
            .and_then(|part| part.parse::<u64>().ok())
    }
    let mut old_next = None;
    let mut new_next = None;
    diff_lines(patch).iter().map(|(kind, text)| match kind {
        DiffKind::Hunk => {
            old_next = hunk_start(text, '-');
            new_next = hunk_start(text, '+');
            (None, None)
        }
        DiffKind::Del => {
            let line = old_next;
            old_next = old_next.map(|n| n + 1);
            (line, None)
        }
        DiffKind::Add | DiffKind::Context if !text.starts_with('\\') => {
            let new = new_next;
            new_next = new_next.map(|n| n + 1);
            if *kind == DiffKind::Context {
                let old = old_next;
                old_next = old_next.map(|n| n + 1);
                (old, new)
            } else {
                (None, new)
            }
        }
        _ => (None, None),
    }).collect()
}

/// Map each patch row to its new-file line, or None for headers and deletions.
pub fn patch_new_lines(patch: &str) -> Vec<Option<u64>> {
    patch_line_numbers(patch).into_iter().map(|(_, new)| new).collect()
}

pub fn thread_patch_row(thread: &ReviewThread, file: &DiffFile) -> Option<usize> {
    if thread.path != file.filename { return None; }
    let line = thread.line?;
    patch_new_lines(&file.patch).iter().position(|n| *n == Some(line))
}

/// Strip a rendered unified-diff row down to its patch text. Code rows
/// carry a 7-cell line-number gutter (`  42 │ code`, blank number for
/// hunk headers). Anything else (split cells, rules, blanks, thread
/// blocks, wrapped continuations, legacy dent) keeps the old two-cell
/// dent strip so non-matching rows behave exactly as before.
pub fn strip_diff_gutter(row: &str) -> &str {
    // The gutter is 4 cells of blanks/digits plus ` │ `; `│` is
    // multi-byte, so walk chars rather than slicing bytes.
    let mut chars = row.chars();
    for _ in 0..4 {
        match chars.next() {
            Some(c) if c == ' ' || c.is_ascii_digit() => {}
            _ => return row.strip_prefix("  ").unwrap_or(row),
        }
    }
    match chars.as_str().strip_prefix(" │ ") {
        Some(code) => code,
        None => row.strip_prefix("  ").unwrap_or(row),
    }
}

/// New-side line for a cursor sitting on a rendered unified-diff section.
///
/// `section` holds the visible buffer rows of one file starting just below
/// its header, `cursor` is the index into `section` of the active row, and
/// `patch` is the file's raw unified patch. Returns the new-side line of
/// the nearest Add/Context row at or above the cursor, or `None` when the
/// cursor sits above the first commentable row (hunk headers, leading
/// deletions, binary notes). Rendered rows carry a line-number gutter
/// that is stripped before comparing; blank, rule, and review-thread
/// rows never match a patch row and are skipped, as are wrapped
/// continuations (a truncated first fragment still prefixes its patch row).
pub fn line_target_at_cursor(section: &[String], cursor: usize, patch: &str) -> Option<u64> {
    let rows: Vec<(DiffKind, String)> = diff_lines(patch);
    let numbers = patch_new_lines(patch);
    let mut target: Option<u64> = None;
    let mut next = 0;
    for row in section.iter().take(cursor + 1) {
        let Some((_, text)) = rows.get(next) else { break };
        let stripped = strip_diff_gutter(row.trim_end());
        if !stripped.is_empty() && (stripped == text || text.starts_with(stripped)) {
            if let Some(n) = numbers.get(next).copied().flatten() {
                target = Some(n);
            }
            next += 1;
        }
    }
    target
}

/// Thread id for a cursor sitting inside a rendered review-thread block.
///
/// Same `section`/`cursor` contract as `line_target_at_cursor`, plus the
/// file and its threads in render order. Threads attach after their patch
/// row (`thread_patch_row`); a block opens at its reply-glyph header row
/// and runs to the next patch row or thread header, so the cursor on a
/// header, author, or comment row answers that thread's id. Code rows,
/// blanks, and rules answer `None` so callers keep the line-comment
/// target there. The k-th header after a patch row names the k-th thread
/// attached there, matching render order.
pub fn thread_at_cursor(
    section: &[String],
    cursor: usize,
    file: &DiffFile,
    threads: &[ReviewThread],
) -> Option<String> {
    let rows = diff_lines(&file.patch);
    let mut next = 0;
    // Patch row the current gap follows; `None` before the first one.
    // Threads with no patch row render in that leading zone.
    let mut attach: Option<usize> = None;
    // Ids of headers seen since the last consumed patch row, in order.
    let mut seen: Vec<String> = vec![];
    // Thread whose block the walk is inside, if any.
    let mut block: Option<String> = None;
    for (i, row) in section.iter().enumerate().take(cursor + 1) {
        // Same gutter strip as `line_target_at_cursor`: patch rows sit
        // behind a line-number gutter since #75.
        let stripped = strip_diff_gutter(row.trim_end());
        let matched = match rows.get(next) {
            Some((_, text)) => !stripped.is_empty() && (stripped == text || text.starts_with(stripped)),
            None => false,
        };
        if matched {
            next += 1;
            attach = Some(next - 1);
            seen.clear();
            block = None;
            continue;
        }
        if stripped.contains('\u{f075}') {
            let attached: Vec<&ReviewThread> = threads
                .iter()
                .filter(|t| t.path == file.filename && thread_patch_row(t, file) == attach)
                .collect();
            if let Some(thread) = attached.get(seen.len()) {
                seen.push(thread.id.clone());
                block = Some(thread.id.clone());
            }
        }
        if i == cursor {
            return block;
        }
    }
    None
}

/// File index of a rendered `▸ `/`▾ ` diff header row. Longest filename
/// wins so `a.rs` never shadows `src/a.rs`. `None` for any other row.
pub fn header_file_index(line: &str, files: &[DiffFile]) -> Option<usize> {
    if !(line.starts_with("▸ ") || line.starts_with("▾ ")) {
        return None;
    }
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| line.contains(&f.filename))
        .max_by_key(|(_, f)| f.filename.len())
        .map(|(i, _)| i)
}

/// Summary for the diff tab header: file count plus summed adds/deletes.
pub fn diff_summary(files: &[DiffFile]) -> String {
    let adds: usize = files.iter().map(|f| f.additions).sum();
    let dels: usize = files.iter().map(|f| f.deletions).sum();
    format!("{} file{} · +{adds} -{dels}", files.len(), if files.len() == 1 { "" } else { "s" })
}

/// One row of a side-by-side rendering of a unified patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitRow {
    /// Hunk header, rendered full-width.
    Hunk(String),
    /// Old line on the left, new line on the right. Context shows the
    /// same text on both sides; a change pairs one `-` line with one `+`.
    /// `old`/`new` are the per-side file lines for the gutter.
    Pair { left: String, right: String, old: Option<u64>, new: Option<u64> },
    /// Unpaired `-` line: right cell stays blank. `line` is old-side.
    LeftOnly { text: String, kind: DiffKind, line: Option<u64> },
    /// Unpaired `+` line: left cell stays blank. `line` is new-side.
    RightOnly { text: String, kind: DiffKind, line: Option<u64> },
}

/// Pair a unified `patch` into side-by-side rows. Runs of `-` lines pair
/// with the `+` lines that follow them, in order; leftovers render alone.
/// Hunk headers pass through full-width. Empty patches yield no rows.
/// Each side keeps its file line from `patch_line_numbers` for the gutter.
pub fn split_rows(patch: &str) -> Vec<SplitRow> {
    let mut rows: Vec<SplitRow> = vec![];
    let mut dels: Vec<(String, Option<u64>)> = vec![];
    let numbers = patch_line_numbers(patch);
    for ((kind, text), (old, new)) in diff_lines(patch).into_iter().zip(numbers) {
        match kind {
            DiffKind::Hunk => {
                for (d, line) in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
                }
                rows.push(SplitRow::Hunk(text));
            }
            DiffKind::Del => dels.push((text, old)),
            DiffKind::Add => {
                if dels.is_empty() {
                    rows.push(SplitRow::RightOnly { text, kind: DiffKind::Add, line: new });
                } else {
                    let (left, old) = dels.remove(0);
                    rows.push(SplitRow::Pair { left, right: text, old, new });
                }
            }
            DiffKind::Context => {
                for (d, line) in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
                }
                rows.push(SplitRow::Pair { left: text.clone(), right: text, old, new });
            }
        }
    }
    for (d, line) in dels.drain(..) {
        rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
    }
    rows
}
