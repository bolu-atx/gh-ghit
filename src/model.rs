use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    #[serde(default)]
    pub author: Option<Author>,
    #[serde(default)]
    pub body: String,
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

pub fn parse_review_threads(value: &serde_json::Value) -> Vec<ReviewThread> {
    let nodes = value.pointer("/reviewThreads/nodes").and_then(serde_json::Value::as_array);
    let mut threads = Vec::new();
    for node in nodes.into_iter().flatten() {
        if !node.is_object() { continue; }
        let mut thread = node.clone();
        thread["comments"] = node.pointer("/comments/nodes").cloned().unwrap_or_else(|| serde_json::json!([]));
        if let Ok(thread) = serde_json::from_value::<ReviewThread>(thread) { threads.push(thread); }
    }
    threads.sort_by_key(|thread| thread.is_resolved);
    threads
}

/// Parse the will-close connection of one issue or PR payload. Issues
/// carry closing PRs under `closedByPullRequestsReferences`; PRs carry
/// closing issues under `closingIssuesReferences`. Missing or malformed
/// nodes yield an empty list.
pub fn paired_refs(value: &serde_json::Value, is_pr: bool) -> Vec<PairedRef> {
    let field = if is_pr {
        "closingIssuesReferences"
    } else {
        "closedByPullRequestsReferences"
    };
    let nodes = value
        .pointer(&format!("/{field}/nodes"))
        .and_then(serde_json::Value::as_array);
    let mut out = Vec::new();
    for node in nodes.into_iter().flatten() {
        let Some(number) = node.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        out.push(PairedRef {
            number,
            state: node.get("state").and_then(serde_json::Value::as_str).unwrap_or("").to_owned(),
        });
    }
    out
}

