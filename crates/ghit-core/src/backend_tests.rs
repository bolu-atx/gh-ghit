use super::*;

#[test]
fn github_shares_the_sections_query() {
    assert!(IssueBackend::GitHub.shares_github_sections());
}

#[test]
fn jira_does_not_share_github_sections() {
    let client = crate::jira::JiraClient::new(
        "example.atlassian.net",
        "me@example.com".to_string(),
        "secret".to_string(),
        "PROJ".to_string(),
    );
    assert!(!IssueBackend::Jira(std::sync::Arc::new(client)).shares_github_sections());
}

#[test]
fn the_project_key_comes_from_the_jira_backend_only() {
    let client = crate::jira::JiraClient::new(
        "example.atlassian.net",
        "me@example.com".to_string(),
        "secret".to_string(),
        "PROJ".to_string(),
    );
    assert_eq!(IssueBackend::Jira(std::sync::Arc::new(client)).project_key(), "PROJ");
    assert_eq!(IssueBackend::GitHub.project_key(), "");
}

#[tokio::test]
async fn jira_detail_is_not_implemented() {
    let client = crate::jira::JiraClient::new(
        "example.atlassian.net",
        "me@example.com".to_string(),
        "secret".to_string(),
        "PROJ".to_string(),
    );
    let backend = IssueBackend::Jira(std::sync::Arc::new(client));
    let number = ItemRef { source: crate::model::Source::Jira, number: 1 };
    assert!(backend.fetch_number("owner/repo", number, false, None).await.unwrap_err().contains("#315"));
    assert!(backend.fetch_sub_issues("owner/repo", number).await.unwrap_err().contains("#315"));
    assert!(backend.set_state("owner/repo", number, true).await.unwrap_err().contains("#315"));
    assert!(backend.post_comment("owner/repo", number, "body").await.unwrap_err().contains("#315"));
}

#[tokio::test]
async fn github_operations_preserve_missing_repo_errors() {
    let backend = IssueBackend::GitHub;
    let number = ItemRef::github(1);
    let expected = "repository unknown: pass -R owner/repo or run inside a checkout";
    assert_eq!(backend.fetch_number("", number, false, None).await.unwrap_err(), expected);
    assert_eq!(backend.fetch_sub_issues("", number).await.unwrap_err(), expected);
    assert_eq!(backend.set_state("", number, false).await.unwrap_err(), expected);
    assert_eq!(backend.set_state("", number, true).await.unwrap_err(), expected);
    assert_eq!(backend.post_comment("", number, "body").await.unwrap_err(), expected);
}
