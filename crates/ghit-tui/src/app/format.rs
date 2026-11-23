//! Format dialog state and persisted view prefs.

use crate::app::{App, FilterItem};
use ghit_core::core::{self, Pane};
use ghit_core::{github, model};
use crate::app::{Entry, FormatEdit, board_open};

impl App {
    /// Open the sidebar Format dialog in navigate mode. The picker lists
    /// load lazily the first time.
    pub(crate) fn open_sidebar_format(&mut self) {
        self.open(Entry::Format(FormatEdit::None));
        self.ensure_repo_meta();
    }

    /// Milestone pills for the dialog: sorted titles, then the
    /// "(no milestone)" pill last, matching the board dialog. Empty until
    /// the first picker fetch lands.
    pub(crate) fn milestone_pills(&self) -> Vec<Option<String>> {
        if !self.sidebar.meta_loaded {
            return vec![];
        }
        // Hidden done milestones stay while a filter selects them, so
        // the filter can still be seen and cleared.
        let hide = self.sidebar.hide_done_milestones;
        let mut titles: Vec<String> = self.sidebar.milestones.iter()
            .filter(|m| !(hide && m.done()) || self.sidebar.milestone_filter.contains(&Some(m.title.clone())))
            .map(|m| m.title.clone()).collect();
        titles.sort();
        let mut out: Vec<Option<String>> = titles.into_iter().map(Some).collect();
        out.push(None);
        out
    }

    /// Number of controls in the left column of the sidebar format dialog.
    pub(crate) fn format_left_widgets(&self) -> usize {
        6 + self.sidebar.saved_queries.len() + self.sidebar.labels.len() + self.milestone_pills().len()
    }

    pub(crate) fn format_widgets(&self) -> usize {
        self.format_left_widgets() + 4
            + model::RowFields::ISSUE_ORDER.len() + model::RowFields::PR_ORDER.len()
    }

    pub(crate) fn toggle_format_widget(&mut self) {
        let focus = self.sidebar.format_focus;
        let labels = self.sidebar.labels.len();
        let pills = self.milestone_pills();
        let left = self.format_left_widgets();
        let saved = self.sidebar.saved_queries.len();
        if (1..=saved).contains(&focus) {
            let query = self.sidebar.saved_queries[focus - 1].query.clone();
            self.sidebar.query_active = query.clone();
            self.sidebar.query_draft = query.clone();
            self.run_gh_search(query);
            self.persist_prefs();
            return;
        }
        let focus = if focus > saved { focus - saved } else { focus };
        let left = left - saved;
        match focus {
            0 => self.start_query_edit(),
            1 => self.set_window(core::cycle_window(self.session.sections.merged_window_hours)),
            2..=5 => self.toggle_list(focus - 2),
            n if n < 6 + labels => {
                let name = self.sidebar.labels[n - 6].clone();
                self.toggle_label_filter(name);
            }
            n if n < left => self.toggle_milestone_filter(pills[n - 6 - labels].clone()),
            n if n == left => {
                self.sidebar.group_by = self.sidebar.group_by.next();
                self.persist_prefs();
            }
            n if n == left + 1 => {
                self.sidebar.sub_issue_tree = !self.sidebar.sub_issue_tree;
                self.persist_prefs();
            }
            n if n == left + 2 => self.cycle_sort(),
            n if n == left + 3 => self.toggle_sort_dir(),
            n => self.toggle_sidebar_row_field(n - left - 4),
        }
    }

    pub(crate) fn toggle_label_filter(&mut self, name: String) {
        if !self.sidebar.label_filter.remove(&name) {
            self.sidebar.label_filter.insert(name);
        }
        self.persist_prefs();
    }

    pub(crate) fn toggle_milestone_filter(&mut self, pill: Option<String>) {
        if !self.sidebar.milestone_filter.remove(&pill) {
            self.sidebar.milestone_filter.insert(pill);
        }
        self.persist_prefs();
    }

    /// Rows of the Labels pane: milestones first, then labels. Shares the
    /// Format dialog's lists, so both views show the same filter state.
    pub(crate) fn filter_items(&self) -> Vec<FilterItem> {
        self.milestone_pills().into_iter().map(FilterItem::Milestone)
            .chain(self.sidebar.labels.iter().cloned().map(FilterItem::Label))
            .collect()
    }

    pub(crate) fn filter_item_on(&self, item: &FilterItem) -> bool {
        match item {
            FilterItem::Milestone(pill) => self.sidebar.milestone_filter.contains(pill),
            FilterItem::Label(name) => self.sidebar.label_filter.contains(name),
        }
    }

    pub(crate) fn toggle_filter_item(&mut self, index: usize) {
        match self.filter_items().into_iter().nth(index) {
            Some(FilterItem::Milestone(pill)) => self.toggle_milestone_filter(pill),
            Some(FilterItem::Label(name)) => self.toggle_label_filter(name),
            None => return,
        }
        self.labels.cursor = index;
    }

