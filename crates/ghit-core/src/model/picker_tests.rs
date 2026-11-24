use super::*;

fn names(meta: &RepoMeta) -> Vec<&str> {
    meta.labels.iter().map(|(name, _)| name.as_str()).collect()
}

fn milestone(title: &str) -> Milestone {
    Milestone { title: title.to_string(), ..Default::default() }
}

#[test]
fn a_side_tags_each_name_with_its_source() {
    let meta = RepoMeta::side(Source::Jira, vec!["bug".into()], vec![milestone("1.0")]);
    let jira_only = NameSource { github: false, jira: true };
    assert_eq!(meta.labels, vec![("bug".to_string(), jira_only)]);
    assert_eq!(meta.milestones[0].1.single(), Some(Source::Jira));
    assert_eq!(meta.error, None);
}

#[test]
fn merge_keeps_the_first_side_order_and_dedupes_names() {
    let github = RepoMeta::side(
        Source::GitHub, vec!["docs".into(), "bug".into()], vec![milestone("v1")]);
    let jira = RepoMeta::side(
        Source::Jira, vec!["checkout".into(), "bug".into()],
        vec![milestone("1.0"), milestone("v1")]);
    let merged = github.merge(jira);
    assert_eq!(names(&merged), ["docs", "bug", "checkout"], "the first side orders the list");
    let both = NameSource { github: true, jira: true };
    assert_eq!(merged.labels[1].1, both, "a shared name lists both");
    assert_eq!(merged.labels[1].1.single(), None, "a shared name shows no marker");
    assert_eq!(merged.labels[2].1.single(), Some(Source::Jira));
    assert_eq!(merged.milestones.len(), 2, "shared milestones list once");
    assert_eq!(merged.milestones[1].0.title, "1.0");
}

#[test]
fn a_failed_arm_keeps_the_other_sides_lists() {
    let meta = RepoMeta::side(Source::Jira, vec!["bug".into()], vec![])
        .with_error("no GitHub".into());
    assert_eq!(names(&meta), ["bug"]);
    assert_eq!(meta.error.as_deref(), Some("no GitHub"));
}
