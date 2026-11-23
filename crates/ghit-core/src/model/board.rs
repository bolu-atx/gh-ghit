//! Board items, card and row field settings, and card text helpers.

use super::*;
use crate::model::ItemRef;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub number: Option<ItemRef>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub repo: Option<String>,
    /// Milestone title, when the card carries one. Defaulted so old
    /// board caches without the field still parse.
    #[serde(default)]
    pub milestone: Option<String>,
    #[serde(default)]
    pub content: serde_json::Value,
}

impl BoardItem {
    /// An issue or PR as a board card, so other views share the card text
    /// helpers. `status` is its project status, when it has one.
    pub fn from_issue(issue: &IssueData, status: Option<&str>) -> Self {
        let logins: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        BoardItem {
            title: issue.title.clone(),
            number: Some(issue.number),
            status: status.map(str::to_string),
            labels: issue.labels.clone(),
            repo: None,
            milestone: issue.milestone.clone(),
            content: serde_json::json!({
                "type": if issue.is_pr { "PullRequest" } else { "Issue" },
                "url": issue.url,
                "assignees": logins,
            }),
        }
    }

    /// The card's web URL, when its content carries one.
    pub fn url(&self) -> Option<String> {
        self.content.get("url").and_then(|u| u.as_str()).map(|s| s.to_string())
    }
}

/// Per-card detail fetched in the background: dates plus comment count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoardDetail {
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub comments: usize,
}