    /// Open GitHub's issue search for the Labels row under the cursor.
    pub(crate) fn open_filter_item(&mut self) {
        let Some(item) = self.filter_items().into_iter().nth(self.labels.cursor) else { return };
        let Some(repo) = self.session.repo_flag.clone().or_else(|| self.session.repo_slug.clone()) else {
            self.notify("no repo to open".to_string());
            return;
        };
        let query = match &item {
            FilterItem::Milestone(Some(title)) => format!("milestone:\"{title}\""),
            FilterItem::Milestone(None) => "no:milestone".to_string(),
            FilterItem::Label(name) => format!("label:\"{name}\""),
        };
        let url = format!("https://github.com/{repo}/issues?q={}", encode_query(&query));
        crate::app::open_in_browser(&url).ok();
        self.notify(format!("opened {query} in browser"));
    }

    /// Move the Labels cursor, clamped to the rows.
    pub(crate) fn move_filter_cursor(&mut self, delta: isize) {
        let last = self.filter_items().len().saturating_sub(1);
        self.labels.cursor = self.labels.cursor.saturating_add_signed(delta).min(last);
    }

    /// `d` on the Filters pane: show or hide done milestones.
    pub(crate) fn toggle_done_milestones(&mut self) {
        self.sidebar.hide_done_milestones = !self.sidebar.hide_done_milestones;
        self.move_filter_cursor(0);
        self.notify(if self.sidebar.hide_done_milestones { "done milestones hidden" } else { "done milestones shown" }.to_string());
        self.persist_prefs();
    }

    pub(crate) fn clear_filter_items(&mut self) {
        self.sidebar.label_filter.clear();
        self.sidebar.milestone_filter.clear();
        self.persist_prefs();
    }

    /// Flip one Row fields checkbox by its dialog row (Issues rows first,
    /// then PRs rows) and persist it. Row fields apply live.
    pub(crate) fn toggle_sidebar_row_field(&mut self, index: usize) {
        let issue_len = model::RowFields::ISSUE_ORDER.len();
        let done = if index < issue_len {
            self.sidebar.row_issue.toggle(false, index)
        } else {
            self.sidebar.row_pr.toggle(true, index - issue_len)
        };
        if done {
            self.persist_prefs();
        }
    }

    /// Pick one option of a Format dialog choice row, as a click does.
    /// Rows mirror the render order; the number keys already set the
    /// window directly, so a click does the same.
    pub(crate) fn select_format_option(&mut self, option: usize) {
        let left = self.format_left_widgets();
        let saved = self.sidebar.saved_queries.len();
        let focus = self.sidebar.format_focus;
        if focus == saved + 1 {
            if let Some(hours) = [1, 24, 168, 0].get(option).copied() {
                self.set_window(hours);
            }
            return;
        }
        match focus.checked_sub(left) {
            Some(0) => {
                let next = [model::GroupBy::None, model::GroupBy::Milestone][option.min(1)];
                if self.sidebar.group_by != next {
                    self.sidebar.group_by = next;
                    self.persist_prefs();
                }
            }
            Some(1) => {
                self.sidebar.sub_issue_tree = option == 1;
                self.persist_prefs();
            }
            Some(2) => {
                use core::SideSort as S;
                if let Some(sort) = [S::Default, S::Updated, S::Author, S::State, S::Label].get(option).copied() {
                    self.sidebar.sort = sort;
                    self.sidebar.sort_desc = sort.natural_desc();
                    self.persist_prefs();
                }
            }
            Some(3) if self.sidebar.sort_desc != (option == 1) => self.toggle_sort_dir(),
            _ => {}
        }
    }

    pub(crate) fn format_step(&mut self, delta: isize) {
        let left = self.format_left_widgets();
        let total = self.format_widgets();
        let (start, end) = if !self.sidebar.format_columns {
            (0, total)
        } else if self.sidebar.format_focus < left {
            (0, left)
        } else {
            (left, total)
        };
        self.sidebar.format_focus = start
            + (self.sidebar.format_focus as isize - start as isize + delta)
                .rem_euclid((end - start) as isize) as usize;
    }

    pub(crate) fn format_switch_column(&mut self) {
        if !self.sidebar.format_columns {
            return;
        }
        let left = self.format_left_widgets();
        let total = self.format_widgets();
        let range = if self.sidebar.format_focus < left { left..total } else { 0..left };
        let row = self.sidebar.format_rows.get(self.sidebar.format_focus).copied().unwrap_or(0);
        self.sidebar.format_focus = range.min_by_key(|&index| {
            self.sidebar.format_rows.get(index).copied().unwrap_or(0).abs_diff(row)
        }).unwrap_or(self.sidebar.format_focus);
    }

