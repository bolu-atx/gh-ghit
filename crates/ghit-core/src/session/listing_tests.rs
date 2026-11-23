use super::*;
use crate::model::{IssueData, Source};

#[test]
fn section_flags_split_prs_from_issues() {
    assert_eq!(split_section_flags([true; 4]), ([true, true, false, false], [true; 2]));
    for mask in 0..16 {
        let show = std::array::from_fn(|i| mask & (1 << i) != 0);
        let (prs, issues) = split_section_flags(show);
        assert_eq!(prs, [show[0], show[1], false, false]);
        assert_eq!(issues, [show[2], show[3]]);
    }
}

fn row(number: ItemRef, is_pr: bool, title: &str) -> github::Row {
    github::Row {
        issue: IssueData { number, is_pr, title: title.to_string(), ..Default::default() },
        statuses: vec![("board".to_string(), Some("ready".to_string()))],
        detail: Default::default(),
        complete: true,
        window_at: None,
    }
}

#[test]
fn merged_sections_keep_both_sources_and_each_feeds_order_and_payload() {
    let github = ItemRef::github(7);
    let jira = ItemRef { source: Source::Jira, number: 7 };
    let prs = vec![row(github, true, "pr"), row(ItemRef::github(8), true, "second")];
    let issues = vec![row(jira, false, "issue")];
    let expected: Vec<_> = prs.iter().chain(&issues)
        .map(|row| serde_json::to_value(row).unwrap()).collect();
    let merged = merge_section_rows(prs, issues);
    assert_eq!(merged.iter().map(|row| row.issue.number).collect::<Vec<_>>(),
        vec![github, ItemRef::github(8), jira]);
    assert_eq!(merged.iter().map(|row| serde_json::to_value(row).unwrap()).collect::<Vec<_>>(),
        expected);
}

#[test]
fn merged_sections_allow_either_feed_to_be_empty() {
    assert!(merge_section_rows(vec![], vec![]).is_empty());
    let number = ItemRef::github(1);
    assert_eq!(merge_section_rows(vec![row(number, true, "pr")], vec![])[0].issue.number,
        number);
    assert_eq!(merge_section_rows(vec![], vec![row(number, false, "issue")])[0].issue.number,
        number);
}

fn jira_session() -> Session {
    let client = crate::jira::JiraClient::new(
        "example.atlassian.net",
        "me@example.com".to_string(),
        "secret".to_string(),
        "proj".to_string(),
    );
    Session::with_backend(None, IssueBackend::Jira(std::sync::Arc::new(client)))
}

#[test]
fn labels_follow_the_backend() {
    let jira = ItemRef { source: Source::Jira, number: 12 };
    assert_eq!(Session::new(None).label(ItemRef::github(12)), "#12");
    assert_eq!(jira_session().label(jira), "PROJ-12");
}

#[test]
fn a_jira_session_hides_github_issues_but_keeps_github_prs() {
    let rows = vec![
        row(ItemRef::github(1), false, "gh issue"),
        row(ItemRef::github(2), true, "gh pr"),
        row(ItemRef { source: Source::Jira, number: 3 }, false, "jira"),
    ];
    let titles = |rows: Vec<github::Row>| -> Vec<String> {
        rows.into_iter().map(|row| row.issue.title).collect()
    };
    assert_eq!(titles(jira_session().own_rows(rows.clone())), ["gh pr", "jira"]);
    assert_eq!(titles(Session::new(None).own_rows(rows)).len(), 3);
}

#[test]
fn a_window_landing_evicts_only_the_vanished_rows_of_either_source() {
    let mut session = jira_session();
    let jira = |number| ItemRef { source: Source::Jira, number };
    let pr = ItemRef::github(2);
    let all = vec![row(pr, true, "pr"), row(jira(1), false, "a"), row(jira(2), false, "b")];
    let held = Held { recent: vec![], keep: HashSet::new() };
    let landed = session.land_rows(all, false, &held);
    assert_eq!(landed.filed.len(), 3);
    let held = Held { recent: landed.filed, keep: HashSet::new() };
    let landed = session.land_rows(
        vec![row(pr, true, "pr"), row(jira(1), false, "a")], false, &held,
    );
    assert_eq!(landed.gone, vec![jira(2)]);
    assert!(session.has_item(pr) && session.has_item(jira(1)) && !session.has_item(jira(2)));
}

#[test]
fn jira_info_is_none_for_github_and_never_holds_the_token() {
    assert_eq!(Session::new(None).jira_info(), None);
    let info = jira_session().jira_info().expect("jira info");
    assert_eq!(info.site, "https://example.atlassian.net");
    assert_eq!(info.project, "proj");
    assert_eq!(info.email, "me@example.com");
    assert!(info.config_path.ends_with("jtui/config.toml"));
    assert!(!format!("{info:?}").contains("secret"));
}
