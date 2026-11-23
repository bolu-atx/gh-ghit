//! Links and refs: `linkify`, `#123` / URL scanning, link targets and hit-testing.

use std::sync::OnceLock;

use regex::Regex;

use crate::model::ItemRef;

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

/// Cap on related-issue rows pulled out of one body.
pub const MAX_REFS: usize = 20;

/// Issue/PR numbers mentioned in a raw body, in first-seen order, deduped.
/// Same-repo `github.com` URLs count; cross-repo URLs and code spans do not.
pub fn referenced_numbers(body: &str, repo: Option<&str>) -> Vec<ItemRef> {
    use std::collections::HashSet;
    let mut out: Vec<ItemRef> = vec![];
    let mut seen: HashSet<ItemRef> = HashSet::new();
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
    Issue(ItemRef),
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
    link_spans(line, repo).into_iter().map(|(start, _, t)| (start, t)).collect()
}

/// Byte ranges and targets of everything `links_in_line` would resolve,
/// so the renderer colors exactly what a click opens.
pub fn link_ranges(line: &str, repo: Option<&str>) -> Vec<(usize, usize, LinkTarget)> {
    // Match offsets skip the `#` of a bare ref and the scheme of a
    // same-repo URL; widen back over them so the whole token colors.
    link_spans(line, repo)
        .into_iter()
        .map(|(start, end, target)| {
            let head = &line[..start];
            let lead = ["https://", "http://", "#"].iter().find(|p| head.ends_with(**p)).map_or(0, |p| p.len());
            (start - lead, end, target)
        })
        .collect()
}

fn link_spans(line: &str, repo: Option<&str>) -> Vec<(usize, usize, LinkTarget)> {
    let mut hits: Vec<(usize, usize, LinkTarget)> = vec![];
    let mut taken: Vec<(usize, usize)> = vec![];
    let claim = |start: usize, end: usize, target: LinkTarget, hits: &mut Vec<(usize, usize, LinkTarget)>, taken: &mut Vec<(usize, usize)>| {
        if taken.iter().any(|&(s, e)| start < e && s < end) {
            return;
        }
        hits.push((start, end, target));
        taken.push((start, end));
    };
    for cap in ref_url_re().captures_iter(line) {
        let m = cap.get(0).unwrap();
        if same_repo(&cap, repo) {
            if let Ok(n) = cap[3].parse::<u64>().map(ItemRef::github) {
                claim(m.start(), m.end(), LinkTarget::Issue(n), &mut hits, &mut taken);
            }
        }
    }
    for cap in ref_bare_re().captures_iter(line) {
        let m = cap.get(1).unwrap();
        if followed_by_url_dest(line, m.end()) {
            continue;
        }
        if let Ok(n) = m.as_str().parse::<u64>().map(ItemRef::github) {
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
            claim(m.start(), m.start() + url.len(), LinkTarget::Url(url), &mut hits, &mut taken);
        }
    }
    hits.sort_by_key(|(idx, _, _)| *idx);
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