    /// Snapshot the Format dialog state to `prefs.json`. Silent until
    /// the startup prefs load has applied: pre-bootstrap input would
    /// otherwise write defaults over saved state. Board filters save
    /// only while the board is open: a fresh session starts with an
    /// empty in-memory filter that must not wipe the saved one.
    pub(crate) fn persist_prefs(&mut self) {
        if !self.prefs_loaded {
            return;
        }
        let mut prefs = github::load_prefs();
        prefs.saved_queries = self.sidebar.saved_queries.clone();
        prefs.show_sidebar = self.sidebar.show;
        prefs.show_right = self.refs.show;
        prefs.board_cards = self.board.fields;
        prefs.row_issue = self.sidebar.row_issue;
        prefs.row_pr = self.sidebar.row_pr;
        prefs.show_open_prs = self.sidebar.show_open_prs;
        prefs.show_merged_prs = self.sidebar.show_merged_prs;
        prefs.show_open_issues = self.sidebar.show_open_issues;
        prefs.show_closed_issues = self.sidebar.show_closed_issues;
        prefs.hide_done_milestones = self.sidebar.hide_done_milestones;
        prefs.merged_window_hours = self.session.sections.merged_window_hours;
        prefs.group_by = self.sidebar.group_by;
        prefs.sub_issue_tree = self.sidebar.sub_issue_tree;
        prefs.sort = self.sidebar.sort;
        prefs.sort_desc = self.sidebar.sort_desc;
        prefs.backfill_months = self.session.sections.backfill.months;
        if let Some(repo) = self.session.repo_key() {
            let key = github::prefs_repo_key(&repo);
            prefs.queries.insert(key.clone(), self.sidebar.query_active.clone());
            let mut labels: Vec<String> = self.sidebar.label_filter.iter().cloned().collect();
            labels.sort();
            let mut milestones: Vec<Option<String>> =
                self.sidebar.milestone_filter.iter().cloned().collect();
            milestones.sort();
            prefs.map_filters.insert(key.clone(), self.map.filter.clone());
            prefs.sidebar_filters.insert(
                key,
                github::SavedSidebarFilter { labels, milestones },
            );
        }
        if board_open(self) {
            let repo = self.session.repo_flag.clone().or_else(|| self.session.repo_slug.clone());
            if let Some(owner) = github::board_owner(repo.as_deref()) {
                let mut milestones: Vec<Option<String>> =
                    self.board.milestones.iter().cloned().collect();
                milestones.sort();
                prefs.board_filters.insert(
                    github::prefs_owner_key(&owner),
                    github::SavedBoardFilter {
                        filter: self.board.filter.clone(),
                        milestones,
                    },
                );
            }
        }
        github::save_prefs(&prefs);
    }

    /// Apply the saved Format dialog state at startup. Per-repo and
    /// per-owner keys resolve from the repo the bootstrap just found.
    /// Marks the prefs load so later input may persist again.
    pub(super) fn apply_prefs(&mut self) {
        self.prefs_loaded = true;
        let prefs = github::load_prefs();
        self.sidebar.saved_queries = prefs.saved_queries.clone();
        self.sidebar.show = prefs.show_sidebar;
        self.refs.show = prefs.show_right;
        if (!self.sidebar.show && matches!(self.nav.pane, Pane::Issues | Pane::PRs))
            || (!self.refs.show && matches!(self.nav.pane, Pane::Refs | Pane::Labels | Pane::History))
        {
            self.nav.pane = Pane::Main;
        }
        self.sidebar.show_open_prs = prefs.show_open_prs;
        self.sidebar.show_merged_prs = prefs.show_merged_prs;
        self.sidebar.show_open_issues = prefs.show_open_issues;
        self.sidebar.show_closed_issues = prefs.show_closed_issues;
        self.sidebar.hide_done_milestones = prefs.hide_done_milestones;
        self.session.sections.merged_window_hours = prefs.merged_window_hours;
        self.session.sections.backfill.months = prefs.backfill_months;
        self.sidebar.row_issue = prefs.row_issue;
        self.sidebar.row_pr = prefs.row_pr;
        self.sidebar.group_by = prefs.group_by;
        self.sidebar.sub_issue_tree = prefs.sub_issue_tree;
        self.sidebar.sort = prefs.sort;
        self.sidebar.sort_desc = prefs.sort_desc;
        self.board.fields = prefs.board_cards;
        self.map.layout.physics = github::load_physics();
        if let Some(key) = self.session.repo_key() {
            let repo_key = github::prefs_repo_key(&key);
            if let Some(q) = prefs.queries.get(&repo_key) {
                self.sidebar.query_active = q.clone();
            }
            if let Some(filter) = prefs.map_filters.get(&repo_key) {
                self.map.filter = filter.clone();
            }
            if let Some(saved) = prefs.sidebar_filters.get(&repo_key) {
                self.sidebar.label_filter = saved.labels.iter().cloned().collect();
                self.sidebar.milestone_filter = saved.milestones.iter().cloned().collect();
            }
        }
    }
}

/// Percent-encodes a search query for a URL: unreserved bytes stay, the
/// rest become `%XX`.
pub(crate) fn encode_query(query: &str) -> String {
    query.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}
