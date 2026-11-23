//! The Ctrl+P finder: a query over every issue and PR the app knows.
//! Candidates are the items in memory plus the repo's local store, read
//! once when the finder opens, so typing never touches the network.

use std::collections::HashSet;

use crate::app::{App, Entry, Slot};
use crate::core;
use crate::github;
use crate::store::Candidate;
use crate::model::ItemRef;

/// State that exists only while the finder is open.
#[derive(Clone, Debug, Default)]
pub(crate) struct FinderState {
    pub(crate) query: String,
    pub(crate) cursor: usize,
    /// The repo's stored items, loaded once on open.
    pub(crate) stored: Vec<Candidate>,
}

/// One result row. `indices` are char positions in `#number title`.
pub(crate) struct FinderRow {
    pub(crate) number: ItemRef,
    pub(crate) title: String,
    pub(crate) state: String,
    pub(crate) is_pr: bool,
    pub(crate) indices: Vec<usize>,
}

impl App {
    pub(crate) fn open_finder(&mut self) {
        // A local SQLite read (a few thousand rows is well under a frame).
        let stored = self.repo_key().map(|repo| github::finder_candidates(&repo)).unwrap_or_default();
        self.open(Entry::Finder(FinderState { stored, ..Default::default() }));
    }

    pub(crate) fn finder(&self) -> Option<&FinderState> {
        self.dialogs.iter().find_map(|e| if let Entry::Finder(f) = e { Some(f) } else { None })
    }

    pub(crate) fn finder_mut(&mut self) -> Option<&mut FinderState> {
        self.dialogs.iter_mut().find_map(|e| if let Entry::Finder(f) = e { Some(f) } else { None })
    }

    /// Items to search, most useful first: numbers with loaded data (the
    /// open item, history newest first, then the sidebar order, pins,
    /// visits and recent rows, then whatever else is cached by number),
    /// then stored items not in memory, newest update first.
    fn finder_candidates(&self) -> Vec<Candidate> {
        let mut seen = HashSet::new();
        let mut order = vec![];
        let mut cached: Vec<ItemRef> = self.issues.data.keys().copied().collect();
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
            let Some(data) = self.issues.data.get(&n) else { continue };
            if seen.insert(n) {
                order.push(Candidate {
                    number: n,
                    is_pr: data.is_pr,
                    state: data.state.clone(),
                    title: data.title.clone(),
                    author: data.author.login.clone(),
                    labels: data.labels.iter().map(|l| l.name.clone()).collect(),
                });
            }
        }
        if let Some(finder) = self.finder() {
            order.extend(finder.stored.iter().filter(|c| seen.insert(c.number)).cloned());
        }
        order
    }

    /// How many items the finder can search.
    pub(crate) fn finder_total(&self) -> usize {
        self.finder_candidates().len()
    }

    /// Ranked results for the open query. Equal scores keep candidate order.
    /// A query matches `#number title`; failing that, the labels and author.
    pub(crate) fn finder_rows(&self) -> Vec<FinderRow> {
        let Some(finder) = self.finder() else { return vec![] };
        let mut hits: Vec<(i32, FinderRow)> = vec![];
        for c in self.finder_candidates() {
            let head = format!("{} {}", c.number.display(""), c.title);
            let found = core::fuzzy_score(&finder.query, &head).or_else(|| {
                let tail = format!("{head} {} {}", c.labels.join(" "), c.author);
                let mut f = core::fuzzy_score(&finder.query, &tail)?;
                f.indices.retain(|i| *i < head.chars().count());
                Some(f)
            });
            let Some(found) = found else { continue };
            hits.push((found.score, FinderRow {
                number: c.number,
                title: c.title,
                state: c.state,
                is_pr: c.is_pr,
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
            (ItemRef::github(241), "Fix flaky auth probe", false),
            (ItemRef::github(198), "Auth probe retries on 401", true),
            (ItemRef::github(77), "Probe for token scopes", false),
            (ItemRef::github(5), "Unrelated chore", false),
        ] {
            let mut data = issue(n, "2026-01-01T00:00:00Z");
            data.title = title.to_string();
            data.is_pr = pr;
            app.issues.data.insert(n, data);
        }
        app.open_finder();
        app
    }

    fn numbers(app: &App) -> Vec<ItemRef> {
        app.finder_rows().iter().map(|r| r.number).collect()
    }

    #[test]
    fn empty_query_lists_history_first_then_the_rest_by_number() {
        let mut app = app();
        app.nav.history = vec![ItemRef::github(5), ItemRef::github(77)];
        assert_eq!(
            numbers(&app),
            vec![
                ItemRef::github(77),
                ItemRef::github(5),
                ItemRef::github(241),
                ItemRef::github(198)
            ]
        );
        assert_eq!(app.finder_total(), 4);
    }

    fn stored(number: ItemRef, title: &str, author: &str, labels: &[&str]) -> Candidate {
        Candidate {
            number,
            is_pr: false,
            state: "CLOSED".into(),
            title: title.into(),
            author: author.into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
        }
    }

    #[test]
    fn store_only_items_follow_memory_and_dedupe_by_number() {
        let mut app = app();
        app.nav.history = vec![ItemRef::github(5)];
        app.finder_mut().unwrap().stored = vec![
            stored(ItemRef::github(900), "Newest stored", "ann", &[]),
            stored(ItemRef::github(77), "stale stored copy", "ann", &[]),
            stored(ItemRef::github(10), "Older stored", "ann", &[]),
        ];
        assert_eq!(
            numbers(&app),
            vec![
                ItemRef::github(5),
                ItemRef::github(241),
                ItemRef::github(198),
                ItemRef::github(77),
                ItemRef::github(900),
                ItemRef::github(10)
            ]
        );
        assert_eq!(app.finder_total(), 6);
        assert_eq!(
            app.finder_rows()
                .iter()
                .find(|r| r.number == ItemRef::github(77))
                .unwrap()
                .title,
            "Probe for token scopes"
        );
    }

    #[test]
    fn stored_items_match_title_label_and_author() {
        let mut app = app();
        app.finder_mut().unwrap().stored = vec![stored(
            ItemRef::github(900),
            "Crash on resize",
            "zelda",
            &["regression"],
        )];
        for q in ["resize", "regress", "zelda"] {
            app.finder_mut().unwrap().query = q.to_string();
            assert_eq!(numbers(&app), vec![ItemRef::github(900)], "{q}");
        }
    }

    #[test]
    fn a_missing_store_falls_back_to_memory() {
        let (_guard, _dir, saved) = crate::app::test_support::temp_xdg();
        let mut app = App::new(Some("o/r".to_string()));
        app.issues.data.insert(
            ItemRef::github(5),
            issue(ItemRef::github(5), "2026-01-01T00:00:00Z"),
        );
        app.open_finder();
        let stored = app.finder().unwrap().stored.len();
        let rows = numbers(&app);
        crate::app::test_support::restore_xdg(saved);
        assert_eq!(stored, 0);
        assert_eq!(rows, vec![ItemRef::github(5)]);
        assert!(App::new(None).finder_rows().is_empty());
    }

    #[test]
    fn opening_a_store_only_item_requests_it_by_number() {
        let mut app = app();
        app.finder_mut().unwrap().stored =
            vec![stored(ItemRef::github(900), "Only in store", "ann", &[])];
        for c in "only in".chars() {
            app.finder_type(c);
        }
        app.finder_open();
        assert_eq!(app.nav.requested, Some(ItemRef::github(900)));
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
        assert_eq!(numbers(&app), vec![ItemRef::github(77)]);
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
        assert_eq!(app.nav.requested, Some(ItemRef::github(198)));
    }
}