/// Merge independently fetched pages in page order.
pub fn merge_comment_pages(mut pages: Vec<(usize, Vec<Comment>)>) -> Vec<Comment> {
    pages.sort_by_key(|(page, _)| *page);
    pages.into_iter().flat_map(|(_, comments)| comments).collect()
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

/// Parse the head-commit rollup of one sections-query PR node. Missing
/// commits or a missing/null rollup yield `None`; an unrecognized state
/// yields `Unknown`, never an error.
pub fn parse_check_state(value: &serde_json::Value) -> Option<CheckState> {
    let state = value
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(serde_json::Value::as_str)?;
    match state {
        "SUCCESS" => Some(CheckState::Pass),
        "FAILURE" | "ERROR" => Some(CheckState::Fail),
        "PENDING" | "EXPECTED" => Some(CheckState::Pending),
        _ => Some(CheckState::Unknown),
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualContent { Diff, Graph }

pub fn visual_content(is_pr: bool) -> VisualContent {
    if is_pr { VisualContent::Diff } else { VisualContent::Graph }
}

/// Map each patch row to its `(old, new)` file lines. Hunk headers and
/// `\ No newline` markers map to `(None, None)`; deletions carry only
/// old, additions only new, context both. Each counter seeds from its
/// side of the hunk header and steps past rows on that side.
pub fn patch_line_numbers(patch: &str) -> Vec<(Option<u64>, Option<u64>)> {
    fn hunk_start(header: &str, marker: char) -> Option<u64> {
        header.split_whitespace().find_map(|part| part.strip_prefix(marker))
            .and_then(|part| part.split(',').next())
            .and_then(|part| part.parse::<u64>().ok())
    }
    let mut old_next = None;
    let mut new_next = None;
    diff_lines(patch).iter().map(|(kind, text)| match kind {
        DiffKind::Hunk => {
            old_next = hunk_start(text, '-');
            new_next = hunk_start(text, '+');
            (None, None)
        }
        DiffKind::Del => {
            let line = old_next;
            old_next = old_next.map(|n| n + 1);
            (line, None)
        }
        DiffKind::Add | DiffKind::Context if !text.starts_with('\\') => {
            let new = new_next;
            new_next = new_next.map(|n| n + 1);
            if *kind == DiffKind::Context {
                let old = old_next;
                old_next = old_next.map(|n| n + 1);
                (old, new)
            } else {
                (None, new)
            }
        }
        _ => (None, None),
    }).collect()
}

/// Map each patch row to its new-file line, or None for headers and deletions.
pub fn patch_new_lines(patch: &str) -> Vec<Option<u64>> {
    patch_line_numbers(patch).into_iter().map(|(_, new)| new).collect()
}

pub fn thread_patch_row(thread: &ReviewThread, file: &DiffFile) -> Option<usize> {
    if thread.path != file.filename { return None; }
    let line = thread.line?;
    patch_new_lines(&file.patch).iter().position(|n| *n == Some(line))
}

/// Strip a rendered unified-diff row down to its patch text. Code rows
/// carry a 7-cell line-number gutter (`  42 │ code`, blank number for
/// hunk headers). Anything else (split cells, rules, blanks, thread
/// blocks, wrapped continuations, legacy dent) keeps the old two-cell
/// dent strip so non-matching rows behave exactly as before.
pub fn strip_diff_gutter(row: &str) -> &str {
    // The gutter is 4 cells of blanks/digits plus ` │ `; `│` is
    // multi-byte, so walk chars rather than slicing bytes.
    let mut chars = row.chars();
    for _ in 0..4 {
        match chars.next() {
            Some(c) if c == ' ' || c.is_ascii_digit() => {}
            _ => return row.strip_prefix("  ").unwrap_or(row),
        }
    }
    match chars.as_str().strip_prefix(" │ ") {
        Some(code) => code,
        None => row.strip_prefix("  ").unwrap_or(row),
    }
}

/// New-side line for a cursor sitting on a rendered unified-diff section.
///
/// `section` holds the visible buffer rows of one file starting just below
/// its header, `cursor` is the index into `section` of the active row, and
/// `patch` is the file's raw unified patch. Returns the new-side line of
/// the nearest Add/Context row at or above the cursor, or `None` when the
/// cursor sits above the first commentable row (hunk headers, leading
/// deletions, binary notes). Rendered rows carry a line-number gutter
/// that is stripped before comparing; blank, rule, and review-thread
/// rows never match a patch row and are skipped, as are wrapped
/// continuations (a truncated first fragment still prefixes its patch row).
pub fn line_target_at_cursor(section: &[String], cursor: usize, patch: &str) -> Option<u64> {
    let rows: Vec<(DiffKind, String)> = diff_lines(patch);
    let numbers = patch_new_lines(patch);
    let mut target: Option<u64> = None;
    let mut next = 0;
    for row in section.iter().take(cursor + 1) {
        let Some((_, text)) = rows.get(next) else { break };
        let stripped = strip_diff_gutter(row.trim_end());
        if !stripped.is_empty() && (stripped == text || text.starts_with(stripped)) {
            if let Some(n) = numbers.get(next).copied().flatten() {
                target = Some(n);
            }
            next += 1;
        }
    }
    target
}

/// Thread id for a cursor sitting inside a rendered review-thread block.
///
/// Same `section`/`cursor` contract as `line_target_at_cursor`, plus the
/// file and its threads in render order. Threads attach after their patch
/// row (`thread_patch_row`); a block opens at its reply-glyph header row
/// and runs to the next patch row or thread header, so the cursor on a
/// header, author, or comment row answers that thread's id. Code rows,
/// blanks, and rules answer `None` so callers keep the line-comment
/// target there. The k-th header after a patch row names the k-th thread
/// attached there, matching render order.
pub fn thread_at_cursor(
    section: &[String],
    cursor: usize,
    file: &DiffFile,
    threads: &[ReviewThread],
) -> Option<String> {
    let rows = diff_lines(&file.patch);
    let mut next = 0;
    // Patch row the current gap follows; `None` before the first one.
    // Threads with no patch row render in that leading zone.
    let mut attach: Option<usize> = None;
    // Ids of headers seen since the last consumed patch row, in order.
    let mut seen: Vec<String> = vec![];
    // Thread whose block the walk is inside, if any.
    let mut block: Option<String> = None;
    for (i, row) in section.iter().enumerate().take(cursor + 1) {
        // Same gutter strip as `line_target_at_cursor`: patch rows sit
        // behind a line-number gutter since #75.
        let stripped = strip_diff_gutter(row.trim_end());
        let matched = match rows.get(next) {
            Some((_, text)) => !stripped.is_empty() && (stripped == text || text.starts_with(stripped)),
            None => false,
        };
        if matched {
            next += 1;
            attach = Some(next - 1);
            seen.clear();
            block = None;
            continue;
        }
        if stripped.contains('\u{f075}') {
            let attached: Vec<&ReviewThread> = threads
                .iter()
                .filter(|t| t.path == file.filename && thread_patch_row(t, file) == attach)
                .collect();
            if let Some(thread) = attached.get(seen.len()) {
                seen.push(thread.id.clone());
                block = Some(thread.id.clone());
            }
        }
        if i == cursor {
            return block;
        }
    }
    None
}

#[cfg(test)]
mod content_polish_tests {
    use super::*;
    #[test]
    fn selects_visual_by_kind() {
        assert_eq!(visual_content(true), VisualContent::Diff);
        assert_eq!(visual_content(false), VisualContent::Graph);
    }
    #[test]
    fn matches_new_side_and_falls_back_for_missing_lines() {
        let file = DiffFile { filename: "a".into(), patch: "@@ -2,2 +4,2 @@\n-old\n+new\n context".into(), ..Default::default() };
        let mut thread = ReviewThread { path: "a".into(), line: Some(4), ..Default::default() };
        assert_eq!(thread_patch_row(&thread, &file), Some(2));
        thread.line = Some(2);
        assert_eq!(thread_patch_row(&thread, &file), None);
        thread.line = Some(4); thread.path = "b".into();
        assert_eq!(thread_patch_row(&thread, &file), None);
    }
    #[test]
    fn cursor_maps_rendered_rows_to_new_side_lines() {
        // Patch rows render with a two-cell dent, exactly as `ui` emits.
        let patch = "@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n+more\n tail";
        let section: Vec<String> = ["  @@ -1,3 +1,3 @@", "   ctx", "  -old", "  +new", "  +more", "   tail"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(line_target_at_cursor(&section, 0, patch), None, "hunk header has no line");
        assert_eq!(line_target_at_cursor(&section, 1, patch), Some(1));
        assert_eq!(line_target_at_cursor(&section, 2, patch), Some(1), "deletion keeps the line above");
        assert_eq!(line_target_at_cursor(&section, 3, patch), Some(2));
        assert_eq!(line_target_at_cursor(&section, 5, patch), Some(4));
    }
    #[test]
    fn cursor_inside_thread_block_answers_thread_id() {
        let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,2 +1,2 @@\n a\n+b".into(), ..Default::default() };
        let threads = vec![ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(2), ..Default::default() }];
        let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "  +b", "      \u{f075} line 2 · unresolved", "      reviewer"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(thread_at_cursor(&section, 0, &file, &threads), None, "hunk header");
        assert_eq!(thread_at_cursor(&section, 1, &file, &threads), None, "code row keeps line target");
        assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "own patch row keeps line target");
        assert_eq!(thread_at_cursor(&section, 3, &file, &threads), Some("t1".into()), "thread header");
        assert_eq!(thread_at_cursor(&section, 4, &file, &threads), Some("t1".into()), "author row");
    }
    #[test]
    fn thread_headers_after_one_row_resolve_in_order() {
        let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,2 +1,2 @@\n a\n+b".into(), ..Default::default() };
        let threads = vec![
            ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(2), ..Default::default() },
            ReviewThread { id: "t2".into(), path: "a.rs".into(), line: Some(2), ..Default::default() },
        ];
        let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "  +b",
            "      \u{f075} line 2 · unresolved", "      \u{f075} line 2 · unresolved"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "own patch row");
        assert_eq!(thread_at_cursor(&section, 3, &file, &threads), Some("t1".into()));
        assert_eq!(thread_at_cursor(&section, 4, &file, &threads), Some("t2".into()));
    }
    #[test]
    fn thread_without_a_patch_row_resolves_in_the_leading_zone() {
        let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,1 +1,1 @@\n a".into(), ..Default::default() };
        let threads = vec![ReviewThread { id: "t9".into(), path: "a.rs".into(), line: None, ..Default::default() }];
        let section: Vec<String> = ["    \u{f075} line unavailable · unresolved", "  @@ -1,1 +1,1 @@", "   a"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(thread_at_cursor(&section, 0, &file, &threads), Some("t9".into()));
        assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "code row");
    }
    #[test]
    fn cursor_skips_thread_and_blank_rows() {
        let patch = "@@ -1,2 +1,2 @@\n a\n+b";
        // Inline review threads, blanks, and wrapped continuations never
        // match a patch row, so the target stays on the last real row.
        let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "      \u{f075} line 2 · unresolved", "      reviewer", "", "  +b"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(line_target_at_cursor(&section, 4, patch), Some(1));
        assert_eq!(line_target_at_cursor(&section, 5, patch), Some(2));
    }
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
    /// `old`/`new` are the per-side file lines for the gutter.
    Pair { left: String, right: String, old: Option<u64>, new: Option<u64> },
    /// Unpaired `-` line: right cell stays blank. `line` is old-side.
    LeftOnly { text: String, kind: DiffKind, line: Option<u64> },
    /// Unpaired `+` line: left cell stays blank. `line` is new-side.
    RightOnly { text: String, kind: DiffKind, line: Option<u64> },
}

