//! Issue loading, refs, pins, eviction, yank and open-in-browser.

use std::collections::HashSet;

use ratatui::widgets::ListState;

use crate::app::{App, Tab, Focus, NavIntent, UndoAction};
use ghit_core::core::{self, Pane};
use ghit_core::{github, model};
use ghit_core::model::ItemRef;

/// Refs pane view state: visibility, the highlight selection and list state.
pub(crate) struct RefsPane {
    /// Shows the pane. The related items live in `Session::refs`.
    pub(crate) show: bool,
    /// Click-selected refs row: highlights its source line instead of opening it.
    pub(crate) highlight: Option<ItemRef>,
    pub(crate) state: ListState,
}

impl RefsPane {
    pub(crate) fn new() -> Self {
        Self { show: true, highlight: None,
            state: ListState::default() }
    }
}

impl App {
    /// Drop background rows the user never opened, with their cached data.
    pub(super) fn drop_unvisited_background(&mut self) {
        let mut keep: HashSet<ItemRef> = self
            .nav
            .visited
            .iter()
            .copied()
            .chain(self.session.pins().iter().copied())
            .chain(self.session.pinned().iter().copied())
            .collect();
        // REST sub-issue rows are a session cache: keep them so the next
        // expand of their parent still renders titles.
        for state in self.session.sub_states().values() {
            if let model::LazyChildren::Loaded(numbers) = state {
                keep.extend(numbers.iter().copied());
            }
        }
        self.nav.recent.retain(|n| keep.contains(n));
        for n in self.session.partial_items().collect::<Vec<_>>() {
            if !keep.contains(&n) {
                self.drop_cached(n);
            }
        }
    }

    /// Remove every cached payload for `n` so a revisit fetches again.
    pub(super) fn drop_cached(&mut self, n: ItemRef) {
        self.session.drop_cached(n);
    }

    /// Evict the highlighted sidebar row: drop it from the pane lists and
    /// every cached payload, then put the cursor on the next row (falling
    /// back to the previous one at the end). Pinned rows resist until
    /// unpinned with `p`; a revisit refetches from the network.
    pub(crate) fn evict_current(&mut self) {
        if !self.guard_nav(NavIntent::Evict) {
            self.evict_current_inner();
        }
    }

    pub(super) fn evict_current_inner(&mut self) {
        if !matches!(self.nav.pane, Pane::Issues | Pane::PRs) {
            return;
        }
        let Some(n) = self.nav.current else {
            self.notify("no row to evict".to_string());
            return;
        };
        if self.session.is_pinned(n) || self.session.pins().contains(&n) {
            self.notify(format!(
                "{n} is pinned — unpin with p first",
                n = self.session.label(n)
            ));
            return;
        }
        let order = match self.nav.pane {
            Pane::Issues => self.issue_numbers(),
            Pane::PRs => self.pr_numbers(),
            _ => return,
        };
        let Some(pos) = order.iter().position(|x| *x == n) else { return };
        let action = UndoAction::Evict {
            number: n,
            visited_at: self.nav.visited.iter().position(|x| *x == n),
            recent_at: self.nav.recent.iter().position(|x| *x == n),
            data: self.session.item(n).cloned().map(Box::new),
            statuses: self.session.statuses(n).map(|s| s.to_vec()),
            row_expanded: self.diff.expanded_rows.contains(&n),
        };
        self.nav.visited.retain(|x| *x != n);
        self.nav.recent.retain(|x| *x != n);
        self.diff.expanded_rows.remove(&n);
        if self.refs.highlight == Some(n) {
            self.refs.highlight = None;
        }
        self.drop_cached(n);
        let next = order
            .get(pos + 1)
            .copied()
            .or_else(|| pos.checked_sub(1).and_then(|i| order.get(i)).copied());
        match next {
            Some(next) => self.show_issue(next),
            None => {
                self.nav.current = None;
                self.nav.requested = None;
                self.session.clear_refs();
                self.nav.scroll = 0;
                self.sync_selection();
            }
        }
        self.arm_undo(action, format!("removed {n} · u undo", n = self.session.label(n)));
    }

    pub(crate) fn load_issue(&mut self, number: ItemRef, force: bool) {
        self.nav.requested = Some(number);
        if !force && self.session.has_item(number) && !self.session.is_partial(number) {
            self.show_issue(number);
            self.bar.clear();
            self.nav.focus = Focus::Main;
            return;
        }
        self.queue_issue(number, force);
    }

