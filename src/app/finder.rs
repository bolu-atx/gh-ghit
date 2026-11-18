//! The Ctrl+P finder: a query over every issue and PR the app knows.
//! Candidates come from memory only, so typing never touches the network.

use std::collections::HashSet;

use crate::app::{App, Entry, Slot};
use crate::core;

/// State that exists only while the finder is open.
#[derive(Clone, Debug, Default)]
pub(crate) struct FinderState {
    pub(crate) query: String,
    pub(crate) cursor: usize,
}

/// One result row. `indices` are char positions in `#number title`.
pub(crate) struct FinderRow {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) state: String,
    pub(crate) is_pr: bool,
    pub(crate) indices: Vec<usize>,
}

impl App {
    pub(crate) fn open_finder(&mut self) {
        self.open(Entry::Finder(FinderState::default()));
    }

    pub(crate) fn finder(&self) -> Option<&FinderState> {
        self.dialogs.iter().find_map(|e| if let Entry::Finder(f) = e { Some(f) } else { None })
    }

    pub(crate) fn finder_mut(&mut self) -> Option<&mut FinderState> {
        self.dialogs.iter_mut().find_map(|e| if let Entry::Finder(f) = e { Some(f) } else { None })
    }

    /// Numbers with loaded data, most useful first: the open item, then
    /// history (newest first), then the sidebar order, pins, visits and
    /// recent rows, then whatever else is cached by number.
    fn finder_candidates(&self) -> Vec<u64> {
        let mut seen = HashSet::new();
        let mut order = vec![];
        let mut cached: Vec<u64> = self.issues.data.keys().copied().collect();
        cached.sort_unstable_by(|a, b| b.cmp(a));
        let sources = [
            self.nav.current.into_iter().collect::<Vec<_>>(),
            self.nav.history.iter().rev().copied().collect(),
            self.issue_numbers(),
            self.pr_numbers(),
            self.nav.pin_order.clone(),
            self.nav.visited.clone(),
            self.nav.recent.clone(),
            cached,
        ];
        for n in sources.into_iter().flatten() {
            if self.issues.data.contains_key(&n) && seen.insert(n) {
                order.push(n);
            }
        }
        order
    }

    /// How many items the finder can search.
    pub(crate) fn finder_total(&self) -> usize {
        self.finder_candidates().len()
    }

    /// Ranked results for the open query. Equal scores keep candidate order.
    pub(crate) fn finder_rows(&self) -> Vec<FinderRow> {
        let Some(finder) = self.finder() else { return vec![] };
        let mut hits: Vec<(i32, FinderRow)> = vec![];
        for n in self.finder_candidates() {
            let Some(data) = self.issues.data.get(&n) else { continue };
            let Some(found) = core::fuzzy_score(&finder.query, &format!("#{n} {}", data.title)) else { continue };
            hits.push((found.score, FinderRow {
                number: n,
                title: data.title.clone(),
                state: data.state.clone(),
                is_pr: data.is_pr,
                indices: found.indices,
            }));
        }
        hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        hits.into_iter().map(|(_, row)| row).collect()
    }

    pub(crate) fn finder_type(&mut self, c: char) {
        if let Some(finder) = self.finder_mut() {
            finder.query.push(c);
            finder.cursor = 0;
        }
    }

    pub(crate) fn finder_backspace(&mut self) {
        if let Some(finder) = self.finder_mut() {
            finder.query.pop();
            finder.cursor = 0;
        }
    }

    pub(crate) fn finder_step(&mut self, delta: isize) {
        let last = self.finder_rows().len().saturating_sub(1) as isize;
        if let Some(finder) = self.finder_mut() {
            finder.cursor = (finder.cursor as isize + delta).clamp(0, last) as usize;
        }
    }

    /// Open the cursor row in Main like Enter on a sidebar row, then close.
    pub(crate) fn finder_open(&mut self) {
        let Some(cursor) = self.finder().map(|f| f.cursor) else { return };
        let number = self.finder_rows().get(cursor).map(|row| row.number);
        self.close(Slot::Finder);
        if let Some(n) = number {
            self.request_issue(n);
            self.ensure_visited(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::issue;

    fn app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        for (n, title, pr) in [
            (241u64, "Fix flaky auth probe", false),
            (198, "Auth probe retries on 401", true),
            (77, "Probe for token scopes", false),
            (5, "Unrelated chore", false),
        ] {
            let mut data = issue(n, "2026-01-01T00:00:00Z");
            data.title = title.to_string();
            data.is_pr = pr;
            app.issues.data.insert(n, data);
        }
        app.open_finder();
        app
    }

    fn numbers(app: &App) -> Vec<u64> {
        app.finder_rows().iter().map(|r| r.number).collect()
    }

    #[test]
    fn empty_query_lists_history_first_then_the_rest_by_number() {
        let mut app = app();
        app.nav.history = vec![5, 77];
        assert_eq!(numbers(&app), vec![77, 5, 241, 198]);
        assert_eq!(app.finder_total(), 4);
    }

    #[test]
    fn typing_filters_and_resets_the_cursor() {
        let mut app = app();
        app.finder_step(2);
        for c in "probe".chars() {
            app.finder_type(c);
        }
        assert_eq!(app.finder().map(|f| f.cursor), Some(0));
        assert_eq!(numbers(&app).len(), 3);
        app.finder_backspace();
        assert_eq!(app.finder().map(|f| f.query.as_str()), Some("prob"));
    }

    #[test]
    fn number_prefix_ranks_first() {
        let mut app = app();
        for c in "77".chars() {
            app.finder_type(c);
        }
        assert_eq!(numbers(&app), vec![77]);
    }

    #[test]
    fn step_clamps_to_the_results() {
        let mut app = app();
        app.finder_step(-1);
        assert_eq!(app.finder().map(|f| f.cursor), Some(0));
        app.finder_step(99);
        assert_eq!(app.finder().map(|f| f.cursor), Some(3));
    }

    #[test]
    fn open_closes_and_requests_the_cursor_item() {
        let mut app = app();
        for c in "198".chars() {
            app.finder_type(c);
        }
        app.finder_open();
        assert!(app.finder().is_none());
        assert_eq!(app.nav.requested, Some(198));
    }
}
