use super::*;

fn jql(text: &str) -> Option<String> {
    from_search("PROJ", text)
}

#[test]
fn label_terms_become_labels_conditions() {
    assert_eq!(
        jql("label:bug"),
        Some("project = \"PROJ\" AND labels = \"bug\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("-label:wip"),
        Some("project = \"PROJ\" AND labels != \"wip\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql(r#"label:"good first""#),
        Some("project = \"PROJ\" AND labels = \"good first\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("label:Bug"),
        Some("project = \"PROJ\" AND labels = \"Bug\" ORDER BY updated DESC".to_string()),
        "the typed case is kept"
    );
}

#[test]
fn people_milestone_and_state_terms_translate() {
    assert_eq!(
        jql("author:ada"),
        Some("project = \"PROJ\" AND reporter = \"ada\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("assignee:amy"),
        Some("project = \"PROJ\" AND assignee = \"amy\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql(r#"milestone:"1.0""#),
        Some("project = \"PROJ\" AND fixVersion = \"1.0\" ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("no:milestone"),
        Some("project = \"PROJ\" AND fixVersion is EMPTY ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("is:open"),
        Some("project = \"PROJ\" AND statusCategory != Done ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("is:closed"),
        Some("project = \"PROJ\" AND statusCategory = Done ORDER BY updated DESC".to_string())
    );
    assert_eq!(
        jql("updated:>=2026-11-09"),
        Some("project = \"PROJ\" AND updated >= \"2026-11-09\" ORDER BY updated DESC".to_string())
    );
}

#[test]
fn every_term_joins_with_and() {
    assert_eq!(
        jql("is:open label:bug assignee:amy"),
        Some(
            "project = \"PROJ\" AND statusCategory != Done AND labels = \"bug\" AND \
             assignee = \"amy\" ORDER BY updated DESC"
                .to_string()
        )
    );
}

#[test]
fn bare_words_search_the_text() {
    assert_eq!(
        jql("checkout retries"),
        Some(concat!(
            "project = \"PROJ\" AND text ~ \"checkout\" AND text ~ \"retries\"",
            " ORDER BY updated DESC"
        ).to_string())
    );
    assert_eq!(
        jql(r#""two words""#),
        Some("project = \"PROJ\" AND text ~ \"two words\" ORDER BY updated DESC".to_string())
    );
}

#[test]
fn is_issue_alone_lists_the_project() {
    assert_eq!(
        jql("is:issue"),
        Some("project = \"PROJ\" ORDER BY updated DESC".to_string())
    );
}

#[test]
fn pr_only_terms_take_jira_out_of_the_search() {
    for text in ["is:pr", "is:merged", "label:bug is:pr", "is:merged label:bug"] {
        assert_eq!(jql(text), None, "{text}");
    }
}

#[test]
fn unknown_terms_take_jira_out_of_the_search() {
    for text in [
        "in:body crash",
        "review:approved",
        "sort:updated",
        "base:main",
        "is:draft",
        "-crash",
        "-author:bo",
        "-is:open",
        "label:a,b",
        "assignee:@me",
        "milestone:",
        "updated:<2026-11-09",
        "updated:>2026-11-09",
        "no:label",
        "",
        "   ",
        r#"label:"open"#,
    ] {
        assert_eq!(jql(text), None, "{text}");
    }
}

#[test]
fn quoting_escapes_the_value_and_the_project() {
    assert_eq!(
        from_search(r#"A"B"#, "label:bug"),
        Some(r#"project = "A\"B" AND labels = "bug" ORDER BY updated DESC"#.to_string())
    );
    assert_eq!(
        from_search("PROJ", r"label:a\b"),
        Some(r#"project = "PROJ" AND labels = "a\\b" ORDER BY updated DESC"#.to_string())
    );
}
