//! Model: shared issue/comment/review types. One submodule per concern holds
//! the rest; the items callers use are re-exported here.

#[allow(dead_code)]
mod adf;
mod board;
mod diff;
mod graph;
mod issue;
mod links;
pub mod physics;
mod rank;
mod sidebar;
mod text;

#[cfg(test)]
mod adf_tests;
#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod issue_tests;
#[cfg(test)]
mod links_tests;
#[cfg(test)]
mod physics_tests;
#[cfg(test)]
mod rank_tests;
#[cfg(test)]
mod sidebar_tests;
#[cfg(test)]
mod text_tests;
#[cfg(test)]
mod test_support;

use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
pub use adf::adf_to_markdown;
pub use board::{BoardData, BoardDetail, BoardItem, CardFields, RowFields, board_columns, board_milestones, card_assignee, card_footer_labels, card_footer_right, card_meta};
pub use diff::{CONTEXT_STEPS, DiffFile, DiffKind, SplitRow, diff_lines, diff_summary, header_file_index, line_target_at_cursor, patch_line_numbers, split_rows, strip_diff_gutter, thread_at_cursor, thread_patch_row, widen_patch};
pub use graph::{LinkKind, MapFilter, MapGraph, MapItem, MapLink, MapNode, MapScope, NodeState, build_map};
#[cfg(test)]
pub use graph::hub_id;
pub use rank::{Ranking, rank};
pub use issue::{merge_comment_pages, paired_refs, parse_check_state, parse_review_threads, thread_reply_comment_id};
pub use links::{LinkTarget, link_ranges, linkify, links_in_line, nearest_link, referenced_numbers};
pub use sidebar::{GroupBy, LazyChildren, SideRow, sidebar_lines};
pub use text::{MetaKind, body_md, build_comments_md, meta_spans, status_text};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: String,
}

fn default_review_comment_state() -> String {
    "SUBMITTED".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    /// Numeric REST id (`databaseId` in GraphQL). A reply posts through
    /// REST `in_reply_to`, which needs this, not the GraphQL node id.
    /// Defaulted so old file caches without it still parse.
    #[serde(default)]
    pub database_id: Option<u64>,
    #[serde(default)]
    pub author: Option<Author>,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_review_comment_state")]
    pub state: String,
}

impl ReviewComment {
    pub fn is_pending(&self) -> bool {
        self.state.eq_ignore_ascii_case("PENDING")
    }
}

impl ReviewThread {
    pub fn has_pending(&self) -> bool {
        self.comments.iter().any(|c| c.is_pending())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub id: String,
    pub path: String,
    pub line: Option<u64>,
    pub is_resolved: bool,
    pub comments: Vec<ReviewComment>,
}

/// A will-close counterpart of an issue or PR: closing PRs for an issue,
/// closing issues for a PR. `state` is the GraphQL state (`OPEN`,
/// `MERGED`, `CLOSED`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedRef {
    pub number: u64,
    #[serde(default)]
    pub state: String,
}

/// Rollup CI check state of a PR's head commit, read from
/// `statusCheckRollup` on the sections query. `None` on the row means
/// the PR runs no checks, so the UI shows nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckState {
    Pass,
    Fail,
    Pending,
    /// Unknown future states parse here so new GraphQL values never fail
    /// the row; the UI treats it like no checks.
    #[default]
    Unknown,
}

/// Sub-issue counts for a parent issue, from GraphQL `subIssuesSummary`
/// or REST `sub_issues_summary`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubSummary {
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub completed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueData {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub assignees: Vec<Author>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub is_pr: bool,
    #[serde(default)]
    pub review_threads: Vec<ReviewThread>,
    /// Will-close counterparts, not body mentions. Defaulted so old file
    /// caches without the field still parse.
    #[serde(default)]
    pub paired: Vec<PairedRef>,
    /// Head-commit CI rollup for PRs (`None` = no checks). Defaulted so
    /// old file caches without the field still parse.
    #[serde(default)]
    pub checks: Option<CheckState>,
    /// Milestone title, when the issue belongs to one. REST ships
    /// `milestone.title`; GraphQL rows add `milestone { title }`.
    /// Defaulted so old file caches without the field still parse.
    #[serde(default)]
    pub milestone: Option<String>,
    /// Parent issue number when this item is a sub-issue. GraphQL
    /// sections fill it; REST search has no parent link. Defaulted so
    /// old file caches without the field still parse.
    #[serde(default)]
    pub parent: Option<u64>,
    /// Sub-issue counts for a parent issue. Defaulted so old file caches
    /// without the field still parse.
    #[serde(default)]
    pub sub_summary: Option<SubSummary>,
    /// Blocking and closing links into this issue, from the GraphQL
    /// listing; the store records them as edges.
    #[serde(default)]
    pub links: IssueLinks,
}

/// Links that point at an issue and show on its own row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLinks {
    /// Issues that block this one.
    pub blocked_by: Vec<u64>,
    /// Pull requests that close this issue.
    pub closed_by: Vec<u64>,
}

pub fn slug(title: &str, width: usize) -> String {
    let s = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= width {
        s
    } else {
        let cut: String = s.chars().take(width - 1).collect();
        format!("{cut}…")
    }
}

/// One repository milestone from the REST milestones listing. `due_on`
/// orders the sidebar's milestone groups.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
    #[serde(default)]
    pub due_on: Option<String>,
    /// Open and closed item counts from the REST listing.
    #[serde(default)]
    pub open_items: u32,
    #[serde(default)]
    pub closed_items: u32,
    /// The milestone itself is closed on GitHub.
    #[serde(default)]
    pub closed: bool,
}

impl Milestone {
    /// Share of items closed, 0..=100. `None` with no items.
    pub fn percent(&self) -> Option<u32> {
        let total = self.open_items + self.closed_items;
        (total > 0).then(|| self.closed_items * 100 / total)
    }

    /// Closed, or every item in it is closed.
    pub fn done(&self) -> bool {
        self.closed || (self.open_items == 0 && self.closed_items > 0)
    }
}
