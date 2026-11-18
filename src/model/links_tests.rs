//! Tests for model::links.

use super::*;
use super::links::MAX_REFS;

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
fn link_ranges_cover_the_whole_token() {
    let line = "see #12, https://example.com/x. and https://github.com/o/r/pull/7 (C#9)";
    let got: Vec<&str> = link_ranges(line, Some("o/r")).into_iter().map(|(a, b, _)| &line[a..b]).collect();
    assert_eq!(got, ["#12", "https://example.com/x", "https://github.com/o/r/pull/7"]);
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
