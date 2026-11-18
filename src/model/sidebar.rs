//! Sidebar rows: row kinds, grouping, relative dates and row lines.

use super::*;

/// A sidebar row: an issue/PR number, or a divider labeled with the
/// section that follows it (`requested`, `recent`, or a milestone group).
/// The leading pinned section needs no divider since its rows carry 📌.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideRow {
    Item(u64),
    /// Sub-issue tree row: `guide` is the ready-to-draw connector column.
    /// `outside` marks a child the current query did not fetch.
    Tree {
        number: u64,
        depth: usize,
        guide: String,
        has_children: bool,
        folded: bool,
        outside: bool,
    },
    /// Child row standing in for a REST sub-issue fetch still in flight.
    Loading { guide: String },
    Divider(String),
}

impl SideRow {
    pub fn number(&self) -> Option<u64> {
        match self {
            SideRow::Item(n) | SideRow::Tree { number: n, .. } => Some(*n),
            SideRow::Loading { .. } | SideRow::Divider(_) => None,
        }
    }
}

/// Per-parent REST sub-issue state for the opt-in sidebar tree. Only
/// parents whose summary reports children the fetched rows do not cover
/// are ever requested; a parent holds one state for the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LazyChildren {
    Loading,
    Loaded(Vec<u64>),
}

/// Display grouping for the sidebar's recent rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    #[default]
    None,
    Milestone,
}

impl GroupBy {
    /// Space in the Format dialog cycles the two modes.
    pub fn next(self) -> Self {
        match self {
            GroupBy::None => GroupBy::Milestone,
            GroupBy::Milestone => GroupBy::None,
        }
    }
}

/// Relative updated time for a sidebar row: under 60 min → `N min ago`,
/// under 24 h → `N hr ago`, under 30 days → `N day ago`, under 12 months
/// (30-day months) → `N m ago`, else `mm/dd/yy`. Future stamps clamp to
/// `0 min ago`; unparseable stamps collapse the segment.
pub fn relative_updated(updated_at: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    use chrono::Datelike;
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(updated_at) else {
        return String::new();
    };
    let stamp = parsed.with_timezone(&chrono::Utc);
    let secs = now.signed_duration_since(stamp).num_seconds().max(0);
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const MONTH: i64 = 30 * DAY;
    if secs < HOUR {
        format!("{} min ago", secs / MIN)
    } else if secs < DAY {
        format!("{} hr ago", secs / HOUR)
    } else if secs < MONTH {
        format!("{} day ago", secs / DAY)
    } else if secs < 12 * MONTH {
        format!("{} m ago", secs / MONTH)
    } else {
        format!("{:02}/{:02}/{:02}", stamp.month(), stamp.day(), stamp.year().rem_euclid(100))
    }
}

/// Sidebar row: `#n title` on line one; line two is up to five `·`
/// separated segments — label chips, relative updated time, project
/// status, author, and milestone. Returns `(first, labels, updated,
/// status, author, milestone)`; a field the Format dialog turned off, or
/// an empty value, collapses its segment. Labels truncate first so the
/// other segments always fit. The status segment comes back separately
/// so the UI can paint it in the board color with the board glyph.
pub fn sidebar_lines(
    issue: &IssueData,
    statuses: &[(String, Option<String>)],
    now: chrono::DateTime<chrono::Utc>,
    width: usize,
    pinned: bool,
    fields: &RowFields,
) -> (String, String, String, String, String, String) {
    let first = format!(
        "#{}{} {}",
        issue.number,
        if pinned { " 📌" } else { "" },
        slug(&issue.title, 30.min(width))
    );
    let plain_status = if fields.status { status_text(statuses) } else { String::new() };
    let status = if plain_status.is_empty() {
        String::new()
    } else {
        let icon = crate::theme::column_icon(&plain_status);
        if icon.is_empty() { plain_status } else { format!("{icon} {plain_status}") }
    };
    let updated = if fields.updated {
        relative_updated(&issue.updated_at, now)
    } else {
        String::new()
    };
    let author = if fields.author { issue.author.login.clone() } else { String::new() };
    let milestone = if fields.milestone {
        issue.milestone.clone().unwrap_or_default()
    } else {
        String::new()
    };
    // Fixed cost: every other non-empty segment plus one ` · ` separator
    // per segment. Labels pay the same separator when one follows them.
    let trailing = [&updated, &status, &author, &milestone];
    let fixed: usize = trailing.iter().map(|s| s.chars().count()).sum();
    let room = width
        .saturating_sub(fixed)
        .saturating_sub(3 * trailing.iter().filter(|s| !s.is_empty()).count());
    let mut labels_text = String::new();
    if fields.labels {
        let chips: Vec<String> = issue.labels.iter().map(|l| format!("● {}", l.name)).collect();
        let mut used = 0;
        for (i, chip) in chips.iter().enumerate() {
            let add = chip.chars().count() + if i > 0 { 2 } else { 0 };
            if used + add > room {
                if i == 0 {
                    let tail = format!("+{} labels", chips.len());
                    if tail.chars().count() <= room {
                        labels_text = tail;
                    }
                }
                break;
            }
            if i > 0 {
                labels_text.push_str("  ");
            }
            labels_text.push_str(chip);
            used += add;
        }
    }
    (first, labels_text, updated, status, author, milestone)
}
