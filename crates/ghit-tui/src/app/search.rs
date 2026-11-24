//! Content search: `/` with the Main pane focused searches what the pane
//! shows. `ui` finds the matches on the rendered lines each frame and
//! publishes them here; `n`/`N` step through them and scroll to the hit.

use crate::app::{App, Focus};

/// One match as the last frame drew it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentMatch {
    /// Absolute body row, wrapped: where the match shows, or the file
    /// header row when the match sits in a folded Diff file.
    pub(crate) row: usize,
    /// The folded Diff file holding the match, which a jump unfolds.
    pub(crate) hidden_file: Option<usize>,
}

#[derive(Default)]
pub(crate) struct ContentSearch {
    pub(crate) query: String,
    /// The find bar is open and scoped to the content.
    pub(crate) typing: bool,
    /// Index into `matches` of the current match.
    pub(crate) current: usize,
    /// The query changed: the next frame picks the first match at or after
    /// the scroll position.
    pub(crate) retarget: bool,
    /// A jump landed: the next frame scrolls the current match into view.
    pub(crate) reveal: bool,
    /// Per-frame fact published by `ui`.
    pub(crate) matches: Vec<ContentMatch>,
}

impl ContentSearch {
    pub(crate) fn active(&self) -> bool {
        !self.query.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// `3/12`, or `0/0` when nothing matches.
    pub(crate) fn counter(&self) -> String {
        let shown = if self.matches.is_empty() { 0 } else { self.current + 1 };
        format!("{shown}/{}", self.matches.len())
    }
}

impl App {
    /// `/` with Main focused: open the bar scoped to the content.
    pub(crate) fn start_content_search(&mut self) {
        self.search.clear();
        self.search.typing = true;
        self.sidebar.searching = false;
        self.nav.focus = Focus::Bar;
    }

    pub(crate) fn content_search_push(&mut self, c: char) {
        self.search.query.push(c);
        self.search.retarget = true;
    }

    pub(crate) fn content_search_pop(&mut self) {
        self.search.query.pop();
        self.search.retarget = true;
    }

    /// Enter in the bar: keep the highlights, close the bar.
    pub(crate) fn commit_content_search(&mut self) {
        self.search.typing = false;
        self.nav.focus = Focus::Main;
    }

    /// `n`/`N`: step to the next or previous match, wrapping. A match in a
    /// folded Diff file unfolds it.
    pub(crate) fn step_content_match(&mut self, delta: isize) {
        if !self.search.active() {
            self.notify("no search: press /".to_string());
            return;
        }
        let len = self.search.matches.len();
        if len == 0 {
            self.notify(format!("no match: {}", self.search.query));
            return;
        }
        let next = (self.search.current as isize + delta).rem_euclid(len as isize) as usize;
        self.search.current = next;
        self.search.reveal = true;
        if let Some(fi) = self.search.matches[next].hidden_file {
            self.unfold_diff_file(fi);
        }
    }

    fn unfold_diff_file(&mut self, fi: usize) {
        let Some(n) = self.nav.current else { return };
        let Some(name) = self.session.diff_files(n).and_then(|f| f.get(fi)).map(|f| f.filename.clone()) else { return };
        self.diff.expanded_files.insert((n, name));
    }

    /// Frame hook: take this frame's matches, settle the current one and
    /// scroll it into view when a jump or a new query asks for it.
    pub(crate) fn publish_content_matches(&mut self, matches: Vec<ContentMatch>, height: usize, total_rows: usize) {
        self.search.matches = matches;
        let scroll = usize::from(self.nav.scroll);
        let Some(last) = self.search.matches.len().checked_sub(1) else {
            self.search.current = 0;
            return;
        };
        let retarget = std::mem::take(&mut self.search.retarget);
        if retarget {
            self.search.current = self.search.matches.iter().position(|m| m.row >= scroll).unwrap_or(0);
        }
        self.search.current = self.search.current.min(last);
        if retarget || std::mem::take(&mut self.search.reveal) {
            let row = self.search.matches[self.search.current].row;
            if row < scroll || row >= scroll + height.max(1) {
                let top = row.saturating_sub(height / 3).min(total_rows.saturating_sub(height));
                self.nav.scroll = top.min(usize::from(u16::MAX)) as u16;
            }
        }
    }
}
