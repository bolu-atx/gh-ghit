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

/// Merge independently fetched pages in page order.
pub fn merge_comment_pages(mut pages: Vec<(usize, Vec<Comment>)>) -> Vec<Comment> {
    pages.sort_by_key(|(page, _)| *page);
    pages.into_iter().flat_map(|(_, comments)| comments).collect()
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
    pub updated_at: String,
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
    pub title: String,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub content: serde_json::Value,
}

/// Per-card detail fetched in the background: dates plus comment count.
#[derive(Debug, Clone, Default)]
pub struct BoardDetail {
    pub created: String,
    pub updated: String,
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

/// A sidebar row: an issue/PR number, or a divider labeled with the
/// section that follows it (`requested`, `recent`; the leading pinned
/// section needs no divider since its rows carry 📌).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideRow {
    Item(u64),
    Divider(&'static str),
}

impl SideRow {
    pub fn number(self) -> Option<u64> {
        match self {
            SideRow::Item(n) => Some(n),
            SideRow::Divider(_) => None,
        }
    }
}

/// Cap on related-issue rows pulled out of one body.
pub const MAX_REFS: usize = 20;

/// Issue/PR numbers mentioned in a raw body, in first-seen order, deduped.
/// Same-repo `github.com` URLs count; cross-repo URLs and code spans do not.
pub fn referenced_numbers(body: &str, repo: Option<&str>) -> Vec<u64> {
    use std::collections::HashSet;
    let mut out: Vec<u64> = vec![];
    let mut seen: HashSet<u64> = HashSet::new();
    // Split off fenced/inline code first so `` `#123` `` stays plain text.
    let code = code_re();
    let mut last = 0;
    let mut plain: Vec<&str> = vec![];
    for m in code.find_iter(body) {
        plain.push(&body[last..m.start()]);
        last = m.end();
    }
    plain.push(&body[last..]);
    for seg in plain {
        for line in seg.lines() {
            for (_, target) in links_in_line(line, repo) {
                if let LinkTarget::Issue(n) = target {
                    if seen.insert(n) {
                        out.push(n);
                        if out.len() >= MAX_REFS {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

/// Something clickable on a visible line, in textual order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// An issue/PR number: opens in the viewer.
    Issue(u64),
    /// A real URL: opens in the browser.
    Url(String),
}

fn http_paren_re() -> &'static Regex {
    // The renderer prints links as `label (destination)`.
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\((https?://[^)\s]+)\)").unwrap())
}

fn http_bare_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:^|[^\w(])(https?://[^\s)\]]+)").unwrap())
}

fn trim_url(u: &str) -> &str {
    u.trim_end_matches(['.', ',', ';', ':', '!', '?', '"', '\''])
}

/// True when `#123` is a rendered link label, i.e. directly followed by its
/// ` (https://…)` destination. Those clicks belong to the URL, not the number.
fn followed_by_url_dest(line: &str, end: usize) -> bool {
    let rest = line[end..].trim_start_matches([' ', '\t']);
    rest.starts_with("(http://") || rest.starts_with("(https://")
}

fn same_repo(cap: &regex::Captures, repo: Option<&str>) -> bool {
    match repo {
        Some(r) => format!("{}/{}", &cap[1], &cap[2]).to_lowercase() == r.to_lowercase(),
        None => false,
    }
}

/// Everything clickable on a visible line, in textual order: issue refs and
/// same-repo issue URLs become `Issue`, anything else with an http(s)
/// destination becomes `Url`. Positions are byte offsets for click
/// disambiguation: the nearest to the click column wins.
pub fn links_in_line(line: &str, repo: Option<&str>) -> Vec<(usize, LinkTarget)> {
    let mut hits: Vec<(usize, LinkTarget)> = vec![];
    let mut taken: Vec<(usize, usize)> = vec![];
    let claim = |start: usize, end: usize, target: LinkTarget, hits: &mut Vec<(usize, LinkTarget)>, taken: &mut Vec<(usize, usize)>| {
        if taken.iter().any(|&(s, e)| start < e && s < end) {
            return;
        }
        hits.push((start, target));
        taken.push((start, end));
    };
    for cap in ref_url_re().captures_iter(line) {
        let m = cap.get(0).unwrap();
        if same_repo(&cap, repo) {
            if let Ok(n) = cap[3].parse::<u64>() {
                claim(m.start(), m.end(), LinkTarget::Issue(n), &mut hits, &mut taken);
            }
        }
    }
    for cap in ref_bare_re().captures_iter(line) {
        let m = cap.get(1).unwrap();
        if followed_by_url_dest(line, m.end()) {
            continue;
        }
        if let Ok(n) = m.as_str().parse::<u64>() {
            claim(m.start(), m.end(), LinkTarget::Issue(n), &mut hits, &mut taken);
        }
    }
    for re in [http_paren_re(), http_bare_re()] {
        for cap in re.captures_iter(line) {
            let m = cap.get(1).unwrap();
            let url = trim_url(m.as_str()).to_string();
            if url.is_empty() {
                continue;
            }
            claim(m.start(), m.end(), LinkTarget::Url(url), &mut hits, &mut taken);
        }
    }
    hits.sort_by_key(|(idx, _)| *idx);
    hits.dedup();
    hits
}

/// The clicked target on a line with several refs: nearest byte offset to
/// the click column wins, so the second `#ref` in a row opens instead of
/// the first. Offsets come from regex matches on this line, so slicing at
/// them is always a char boundary.
pub fn nearest_link(line: &str, links: &[(usize, LinkTarget)], col: usize) -> Option<LinkTarget> {
    links
        .iter()
        .min_by_key(|(idx, _)| line[..*idx].chars().count().abs_diff(col))
        .map(|(_, t)| t.clone())
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

/// Collapsed card footer, right side: updated date plus comment count, or
/// an ellipsis while the background detail fetch is still in flight.
pub fn card_footer_date(detail: Option<&BoardDetail>) -> String {
    match detail {
        None => "…".to_string(),
        Some(d) => {
            let day = d.updated.get(..10).unwrap_or(&d.updated);
            let comments = if d.comments == 0 {
                "no comments".to_string()
            } else {
                format!("{} comment{}", d.comments, if d.comments == 1 { "" } else { "s" })
            };
            format!("{comments} · {day}")
        }
    }
}

/// Semantic role of a meta-line segment; the UI maps each to a theme color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaKind {
    Dim,
    State,
    Label,
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

/// One changed file in a PR diff, as returned by the REST files API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
    #[serde(default)]
    pub patch: String,
}

/// Semantic role of one patch line; the UI maps each to a theme color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Hunk,
    Add,
    Del,
    Context,
}

/// Split a unified `patch` into tagged lines. Hunk headers (`@@`) tag as
/// `Hunk`, `+` as `Add`, `-` as `Del`, everything else as `Context`.
/// Empty or missing patches yield no lines: the UI shows the file header
/// with a binary/large-file note instead.
pub fn diff_lines(patch: &str) -> Vec<(DiffKind, String)> {
    patch
        .lines()
        .map(|l| {
            let kind = if l.starts_with("@@") {
                DiffKind::Hunk
            } else if l.starts_with('+') {
                DiffKind::Add
            } else if l.starts_with('-') {
                DiffKind::Del
            } else {
                DiffKind::Context
            };
            (kind, l.to_string())
        })
        .collect()
}

/// File index of a rendered `▸ `/`▾ ` diff header row. Longest filename
/// wins so `a.rs` never shadows `src/a.rs`. `None` for any other row.
pub fn header_file_index(line: &str, files: &[DiffFile]) -> Option<usize> {
    if !(line.starts_with("▸ ") || line.starts_with("▾ ")) {
        return None;
    }
    files
        .iter()
        .enumerate()
        .filter(|(_, f)| line.contains(&f.filename))
        .max_by_key(|(_, f)| f.filename.len())
        .map(|(i, _)| i)
}

/// Summary for the diff tab header: file count plus summed adds/deletes.
pub fn diff_summary(files: &[DiffFile]) -> String {
    let adds: usize = files.iter().map(|f| f.additions).sum();
    let dels: usize = files.iter().map(|f| f.deletions).sum();
    format!("{} file{} · +{adds} -{dels}", files.len(), if files.len() == 1 { "" } else { "s" })
}

/// One row of a side-by-side rendering of a unified patch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitRow {
    /// Hunk header, rendered full-width.
    Hunk(String),
    /// Old line on the left, new line on the right. Context shows the
    /// same text on both sides; a change pairs one `-` line with one `+`.
    Pair { left: String, right: String },
    /// Unpaired `-` line: right cell stays blank.
    LeftOnly { text: String, kind: DiffKind },
    /// Unpaired `+` line: left cell stays blank.
    RightOnly { text: String, kind: DiffKind },
}

/// Pair a unified `patch` into side-by-side rows. Runs of `-` lines pair
/// with the `+` lines that follow them, in order; leftovers render alone.
/// Hunk headers pass through full-width. Empty patches yield no rows.
pub fn split_rows(patch: &str) -> Vec<SplitRow> {
    let mut rows: Vec<SplitRow> = vec![];
    let mut dels: Vec<String> = vec![];
    for (kind, text) in diff_lines(patch) {
        match kind {
            DiffKind::Hunk => {
                for d in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del });
                }
                rows.push(SplitRow::Hunk(text));
            }
            DiffKind::Del => dels.push(text),
            DiffKind::Add => {
                if dels.is_empty() {
                    rows.push(SplitRow::RightOnly { text, kind: DiffKind::Add });
                } else {
                    rows.push(SplitRow::Pair { left: dels.remove(0), right: text });
                }
            }
            DiffKind::Context => {
                for d in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del });
                }
                rows.push(SplitRow::Pair { left: text.clone(), right: text });
            }
        }
    }
    for d in dels.drain(..) {
        rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del });
    }
    rows
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

