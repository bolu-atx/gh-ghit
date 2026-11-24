//! Tests for model::links.

use super::*;
use super::links::MAX_REFS;
use crate::model::ItemRef;

#[test]
fn linkify_skips_code() {
    // Issue refs render as plain text; only real URLs keep destinations.
    assert_eq!(linkify("see (#1) and #2"), "see (#1) and #2");
    assert!(!linkify("see (#1)").contains("issue:"));
    assert_eq!(linkify("`#3` and ```\n(#4)\n```"), "`#3` and ```\n(#4)\n```");
    assert_eq!(linkify("[#4](http://x)"), "[#4](http://x)");
}

fn nums(line: &str, repo: Option<&str>) -> Vec<ItemRef> {
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
    assert_eq!(
        nums("see (#1) and #2", None),
        vec![ItemRef::github(1), ItemRef::github(2)]
    );
    assert_eq!(nums("[#4](issue:4)", None), vec![ItemRef::github(4)]);
    assert_eq!(nums("no refs here", None), Vec::<ItemRef>::new());
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
        vec![
            Url("https://example.com/a".to_string()),
            Issue(ItemRef::github(34))
        ]
    );
}

#[test]
fn links_in_line_sends_same_repo_urls_in_app() {
    use LinkTarget::{Issue, Url};
    let line = "backport of #12 (https://github.com/o/r/pull/12), see #34 (https://github.com/x/y/issues/34)";
    assert_eq!(
        targets(line, Some("o/r")),
        vec![
            Issue(ItemRef::github(12)),
            Url("https://github.com/x/y/issues/34".to_string())
        ]
    );
}

#[test]
fn refs_in_line_filters_urls_by_repo() {
    let line = "see https://github.com/o/r/pull/12 and https://github.com/x/y/issues/34";
    assert_eq!(nums(line, Some("o/r")), vec![ItemRef::github(12)]);
    assert_eq!(nums(line, Some("O/R")), vec![ItemRef::github(12)]);
    assert_eq!(nums(line, None), Vec::<ItemRef>::new());
}

#[test]
fn referenced_numbers_dedups_orders_and_skips_code() {
    let body = "blocked by #2 and (#3), see #2 again\n`#9` ignored\n```\n#10 ignored\n```\nhttps://github.com/o/r/pull/4 and https://github.com/x/y/issues/5";
    assert_eq!(
        referenced_numbers(body, Some("o/r")),
        vec![ItemRef::github(2), ItemRef::github(3), ItemRef::github(4)]
    );
}

#[test]
fn referenced_numbers_caps_at_max_refs() {
    let body = (1..=30).map(|n| format!("#{n}")).collect::<Vec<_>>().join(" ");
    let refs = referenced_numbers(&body, Some("o/r"));
    assert_eq!(refs.len(), MAX_REFS);
    assert_eq!(refs[0], ItemRef::github(1));
}

fn jira(number: u64) -> ItemRef {
    ItemRef { source: crate::model::Source::Jira, number }
}

#[test]
fn jira_scan_matches_only_the_configured_key_case_insensitively() {
    assert_eq!(
        jira_refs_for_key("PROJ", "Fix proj-12 and PROJ-12", "", "also Proj-34"),
        vec![jira(12), jira(34)]
    );
    assert_eq!(
        jira_refs_for_key("PROJ", "OTHER-9 and PROJX-9", "", ""),
        Vec::<ItemRef>::new()
    );
    assert_eq!(jira_refs_for_key("", "PROJ-1", "", ""), Vec::<ItemRef>::new());
    assert_eq!(jira_refs_for_key("PROJ", "", "", ""), Vec::<ItemRef>::new());
}

#[test]
fn jira_scan_respects_word_boundaries_and_branch_names() {
    assert_eq!(
        jira_refs_for_key("PROJ", "x", "feat/PROJ-12-thing", ""),
        vec![jira(12)]
    );
    assert_eq!(
        jira_refs_for_key("PROJ", "XPROJ-12 PROJ-12abc (PROJ-13) PROJ-14.", "", ""),
        vec![jira(13), jira(14)]
    );
    // First-seen order across title, branch and body, deduped.
    assert_eq!(
        jira_refs_for_key("PROJ", "PROJ-2", "feat/PROJ-1-x", "PROJ-2 again, PROJ-3"),
        vec![jira(2), jira(1), jira(3)]
    );
}

#[test]
fn jira_refs_are_clickable_links_when_a_key_is_set() {
    crate::model::set_jira_key("PROJ");
    assert_eq!(
        nums("see PROJ-123 and proj-45, OTHER-9", None),
        vec![jira(123), jira(45)]
    );
    let line = "see PROJ-123, done";
    let got: Vec<&str> = link_ranges(line, None)
        .into_iter()
        .map(|(a, b, _)| &line[a..b])
        .collect();
    assert_eq!(got, ["PROJ-123"]);
    assert_eq!(
        referenced_numbers("fixes PROJ-7 and #8", None),
        vec![jira(7), ItemRef::github(8)]
    );
}

#[test]
fn nearest_link_picks_closest_ref_on_shared_row() {
    let line = "backport of #12, see also #34 for context";
    let links = links_in_line(line, None);
    assert_eq!(links.len(), 2);
    let near_first = line.find("#12").unwrap();
    let near_second = line.find("#34").unwrap();
    assert_eq!(
        nearest_link(line, &links, near_first),
        Some(LinkTarget::Issue(ItemRef::github(12)))
    );
    assert_eq!(
        nearest_link(line, &links, near_second),
        Some(LinkTarget::Issue(ItemRef::github(34)))
    );
    assert_eq!(nearest_link(line, &[], 0), None);
}
