use serde_json::Value;
use std::sync::OnceLock;

/// Scopes ghit needs on the token: repo data (issues, PRs, comments,
/// search, diffs), org boards, and Projects v2 fields.
pub const REQUIRED_SCOPES: &[&str] = &["repo", "read:org", "project"];

/// Exact re-auth command naming every required scope, so the prompt
/// shows it and the user can also run it by hand.
pub fn reauth_command() -> String {
    let mut cmd = String::from("gh auth refresh");
    for s in REQUIRED_SCOPES {
        cmd.push_str(" -s ");
        cmd.push_str(s);
    }
    cmd
}

/// True when a fetch failure smells like auth (bad/expired token or
/// missing scopes) rather than a bad repo, number, payload, or rate
/// limit. Bare 403s stay out: rate limits are 403s with a quota body,
/// and only a scope-denial body (or a 401) may point at re-auth.
pub fn is_auth_error(e: &str) -> bool {
    const MARKERS: &[&str] = &[
        "401 Unauthorized", "Bad credentials", "Requires authentication",
        "Resource not accessible", "requires authentication",
        "missing scope", "missing required scopes", "Insufficient scopes",
        "INSUFFICIENT_SCOPES", "Could not resolve to a Repository",
        "FORBIDDEN", "UNAUTHORIZED", "SAML", "Must have admin rights",
    ];
    MARKERS.iter().any(|m| e.contains(m))
}

/// Append the re-auth command to an auth-class failure, so a bare
/// status line still tells the user how to fix it.
pub fn with_auth_hint(e: String) -> String {
    if is_auth_error(&e) {
        format!("{e} — re-auth with more permissions: {}", reauth_command())
    } else {
        e
    }
}

/// Scopes still missing from a granted set: pure so tests pin it
/// without the network.
pub fn missing_scopes(granted: &[&str]) -> Vec<String> {
    REQUIRED_SCOPES
        .iter()
        .filter(|s| !granted.contains(s))
        .map(|s| s.to_string())
        .collect()
}

/// Early auth probe: validates the token against `GET /rate_limit`.
/// That endpoint answers for every token kind (OAuth, PAT, GitHub App
/// installation): a dead token 401s, anything alive 200s. Scope gaps
/// need a repo, so those wait for `check_repo_access` below.
pub async fn check_auth(bearer: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/rate_limit", rest_base());
    let res = client()
        .get(&url)
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub auth check failed", &url, res).await);
    }
    Ok(vec![])
}

/// Repo-scoped access check, run once the repo is known: a 200 proves
/// the token reaches this repo, and the OAuth scopes header (absent on
/// installation and fine-grained tokens) names any missing scope.
/// Auth-class failures carry the re-auth hint; a 404 names the repo
/// plainly, since re-auth cannot fix a wrong slug.
pub async fn check_repo_access(bearer: &str, repo: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/repos/{repo}", rest_base());
    let res = client()
        .get(&url)
        .bearer_auth(bearer)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub repo access check failed", &url, res).await);
    }
    let granted: Vec<&str> = res
        .headers()
        .get("x-oauth-scopes")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(str::trim).collect())
        .unwrap_or_default();
    // No scopes header (installation or fine-grained token, or a blank
    // one): reachability above is all we know, and a blank list proves
    // no gap.
    if granted.iter().all(|s| s.is_empty()) {
        return Ok(vec![]);
    }
    Ok(missing_scopes(&granted))
}

static TOKEN_CACHE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn token() -> Option<String> {
    // Env wins and is re-read every call, so an export mid-run applies.
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            let t = t.trim().to_owned();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    let mut cached = TOKEN_CACHE.lock().unwrap();
    if let Some(t) = cached.clone() {
        return Some(t);
    }
    let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() { return None; }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    if token.is_empty() { return None; }
    *cached = Some(token.clone());
    Some(token)
}

/// Drop the cached `gh auth token` value so a re-auth (or logout)
/// takes effect without restarting the app.
pub fn forget_token() {
    *TOKEN_CACHE.lock().unwrap() = None;
}

/// Shared HTTP client: one connection pool for all background fetches.
/// `Client::clone` is cheap; every task shares the same pool.
pub(super) fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("ghit")
                .build()
                .expect("reqwest client")
        })
        .clone()
}

/// Map a failed response: the status plus the API's own first body
/// line, so a 403 rate limit never reads as a scope denial. Auth-class
/// failures carry the re-auth hint; everything else stays a plain error
/// for the status line.
pub(super) async fn http_fail(context: &str, url: &str, res: reqwest::Response) -> String {
    let status = res.status();
    let snippet: String = res
        .text()
        .await
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect();
    with_auth_hint(format!("{context}: {status} for {url}: {snippet}"))
}

pub(super) async fn get(client: &reqwest::Client, url: &str, bearer: &str) -> Result<Value, String> {
    let res = client
        .get(url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub request failed", url, res).await);
    }
    res.json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
}

pub(super) async fn post_json(client: &reqwest::Client, url: &str, bearer: &str, body: &Value) -> Result<Value, String> {
    let res = client
        .post(url)
        .bearer_auth(bearer)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("GitHub request failed: {e}"))?;
    if res.status().is_client_error() || res.status().is_server_error() {
        return Err(http_fail("GitHub request failed", url, res).await);
    }
    res.json()
        .await
        .map_err(|e| format!("bad GitHub response: {e}"))
}

/// Auth-class failure hiding inside a 200 GraphQL payload: the data
/// may be partial (nested fields denied), so surface it instead of
/// rendering the partial result as if it were whole.
pub(super) fn graphql_auth_failure(value: &Value) -> Option<String> {
    let errors = value.get("errors")?.as_array()?;
    if errors.is_empty() {
        return None;
    }
    let first = errors.first().map(|v| v.to_string()).unwrap_or_default();
    is_auth_error(&first).then(|| with_auth_hint(format!("GitHub GraphQL: {first}")))
}