    /// Queue the full fetch for `number` without opening it. No-op while
    /// one is already in flight.
    pub(super) fn queue_issue(&mut self, number: ItemRef, force: bool) {
        let view = self.fetch_view();
        self.session.load(number, force, &view);
    }

    /// Title subtitle for Refs: mention/pair counts (Refs has no sort).
    pub(crate) fn refs_subtitle(&self) -> String {
        if self.session.paired().is_empty() {
            format!("{} mentioned", self.session.mentioned().len())
        } else {
            format!(
                "{} mentioned · {} paired",
                self.session.mentioned().len(),
                self.session.paired().len()
            )
        }
    }

    /// Footer for Refs: per-group counts matching the subtitle.
    pub(crate) fn refs_footer(&self) -> String {
        format!(
            "mentioned ({}) · paired ({})",
            self.session.mentioned().len(),
            self.session.paired().len()
        )
    }

    /// Toggle the detail block on the focused pane's selected row. The set
    /// is keyed by issue number, so expansion sticks across cursor moves.
    /// The body has no row selection, so there the key is a no-op.
    pub(crate) fn toggle_row_expansion(&mut self) {
        let selected = match self.nav.pane {
            Pane::Issues => self.nav.current.filter(|n| self.issue_numbers().contains(n)),
            Pane::PRs => self.nav.current.filter(|n| self.pr_numbers().contains(n)),
            Pane::Refs => self.refs.highlight,
            Pane::Main | Pane::Labels | Pane::History => None,
        };
        let Some(n) = selected else { return };
        if !self.diff.expanded_rows.remove(&n) {
            self.diff.expanded_rows.insert(n);
        }
    }

    /// Display rows for the refs pane: mentioned numbers, then a `Paired`
    /// divider with the will-close counterparts. Empty sections collapse.
    pub(crate) fn refs_rows(&self) -> Vec<model::SideRow> {
        let mut rows: Vec<model::SideRow> = self.session.mentioned().iter().copied().map(model::SideRow::Item).collect();
        if !self.session.paired().is_empty() {
            rows.push(model::SideRow::Divider("Paired".to_string()));
            rows.extend(self.session.paired().iter().copied().map(model::SideRow::Item));
        }
        rows
    }

    /// j/k order for the refs pane, skipping the divider.
    pub(crate) fn ref_order(&self) -> Vec<ItemRef> {
        self.session.mentioned().iter().chain(self.session.paired().iter()).copied().collect()
    }

    /// Enter on a refs row: paired rows open like sidebar rows; mention
    /// rows stay highlight-only, so they are a no-op here.
    /// Enter in Refs: open the highlighted reference, paired or mention,
    /// and focus its content. Esc returns to Refs.
    pub(crate) fn open_selected_ref(&mut self) {
        let Some(n) = self.refs.highlight else { return };
        self.focus_content(Pane::Refs, Some(n));
        self.open_issue(n);
    }

    /// Flip the right refs pane, keeping the current issue in place.
    pub(crate) fn toggle_refs(&mut self) {
        self.refs.show = !self.refs.show;
        if !self.refs.show && matches!(self.nav.pane, Pane::Refs | Pane::Labels | Pane::History) {
            self.nav.pane = Pane::Main;
        }
        self.persist_prefs();
        self.notify(if self.refs.show { "right pane: shown" } else { "right pane: hidden" }.to_string());
    }

    /// Recompute refs from the current body and pairs, then queue
    /// background fetches for rows we have never loaded. Never changes
    /// `current`.
    pub(crate) fn refresh_refs(&mut self) {
        let Some(n) = self.nav.current else {
            self.session.clear_refs();
            return;
        };
        let Some(d) = self.session.item(n) else {
            self.session.clear_refs();
            return;
        };
        // Pairs win over mentions so each row has one behavior: the
        // pairing survives, and Enter on it opens the counterpart.
        let paired: Vec<ItemRef> = d
            .paired
            .iter()
            .map(|p| p.number)
            .filter(|m| *m != n)
            .collect();
        let repo = self.session.repo_key();
        let nums: Vec<ItemRef> = model::referenced_numbers(&d.body, repo.as_deref())
            .into_iter()
            .filter(|m| *m != n && !paired.contains(m))
            .collect();
        self.session.set_refs(nums, paired);
        for m in self.ref_order() {
            if !self.session.has_item(m) {
                self.queue_issue(m, false);
            }
        }
    }

