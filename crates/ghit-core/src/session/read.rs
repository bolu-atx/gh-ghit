//! Read accessors: the session state a front end shows, named after what
//! it means rather than the field that holds it.

use std::collections::{HashMap, HashSet};

use super::Session;
use crate::model::{BoardData, BoardDetail, DiffFile, IssueData, ItemRef, LazyChildren};

impl Session {
    /// The Jira connection for a Jira session, `None` for GitHub.
    pub fn jira_info(&self) -> Option<crate::backend::JiraInfo> {
        self.issue_backend.jira_info()
    }

    /// How a list shows `n`: `#N` for GitHub, `<KEY>-N` for Jira.
    pub fn label(&self, n: ItemRef) -> String {
        n.display(&self.issue_backend.project_key().to_ascii_uppercase())
    }

    /// Every cached item payload, listing rows included.
    pub fn items(&self) -> &HashMap<ItemRef, IssueData> {
        &self.issues.data
    }

    /// The cached payload of `n`, full or a listing row.
    pub fn item(&self, n: ItemRef) -> Option<&IssueData> {
        self.issues.data.get(&n)
    }

    /// A payload for `n` is cached; it may still want the full fetch.
    pub fn has_item(&self, n: ItemRef) -> bool {
        self.issues.data.contains_key(&n)
    }

    /// `n` came from a listing and still wants its full fetch.
    pub fn is_partial(&self, n: ItemRef) -> bool {
        self.issues.partial.contains(&n)
    }

    /// The listing rows that still want their full fetch.
    pub fn partial_items(&self) -> impl Iterator<Item = ItemRef> + '_ {
        self.issues.partial.iter().copied()
    }

    /// A full fetch for `n` is in flight.
    pub fn is_pending(&self, n: ItemRef) -> bool {
        self.issues.pending.contains(&n)
    }

    /// Full fetches in flight.
    pub fn pending_count(&self) -> usize {
        self.issues.pending.len()
    }

    /// The statuses `n` carries as its listing read them.
    pub fn statuses(&self, n: ItemRef) -> Option<&[(String, Option<String>)]> {
        self.issues.statuses.get(&n).map(Vec::as_slice)
    }

    /// Comment pages done vs total across the full fetches, when any is
    /// known. Concurrent fetches report separately, so the counts add up.
    pub fn comment_progress(&self) -> Option<(usize, usize)> {
        if self.issues.progress.is_empty() {
            return None;
        }
        Some(
            self.issues
                .progress
                .values()
                .copied()
                .fold((0, 0), |(d, t), (dd, tt)| (d + dd, t + tt)),
        )
    }

    /// PR `n`'s diff files, widened to the front end's context setting.
    pub fn diff_files(&self, n: ItemRef) -> Option<&[DiffFile]> {
        self.diff.files.get(&n).map(Vec::as_slice)
    }

    /// A diff for `n` is cached.
    pub fn has_diff(&self, n: ItemRef) -> bool {
        self.diff.files.contains_key(&n)
    }

    /// A diff fetch for `n` is in flight.
    pub fn diff_pending(&self, n: ItemRef) -> bool {
        self.diff.pending.contains(&n)
    }

    /// Diff fetches in flight.
    pub fn diff_pending_count(&self) -> usize {
        self.diff.pending.len()
    }

    /// `n`'s patches exactly as GitHub sent them.
    pub fn raw_diff(&self, n: ItemRef) -> Option<&[DiffFile]> {
        self.diff.raw.get(&n).map(Vec::as_slice)
    }

    /// PRs whose raw patches are held, so a context change re-widens them.
    pub fn raw_diff_prs(&self) -> Vec<ItemRef> {
        self.diff.raw.keys().copied().collect()
    }

    /// The head-side lines of `name` in PR `n`, when the fetch landed.
    pub fn head_lines(&self, n: ItemRef, name: &str) -> Option<&[String]> {
        self.diff
            .heads
            .iter()
            .find(|((i, file), _)| *i == n && file == name)
            .and_then(|(_, lines)| lines.as_deref())
    }

    /// The head side of `name` in `n` was fetched, successfully or not.
    pub fn head_known(&self, n: ItemRef, name: &str) -> bool {
        self.diff
            .heads
            .keys()
            .any(|(i, file)| *i == n && file == name)
    }

    /// The head side of `name` in `n` is being fetched.
    pub fn head_pending(&self, n: ItemRef, name: &str) -> bool {
        self.diff
            .heads_pending
            .iter()
            .any(|(i, file)| *i == n && file == name)
    }

    /// The project board payload, once fetched.
    pub fn board(&self) -> Option<&BoardData> {
        self.board.data.as_ref()
    }

    /// The board fetch is in flight.
    pub fn board_loading(&self) -> bool {
        self.board.loading
    }

    /// The card detail of `n`, from a listing, an issue or a batch fetch.
    pub fn board_detail(&self, n: ItemRef) -> Option<&BoardDetail> {
        self.board.details.get(&n)
    }

    /// Card-detail fetches in flight.
    pub fn board_details_pending(&self) -> usize {
        self.board.detail_pending.len()
    }

    /// Pinned items in display order.
    pub fn pins(&self) -> &[ItemRef] {
        &self.pins.order
    }

    /// The pinned items, for membership tests.
    pub fn pinned(&self) -> &HashSet<ItemRef> {
        &self.pins.set
    }

    /// `n` is pinned.
    pub fn is_pinned(&self, n: ItemRef) -> bool {
        self.pins.set.contains(&n)
    }

    /// Items the open issue mentions, in body order.
    pub fn mentioned(&self) -> &[ItemRef] {
        &self.refs.numbers
    }

    /// Will-close counterparts of the open item, kept apart from mentions.
    pub fn paired(&self) -> &[ItemRef] {
        &self.refs.paired
    }

    /// A listing fetch is in flight.
    pub fn sections_busy(&self) -> bool {
        self.sections.rx.is_some()
    }

    /// Listing pages done vs the row cap, while pages remain.
    pub fn section_progress(&self) -> Option<(usize, usize)> {
        self.sections.progress
    }

    /// Age limit in hours of the merged/closed window; 0 is all time.
    pub fn merged_window_hours(&self) -> u64 {
        self.sections.merged_window_hours
    }

    /// Backfill age limit in months; 0 is all history.
    pub fn backfill_months(&self) -> u32 {
        self.sections.backfill.months
    }

    /// The startup repo resolution and auth probe are in flight.
    pub fn bootstrapping(&self) -> bool {
        self.auth.bootstrap_rx.is_some()
    }

    /// An auth problem was seen and no re-auth has fixed it yet.
    pub fn auth_degraded(&self) -> bool {
        self.auth.degraded
    }

    /// The user dismissed the re-auth question.
    pub fn auth_dismissed(&self) -> bool {
        self.auth.dismissed
    }

    /// Input asked for a re-auth that the loop has not run yet.
    pub fn reauth_requested(&self) -> bool {
        self.auth.reauth_requested
    }

    /// Sub-issue loads by parent: loading, or the loaded children.
    pub fn sub_states(&self) -> &HashMap<ItemRef, LazyChildren> {
        &self.subs.state
    }

    /// A review submit for `n` is in flight.
    pub fn review_submitting(&self, n: ItemRef) -> bool {
        self.sends.submitting.contains(&n)
    }

    /// The repo the session works on, in its original case: the flag wins
    /// over the checkout's remote.
    pub fn repo(&self) -> Option<&str> {
        self.repo_flag.as_deref().or(self.repo_slug.as_deref())
    }
}
