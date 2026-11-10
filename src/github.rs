//! Data access over HTTPS (`api` + `cache`).
//!
//! The `gh` CLI subprocess backend is deprecated: `gh.rs` keeps only the
//! project board until it moves to GraphQL, and nothing else falls back to
//! it. Missing auth or repo is a status-line error, never silent behavior.

use crate::{api, cache, model};
use crate::model::IssueData;

/// `(issue, [(project title, status|None)])`.
pub type IssueBundle = (IssueData, Vec<(String, Option<String>)>);

/// Owner half of an `owner/repo` slug, for board fetches.
pub fn board_owner(repo: Option<&str>) -> Option<String> {
    let r = repo?;
    let (owner, _) = r.split_once('/')?;
    if owner.is_empty() { None } else { Some(owner.to_string()) }
}

/// Resolve the working repository: explicit `-R` flag wins, otherwise the
/// `origin` remote of this checkout.
pub fn resolve_repo(flag: Option<&str>) -> Option<String> {
    if let Some(f) = flag {
        return Some(f.to_string());
    }
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_remote_url(String::from_utf8_lossy(&out.stdout).trim())
}

/// `owner/repo` out of a remote URL: `git@host:o/r.git`,
/// `https://host/o/r(.git)`, or any `<host>/o/r` shape. Pure so tests pin
/// the shapes; deeper nestings resolve to their last two segments.
pub fn parse_remote_url(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches('/');
    let t = t.strip_suffix(".git").unwrap_or(t);
    let t = t.rsplit_once("://").map(|(_, rest)| rest).unwrap_or(t);
    let segs: Vec<&str> = t.split(['/', ':']).filter(|s| !s.is_empty()).collect();
    if segs.len() < 3 {
        return None;
    }
    let (owner, repo) = (segs[segs.len() - 2], segs[segs.len() - 1]);
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

fn require_repo(repo: &str) -> Result<(), String> {
    if repo.is_empty() {
        return Err("repository unknown: pass -R owner/repo or run inside a checkout".to_string());
    }
    Ok(())
}

fn require_token() -> Result<&'static str, String> {
    api::token().ok_or_else(|| "GitHub token missing: set GH_TOKEN or run `gh auth login`".to_string())
}

/// Fetch one issue/PR plus its project statuses. Failures are strings for
/// the status line, never panics.
pub async fn fetch_number(repo: &str, number: u64, force: bool) -> Result<IssueBundle, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let raw = repo.to_lowercase().replace('/', "__");
    let iname = format!("issue-{raw}-{number}");
    let sname = format!("status-{raw}-{number}");

    if !force {
        if let (Some(data), Some(statuses)) =
            (cache::cache_get(&iname), cache::cache_get(&sname))
        {
            return Ok((data, statuses));
        }
    }
    let (data, statuses) = api::issue(repo, number, token).await?;
    cache::cache_put(&iname, &data);
    cache::cache_put(&sname, &statuses);
    Ok((data, statuses))
}

/// Recent rows: open + windowed merged PRs, open + windowed closed issues.
pub async fn fetch_recent_sections(repo: &str, hours: u64) -> Result<Vec<api::Row>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::sections(repo, hours, token).await
}

/// Server-side issue/PR search scoped to the repo.
pub async fn search_repo(repo: &str, query: &str) -> Result<Vec<api::Row>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::search(repo, query, token).await
}

/// Per-card detail: dates plus comment count.
pub async fn fetch_board_detail(repo: &str, number: u64) -> Result<model::BoardDetail, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::issue(repo, number, token).await.map(|(d, _)| model::BoardDetail::from_issue(&d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_resolve_to_owner_repo() {
        assert_eq!(parse_remote_url("git@github.com:o/r.git"), Some("o/r".to_string()));
        assert_eq!(parse_remote_url("https://github.com/o/r.git"), Some("o/r".to_string()));
        assert_eq!(parse_remote_url("https://github.com/o/r"), Some("o/r".to_string()));
        assert_eq!(parse_remote_url("ssh://git@ghe.corp:22/o/r.git"), Some("o/r".to_string()));
        assert_eq!(parse_remote_url("https://ghe.corp/o/r/"), Some("o/r".to_string()));
    }

    #[test]
    fn remote_urls_reject_non_repos() {
        assert_eq!(parse_remote_url("origin"), None);
        assert_eq!(parse_remote_url(""), None);
        assert_eq!(parse_remote_url("https://github.com/o"), None);
    }

    #[tokio::test]
    async fn fetches_fail_before_the_network_without_a_repo() {
        // Empty repo errors before any token lookup or request, so these
        // hold with or without auth configured.
        assert!(fetch_number("", 1, false).await.is_err());
        assert!(fetch_recent_sections("", 24).await.is_err());
        assert!(search_repo("", "bug").await.is_err());
        assert!(fetch_board_detail("", 1).await.is_err());
    }
}
