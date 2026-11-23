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
