use super::*;

const GOOD: &str = r#"
[repos."owner/repo"]
site = "example.atlassian.net"
project = "PROJ"
email = "me@example.com"
"#;

fn path() -> PathBuf {
    PathBuf::from("/tmp/jtui-test/config.toml")
}

#[test]
fn good_config_parses_for_its_repo() {
    let parsed = parse_config(GOOD, &path()).unwrap();
    let config = repo_config(&parsed, &path(), "owner/repo").unwrap();
    assert_eq!(
        config,
        JiraRepoConfig {
            site: "example.atlassian.net".to_string(),
            project: "PROJ".to_string(),
            email: "me@example.com".to_string(),
        }
    );
}

#[test]
fn missing_repo_is_a_clear_error() {
    let parsed = parse_config(GOOD, &path()).unwrap();
    let error = repo_config(&parsed, &path(), "other/repo").unwrap_err();
    assert_eq!(
        error,
        StartupError::NoRepo { path: path(), repo: "other/repo".to_string() }
    );
    assert!(error.message().contains("other/repo"), "{}", error.message());
}

#[test]
fn missing_field_is_a_clear_error() {
    let text = r#"
[repos."owner/repo"]
site = "example.atlassian.net"
email = "me@example.com"
"#;
    let parsed = parse_config(text, &path()).unwrap();
    let error = repo_config(&parsed, &path(), "owner/repo").unwrap_err();
    assert_eq!(
        error,
        StartupError::MissingField { repo: "owner/repo".to_string(), field: "project" }
    );
    assert!(error.message().contains("project"), "{}", error.message());
}

#[test]
fn bad_toml_is_a_clear_error() {
    let error = parse_config("this is [not toml", &path()).unwrap_err();
    assert!(matches!(error, StartupError::BadToml { .. }), "{error:?}");
    assert!(error.message().contains("config.toml"), "{}", error.message());
}

#[test]
fn config_path_prefers_xdg_then_home() {
    assert_eq!(
        config_path_from(Some("/cfg"), Some("/home/me")),
        PathBuf::from("/cfg/jtui/config.toml")
    );
    assert_eq!(
        config_path_from(None, Some("/home/me")),
        PathBuf::from("/home/me/.config/jtui/config.toml")
    );
    assert_eq!(
        config_path_from(Some("  "), Some("/home/me")),
        PathBuf::from("/home/me/.config/jtui/config.toml")
    );
}

#[test]
fn missing_config_file_names_the_looked_up_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("jtui").join("config.toml");
    let error = load_repo_config_at("owner/repo", &missing).unwrap_err();
    assert_eq!(error, StartupError::NoConfig { path: missing.clone() });
    let message = error.message();
    assert!(message.contains(&missing.display().to_string()), "{message}");
    assert!(message.contains("example.atlassian.net"), "{message}");
    assert!(message.contains("PROJ"), "{message}");
}

#[test]
fn status_maps_to_startup_messages() {
    assert_eq!(startup_error_for_status(200, "https://x/myself", None), None);
    assert_eq!(
        startup_error_for_status(401, "https://x/myself", None),
        Some(StartupError::BadAuth)
    );
    assert_eq!(
        startup_error_for_status(403, "https://x/myself", None),
        Some(StartupError::BadAuth)
    );
    assert_eq!(
        startup_error_for_status(404, "https://x/project/PROJ", Some("PROJ")),
        Some(StartupError::UnknownProject { project: "PROJ".to_string() })
    );
    assert!(StartupError::BadAuth.message().contains("JIRA_API_TOKEN"));
    assert!(StartupError::UnknownProject { project: "PROJ".to_string() }
        .message()
        .contains("PROJ"));
    assert!(StartupError::MissingToken.message().contains("JIRA_API_TOKEN"));
    assert!(StartupError::Network { detail: "dns failed".to_string() }
        .message()
        .contains("dns failed"));
}

#[test]
fn base_url_uses_site_unless_the_env_overrides_it() {
    assert_eq!(
        base_url_with_override("example.atlassian.net", None),
        "https://example.atlassian.net"
    );
    assert_eq!(
        base_url_with_override("example.atlassian.net/", Some("http://127.0.0.1:9/")),
        "http://127.0.0.1:9"
    );
    assert_eq!(
        base_url_with_override("example.atlassian.net", Some("  ")),
        "https://example.atlassian.net"
    );
}

#[test]
fn client_urls_hit_api_v3() {
    let client = JiraClient::new(
        "example.atlassian.net",
        "me@example.com".to_string(),
        "secret".to_string(),
        "PROJ".to_string(),
    );
    assert_eq!(client.myself_url(), "https://example.atlassian.net/rest/api/3/myself");
    assert_eq!(
        client.project_url(),
        "https://example.atlassian.net/rest/api/3/project/PROJ"
    );
    assert_eq!(client.project(), "PROJ");
    assert_eq!(client.email(), "me@example.com");
}

#[test]
fn debug_output_redacts_the_token() {
    let (email, token) = ("me@example.com".to_string(), "s3cret-token".to_string());
    let client = JiraClient::new("example.atlassian.net", email, token, "PROJ".to_string());
    let shown = format!("{client:?}");
    assert!(!shown.contains("s3cret-token"));
    assert!(shown.contains("<redacted>"));
}