impl BoardDetail {
    pub fn from_issue(d: &IssueData) -> Self {
        Self {
            created: d.created_at.get(..10).unwrap_or("").to_string(),
            updated: d.updated_at.get(..10).unwrap_or("").to_string(),
            comments: d.comments.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardData {
    pub title: String,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub items: Vec<BoardItem>,
    /// Project page URL from the board query, or the
    /// `orgs/{owner}/projects/{number}` fallback. Defaulted so old
    /// board caches without the field still parse.
    #[serde(default)]
    pub url: String,
}

/// Collapsed-card fields toggled in the board Format dialog. Defaults
/// reproduce the shipped card: number, labels, dates, and comment count
/// on, everything else off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFields {
    #[serde(default = "default_true")]
    pub number: bool,
    #[serde(default = "default_true")]
    pub labels: bool,
    #[serde(default)]
    pub milestone: bool,
    #[serde(default)]
    pub status: bool,
    #[serde(default)]
    pub assignee: bool,
    #[serde(default)]
    pub pr_state: bool,
    #[serde(default = "default_true")]
    pub dates: bool,
    #[serde(default = "default_true")]
    pub comments: bool,
}

fn default_true() -> bool {
    true
}

impl Default for CardFields {
    fn default() -> Self {
        Self { number: true, labels: true, milestone: false, status: false,
            assignee: false, pr_state: false, dates: true, comments: true }
    }
}

impl CardFields {
    /// Checkbox rows in dialog order. `toggle` and `shown` share this
    /// order so the renderer and input never drift apart.
    pub const ORDER: [&'static str; 8] = [
        "number", "labels", "milestone", "status", "assignee", "pr state", "dates",
        "comments",
    ];

    /// Value of the checkbox at `index`, or `None` past the dialog rows.
    pub fn shown(&self, index: usize) -> Option<bool> {
        match index {
            0 => Some(self.number),
            1 => Some(self.labels),
            2 => Some(self.milestone),
            3 => Some(self.status),
            4 => Some(self.assignee),
            5 => Some(self.pr_state),
            6 => Some(self.dates),
            7 => Some(self.comments),
            _ => None,
        }
    }

    /// Flip the checkbox at `index`. False past the dialog rows.
    pub fn toggle(&mut self, index: usize) -> bool {
        let Some(on) = self.shown(index) else { return false };
        match index {
            0 => self.number = !on,
            1 => self.labels = !on,
            2 => self.milestone = !on,
            3 => self.status = !on,
            4 => self.assignee = !on,
            5 => self.pr_state = !on,
            6 => self.dates = !on,
            7 => self.comments = !on,
            _ => return false,
        }
        true
    }
}

/// Sidebar-row fields toggled in the Format dialog, one set per pane.
/// Defaults reproduce the shipped rows: labels, updated age, status, and
/// the PR check glyph on; author and milestone off. `checks` only applies
/// to PR rows; the Issues pane ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowFields {
    #[serde(default = "default_true")]
    pub labels: bool,
    #[serde(default = "default_true")]
    pub updated: bool,
    #[serde(default = "default_true")]
    pub status: bool,
    #[serde(default)]
    pub author: bool,
    #[serde(default)]
    pub milestone: bool,
    #[serde(default = "default_true")]
    pub checks: bool,
}

impl Default for RowFields {
    fn default() -> Self {
        Self { labels: true, updated: true, status: true, author: false,
            milestone: false, checks: true }
    }
}

impl RowFields {
    /// Checkbox rows for the Issues pane, in dialog order.
    pub const ISSUE_ORDER: [&'static str; 5] =
        ["labels", "updated age", "status", "author", "milestone"];

    /// Checkbox rows for the PRs pane, in dialog order.
    pub const PR_ORDER: [&'static str; 6] =
        ["labels", "updated age", "status", "author", "milestone", "ci checks"];

    /// Value of the checkbox at `index` within one pane's rows, or `None`
    /// past them. `pr` selects the PRs pane order, else the Issues order.
    pub fn shown(&self, pr: bool, index: usize) -> Option<bool> {
        if pr {
            match index {
                0 => Some(self.labels),
                1 => Some(self.updated),
                2 => Some(self.status),
                3 => Some(self.author),
                4 => Some(self.milestone),
                5 => Some(self.checks),
                _ => None,
            }
        } else {
            match index {
                0 => Some(self.labels),
                1 => Some(self.updated),
                2 => Some(self.status),
                3 => Some(self.author),
                4 => Some(self.milestone),
                _ => None,
            }
        }
    }

    /// Flip the checkbox at `index` within one pane's rows. False past
    /// the pane rows.
    pub fn toggle(&mut self, pr: bool, index: usize) -> bool {
        let Some(on) = self.shown(pr, index) else { return false };
        let slot = if pr {
            match index {
                0 => &mut self.labels,
                1 => &mut self.updated,
                2 => &mut self.status,
                3 => &mut self.author,
                4 => &mut self.milestone,
                5 => &mut self.checks,
                _ => return false,
            }
        } else {
            match index {
                0 => &mut self.labels,
                1 => &mut self.updated,
                2 => &mut self.status,
                3 => &mut self.author,
                4 => &mut self.milestone,
                _ => return false,
            }
        };
        *slot = !on;
        true
    }
}

/// Collapsed card footer, left side: label chips, or a dash when bare.
pub fn card_footer_labels(c: &BoardItem) -> String {
    if c.labels.is_empty() {
        return "–".to_string();
    }
    let chips: Vec<String> = c.labels.iter().take(4).map(|l| format!("● {}", l.name)).collect();
    if c.labels.len() > 4 {
        format!("{}  +{}", chips.join("  "), c.labels.len() - 4)
    } else {
        chips.join("  ")
    }
}

/// Collapsed footer, right side, honoring the dates and comment count
/// fields. Both on reproduces `card_footer_date` exactly; either off
/// drops its half; both off draws nothing.
pub fn card_footer_right(detail: Option<&BoardDetail>, fields: &CardFields) -> String {
    if !fields.dates && !fields.comments {
        return String::new();
    }
    match detail {
        None => "…".to_string(),
        Some(d) => {
            let day = d.updated.get(..10).unwrap_or(&d.updated).to_string();
            let comments = if d.comments == 0 {
                "no comments".to_string()
            } else {
                format!("{} comment{}", d.comments, if d.comments == 1 { "" } else { "s" })
            };
            match (fields.comments, fields.dates) {
                (true, true) => format!("{comments} · {day}"),
                (true, false) => comments,
                (false, true) => day,
                (false, false) => String::new(),
            }
        }
    }
}

/// Draft vs ready label for a PR card, from the slimmed `content` JSON.
/// Issues and draft-issue cards show nothing. A warm cache entry that
/// predates `isDraft` shows nothing too, never a guessed "ready".
pub fn card_pr_state(c: &BoardItem) -> Option<&'static str> {
    if c.content.get("type").and_then(|t| t.as_str()) != Some("PullRequest") {
        return None;
    }
    let draft = c.content.get("isDraft").and_then(|v| v.as_bool())?;
    Some(if draft { "draft" } else { "ready" })
}

/// First assignee login off the card payload, when the board ships one.
pub fn card_assignee(c: &BoardItem) -> Option<String> {
    let arr = c.content.get("assignees").and_then(|v| v.as_array())?;
    arr.iter()
        .filter_map(|a| a.get("login").and_then(|l| l.as_str()).or_else(|| a.as_str()))
        .map(str::to_string)
        .next()
}

/// Second collapsed-card line from the enabled extra fields: milestone,
/// swimlane status, assignee, and PR draft state. Empty or disabled
/// segments drop out; nothing to show renders blank like today. `room`
/// is the usable width past the indent, so the line never wraps.
pub fn card_meta(c: &BoardItem, fields: &CardFields, room: usize) -> String {
    let mut segs: Vec<String> = vec![];
    if fields.milestone {
        if let Some(m) = c.milestone.as_deref() {
            segs.push(m.to_string());
        }
    }
    if fields.status {
        if let Some(s) = c.status.as_deref() {
            segs.push(s.to_string());
        }
    }
    if fields.assignee {
        if let Some(login) = card_assignee(c) {
            segs.push(format!("@{login}"));
        }
    }
    if fields.pr_state {
        if let Some(state) = card_pr_state(c) {
            segs.push(state.to_string());
        }
    }
    let joined = segs.join(" · ");
    if joined.chars().count() > room && room > 1 {
        format!("{}…", joined.chars().take(room.saturating_sub(1)).collect::<String>())
    } else {
        joined
    }
}

/// Group cards by status column; unknowns go to an `Other` column.
pub fn board_columns<'a>(items: &'a [BoardItem], order: &[String]) -> Vec<(String, Vec<&'a BoardItem>)> {
    let mut cols: Vec<(String, Vec<&'a BoardItem>)> = order.iter().map(|s| (s.clone(), vec![])).collect();
    let mut other: Vec<&'a BoardItem> = vec![];
    for it in items {
        let st = it.status.clone().unwrap_or_else(|| "No status".to_string());
        match cols.iter_mut().find(|(name, _)| *name == st) {
            Some((_, lst)) => lst.push(it),
            None => other.push(it),
        }
    }
    if !other.is_empty() {
        cols.push(("Other".to_string(), other));
    }
    cols
}

/// Distinct milestones across board cards, sorted, with `None` last when
/// some card carries no milestone. The format dialog offers one pill
/// per entry; an empty board yields no pills.
pub fn board_milestones(items: &[BoardItem]) -> Vec<Option<String>> {
    let mut names: Vec<String> = vec![];
    let mut has_none = false;
    for it in items {
        match &it.milestone {
            Some(m) => {
                if !names.iter().any(|n| n == m) {
                    names.push(m.clone());
                }
            }
            None => has_none = true,
        }
    }
    names.sort();
    let mut out: Vec<Option<String>> = names.into_iter().map(Some).collect();
    if has_none {
        out.push(None);
    }
    out
}