/// Host override for GHES and tests: the one `GH_HOST` read behind
/// `rest_base` and `graphql_url`.
fn gh_host() -> String {
    std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".to_owned())
}

/// `GHIT_API_BASE` overrides both API endpoints for the local e2e fixture server.
fn api_endpoints(host: &str, base: Option<&str>) -> (String, String) {
    if let Some(base) = base.map(str::trim).filter(|base| !base.is_empty()) {
        let base = base.trim_end_matches('/');
        return (base.to_owned(), format!("{base}/graphql"));
    }
    if host == "github.com" {
        ("https://api.github.com".to_owned(), "https://api.github.com/graphql".to_owned())
    } else {
        (format!("https://{host}/api/v3"), format!("https://{host}/api/graphql"))
    }
}

pub(super) fn rest_base() -> String {
    api_endpoints(&gh_host(), std::env::var("GHIT_API_BASE").ok().as_deref()).0
}

pub(super) fn graphql_url() -> String {
    api_endpoints(&gh_host(), std::env::var("GHIT_API_BASE").ok().as_deref()).1
}

/// Unwrap a GraphQL response: transport errors already surfaced, so a
/// top-level `errors` array is the failure, and `data` the payload.
pub(super) fn graphql_data(value: &Value) -> Result<&Value, String> {
    if let Some(errors) = value.get("errors") {
        return Err(format!("GitHub GraphQL: {errors}"));
    }
    value.get("data").filter(|v| v.is_object()).ok_or_else(|| format!("GitHub GraphQL: {value}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn api_base_override_wins_over_host() {
        assert_eq!(api_endpoints("invalid.example", Some("http://127.0.0.1:1234/")),
            ("http://127.0.0.1:1234".into(), "http://127.0.0.1:1234/graphql".into()));
        for base in [Some(""), Some("  \t ")] {
            assert_eq!(api_endpoints("github.com", base), api_endpoints("github.com", None));
        }
        assert_eq!(api_endpoints("github.com", None),
            ("https://api.github.com".into(), "https://api.github.com/graphql".into()));
        assert_eq!(api_endpoints("enterprise.example", None),
            ("https://enterprise.example/api/v3".into(),
             "https://enterprise.example/api/graphql".into()));
    }

    #[test]
    fn auth_failures_carry_the_reauth_command() {
        for e in [
            "GitHub request failed: 401 Unauthorized for https://api.github.com/user: {\"message\":\"Bad credentials\"}",
            "GitHub request failed: 403 Forbidden for https://api.github.com/graphql: {\"message\":\"Resource not accessible by personal access token\"}",
            "GitHub GraphQL: Bad credentials",
            "GitHub GraphQL: Resource not accessible by personal access token",
            "Could not resolve to a Repository with the name of o/r",
        ] {
            assert!(is_auth_error(e), "auth: {e}");
            let hinted = with_auth_hint(e.to_string());
            assert!(hinted.contains("gh auth refresh -s repo -s read:org -s project"), "hint: {hinted}");
        }
        for e in [
            "GitHub request failed: 404 Not Found for https://api.github.com/repos/o/r/issues/401: {\"message\":\"Not Found\"}",
            "GitHub request failed: 403 Forbidden for https://api.github.com/rate_limit: {\"message\":\"API rate limit exceeded\"}",
            "bad search payload",
            "missing row number",
        ] {
            assert!(!is_auth_error(e), "not auth: {e}");
            assert_eq!(with_auth_hint(e.to_string()), e, "untouched: {e}");
        }
    }

    #[test]
    fn scope_gap_formats_classify_as_auth() {
        // GraphQL scope denial rides a 200 with the INSUFFICIENT_SCOPES
        // type plus GitHub's own message.
        let denied = json!({
            "data": {"repository": null},
            "errors": [{"type": "INSUFFICIENT_SCOPES",
                "message": "Your token has not been granted the required scopes to execute this query."}],
        });
        let err = graphql_auth_failure(&denied).expect("scope denial is auth-class");
        assert!(err.contains("gh auth refresh"), "hint: {err}");
        assert!(is_auth_error(&denied.to_string()), "raw payload: {denied}");
        // The gh CLI names the missing scopes in its own line.
        let cli = "error: your authentication token is missing required scopes [read:project]";
        assert!(is_auth_error(cli), "gh cli: {cli}");
        let hinted = with_auth_hint(cli.to_string());
        assert!(hinted.contains("gh auth refresh"), "hint: {hinted}");
    }

    #[test]
    fn graphql_auth_errors_fail_loudly_not_partially() {
        // Nested scope denial rides a 200 with partial data: surface it
        // instead of rendering the partial result as whole.
        let denied = json!({
            "data": {"repository": {"open": {"nodes": []}}},
            "errors": [{"type": "FORBIDDEN", "message": "Resource not accessible by personal access token"}],
        });
        let err = graphql_auth_failure(&denied).unwrap();
        assert!(err.contains("gh auth refresh"), "hint: {err}");
        // Auth-class failures elsewhere classify the same way.
        assert!(graphql_auth_failure(&json!({"errors": [{"message": "Bad credentials"}]})).is_some());
        // Non-auth field errors stay best-effort: no auth failure here.
        let benign = json!({
            "data": {"repository": {
                "open": {"nodes": []}, "merged": {"nodes": []},
                "openIssues": {"nodes": []}, "closedIssues": {"nodes": []}}},
            "errors": [{"message": "something odd on one field"}],
        });
        assert!(graphql_auth_failure(&benign).is_none());
    }
}