/// Pair a unified `patch` into side-by-side rows. Runs of `-` lines pair
/// with the `+` lines that follow them, in order; leftovers render alone.
/// Hunk headers pass through full-width. Empty patches yield no rows.
/// Each side keeps its file line from `patch_line_numbers` for the gutter.
pub fn split_rows(patch: &str) -> Vec<SplitRow> {
    let mut rows: Vec<SplitRow> = vec![];
    let mut dels: Vec<(String, Option<u64>)> = vec![];
    let numbers = patch_line_numbers(patch);
    for ((kind, text), (old, new)) in diff_lines(patch).into_iter().zip(numbers) {
        match kind {
            DiffKind::Hunk => {
                for (d, line) in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
                }
                rows.push(SplitRow::Hunk(text));
            }
            DiffKind::Del => dels.push((text, old)),
            DiffKind::Add => {
                if dels.is_empty() {
                    rows.push(SplitRow::RightOnly { text, kind: DiffKind::Add, line: new });
                } else {
                    let (left, old) = dels.remove(0);
                    rows.push(SplitRow::Pair { left, right: text, old, new });
                }
            }
            DiffKind::Context => {
                for (d, line) in dels.drain(..) {
                    rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
                }
                rows.push(SplitRow::Pair { left: text.clone(), right: text, old, new });
            }
        }
    }
    for (d, line) in dels.drain(..) {
        rows.push(SplitRow::LeftOnly { text: d, kind: DiffKind::Del, line });
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

/// Sidebar row: `#n title` on line one; line two is up to three `·`
/// separated segments — label chips, relative updated time, project
/// status. Returns `(first, labels, updated, status)`; empty segments
/// collapse and labels truncate first so date and status always fit.
/// The status segment comes back separately so the UI can paint it in the
/// board color with the board glyph.
pub fn sidebar_lines(
    number: u64,
    title: &str,
    labels: &[Label],
    statuses: &[(String, Option<String>)],
    updated_at: &str,
    now: chrono::DateTime<chrono::Utc>,
    width: usize,
    pinned: bool,
) -> (String, String, String, String) {
    let first = format!("#{}{} {}", number, if pinned { " 📌" } else { "" }, slug(title, 30.min(width)));
    let plain_status = status_text(statuses);
    let status = if plain_status.is_empty() {
        String::new()
    } else {
        let icon = crate::theme::column_icon(&plain_status);
        if icon.is_empty() { plain_status } else { format!("{icon} {plain_status}") }
    };
    let updated = relative_updated(updated_at, now);
    // Fixed cost: the date and status segments plus their ` · ` separator.
    let fixed = updated.chars().count()
        + status.chars().count()
        + if !updated.is_empty() && !status.is_empty() { 3 } else { 0 };
    // Labels also pay one separator when another segment follows them.
    let room = width
        .saturating_sub(fixed)
        .saturating_sub(if fixed > 0 { 3 } else { 0 });
    let chips: Vec<String> = labels.iter().map(|l| format!("● {}", l.name)).collect();
    let mut labels_text = String::new();
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
    (first, labels_text, updated, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_review_threads_in_unresolved_first_order() {
        let input = serde_json::json!({"reviewThreads":{"nodes":[
            {"id":"t1","path":"a.rs","line":null,"isResolved":true,"comments":{"nodes":[{"id":"c1","author":{"login":"ada"},"body":"done"}]}},
            {"id":"t2","path":"b.rs","line":42,"isResolved":false,"comments":{"nodes":[{"id":"c2","author":{"login":"bob"},"body":"**fix**"}]}},
            {"id":"bad"}, null
        ]}});
        let threads = parse_review_threads(&input);
        assert_eq!(threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t2", "t1"]);
        assert_eq!((&*threads[0].path, threads[0].line, threads[0].is_resolved), ("b.rs", Some(42), false));
        assert_eq!((&*threads[0].comments[0].id, &*threads[0].comments[0].author.as_ref().unwrap().login, &*threads[0].comments[0].body), ("c2", "bob", "**fix**"));
        assert_eq!(threads[1].line, None);
        assert!(threads[1].is_resolved);
    }

    #[test]
    fn paired_refs_parse_both_directions() {
        let issue = serde_json::json!({"closedByPullRequestsReferences":{"nodes":[
            {"number": 44, "state": "MERGED"}, {"number": 45, "state": "OPEN"}, null
        ]}});
        assert_eq!(
            paired_refs(&issue, false),
            vec![
                PairedRef { number: 44, state: "MERGED".to_string() },
                PairedRef { number: 45, state: "OPEN".to_string() },
            ]
        );
        let pr = serde_json::json!({"closingIssuesReferences":{"nodes":[
            {"number": 59, "state": "OPEN"}
        ]}});
        assert_eq!(
            paired_refs(&pr, true),
            vec![PairedRef { number: 59, state: "OPEN".to_string() }]
        );
        assert!(paired_refs(&serde_json::json!({}), true).is_empty());
    }

    #[test]
    fn check_rollup_parses_pass_fail_pending_and_absent() {
        use CheckState::{Fail, Pass, Pending};
        let node = |state: &str| serde_json::json!({"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":state}}}]}});
        assert_eq!(parse_check_state(&node("SUCCESS")), Some(Pass));
        assert_eq!(parse_check_state(&node("FAILURE")), Some(Fail));
        assert_eq!(parse_check_state(&node("ERROR")), Some(Fail));
        assert_eq!(parse_check_state(&node("PENDING")), Some(Pending));
        assert_eq!(parse_check_state(&node("EXPECTED")), Some(Pending));
        // Missing commits, empty nodes, or a null rollup mean no checks.
        assert_eq!(parse_check_state(&serde_json::json!({})), None);
        assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[]}})), None);
        assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}})), None);
        assert_eq!(parse_check_state(&serde_json::json!({"commits":{"nodes":[{"commit":{}}]}})), None);
        // Unknown future states never fail the row.
        assert_eq!(parse_check_state(&node("QUEUED")), Some(CheckState::Unknown));
    }

    #[test]
    fn old_cached_issue_without_paired_still_parses() {
        let old = serde_json::json!({
            "number": 7, "title": "cached", "state": "OPEN",
            "author": {"login": "bo"}, "labels": [], "assignees": [],
            "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
            "url": "https://github.com/o/r/issues/7", "body": "",
            "comments": [], "isPr": false, "reviewThreads": []
        });
        let data: IssueData = serde_json::from_value(old).expect("old cache parses");
        assert!(data.paired.is_empty());
        assert!(data.checks.is_none());
    }

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
            review_threads: vec![],
            paired: vec![],
            checks: None,
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

    fn at(stamp: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(stamp).unwrap().with_timezone(&chrono::Utc)
    }

    #[test]
    fn relative_time_buckets_at_each_boundary() {
        let now = at("2026-11-10T12:00:00Z");
        assert_eq!(relative_updated("2026-11-10T12:00:00Z", now), "0 min ago");
        assert_eq!(relative_updated("2026-11-10T11:00:01Z", now), "59 min ago");
        assert_eq!(relative_updated("2026-11-10T11:00:00Z", now), "1 hr ago");
        assert_eq!(relative_updated("2026-11-09T12:00:01Z", now), "23 hr ago");
        assert_eq!(relative_updated("2026-11-09T12:00:00Z", now), "1 day ago");
        assert_eq!(relative_updated("2026-10-11T12:00:01Z", now), "29 day ago");
        assert_eq!(relative_updated("2026-10-11T12:00:00Z", now), "1 m ago");
        assert_eq!(relative_updated("2025-11-16T12:00:00Z", now), "11 m ago");
        assert_eq!(relative_updated("2025-11-15T12:00:00Z", now), "11/15/25");
        // Future stamps clamp; unparseable stamps collapse the segment.
        assert_eq!(relative_updated("2026-11-10T12:00:01Z", now), "0 min ago");
        assert_eq!(relative_updated("not a date", now), "");
    }

    #[test]
    fn relative_time_old_date_reads_mm_dd_yy() {
        let now = at("2026-11-10T12:00:00Z");
        assert_eq!(relative_updated("2020-01-05T00:00:00Z", now), "01/05/20");
    }

    #[test]
    fn sidebar_row_joins_labels_date_and_status() {
        let d = sample_issue();
        let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
        let now = at("2026-01-07T00:00:00Z");
        let (first, labels, updated, status) =
            sidebar_lines(d.number, &d.title, &d.labels, &st, &d.updated_at, now, 40, false);
        assert!(first.starts_with("#1 some title here"), "{first}");
        assert_eq!(labels, "● needs-triage");
        assert_eq!(updated, "2 day ago");
        assert_eq!(status, format!("{} Todo", crate::theme::column_icon("Todo")));
        let line = format!("{labels} · {updated} · {status}");
        assert!(line.chars().count() <= 40, "{line}");
    }

    #[test]
    fn sidebar_labels_truncate_before_date_and_status() {
        let labels = vec![
            Label { name: "alpha".to_string() },
            Label { name: "beta".to_string() },
        ];
        let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
        let now = at("2026-01-07T00:00:00Z");
        let (_, kept, updated, status) =
            sidebar_lines(7, "t", &labels, &st, "2026-01-05T00:00:00Z", now, 30, false);
        assert_eq!(kept, "● alpha", "only the first chip fits");
        assert_eq!(updated, "2 day ago");
        assert!(status.ends_with("Todo"));
    }

    #[test]
    fn sidebar_segments_collapse_when_empty() {
        let now = at("2026-01-07T00:00:00Z");
        let (_, labels, updated, status) =
            sidebar_lines(7, "t", &[], &[], "", now, 40, false);
        assert_eq!((labels.as_str(), updated.as_str(), status.as_str()), ("", "", ""));
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
        let now = at("2026-01-07T00:00:00Z");
        let (first, _, _, _) = sidebar_lines(7, "t", &[], &[], "", now, 40, true);
        assert!(first.contains("📌"));
        let (plain, _, _, _) = sidebar_lines(7, "t", &[], &[], "", now, 40, false);
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
        assert_eq!(rows[1], SplitRow::Pair { left: " ctx".to_string(), right: " ctx".to_string(), old: Some(1), new: Some(1) });
        assert_eq!(rows[2], SplitRow::Pair { left: "-old".to_string(), right: "+new".to_string(), old: Some(2), new: Some(2) });
        assert_eq!(rows[3], SplitRow::Pair { left: "trail".to_string(), right: "trail".to_string(), old: Some(3), new: Some(3) });
    }

    #[test]
    fn split_rows_leave_unpaired_lines_alone() {
        let rows = split_rows("@@ -1 +1 @@\n-a\n-b\n+c");
        assert_eq!(rows[1], SplitRow::Pair { left: "-a".to_string(), right: "+c".to_string(), old: Some(1), new: Some(1) });
        assert_eq!(rows[2], SplitRow::LeftOnly { text: "-b".to_string(), kind: DiffKind::Del, line: Some(2) });
        assert!(split_rows("").is_empty());
    }

    #[test]
    fn patch_line_numbers_tracks_both_sides_across_hunks() {
        let numbers = patch_line_numbers("@@ -1,2 +10,3 @@\n ctx\n-old\n+new\n+more\n@@ -5,1 +15,1 @@\n tail");
        assert_eq!(numbers, vec![
            (None, None),
            (Some(1), Some(10)),
            (Some(2), None),
            (None, Some(11)),
            (None, Some(12)),
            (None, None),
            (Some(5), Some(15)),
        ]);
        // `\ No newline` markers carry no line on either side.
        let numbers = patch_line_numbers("@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file");
        assert_eq!(numbers, vec![(None, None), (Some(1), None), (None, Some(1)), (None, None)]);
    }

    #[test]
    fn cursor_maps_guttered_rows_to_new_side_lines() {
        // Rows render with a line-number gutter, exactly as `ui` emits.
        let patch = "@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n+more\n tail";
        let section: Vec<String> = ["     │ @@ -1,3 +1,3 @@", "   1 │  ctx", "   2 │ -old", "   2 │ +new", "   3 │ +more", "   3 │  tail"]
            .iter().map(|s| s.to_string()).collect();
        assert_eq!(line_target_at_cursor(&section, 0, patch), None, "hunk header has no line");
        assert_eq!(line_target_at_cursor(&section, 1, patch), Some(1));
        assert_eq!(line_target_at_cursor(&section, 2, patch), Some(1), "deletion keeps the line above");
        assert_eq!(line_target_at_cursor(&section, 3, patch), Some(2));
        assert_eq!(line_target_at_cursor(&section, 5, patch), Some(4));
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

/// Cap on nodes in one dependency graph (center included). The graph is a
/// 1-hop star around the current issue; a full-repo layout is out of scope.
pub const MAX_GRAPH_NODES: usize = 24;

/// Label chips shown per graph node before the `+k` overflow.


/// One relationship in the dependency graph, named from the center issue's
/// perspective: `Blocks` means the center blocks the neighbor, `BlockedBy`
/// the reverse, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphEdgeKind {
    Blocks,
    BlockedBy,
    Parent,
    Child,
    References,
    ReferencedBy,
    Closes,
}

