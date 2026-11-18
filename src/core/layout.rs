//! Layout constants and math shared by `app`, `input` and `ui`.

/// First 50 files render; the rest collapse to a one-line note so a
/// huge PR cannot flood the pane.
pub(crate) const MAX_DIFF_FILES: usize = 50;
