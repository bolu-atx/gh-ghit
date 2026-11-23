//! Soft wrap of a draft into visual rows, and the cursor mapping both
//! ways. Pure: widths count chars, so the render and the mouse path
//! share one answer.

/// One visual row: a span `[start, end)` of char indices into the body.
/// `line` is the logical line it belongs to. A row cut by a soft break
/// ends where the next one starts; a row ending a logical line stops
/// before its `\n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Rows of `body` for a text column `cells` wide. The last cell stays
/// free so the cursor fits after a full row.
pub fn text_rows(body: &str, cells: usize) -> Vec<Row> {
    wrap(body, cells.saturating_sub(1).max(1))
}

/// Soft-wrap `body` to rows of at most `width` chars. A row breaks after
/// its last space when it has one, else in the middle of a word.
pub(crate) fn wrap(body: &str, width: usize) -> Vec<Row> {
    let chars: Vec<char> = body.chars().collect();
    let width = width.max(1);
    let mut rows = vec![];
    let mut start = 0;
    for (line, text) in body.split('\n').enumerate() {
        let line_end = start + text.chars().count();
        let mut at = start;
        loop {
            if line_end - at <= width {
                rows.push(Row { line, start: at, end: line_end });
                break;
            }
            let window = &chars[at..at + width + 1];
            let cut = window.iter().rposition(|c| *c == ' ').filter(|k| *k > 0).map_or(at + width, |k| at + k + 1);
            rows.push(Row { line, start: at, end: cut });
            at = cut;
        }
        start = line_end + 1;
    }
    rows
}

/// Whether `rows[i]` is the last row of its logical line.
fn ends_line(rows: &[Row], i: usize) -> bool {
    rows.get(i + 1).is_none_or(|next| next.line != rows[i].line)
}

/// Whether `rows[i]` is the first row of its logical line.
pub fn starts_line(rows: &[Row], i: usize) -> bool {
    i == 0 || rows[i - 1].line != rows[i].line
}

/// Visual (row, col) of the char index `cursor`.
pub fn cursor_pos(rows: &[Row], cursor: usize) -> (usize, usize) {
    let hit = rows.iter().enumerate()
        .find(|(i, r)| cursor >= r.start && (cursor < r.end || (cursor == r.end && ends_line(rows, *i))))
        .map(|(i, _)| i);
    let i = hit.unwrap_or(rows.len().saturating_sub(1));
    (i, cursor.saturating_sub(rows.get(i).map_or(0, |r| r.start)))
}

/// Char index at visual (`row`, `col`), both clamped into the text.
pub fn index_at(rows: &[Row], row: usize, col: usize) -> usize {
    let i = row.min(rows.len().saturating_sub(1));
    let Some(r) = rows.get(i) else { return 0 };
    let len = r.end - r.start;
    let room = if ends_line(rows, i) { len } else { len.saturating_sub(1) };
    r.start + col.min(room)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(body: &str, width: usize) -> Vec<(usize, usize)> {
        wrap(body, width).iter().map(|r| (r.start, r.end)).collect()
    }

    #[test]
    fn short_lines_stay_whole() {
        assert_eq!(spans("ab\ncd", 10), vec![(0, 2), (3, 5)]);
        assert_eq!(spans("", 10), vec![(0, 0)]);
    }

    #[test]
    fn trailing_newline_adds_an_empty_row() {
        let rows = wrap("ab\n", 10);
        assert_eq!(rows.len(), 2);
        assert_eq!(cursor_pos(&rows, 3), (1, 0));
        assert_eq!(cursor_pos(&rows, 2), (0, 2));
    }

    #[test]
    fn long_line_breaks_after_a_space() {
        assert_eq!(spans("aaa bbb ccc", 4), vec![(0, 4), (4, 8), (8, 11)]);
        assert_eq!(spans("aaa bbb ccc", 7), vec![(0, 8), (8, 11)]);
    }

    #[test]
    fn unbroken_word_breaks_hard() {
        assert_eq!(spans("abcdefgh", 3), vec![(0, 3), (3, 6), (6, 8)]);
    }

    #[test]
    fn cursor_at_a_soft_break_sits_on_the_next_row() {
        let rows = wrap("abcdefgh", 3);
        assert_eq!(cursor_pos(&rows, 3), (1, 0));
        assert_eq!(cursor_pos(&rows, 8), (2, 2));
        assert_eq!(cursor_pos(&rows, 0), (0, 0));
    }

    #[test]
    fn cursor_at_end_of_body() {
        let rows = wrap("one\ntwo", 10);
        assert_eq!(cursor_pos(&rows, 7), (1, 3));
        assert_eq!(cursor_pos(&wrap("", 10), 0), (0, 0));
    }

    #[test]
    fn click_maps_back_to_a_char_index() {
        let rows = wrap("abcdefgh\nij", 3);
        assert_eq!(index_at(&rows, 0, 1), 1);
        assert_eq!(index_at(&rows, 1, 0), 3);
        assert_eq!(index_at(&rows, 2, 99), 8, "past a soft row stops before the break");
        assert_eq!(index_at(&rows, 3, 99), 11, "past the last row lands at the end");
        assert_eq!(index_at(&rows, 99, 0), 9);
    }

    #[test]
    fn click_and_cursor_round_trip() {
        let body = "the quick brown fox\n\njumps over";
        let rows = wrap(body, 8);
        for cursor in 0..=body.chars().count() {
            let (row, col) = cursor_pos(&rows, cursor);
            assert_eq!(index_at(&rows, row, col), cursor, "cursor {cursor}");
        }
    }

    #[test]
    fn text_rows_reserves_a_cell_for_the_cursor() {
        assert_eq!(text_rows("abcd", 4).len(), 2);
        assert_eq!(text_rows("abc", 4).len(), 1);
    }
}
