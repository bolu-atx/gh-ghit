//! Content search on the Main pane text: finds matches on the rendered
//! lines, paints them, and publishes them with their wrapped rows so
//! `n`/`N` can scroll to them.

use std::collections::HashMap;

use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{App, ContentMatch};
use ghit_core::core::find_ranges;

/// Matches the Diff text holds without drawing them: a folded file's
/// patch. `line` is the last drawn line of the file block, so the matches
/// keep their document order against the drawn ones.
pub(super) struct HiddenSite {
    pub(super) line: usize,
    pub(super) file: usize,
    pub(super) count: usize,
}

/// What the Diff text tells search beyond its lines.
#[derive(Default)]
pub(super) struct SearchAux {
    /// Chars at the start of a line that search skips (the line-number gutter).
    pub(super) skips: HashMap<usize, usize>,
    pub(super) hidden: Vec<HiddenSite>,
}

fn plain(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Wrapped rows of one line at `width` cells, the way the pane wraps it.
fn line_rows(line: &Line, width: usize) -> usize {
    if line.width() <= width {
        return 1;
    }
    let paragraph = Paragraph::new(Text::from(line.clone())).wrap(Wrap { trim: false });
    paragraph.line_count(width.min(usize::from(u16::MAX)) as u16).max(1)
}

/// Rows from the line top to the row holding char `end` of the line.
fn row_in_line(line: &Line, end: usize, width: usize) -> usize {
    if line.width() <= width {
        return 0;
    }
    let prefix: String = plain(line).chars().take(end).collect();
    let paragraph = Paragraph::new(Text::raw(prefix)).wrap(Wrap { trim: false });
    paragraph.line_count(width.min(usize::from(u16::MAX)) as u16).saturating_sub(1)
}

/// Restyle char ranges of a line, splitting spans at the range edges.
/// `ranges` are sorted and do not overlap.
fn paint(line: &mut Line<'_>, ranges: &[(usize, usize, Style)]) {
    let mut out: Vec<Span<'_>> = Vec::with_capacity(line.spans.len() + ranges.len() * 2);
    let mut offset = 0;
    for span in std::mem::take(&mut line.spans) {
        let chars: Vec<char> = span.content.chars().collect();
        let (start, end) = (offset, offset + chars.len());
        offset = end;
        let mut cuts: Vec<usize> = ranges.iter().flat_map(|&(a, b, _)| [a, b]).filter(|&c| c > start && c < end).collect();
        cuts.sort_unstable();
        cuts.dedup();
        let mut from = start;
        for to in cuts.into_iter().chain(std::iter::once(end)) {
            let piece: String = chars[from - start..to - start].iter().collect();
            let style = match ranges.iter().find(|&&(a, b, _)| from >= a && to <= b) {
                Some(&(_, _, hit)) => span.style.patch(hit),
                None => span.style,
            };
            if !piece.is_empty() {
                out.push(Span::styled(piece, style));
            }
            from = to;
        }
    }
    line.spans = out;
}

/// Finds the matches of the live query in `text` (and in `aux.hidden`),
/// publishes them, and paints them. `width` and `height` are the body
/// cells the text wraps and scrolls in. No-op without a query.
pub(super) fn apply(app: &mut App, text: &mut Text<'_>, width: usize, height: usize, aux: &SearchAux) {
    if !app.search.active() {
        app.search.matches.clear();
        return;
    }
    let query = app.search.query.clone();
    let mut matches: Vec<ContentMatch> = vec![];
    // (match index, line, char range) of every match drawn on a line.
    let mut drawn: Vec<(usize, usize, (usize, usize))> = vec![];
    let mut row = 0;
    for (at, line) in text.lines.iter().enumerate() {
        let skip = aux.skips.get(&at).copied().unwrap_or(0);
        let body: String = plain(line).chars().skip(skip).collect();
        for (a, b) in find_ranges(&body, &query) {
            let (a, b) = (a + skip, b + skip);
            drawn.push((matches.len(), at, (a, b)));
            matches.push(ContentMatch { row: row + row_in_line(line, b, width), hidden_file: None });
        }
        for site in aux.hidden.iter().filter(|s| s.line == at) {
            for _ in 0..site.count {
                matches.push(ContentMatch { row: row + line_rows(line, width) - 1, hidden_file: Some(site.file) });
            }
        }
        row += line_rows(line, width);
    }
    app.publish_content_matches(matches, height, row);
    let current = app.search.current;
    let mut at = 0;
    while at < drawn.len() {
        let line = drawn[at].1;
        let mut ranges = vec![];
        while at < drawn.len() && drawn[at].1 == line {
            let (index, _, (a, b)) = drawn[at];
            let style = if index == current { crate::theme::search_current() } else { crate::theme::search_match() };
            ranges.push((a, b, style));
            at += 1;
        }
        paint(&mut text.lines[line], &ranges);
    }
}