/// Section order for the graph pane: blast radius first, then origins,
/// then plain references.
pub const GRAPH_SECTION_ORDER: [GraphEdgeKind; 7] = [
    GraphEdgeKind::Blocks,
    GraphEdgeKind::BlockedBy,
    GraphEdgeKind::Parent,
    GraphEdgeKind::Child,
    GraphEdgeKind::Closes,
    GraphEdgeKind::References,
    GraphEdgeKind::ReferencedBy,
];

/// Pane section header per edge kind.
pub fn graph_edge_label(kind: GraphEdgeKind) -> &'static str {
    match kind {
        GraphEdgeKind::Blocks => "Blocks",
        GraphEdgeKind::BlockedBy => "Blocked by",
        GraphEdgeKind::Parent => "Parent",
        GraphEdgeKind::Child => "Sub-issues",
        GraphEdgeKind::References => "References",
        GraphEdgeKind::ReferencedBy => "Referenced by",
        GraphEdgeKind::Closes => "Closing PRs",
    }
}

/// A PR linked to a graph node (closing reference), with its merge state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphLinkedPr {
    pub number: u64,
    #[serde(default)]
    pub state: String,
}

/// One issue or PR in the dependency graph: number, slug title, state,
/// project status, label chips, and linked PR numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub is_pr: bool,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub project_status: Option<String>,
    #[serde(default)]
    pub linked_prs: Vec<GraphLinkedPr>,
}

