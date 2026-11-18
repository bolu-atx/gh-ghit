//! Layout constants and math shared by `app`, `input` and `ui`.

/// Fixed widths of the side columns.
pub(crate) const SIDEBAR_WIDTH: u16 = 44;
pub(crate) const RIGHT_WIDTH: u16 = 36;
/// Narrowest main pane. The issue title and body stay readable at this
/// width; below it the body turns into a vertical strip.
pub(crate) const MIN_MAIN_WIDTH: u16 = 40;

/// Widths of the sidebar and right column for a terminal `total` columns
/// wide. A column that is not wanted gets 0. When main would fall under
/// `MIN_MAIN_WIDTH`, the right column drops first, then the sidebar.
pub(crate) fn columns(total: u16, sidebar_wanted: bool, right_wanted: bool) -> (u16, u16) {
    let mut sidebar = if sidebar_wanted { SIDEBAR_WIDTH } else { 0 };
    let mut right = if right_wanted { RIGHT_WIDTH } else { 0 };
    if total < sidebar + right + MIN_MAIN_WIDTH {
        right = 0;
    }
    if total < sidebar + right + MIN_MAIN_WIDTH {
        sidebar = 0;
    }
    (sidebar, right)
}

/// First 50 files render; the rest collapse to a one-line note so a
/// huge PR cannot flood the pane.
pub(crate) const MAX_DIFF_FILES: usize = 50;

/// Largest diff (additions plus deletions, the count a file header shows)
/// that expand-all opens. Bigger files stay folded until opened by hand.
pub(crate) const SMALL_DIFF_LINES: usize = 300;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_terminal_drops_right_then_sidebar() {
        let full = SIDEBAR_WIDTH + RIGHT_WIDTH + MIN_MAIN_WIDTH;
        assert_eq!(columns(full, true, true), (SIDEBAR_WIDTH, RIGHT_WIDTH));
        assert_eq!(columns(full - 1, true, true), (SIDEBAR_WIDTH, 0), "right drops first");
        let both = SIDEBAR_WIDTH + MIN_MAIN_WIDTH;
        assert_eq!(columns(both, true, true), (SIDEBAR_WIDTH, 0));
        assert_eq!(columns(both - 1, true, true), (0, 0), "then the sidebar");
        assert_eq!(columns(80, true, false), (0, 0));
        assert_eq!(columns(200, false, true), (0, RIGHT_WIDTH), "unwanted stays 0");
        assert_eq!(columns(MIN_MAIN_WIDTH + RIGHT_WIDTH - 1, false, true), (0, 0));
    }
}
