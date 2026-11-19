//! Issue loading, refs, pins, eviction, yank and open-in-browser.

use std::collections::{HashMap, HashSet};

use ratatui::widgets::ListState;

use crate::app::{App, Tab, Focus, NavIntent, UndoAction};
use crate::core::{self, Pane};
use crate::{github, model};
use crate::model::IssueData;

/// Issues owned by the app: fetched payloads plus the fetch channel
/// triplet (sender, receiver, in-flight set) and per-issue page progress.
pub(crate) struct IssueStore {
    pub(crate) data: HashMap<u64, IssueData>,
    pub(crate) statuses: HashMap<u64, Vec<(String, Option<String>)>>,
    pub(crate) partial: HashSet<u64>,
    pub(crate) tx: std::sync::mpsc::Sender<(u64, github::IssueEvent)>,
    pub(crate) rx: std::sync::mpsc::Receiver<(u64, github::IssueEvent)>,
    pub(crate) pending: HashSet<u64>,
    /// Comment-page progress per issue: pages done vs total. Cleared when
    /// the terminal result lands.
    pub(crate) progress: HashMap<u64, (usize, usize)>,
}

impl IssueStore {
    pub(crate) fn new(
        tx: std::sync::mpsc::Sender<(u64, github::IssueEvent)>,
        rx: std::sync::mpsc::Receiver<(u64, github::IssueEvent)>,
    ) -> Self {
        Self { data: HashMap::new(), statuses: HashMap::new(), partial: HashSet::new(),
            tx, rx, pending: HashSet::new(), progress: HashMap::new() }
    }
}

/// Refs pane: related issues mentioned in the current body, with its
/// highlight selection and list state.
pub(crate) struct RefsPane {
    /// Related issues mentioned in the current body. Hidden when empty.
    pub(crate) show: bool,
    pub(crate) numbers: Vec<u64>,
    /// Will-close counterparts of the current issue/PR, listed under a
    /// `Paired` divider. Kept disjoint from `numbers` so a row has one
    /// behavior: paired rows open, mention rows only highlight.
    pub(crate) paired: Vec<u64>,
    /// Click-selected refs row: highlights its source line instead of opening it.
    pub(crate) highlight: Option<u64>,
    pub(crate) state: ListState,
}

impl RefsPane {
    pub(crate) fn new() -> Self {
        Self { show: true, numbers: vec![], paired: vec![], highlight: None,
            state: ListState::default() }
    }
}

impl App {
    /// Drop background rows the user never opened, with their cached data.
    pub(super) fn drop_unvisited_background(&mut self) {
        let mut keep: HashSet<u64> = self.nav.visited.iter().copied()
            .chain(self.nav.pin_order.iter().copied())
            .chain(self.nav.pinned.iter().copied())
            .collect();
        // REST sub-issue rows are a session cache: keep them so the next
        // expand of their parent still renders titles.
        for state in self.subs.state.values() {
            if let model::LazyChildren::Loaded(numbers) = state {
                keep.extend(numbers.iter().copied());
            }
        }
        self.nav.recent.retain(|n| keep.contains(n));
        for n in self.issues.partial.iter().copied().collect::<Vec<_>>() {
            if !keep.contains(&n) {
                self.drop_cached(n);
            }
        }
    }

