use super::{ItemRef, Source};
use std::collections::{BTreeSet, HashSet};

#[test]
fn source_distinguishes_equal_numbers_in_sets_and_serialization() {
    let github = ItemRef::github(45);
    let jira = ItemRef {
        source: Source::Jira,
        number: 45,
    };
    assert_ne!(github, jira);
    assert_eq!(HashSet::from([github, jira]).len(), 2);
    assert_eq!(BTreeSet::from([github, jira]).len(), 2);
    for item in [github, jira] {
        let json = serde_json::to_string(&item).unwrap();
        assert_eq!(serde_json::from_str::<ItemRef>(&json).unwrap(), item);
    }
    assert_eq!(github.display("PROJ"), "#45");
    assert_eq!(jira.display("PROJ"), "PROJ-45");
    assert_eq!(jira.display("OTHER"), "OTHER-45");
}

#[test]
fn source_tags_are_stable_and_parse_only_known_sources() {
    assert_eq!(Source::GitHub.as_str(), "github");
    assert_eq!(Source::Jira.as_str(), "jira");
    for source in [Source::GitHub, Source::Jira] {
        assert_eq!(Source::parse(source.as_str()), Some(source));
    }
    for tag in ["", "GitHub", "Jira", "other"] {
        assert_eq!(Source::parse(tag), None);
    }
}
