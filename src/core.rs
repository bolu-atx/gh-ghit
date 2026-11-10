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

/// Pin-first ordering: pinned numbers in pin order, then everything else in
/// existing order. Members missing from both lists are dropped; non-members
/// in the pin order are ignored.
pub fn apply_pin_order(list: &[u64], pin_order: &[u64], pinned: &HashSet<u64>) -> Vec<u64> {
    let pins: Vec<u64> =
        pin_order.iter().filter(|n| list.contains(n)).copied().collect();
    let rest: Vec<u64> = list.iter().filter(|n| !pinned.contains(n)).copied().collect();
    [pins, rest].concat()
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
    fn pins_order_first_and_toggle() {
        let pinned: HashSet<u64> = [9].into_iter().collect();
        assert_eq!(apply_pin_order(&[7, 9], &[9], &pinned), vec![9, 7]);
        assert_eq!(apply_pin_order(&[7], &[9], &pinned), vec![7]);
        assert_eq!(toggle_pin(&[7], 9), vec![7, 9]);
        assert_eq!(toggle_pin(&[7, 9], 9), vec![7]);
    }
}