/// A directed relationship between two graph nodes. `Blocks` edges point
/// from the blocker to the blocked; every other kind points from the
/// neighbor toward the center or vice versa per `edge_for`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: u64,
    pub to: u64,
    pub kind: GraphEdgeKind,
}

/// The 1-hop dependency star around one issue: the center plus its capped
/// neighbors. `truncated` means neighbors were dropped at the cap;
/// `from_references` means blocked-by/blocking fields were unavailable and
/// reference edges were derived from body `#` refs instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphData {
    pub center: u64,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub from_references: bool,
}

/// Directed edge for one neighbor `kind`, from the center's perspective.
pub fn edge_for(center: u64, neighbor: u64, kind: GraphEdgeKind) -> GraphEdge {
    match kind {
        GraphEdgeKind::Blocks
        | GraphEdgeKind::Child
        | GraphEdgeKind::References => GraphEdge { from: center, to: neighbor, kind },
        GraphEdgeKind::BlockedBy
        | GraphEdgeKind::Parent
        | GraphEdgeKind::ReferencedBy
        | GraphEdgeKind::Closes => GraphEdge { from: neighbor, to: center, kind },
    }
}

/// Build the graph from the center plus neighbors in priority order (first
/// edge wins on duplicates). Nodes past `MAX_GRAPH_NODES` drop and set
/// `truncated`. The center node itself carries no edge.
pub fn build_graph(center: GraphNode, neighbors: Vec<(GraphNode, GraphEdgeKind)>) -> GraphData {
    let center_number = center.number;
    let mut nodes = vec![center];
    let mut edges = vec![];
    let mut truncated = false;
    for (node, kind) in neighbors {
        if node.number == center_number || nodes.iter().any(|n| n.number == node.number) {
            continue;
        }
        if nodes.len() >= MAX_GRAPH_NODES {
            truncated = true;
            continue;
        }
        edges.push(edge_for(center_number, node.number, kind));
        nodes.push(node);
    }
    GraphData { center: center_number, nodes, edges, truncated, from_references: false }
}

