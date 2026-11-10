//! Data-access facade: the only layer that talks to `api`, `gh`, and `cache`.
//!
//! Everything here is a free function over plain data. App state lives in
//! `app`, rendering in `ui`; neither layer imports `api`/`gh`/`cache`
//! directly. Cache keys stay in one place so both backends share them.

use crate::{api, cache, gh, model};
use crate::model::IssueData;

/// `(issue, [(project title, status|None)])`.
pub type IssueBundle = (IssueData, Vec<(String, Option<String>)>);

/// Owner half of an `owner/repo` slug, for board fetches.
pub fn board_owner(repo: Option<&str>) -> Option<String> {
    let r = repo?;
    let (owner, _) = r.split_once('/')?;
    if owner.is_empty() { None } else { Some(owner.to_string()) }
}

/// Resolve the working repository: explicit `-R` flag wins, otherwise ask
/// `gh` what repo the cwd belongs to.
pub fn resolve_repo(flag: Option<&str>) -> Option<String> {
    if let Some(f) = flag {
        return Some(f.to_string());
    }
    let out = std::process::Command::new("gh")
        .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Fetch one issue/PR plus its project statuses. Token-backed REST/GraphQL
/// when authenticated, `gh` CLI otherwise. Failures are strings for the
/// status line, never panics.
pub fn fetch_number(repo: &str, number: u64, force: bool) -> Result<IssueBundle, String> {
    let raw = repo.to_lowercase().replace('/', "__");
    let tag = if raw.is_empty() { "local".to_string() } else { raw };
    let iname = format!("issue-{tag}-{number}");
    let sname = format!("status-{tag}-{number}");

    if let Some(token) = api::token().filter(|_| !repo.is_empty()) {
        if !force {
            if let (Some(data), Some(statuses)) =
                (cache::cache_get(&iname), cache::cache_get(&sname))
            {
                return Ok((data, statuses));
            }
        }
        let (data, statuses) = api::issue(repo, number, token)?;
        cache::cache_put(&iname, &data);
        cache::cache_put(&sname, &statuses);
        return Ok((data, statuses));
    }

    let data: IssueData = if !force {
        match cache::cache_get::<IssueData>(&iname) {
            Some(d) => d,
            None => {
                let d = gh::gh_issue_json(repo, number)?;
                cache::cache_put(&iname, &d);
                d
            }
        }
    } else {
        let d = gh::gh_issue_json(repo, number)?;
        cache::cache_put(&iname, &d);
        d
    };

    let statuses: Vec<(String, Option<String>)> = if !force {
        match cache::cache_get(&sname) {
            Some(s) => s,
            None => {
                let s = gh::gh_project_statuses(repo, number, data.is_pr);
                cache::cache_put(&sname, &s);
                s
            }
        }
    } else {
        let s = gh::gh_project_statuses(repo, number, data.is_pr);
        cache::cache_put(&sname, &s);
        s
    };
    Ok((data, statuses))
}

/// Open PRs plus PRs merged inside `hours`, as full rows. Token backend
/// returns sections directly; the `gh` fallback resolves numbers then
/// fetches each one (cached, so warm rows skip the network).
pub fn fetch_pr_sections(repo: &str, hours: u64) -> Result<Vec<api::Row>, String> {
    if let Some(token) = api::token() {
        return api::sections(repo, hours, token);
    }
    let mut numbers = gh::gh_open_prs(repo);
    numbers.extend(gh::gh_recently_merged_prs(repo, hours));
    Ok(numbers
        .into_iter()
        .filter_map(|n| fetch_number(repo, n, false).ok())
        .map(|(issue, statuses)| {
            let detail = model::BoardDetail::from_issue(&issue);
            api::Row { issue, statuses, detail, complete: true }
        })
        .collect())
}

/// Per-card detail: dates plus comment count.
pub fn fetch_board_detail(repo: &str, number: u64) -> Result<model::BoardDetail, String> {
    if let Some(token) = api::token() {
        return api::issue(repo, number, token).map(|(d, _)| model::BoardDetail::from_issue(&d));
    }
    gh::gh_issue_json(repo, number).map(|d| model::BoardDetail::from_issue(&d))
}