    /// Remove every cached payload for `n` so a revisit fetches again.
    pub(super) fn drop_cached(&mut self, n: u64) {
        self.issues.partial.remove(&n);
        self.issues.progress.remove(&n);
        self.issues.data.remove(&n);
        self.issues.statuses.remove(&n);
        self.board.details.remove(&n);
        self.diff.files.remove(&n);
        self.diff.pending.remove(&n);
        self.graph.data.remove(&n);
        self.graph.pending.remove(&n);
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
        if self.nav.pinned.contains(&n) || self.nav.pin_order.contains(&n) {
            self.notify(format!("#{n} is pinned — unpin with p first"));
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
            data: self.issues.data.get(&n).cloned().map(Box::new),
            statuses: self.issues.statuses.get(&n).cloned(),
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
                self.refs.numbers.clear();
                self.refs.paired.clear();
                self.nav.scroll = 0;
                self.sync_selection();
            }
        }
        self.arm_undo(action, format!("removed #{n} · u undo"));
    }

    pub(crate) fn load_issue(&mut self, number: u64, force: bool) {
        self.nav.requested = Some(number);
        if !force && self.issues.data.contains_key(&number) && !self.issues.partial.contains(&number) {
            self.show_issue(number);
            self.bar.clear();
            self.nav.focus = Focus::Main;
            return;
        }
        self.queue_issue(number, force);
    }

    /// Queue the full fetch for `number` without opening it. No-op while
    /// one is already in flight.
    pub(super) fn queue_issue(&mut self, number: u64, force: bool) {
        if !self.issues.pending.insert(number) { return; }
        let tx = self.issues.tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        self.enqueue(core::Kind::Details, Some(number), async move {
            let result = github::fetch_number(&repo, number, force, Some(tx.clone())).await;
            let _ = tx.send((number, github::IssueEvent::Done(result.map(Box::new))));
        });
    }

    /// Title subtitle for Refs: mention/pair counts (Refs has no sort).
    pub(crate) fn refs_subtitle(&self) -> String {
        if self.refs.paired.is_empty() {
            format!("{} mentioned", self.refs.numbers.len())
        } else {
            format!(
                "{} mentioned · {} paired",
                self.refs.numbers.len(),
                self.refs.paired.len()
            )
        }
    }

    /// Footer for Refs: per-group counts matching the subtitle.
    pub(crate) fn refs_footer(&self) -> String {
        format!(
            "mentioned ({}) · paired ({})",
            self.refs.numbers.len(),
            self.refs.paired.len()
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
        let mut rows: Vec<model::SideRow> = self.refs.numbers.iter().copied().map(model::SideRow::Item).collect();
        if !self.refs.paired.is_empty() {
            rows.push(model::SideRow::Divider("Paired".to_string()));
            rows.extend(self.refs.paired.iter().copied().map(model::SideRow::Item));
        }
        rows
    }

    /// j/k order for the refs pane, skipping the divider.
    pub(crate) fn ref_order(&self) -> Vec<u64> {
        self.refs.numbers.iter().chain(self.refs.paired.iter()).copied().collect()
    }

    /// Enter on a refs row: paired rows open like sidebar rows; mention
    /// rows stay highlight-only, so they are a no-op here.
    pub(crate) fn open_selected_ref(&mut self) {
        let Some(n) = self.refs.highlight else { return };
        if !self.refs.paired.contains(&n) {
            return;
        }
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
            self.refs.numbers.clear();
            self.refs.paired.clear();
            return;
        };
        let Some(d) = self.issues.data.get(&n) else {
            self.refs.numbers.clear();
            self.refs.paired.clear();
            return;
        };
        // Pairs win over mentions so each row has one behavior: the
        // pairing survives, and Enter on it opens the counterpart.
        let paired: Vec<u64> = d.paired.iter().map(|p| p.number).filter(|m| *m != n).collect();
        let repo = self.repo_key();
        let nums: Vec<u64> = model::referenced_numbers(&d.body, repo.as_deref())
            .into_iter()
            .filter(|m| *m != n && !paired.contains(m))
            .collect();
        self.refs.numbers = nums;
        self.refs.paired = paired;
        for m in self.ref_order() {
            if !self.issues.data.contains_key(&m) {
                self.queue_issue(m, false);
            }
        }
    }

    pub(crate) fn toggle_pin(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let Some(key) = self.repo_key() else {
            self.notify("pinning needs a repo: use -R or run inside one".to_string());
            return;
        };
        let loaded = github::load_pins(&key);
        let was_pinned = loaded.contains(&n);
        let pins = core::toggle_pin(&loaded, n);
        if was_pinned {
            self.nav.pinned.remove(&n);
            self.notify(format!("unpinned #{n}"));
        } else {
            self.nav.pinned.insert(n);
            self.notify(format!("pinned #{n}"));
        }
        self.nav.pin_order = pins.clone();
        github::save_pins(&key, &pins);
        self.show_issue(n);
    }

    pub(crate) fn refresh_current(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone());
        if let Some(owner) = github::board_owner(repo.as_deref()) {
            github::invalidate(&[github::Key::Board { owner }]);
        }
        self.load_issue(n, true);
        self.load_diff(n, true);
        self.load_graph(n, true);
        self.refresh_listing();
        self.notify(format!("refreshed #{n}"));
    }

    /// A search listing refreshes from the server; a window listing
    /// re-walks every list (idempotent through the contains checks in polling).
    /// The walk takes the sync lease first, so a manual refresh works from
    /// any process; the old holder stops syncing on its next renewal.
    pub(crate) fn refresh_listing(&mut self) {
        if self.sidebar.query_active.trim().is_empty() {
            if let Some(link) = self.sections.link.as_mut() {
                link.take_over();
            }
            self.sections.refresh = github::Refresh::Full;
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
        let d = self.issues.data.get(&current)?;
        let repo = self.repo_key();
        if self.nav.pane == Pane::Refs {
            let t = self.refs.highlight?;
            let target = self.issues.data.get(&t);
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
            return Some(format!("#{t}"));
        }
        if self.nav.tab == Tab::Visual && d.is_pr {
            let files = self.diff.files.get(&current)?;
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