/// True when `number` blocks something: it is the blocker end of a
/// blocking relationship. Both `Blocks` and `BlockedBy` edges point from
/// the blocker to the blocked (the kind names the center's perspective),
/// so the blocker is always `from`. Nodes without one render gray/dim.
#[cfg(test)]
pub fn graph_blocks_something(data: &GraphData, number: u64) -> bool {
    data.edges.iter().any(|e| {
        e.from == number && matches!(e.kind, GraphEdgeKind::Blocks | GraphEdgeKind::BlockedBy)
    })
}

/// Node indices grouped by edge kind in `GRAPH_SECTION_ORDER`. The center
/// node carries no edge, so it never appears here; see `graph_order`.
pub fn graph_sections(data: &GraphData) -> Vec<(GraphEdgeKind, Vec<usize>)> {
    let kind_of = |idx: usize| {
        let n = data.nodes[idx].number;
        data.edges
            .iter()
            .find(|e| (e.from == n && e.to == data.center) || (e.from == data.center && e.to == n))
            .map(|e| e.kind)
    };
    let mut out = vec![];
    for kind in GRAPH_SECTION_ORDER {
        let mut group = vec![];
        for (idx, node) in data.nodes.iter().enumerate() {
            if node.number != data.center && kind_of(idx) == Some(kind) {
                group.push(idx);
            }
        }
        if !group.is_empty() {
            out.push((kind, group));
        }
    }
    out
}

