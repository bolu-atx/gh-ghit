use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub url: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub is_pr: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardItem {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub content: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardData {
    pub number: i64,
    pub title: String,
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub items: Vec<BoardItem>,
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

fn issue_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\(#(\d+)\)").unwrap())
}
fn bare_re() -> &'static Regex {
    // No look-behind in the `regex` crate: capture the preceding char instead.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(^|[^\w\[(])#(\d+)\b").unwrap())
}
fn code_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)(```.*?```|`[^`\n]+)").unwrap())
}

/// Turn `(#123)` and bare `#123` refs into `issue:` links, skipping code spans.
pub fn linkify(text: &str) -> String {
    let code = code_re();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for m in code.find_iter(text) {
        out.push_str(&linkify_plain(&text[last..m.start()]));
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&linkify_plain(&text[last..]));
    out
}

fn linkify_plain(seg: &str) -> String {
    // Issue refs stay plain `#123` text: the renderer prints link
    // destinations verbatim, and `(issue:123)` echoes are visual noise.
    // Click-to-open works off the visible text, not the markup.
    let s = issue_re().replace_all(seg, "(#$1)");
    bare_re().replace_all(&s, "${1}#$2").into_owned()
}

fn ref_bare_re() -> &'static Regex {
    // Only word characters disqualify a prefix (avoids `C#12`); `[` and `(`
    // are kept because rendered markdown link labels look like `[#12]`.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^\w])#(\d+)\b").unwrap())
}

fn ref_url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"github\.com/([^/\s)]+)/([^/\s)]+)/(?:issues|pull)/(\d+)").unwrap()
    })
}

/// Issue/PR numbers referenced in a visible line, in textual order.
///
/// Matches bare `#123` refs (code-adjacent ones excluded as in `linkify`) and
/// `github.com` URLs that point at `repo` (`owner/repo`, case-insensitive).
pub fn refs_in_line(line: &str, repo: Option<&str>) -> Vec<u64> {
    let mut hits: Vec<(usize, u64)> = vec![];
    for cap in ref_bare_re().captures_iter(line) {
        let m = cap.get(1).unwrap();
        if let Ok(n) = m.as_str().parse::<u64>() {
            hits.push((m.start(), n));
        }
    }
    for cap in ref_url_re().captures_iter(line) {
        let same_repo = match repo {
            Some(r) => {
                let got = format!("{}/{}", &cap[1], &cap[2]).to_lowercase();
                got == r.to_lowercase()
            }
            None => false,
        };
        if same_repo {
            if let Ok(n) = cap[3].parse::<u64>() {
                hits.push((cap.get(3).unwrap().start(), n));
            }
        }
    }
    hits.sort();
    hits.dedup();
    hits.into_iter().map(|(_, n)| n).collect()
}