    pub(crate) fn toggle_pin(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        match self.session.toggle_pin(n) {
            Ok(true) => self.notify(format!("pinned {n}", n = self.session.label(n))),
            Ok(false) => self.notify(format!("unpinned {n}", n = self.session.label(n))),
            Err(e) => {
                self.notify(e);
                return;
            }
        }
        self.show_issue(n);
    }

    pub(crate) fn refresh_current(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if let Some(owner) = github::board_owner(self.session.repo()) {
            github::invalidate(&[github::Key::Board { owner }]);
        }
        self.load_issue(n, true);
        self.load_diff(n, true);
        self.map.loaded = false;
        self.refresh_listing();
        self.notify(format!("refreshed {n}", n = self.session.label(n)));
    }

    /// A search listing refreshes from the server; a window listing
    /// re-walks every list (idempotent through the contains checks in polling).
    /// The walk takes the sync lease first, so a manual refresh works from
    /// any process; the old holder stops syncing on its next renewal.
    pub(crate) fn refresh_listing(&mut self) {
        if self.sidebar.query_active.trim().is_empty() {
            self.session.take_over_lease();
            self.session.request_full_listing();
            self.merge_recent_sections();
        } else if !self.reread_github(false) {
            let q = self.sidebar.query_active.clone();
            self.run_gh_search(q);
        }
    }

    /// Full yank text for the current context, without side effects.
    /// Refs highlight wins, then the Diff file cursor, else the current
    /// issue/PR. `None` means `yank` should explain instead of copying.
    pub(crate) fn yank_text(&self) -> Option<String> {
        let current = self.nav.current?;
        let d = self.session.item(current)?;
        let repo = self.session.repo_key();
        if self.nav.pane == Pane::Refs {
            let t = self.refs.highlight?;
            let target = self.session.item(t);
            if let Some(text) = core::yank_ref_text(
                current,
                t,
                &d.body,
                repo.as_deref(),
                target.map(|x| x.title.as_str()),
                target.map(|x| x.url.as_str()),
            ) {
                return Some(text);
            }
            if let Some(td) = target {
                return Some(core::yank_issue_text(t, &td.title, &td.url));
            }
            return Some(self.session.label(t));
        }
        if self.nav.tab == Tab::Visual && d.is_pr {
            let files = self.session.diff_files(current)?;
            if files.is_empty() {
                return None;
            }
            // A patch content row under the active line yanks that line;
            // headers, blanks, and rules fall back to the file summary.
            if let Some(abs) = self.diff.line {
                let vis = abs.saturating_sub(self.hits.body_base as usize);
                if let (Some(line), Some(fi)) =
                    (self.hits.body_rows.get(vis), self.diff_file_at_cursor())
                {
                    let is_header = line.starts_with("▸ ") || line.starts_with("▾ ");
                    let trimmed = line.trim();
                    let is_meta = trimmed.is_empty()
                        || trimmed.chars().all(|c| c == '─')
                        || trimmed.starts_with("… and");
                    if !is_header && !is_meta {
                        if let Some(file) = files.get(fi) {
                            return Some(core::yank_diff_line_text(
                                current,
                                &d.title,
                                &d.url,
                                &file.filename,
                                model::strip_diff_gutter(line),
                            ));
                        }
                    }
                }
            }
            let file = &files[self.diff.cursor.min(files.len() - 1)];
            let expanded = self.diff.expanded_files.contains(&(current, file.filename.clone()));
            let hunk = if expanded {
                core::first_hunk(&file.patch)
            } else {
                None
            };
            return Some(core::yank_diff_text(current, &d.title, &d.url, file, hunk.as_deref()));
        }
        Some(core::yank_issue_text(current, &d.title, &d.url))
    }

    /// Copy the context yank to the clipboard. Every path toasts: success
    /// shows the yanked text trimmed to fit, failures name the reason.
    pub(crate) fn yank(&mut self) {
        match self.yank_text() {
            Some(text) => {
                if crate::clipboard::copy(&text) {
                    self.notify(format!("yanked {}", core::toast_snippet(&text, 80)));
                } else {
                    self.notify("copy failed".to_string());
                }
            }
            None => {
                if self.nav.current.is_none() {
                    self.notify("view an issue first".to_string());
                } else if self.nav.pane == Pane::Refs {
                    self.notify("no ref selected".to_string());
                } else if self.nav.tab == Tab::Visual {
                    self.notify("no diff loaded yet".to_string());
                } else {
                    self.notify("nothing to yank".to_string());
                }
            }
        }
    }
}
