use super::*;

#[test]
fn github_shares_the_sections_query() {
    assert!(IssueBackend::GitHub.shares_github_sections());
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
