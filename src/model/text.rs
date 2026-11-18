//! Issue meta spans and markdown text for the detail pane.

use super::*;

/// Semantic role of a meta-line segment; the UI maps each to a theme color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaKind {
    Dim,
    State,
    Label,
    Milestone,
    Normal,
    Project,
    Url,
}

/// The `by … · labels: … · project: …` line as styled segments.
/// `meta_line` is the plain-text join of these; both stay in sync by construction.
pub fn meta_spans(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<(MetaKind, String)> {
    let labels = if d.labels.is_empty() {
        "none".to_string()
    } else {
        d.labels.iter().map(|x| x.name.clone()).collect::<Vec<_>>().join(", ")
    };
    let people = if d.assignees.is_empty() {
        "none".to_string()
    } else {
        d.assignees.iter().map(|x| x.login.clone()).collect::<Vec<_>>().join(", ")
    };
    let kind = if d.is_pr { "PR" } else { "issue" };
    let opened = d.created_at.get(..10).unwrap_or(&d.created_at).to_string();
    let project = {
        let s = status_text(statuses);
        if s.is_empty() { "—".to_string() } else { s }
    };
    vec![
        (MetaKind::Dim, format!("{kind} · ")),
        (MetaKind::State, d.state.clone()),
        (MetaKind::Dim, format!(" · by {} · opened {opened} · labels: ", d.author.login)),
        (MetaKind::Label, labels),
        (MetaKind::Dim, " · milestone: ".to_string()),
        (MetaKind::Milestone, d.milestone.clone().unwrap_or_else(|| "—".to_string())),
        (MetaKind::Dim, " · assignees: ".to_string()),
        (MetaKind::Normal, people),
        (MetaKind::Dim, " · project: ".to_string()),
        (MetaKind::Project, project),
        (MetaKind::Dim, "\n\n".to_string()),
        (MetaKind::Url, d.url.clone()),
    ]
}

/// Body markdown without the title/meta header (the header renders styled).
pub fn body_md(d: &IssueData) -> String {
    let raw = d.body.trim();
    linkify(if raw.is_empty() { "(no description)" } else { raw }).to_string()
}

pub fn status_text(statuses: &[(String, Option<String>)]) -> String {
    statuses
        .iter()
        .take(2)
        .map(|(_, s)| s.clone().unwrap_or_else(|| "—".to_string()))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn build_comments_md(d: &IssueData) -> String {
    let n = d.comments.len();
    let mut out = format!("_{n} comment{}_\n", if n == 1 { "" } else { "s" });
    if d.comments.is_empty() {
        out.push_str("_No comments yet._\n");
        return out;
    }
    for c in &d.comments {
        let when = c.created_at.get(..10).unwrap_or(&c.created_at);
        out.push_str(&format!(
            "---\n\n**{}** · {}\n\n{}\n",
            c.author.login,
            when,
            linkify(c.body.trim())
        ));
    }
    out
}