pub fn status_text(statuses: &[(String, Option<String>)]) -> String {
    statuses
        .iter()
        .take(2)
        .map(|(_, s)| s.clone().unwrap_or_else(|| "—".to_string()))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn meta_line(d: &IssueData, statuses: &[(String, Option<String>)]) -> String {
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
    let opened = d.created_at.get(..10).unwrap_or(&d.created_at);
    let project = {
        let s = status_text(statuses);
        if s.is_empty() { "—".to_string() } else { s }
    };
    format!(
        "{kind} · {} · by {} · opened {opened} · labels: {labels} · assignees: {people} · project: {project}\n\n{}",
        d.state, d.author.login, d.url
    )
}

pub fn build_body_md(d: &IssueData, statuses: &[(String, Option<String>)]) -> String {
    let raw = d.body.trim();
    let body = linkify(if raw.is_empty() { "(no description)" } else { raw });
    format!(
        "# #{}: {}\n\n{}\n\n---\n\n{}\n",
        d.number,
        linkify(&d.title),
        meta_line(d, statuses),
        body
    )
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

/// Sidebar row: `#n title` on line one, label chips + project status on line two.
pub fn sidebar_lines(
    number: u64,
    title: &str,
    labels: &[Label],
    statuses: &[(String, Option<String>)],
    width: usize,
    pinned: bool,
) -> (String, String) {
    let first = format!("#{}{} {}", number, if pinned { " 📌" } else { "" }, slug(title, 30.min(width)));
    let status = status_text(statuses);
    let chips: Vec<String> = labels.iter().map(|l| format!("● {}", l.name)).collect();
    let mut second = String::new();
    let room = if status.is_empty() {
        width
    } else {
        width.saturating_sub(status.len() + 1)
    };
    let mut used = 0;
    for (i, chip) in chips.iter().enumerate() {
        let add = chip.len() + 2;
        if used + add > room {
            if i == 0 {
                second.push_str(&format!("+{} labels", chips.len()));
            }
            break;
        }
        if i > 0 {
            second.push_str("  ");
        }
        second.push_str(chip);
        used += add;
    }
    if second.is_empty() && !chips.is_empty() {
        second = format!("+{} labels", chips.len());
    } else if second.is_empty() {
        second = "–".to_string();
    }
    let second = if status.is_empty() {
        second
    } else {
        let pad = width.saturating_sub(second.len() + status.len()).max(1);
        format!("{}{}{}", second, " ".repeat(pad), status)
    };
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_issue() -> IssueData {
        IssueData {
            number: 1,
            title: "some title here".to_string(),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![Label { name: "needs-triage".to_string() }],
            assignees: vec![],
            created_at: "2026-01-02T00:00:00Z".to_string(),
            url: "https://github.com/o/r/issues/1".to_string(),
            body: "see (#1) and #2".to_string(),
            comments: vec![],
            is_pr: false,
        }
    }

    #[test]
    fn linkify_skips_code() {
        // Issue refs render as plain text; only real URLs keep destinations.
        assert_eq!(linkify("see (#1) and #2"), "see (#1) and #2");
        assert!(!linkify("see (#1)").contains("issue:"));
        assert_eq!(linkify("`#3` and ```\n(#4)\n```"), "`#3` and ```\n(#4)\n```");
        assert_eq!(linkify("[#4](http://x)"), "[#4](http://x)");
    }

    #[test]
    fn refs_in_line_finds_bare_refs() {
        assert_eq!(refs_in_line("see (#1) and #2", None), vec![1, 2]);
        assert_eq!(refs_in_line("[#4](issue:4)", None), vec![4]);
        assert_eq!(refs_in_line("no refs here", None), Vec::<u64>::new());
    }

    #[test]
    fn refs_in_line_filters_urls_by_repo() {
        let line = "see https://github.com/o/r/pull/12 and https://github.com/x/y/issues/34";
        assert_eq!(refs_in_line(line, Some("o/r")), vec![12]);
        assert_eq!(refs_in_line(line, Some("O/R")), vec![12]);
        assert_eq!(refs_in_line(line, None), Vec::<u64>::new());
    }

    #[test]
    fn status_and_meta() {
        assert_eq!(status_text(&[("Roadmap".into(), Some("In Progress".into()))]), "In Progress");
        assert_eq!(status_text(&[]), "");
        let d = sample_issue();
        assert!(meta_line(&d, &[]).contains("project: —"));
    }

    #[test]
    fn sidebar_row_ends_with_status() {
        let d = sample_issue();
        let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
        let (_, second) = sidebar_lines(d.number, &d.title, &d.labels, &st, 40, false);
        assert!(second.ends_with("Todo"), "{second}");
        assert_eq!(second.len(), 40, "{second}");
    }

    #[test]
    fn board_groups_unknowns() {
        let items = vec![
            BoardItem { id: "a".into(), title: "a".into(), number: Some(1), status: Some("Todo".into()), labels: vec![], content: serde_json::Value::Null },
            BoardItem { id: "b".into(), title: "b".into(), number: Some(2), status: Some("Done".into()), labels: vec![], content: serde_json::Value::Null },
            BoardItem { id: "c".into(), title: "c".into(), number: Some(3), status: Some("Mystery".into()), labels: vec![], content: serde_json::Value::Null },
        ];
        let cols = board_columns(&items, &["Todo".to_string(), "Done".to_string()]);
        assert_eq!(cols.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["Todo", "Done", "Other"]);
        assert_eq!(cols[2].1.iter().map(|c| c.number).collect::<Vec<_>>(), vec![Some(3)]);
    }

    #[test]
    fn pinned_marker() {
        let (first, _) = sidebar_lines(7, "t", &[], &[], 40, true);
        assert!(first.contains("📌"));
        let (plain, _) = sidebar_lines(7, "t", &[], &[], 40, false);
        assert!(!plain.contains("📌"));
    }
}