/// Sidebar row: `#n title` on line one, label chips plus the project
/// status on line two. Returns `(first, body, status)` with the status
/// segment split out so the UI can paint it in the board color with the
/// board glyph; `body` already carries the trailing pad so the joined
/// row still fills exactly `width` cells.
pub fn sidebar_lines(
    number: u64,
    title: &str,
    labels: &[Label],
    statuses: &[(String, Option<String>)],
    width: usize,
    pinned: bool,
) -> (String, String, String) {
    let first = format!("#{}{} {}", number, if pinned { " 📌" } else { "" }, slug(title, 30.min(width)));
    let status = status_text(statuses);
    let icon = crate::theme::column_icon(&status);
    let seg = if status.is_empty() {
        String::new()
    } else if icon.is_empty() {
        status.clone()
    } else {
        format!("{icon} {status}")
    };
    let status_w = seg.chars().count();
    let chips: Vec<String> = labels.iter().map(|l| format!("● {}", l.name)).collect();
    let mut second = String::new();
    let room = if seg.is_empty() {
        width
    } else {
        width.saturating_sub(status_w + 1)
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
    let (body, seg) = if seg.is_empty() {
        (second, String::new())
    } else {
        let pad = width
            .saturating_sub(second.chars().count() + status_w)
            .max(1);
        (format!("{}{}", second, " ".repeat(pad)), seg)
    };
    (first, body, seg)
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
            updated_at: "2026-01-05T00:00:00Z".to_string(),
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

    fn nums(line: &str, repo: Option<&str>) -> Vec<u64> {
        links_in_line(line, repo)
            .into_iter()
            .filter_map(|(_, t)| match t {
                LinkTarget::Issue(n) => Some(n),
                LinkTarget::Url(_) => None,
            })
            .collect()
    }

    #[test]
    fn refs_in_line_finds_bare_refs() {
        assert_eq!(nums("see (#1) and #2", None), vec![1, 2]);
        assert_eq!(nums("[#4](issue:4)", None), vec![4]);
        assert_eq!(nums("no refs here", None), Vec::<u64>::new());
    }

    fn targets(line: &str, repo: Option<&str>) -> Vec<LinkTarget> {
        links_in_line(line, repo).into_iter().map(|(_, t)| t).collect()
    }

    #[test]
    fn links_in_line_orders_issues_and_urls() {
        use LinkTarget::{Issue, Url};
        // Rendered markdown links show the destination in parens.
        let line = "see #12 (https://example.com/a) then #34";
        assert_eq!(
            targets(line, Some("o/r")),
            vec![Url("https://example.com/a".to_string()), Issue(34)]
        );
    }

    #[test]
    fn links_in_line_sends_same_repo_urls_in_app() {
        use LinkTarget::{Issue, Url};
        let line = "backport of #12 (https://github.com/o/r/pull/12), see #34 (https://github.com/x/y/issues/34)";
        assert_eq!(
            targets(line, Some("o/r")),
            vec![Issue(12), Url("https://github.com/x/y/issues/34".to_string())]
        );
    }

    #[test]
    fn refs_in_line_filters_urls_by_repo() {
        let line = "see https://github.com/o/r/pull/12 and https://github.com/x/y/issues/34";
        assert_eq!(nums(line, Some("o/r")), vec![12]);
        assert_eq!(nums(line, Some("O/R")), vec![12]);
        assert_eq!(nums(line, None), Vec::<u64>::new());
    }

    #[test]
    fn referenced_numbers_dedups_orders_and_skips_code() {
        let body = "blocked by #2 and (#3), see #2 again\n`#9` ignored\n```\n#10 ignored\n```\nhttps://github.com/o/r/pull/4 and https://github.com/x/y/issues/5";
        assert_eq!(referenced_numbers(body, Some("o/r")), vec![2, 3, 4]);
    }

    #[test]
    fn referenced_numbers_caps_at_max_refs() {
        let body = (1..=30).map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
        let refs = referenced_numbers(&body, Some("o/r"));
        assert_eq!(refs.len(), MAX_REFS);
        assert_eq!(refs[0], 1);
    }

    #[test]
    fn meta_spans_carry_roles_and_join_to_plain_text() {
        let d = sample_issue();
        let parts = meta_spans(&d, &[]);
        assert!(parts.iter().any(|(k, t)| *k == MetaKind::State && t == "OPEN"));
        assert!(parts.iter().any(|(k, t)| *k == MetaKind::Label && t == "needs-triage"));
        assert!(parts.iter().any(|(k, _)| *k == MetaKind::Url));
        let joined: String = parts.into_iter().map(|(_, t)| t).collect();
        assert!(joined.contains("project: —"));
    }

    #[test]
    fn status_and_meta() {
        assert_eq!(status_text(&[("Roadmap".into(), Some("In Progress".into()))]), "In Progress");
        assert_eq!(status_text(&[]), "");
    }

    #[test]
    fn sidebar_row_ends_with_status() {
        let d = sample_issue();
        let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
        let (_, body, seg) = sidebar_lines(d.number, &d.title, &d.labels, &st, 40, false);
        let second = format!("{body}{seg}");
        assert!(second.ends_with("Todo"), "{second}");
        assert_eq!(second.chars().count(), 40, "{second}");
        assert!(seg.starts_with(crate::theme::column_icon("Todo")), "{seg}");
    }

    #[test]
    fn board_groups_unknowns() {
        let items = vec![
            BoardItem { title: "a".into(), number: Some(1), status: Some("Todo".into()), labels: vec![], repo: None, content: serde_json::Value::Null },
            BoardItem { title: "b".into(), number: Some(2), status: Some("Done".into()), labels: vec![], repo: None, content: serde_json::Value::Null },
            BoardItem { title: "c".into(), number: Some(3), status: Some("Mystery".into()), labels: vec![], repo: None, content: serde_json::Value::Null },
        ];
        let cols = board_columns(&items, &["Todo".to_string(), "Done".to_string()]);
        assert_eq!(cols.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), vec!["Todo", "Done", "Other"]);
        assert_eq!(cols[2].1.iter().map(|c| c.number).collect::<Vec<_>>(), vec![Some(3)]);
    }

    #[test]
    fn nearest_link_picks_closest_ref_on_shared_row() {
        let line = "backport of #12, see also #34 for context";
        let links = links_in_line(line, None);
        assert_eq!(links.len(), 2);
        let near_first = line.find("#12").unwrap();
        let near_second = line.find("#34").unwrap();
        assert_eq!(nearest_link(line, &links, near_first), Some(LinkTarget::Issue(12)));
        assert_eq!(nearest_link(line, &links, near_second), Some(LinkTarget::Issue(34)));
        assert_eq!(nearest_link(line, &[], 0), None);
    }

    #[test]
    fn card_footer_splits_labels_left_and_date_right() {
        let item = BoardItem { title: "a".into(), number: Some(1), status: Some("Todo".into()), labels: vec![Label { name: "bug".into() }], repo: None, content: serde_json::Value::Null };
        assert_eq!(card_footer_labels(&item), "● bug");
        let bare = BoardItem { labels: vec![], ..item.clone() };
        assert_eq!(card_footer_labels(&bare), "–");
        assert_eq!(card_footer_date(None), "…");
        let d = BoardDetail { created: "2026-01-02".into(), updated: "2026-01-05T00:00:00Z".into(), comments: 3 };
        assert_eq!(card_footer_date(Some(&d)), "3 comments · 2026-01-05");
    }

    #[test]
    fn pinned_marker() {
        let (first, _, _) = sidebar_lines(7, "t", &[], &[], 40, true);
        assert!(first.contains("📌"));
        let (plain, _, _) = sidebar_lines(7, "t", &[], &[], 40, false);
        assert!(!plain.contains("📌"));
    }

    #[test]
    fn diff_lines_tag_hunks_adds_and_dels() {
        let patch = "@@ -1,2 +1,2 @@\n context\n-old\n+new";
        let kinds: Vec<DiffKind> = diff_lines(patch).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, vec![DiffKind::Hunk, DiffKind::Context, DiffKind::Del, DiffKind::Add]);
        assert!(diff_lines("").is_empty());
    }

    #[test]
    fn split_rows_pair_changes_and_keep_context() {
        let rows = split_rows("@@ -1,3 +1,3 @@\n ctx\n-old\n+new\ntrail");
        assert_eq!(rows[0], SplitRow::Hunk("@@ -1,3 +1,3 @@".to_string()));
        assert_eq!(rows[1], SplitRow::Pair { left: " ctx".to_string(), right: " ctx".to_string() });
        assert_eq!(rows[2], SplitRow::Pair { left: "-old".to_string(), right: "+new".to_string() });
        assert_eq!(rows[3], SplitRow::Pair { left: "trail".to_string(), right: "trail".to_string() });
    }

    #[test]
    fn split_rows_leave_unpaired_lines_alone() {
        let rows = split_rows("@@ -1 +1 @@\n-a\n-b\n+c");
        assert_eq!(rows[1], SplitRow::Pair { left: "-a".to_string(), right: "+c".to_string() });
        assert_eq!(rows[2], SplitRow::LeftOnly { text: "-b".to_string(), kind: DiffKind::Del });
        assert!(split_rows("").is_empty());
    }

    #[test]
    fn diff_summary_counts_files_and_lines() {
        let files = vec![
            DiffFile { filename: "a".into(), status: "modified".into(), additions: 10, deletions: 2, patch: String::new() },
            DiffFile { filename: "b".into(), status: "added".into(), additions: 5, deletions: 0, patch: String::new() },
        ];
        assert_eq!(diff_summary(&files), "2 files · +15 -2");
        assert_eq!(diff_summary(&files[..1]), "1 file · +10 -2");
    }

    #[test]
    fn header_file_index_matches_longest_filename() {
        let files = vec![
            DiffFile { filename: "a.rs".into(), status: String::new(), additions: 0, deletions: 0, patch: String::new() },
            DiffFile { filename: "src/a.rs".into(), status: String::new(), additions: 0, deletions: 0, patch: String::new() },
        ];
        assert_eq!(header_file_index("▸ \u{f15c} src/a.rs   modified", &files), Some(1));
        assert_eq!(header_file_index("▾ \u{f15c} a.rs   added", &files), Some(0));
        assert_eq!(header_file_index("  -old", &files), None);
        assert_eq!(header_file_index("───", &files), None);
    }
}

#[cfg(test)]
mod comment_page_tests {
    use super::*;

    #[test]
    fn merges_pages_in_oldest_first_order() {
        let comment = |body: &str| Comment { body: body.to_owned(), ..Comment::default() };
        let merged = merge_comment_pages(vec![(2, vec![comment("third")]), (1, vec![comment("first"), comment("second")])]);
        assert_eq!(merged.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(), vec!["first", "second", "third"]);
    }
}