/// Flat navigation order: the center first, then neighbors section by
/// section. Indices into `data.nodes`.
pub fn graph_order(data: &GraphData) -> Vec<usize> {
    let mut order = vec![];
    if let Some(center) = data.nodes.iter().position(|n| n.number == data.center) {
        order.push(center);
    }
    for (_, group) in graph_sections(data) {
        order.extend(group);
    }
    order
}

#[cfg(test)]
mod graph_tests {
    use super::*;

    fn node(number: u64) -> GraphNode {
        GraphNode {
            number,
            title: format!("issue {number}"),
            state: "OPEN".to_string(),
            is_pr: false,
            labels: vec![],
            project_status: None,
            linked_prs: vec![],
        }
    }

    #[test]
    fn edge_kinds_point_the_documented_way() {
        assert_eq!(edge_for(7, 9, GraphEdgeKind::Blocks), GraphEdge { from: 7, to: 9, kind: GraphEdgeKind::Blocks });
        assert_eq!(edge_for(7, 9, GraphEdgeKind::BlockedBy), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::BlockedBy });
        assert_eq!(edge_for(7, 9, GraphEdgeKind::Parent), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::Parent });
        assert_eq!(edge_for(7, 9, GraphEdgeKind::Child), GraphEdge { from: 7, to: 9, kind: GraphEdgeKind::Child });
        assert_eq!(edge_for(7, 9, GraphEdgeKind::Closes), GraphEdge { from: 9, to: 7, kind: GraphEdgeKind::Closes });
    }

    #[test]
    fn build_dedups_and_caps() {
        let neighbors = vec![
            (node(9), GraphEdgeKind::Blocks),
            (node(9), GraphEdgeKind::References),
            (node(7), GraphEdgeKind::Child),
        ];
        let data = build_graph(node(7), neighbors);
        assert_eq!(data.nodes.iter().map(|n| n.number).collect::<Vec<_>>(), vec![7, 9]);
        assert_eq!(data.edges, vec![edge_for(7, 9, GraphEdgeKind::Blocks)]);
        assert!(!data.truncated);

        let many: Vec<(GraphNode, GraphEdgeKind)> =
            (1..=40).map(|n| (node(n), GraphEdgeKind::References)).collect();
        let capped = build_graph(node(100), many);
        assert_eq!(capped.nodes.len(), MAX_GRAPH_NODES);
        assert!(capped.truncated);
    }

    #[test]
    fn blocker_end_of_either_kind_counts_as_blocking() {
        let data = build_graph(node(7), vec![(node(9), GraphEdgeKind::Blocks), (node(10), GraphEdgeKind::BlockedBy)]);
        assert!(graph_blocks_something(&data, 7));
        assert!(!graph_blocks_something(&data, 9));
        assert!(graph_blocks_something(&data, 10));
        assert!(!graph_blocks_something(&data, 999));
    }

    #[test]
    fn sections_follow_blast_radius_order_and_order_leads_with_center() {
        let data = build_graph(
            node(7),
            vec![
                (node(11), GraphEdgeKind::ReferencedBy),
                (node(9), GraphEdgeKind::Blocks),
                (node(10), GraphEdgeKind::Parent),
            ],
        );
        let labels: Vec<GraphEdgeKind> =
            graph_sections(&data).into_iter().map(|(k, _)| k).collect();
        assert_eq!(labels, vec![GraphEdgeKind::Blocks, GraphEdgeKind::Parent, GraphEdgeKind::ReferencedBy]);
        let order: Vec<u64> = graph_order(&data).into_iter().map(|i| data.nodes[i].number).collect();
        assert_eq!(order, vec![7, 9, 10, 11]);
    }

    #[test]
    fn edge_labels_cover_every_kind() {
        for kind in GRAPH_SECTION_ORDER {
            assert!(!graph_edge_label(kind).is_empty());
        }
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
