//! Data access over HTTPS (`api` + `cache` + `store`).
//!
//! Missing auth or repo is a status-line error, never silent behavior.

use crate::{api, cache, model, store};
use crate::model::IssueData;

pub use crate::api::{Row, REQUIRED_SCOPES, forget_token, is_auth_error, rate_limited, reauth_command, token};

/// `(issue, [(project title, status|None)])`.
pub type IssueBundle = (IssueData, Vec<(String, Option<String>)>);

/// Issue-fetch traffic on the existing issue channel: comment-page
/// progress (`done` pages of `total`) while a large thread streams in,
/// then the terminal result. Single-request fetches send only `Done`.
#[derive(Debug)]
pub enum IssueEvent {
    Progress { done: usize, total: usize },
    Done(Result<Box<IssueBundle>, String>),
}

/// Section-fetch traffic on the existing section channel: rows fetched
/// vs the row cap while more pages remain, then the terminal result.
#[derive(Debug)]
pub enum SectionEvent {
    Progress { done: usize, total: usize },
    /// The fetch stopped at the shared sidebar row cap. Sent before
    /// `Done`.
    Capped,
    /// The change probe found nothing new: no fetch ran, rows stay put.
    Unchanged,
    Done(Result<Vec<api::Row>, String>),
}

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

fn require_token() -> Result<String, String> {
    api::token().ok_or_else(|| "GitHub token missing: set GH_TOKEN or run `gh auth login`".to_string())
}

/// Early auth probe result: a dead token errors, a live one reports
/// which required scopes it still lacks (empty means fully covered).
pub async fn check_auth() -> Result<Vec<String>, String> {
    let token = require_token()?;
    api::check_auth(&token).await
}

/// Repo-scoped probe: reachability plus the OAuth scope gap for one
/// slug. Same result shape as `check_auth`.
pub async fn check_repo_access(repo: &str) -> Result<Vec<String>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::check_repo_access(&token, repo).await
}

/// Fetch one issue/PR plus its project statuses. Failures are strings for
/// the status line, never panics. File-cache access runs on the blocking
/// pool so async workers never touch the disk. `progress` re-tags the
/// comment-page counts onto the caller's existing issue channel; `None`
/// keeps the fetch silent. Same requests either way.
pub async fn fetch_number(
    repo: &str,
    number: u64,
    force: bool,
    progress: Option<std::sync::mpsc::Sender<(u64, IssueEvent)>>,
) -> Result<IssueBundle, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let iname = cache::Key::Issue { repo: repo.to_string(), number }.name();
    let sname = cache::Key::Status { repo: repo.to_string(), number }.name();

    if !force {
        let iname_c = iname.clone();
        let sname_c = sname.clone();
        let hit: Option<IssueBundle> = tokio::task::spawn_blocking(move || {
            match (cache::cache_get(&iname_c), cache::cache_get(&sname_c)) {
                (Some(data), Some(statuses)) => Some((data, statuses)),
                _ => None,
            }
        })
        .await
        .map_err(|e| format!("cache read failed: {e}"))?;
        if let Some(bundle) = hit {
            return Ok(bundle);
        }
    }
    let comments_name = cache::Key::Comments { repo: repo.to_string(), number }.name();
    let cached_comments = if force { None } else {
        let name = comments_name.clone();
        tokio::task::spawn_blocking(move || cache::cache_get(&name)).await.unwrap_or(None)
    };
    let comments_fetched = cached_comments.is_none();
    let report = |done: usize, total: usize| {
        if let Some(tx) = &progress {
            let _ = tx.send((number, IssueEvent::Progress { done, total }));
        }
    };
    let (data, statuses) =
        api::issue(repo, number, &token, cached_comments, Some(&report as &(dyn Fn(usize, usize) + Send + Sync))).await?;
    let (data_c, statuses_c) = (data.clone(), statuses.clone());
    let _ = tokio::task::spawn_blocking(move || {
        if comments_fetched { cache::cache_put(&comments_name, &data_c.comments); }
        cache::cache_put(&iname, &data_c);
        cache::cache_put(&sname, &statuses_c);
    })
    .await;
    Ok((data, statuses))
}

/// Stored rows that can stand in for this window and list mask, with
/// the sync that wrote them: the shown rows only, in sidebar order.
/// `None` when the store is empty or unreadable, the last sync is an
/// hour old or more (`api::delta_since`), or its walk did not cover this window and
/// mask. Blocking: call from the blocking pool.
fn load_sections(repo: &str, hours: u64, show: [bool; 4]) -> Option<(store::Sync, Vec<api::Row>)> {
    let store = store::open(repo).ok()?;
    let sync = store.last_sync()?;
    let now = chrono::Utc::now();
    api::delta_since(Some(&sync.started_at), now, false)?;
    if !sync.covers(hours, show) {
        return None;
    }
    let rows = api::visible_rows(store.rows().ok()?, now, hours, show);
    Some((sync, rows))
}

/// Every stored item of `repo` for the finder, newest update first. Empty
/// when the repo has no store file yet (never creates one). Local read only.
pub fn finder_candidates(repo: &str) -> Vec<store::Candidate> {
    if !store::path(repo).is_some_and(|p| p.exists()) {
        return vec![];
    }
    store::open(repo).and_then(|s| s.finder_candidates()).unwrap_or_default()
}

/// Write the sidebar result `rows` in order, plus every other row the
/// fetch returned (rows no shown list holds now keep their new state),
/// and record `sync`. Best effort: the next sync rewrites. Blocking.
fn save_sections(repo: &str, sync: &store::Sync, rows: &[api::Row], fetched: Vec<api::Row>) {
    let listed: std::collections::HashSet<u64> = rows.iter().map(|r| r.issue.number).collect();
    let other: Vec<api::Row> = fetched.into_iter().filter(|r| !listed.contains(&r.issue.number)).collect();
    if let Ok(mut store) = store::open(repo) {
        let _ = store.write(sync, rows, &other);
    }
}

/// How a sections refresh runs: a change-only search (`Delta`), the same
/// plus a CI/project status refresh of the open rows (`Status`), or a
/// full walk of every list (`Full`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Refresh {
    #[default]
    Delta,
    Status,
    Full,
}

/// Recent rows: open + windowed merged PRs, open + windowed closed issues.
/// `show` is the visible-list mask (open PRs, merged PRs, open issues, closed
/// issues); hidden lists are not queried.
/// Rows persist in the repo's store (`store`), with the start of the
/// fetch that wrote them and the window and mask its walk covered. A sync
/// under an hour old that covered this window and mask refreshes through
/// one change-only search (`api::changed_rows`) merged into the stored rows;
/// `Refresh::Full`, no usable sync, or an older one walks the lists head-first
/// and stops on overlap with the stored rows. `progress` re-tags the page counts onto the
/// caller's existing section channel. `Refresh::Status` also refetches CI and
/// project status of the open rows (best effort) after its change-only search.
pub async fn fetch_recent_sections(
    repo: &str,
    hours: u64,
    show: [bool; 4],
    refresh: Refresh,
    progress: Option<std::sync::mpsc::Sender<SectionEvent>>,
) -> Result<Vec<api::Row>, String> {
    require_repo(repo)?;
    // Every list hidden: nothing to fetch.
    if !show.iter().any(|s| *s) {
        return Ok(Vec::new());
    }
    let token = require_token()?;
    let started = chrono::Utc::now();
    let read_repo = repo.to_string();
    let cached = tokio::task::spawn_blocking(move || load_sections(&read_repo, hours, show)).await.unwrap_or(None);
    let since = api::delta_since(cached.as_ref().map(|c| c.0.started_at.as_str()), started, refresh == Refresh::Full);
    let mut sync = store::Sync { started_at: started.to_rfc3339(), hours, show };
    let mut walked = false;
    let (mut rows, capped, fetched) = match (since, cached) {
        (Some(since), Some((last, cached))) => {
            // A change-only search keeps the walk coverage it builds on.
            (sync.hours, sync.show) = (last.hours, last.show);
            let delta = api::changed_rows(repo, since, &token).await?;
            (api::merge_delta(delta.clone(), cached, chrono::Utc::now(), hours, show), false, delta)
        }
        (_, cached) => {
            let cached = cached.map(|c| c.1);
            let cached_numbers: Option<std::collections::HashSet<u64>> =
                cached.as_ref().map(|rows| rows.iter().map(|r| r.issue.number).collect());
            let report = |done: usize, total: usize| {
                if let Some(tx) = &progress {
                    let _ = tx.send(SectionEvent::Progress { done, total });
                }
            };
            let fresh = api::sections(
                repo,
                hours,
                &token,
                cached_numbers.as_ref(),
                show,
                Some(&report as &(dyn Fn(usize, usize) + Send + Sync)),
            )
            .await?;
            let rows = api::merge_rows(fresh.rows.clone(), cached.unwrap_or_default(), chrono::Utc::now(), hours);
            walked = true;
            (rows, fresh.capped, fresh.rows)
        }
    };
    if capped || rows.len() >= api::MAX_SECTION_ROWS {
        if let Some(tx) = &progress {
            let _ = tx.send(SectionEvent::Capped);
        }
    }
    // A walk just read every status; otherwise refresh the open rows'.
    if refresh == Refresh::Status && !walked {
        let targets = api::status_targets(&rows);
        if !targets.is_empty() {
            if let Ok(updates) = api::fetch_statuses(repo, &targets, &token).await {
                api::apply_statuses(&mut rows, updates);
            }
        }
    }
    let (save_repo, saved) = (repo.to_string(), rows.clone());
    let _ = tokio::task::spawn_blocking(move || save_sections(&save_repo, &sync, &saved, fetched)).await;
    Ok(rows)
}

/// Stored rows for a repo, window and list mask (`None` when no usable
/// sync exists; see `load_sections`). Startup hydration reads through
/// here so the sidebar paints previous state before the background
/// refresh lands.
pub(crate) fn cached_sections(repo: &str, hours: u64, show: [bool; 4]) -> Option<Vec<api::Row>> {
    load_sections(repo, hours, show).map(|c| c.1)
}

/// Typed cache keys, re-exported so UI layers never touch `cache`.
pub use crate::cache::Key;

/// Format-dialog prefs, re-exported so UI layers never touch `cache`.
pub use crate::cache::{SavedBoardFilter, SavedQuery, SavedSidebarFilter, load_prefs, prefs_owner_key, prefs_repo_key, save_prefs};

/// Drop the named entries. `refresh_current` routes through here so UI
/// layers never touch `cache` directly.
pub fn invalidate(keys: &[Key]) {
    for key in keys {
        cache::cache_drop(&key.name());
    }
}

/// Pinned-issue passthroughs so UI layers never touch `cache` directly.
pub fn load_pins(repo: &str) -> Vec<u64> {
    cache::load_pins(repo)
}

/// Pinned-issue passthroughs so UI layers never touch `cache` directly.
pub fn save_pins(repo: &str, numbers: &[u64]) {
    cache::save_pins(repo, numbers);
}

/// Auto-refresh gate: false when the repo's issues and PRs are
/// unchanged since the last probe (a free 304).
pub async fn repo_changed(repo: &str) -> Result<bool, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::repo_changed(repo, &token).await
}

/// Server-side issue/PR search scoped to the repo.
pub async fn search_repo(
    repo: &str, query: &str,
    progress: Option<std::sync::mpsc::Sender<SectionEvent>>,
) -> Result<Vec<api::Row>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let report = |done, total| {
        if let Some(tx) = &progress { let _ = tx.send(SectionEvent::Progress { done, total }); }
    };
    let result = api::search(repo, query, &token, Some(&report)).await?;
    if result.capped {
        if let Some(tx) = &progress {
            let _ = tx.send(SectionEvent::Capped);
        }
    }
    Ok(result.rows)
}

/// Sidebar row cap, re-exported for the cap notice.
pub use crate::api::MAX_SECTION_ROWS;

/// Repository labels plus milestones for the Format dialog's pickers,
/// through the 30-minute cache. A partial cache fills only the missing
/// half from the network.
pub async fn fetch_repo_meta(repo: &str) -> Result<(Vec<String>, Vec<model::Milestone>), String> {
    require_repo(repo)?;
    let token = require_token()?;
    let labels_name = cache::Key::Labels { repo: repo.to_string() }.name();
    let milestones_name = cache::Key::Milestones { repo: repo.to_string() }.name();
    let (read_labels, read_milestones) = (labels_name.clone(), milestones_name.clone());
    let (labels, milestones) = tokio::task::spawn_blocking(move || {
        (
            cache::cache_get::<Vec<String>>(&read_labels),
            cache::cache_get::<Vec<model::Milestone>>(&read_milestones),
        )
    })
    .await
    .map_err(|e| format!("cache read failed: {e}"))?;
    let labels: Vec<String> = match labels {
        Some(labels) => labels,
        None => {
            let fresh = api::repo_labels(repo, &token).await?;
            let saved = fresh.clone();
            let name = labels_name.clone();
            let _ = tokio::task::spawn_blocking(move || cache::cache_put(&name, &saved)).await;
            fresh
        }
    };
    let milestones: Vec<model::Milestone> = match milestones {
        Some(milestones) => milestones,
        None => {
            let fresh = api::repo_milestones(repo, &token).await?;
            let saved = fresh.clone();
            let name = milestones_name.clone();
            let _ = tokio::task::spawn_blocking(move || cache::cache_put(&name, &saved)).await;
            fresh
        }
    };
    Ok((labels, milestones))
}

/// First Projects v2 board for an owner, through the read-through cache.
pub async fn fetch_board(owner: &str) -> Result<model::BoardData, String> {
    if owner.is_empty() {
        return Err("board needs an owner".to_string());
    }
    let token = require_token()?;
    let name = cache::Key::Board { owner: owner.to_string() }.name();
    let read_name = name.clone();
    let hit: Option<model::BoardData> =
        tokio::task::spawn_blocking(move || cache::cache_get(&read_name))
            .await
            .map_err(|e| format!("cache read failed: {e}"))?;
    if let Some(data) = hit {
        return Ok(data);
    }
    let data = api::fetch_board(owner, &token).await?;
    let data_c = data.clone();
    let _ = tokio::task::spawn_blocking(move || cache::cache_put(&name, &data_c)).await;
    Ok(data)
}

/// Batch board card details in one GraphQL request.
pub async fn fetch_board_details(cards: &[(String, u64)]) -> Result<Vec<Option<model::BoardDetail>>, String> {
    let token = require_token()?;
    api::board_details(cards, &token).await
}

/// Per-card detail: dates plus comment count.
pub async fn fetch_board_detail(repo: &str, number: u64) -> Result<model::BoardDetail, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let name = cache::Key::Comments { repo: repo.to_string(), number }.name();
    let read_name = name.clone();
    let comments = tokio::task::spawn_blocking(move || cache::cache_get(&read_name)).await.unwrap_or(None);
    let comments_fetched = comments.is_none();
    let (data, _) = api::issue(repo, number, &token, comments, None).await?;
    let saved_comments = data.comments.clone();
    if comments_fetched {
        let _ = tokio::task::spawn_blocking(move || cache::cache_put(&name, &saved_comments)).await;
    }
    Ok(model::BoardDetail::from_issue(&data))
}

/// Dependency graph for one issue, cached like single issues.
pub async fn fetch_graph(repo: &str, number: u64, force: bool) -> Result<model::GraphData, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let name = cache::Key::Graph { repo: repo.to_string(), number }.name();
    if !force {
        let name_c = name.clone();
        let hit: Option<model::GraphData> =
            tokio::task::spawn_blocking(move || cache::cache_get(&name_c))
                .await
                .map_err(|e| format!("cache read failed: {e}"))?;
        if let Some(data) = hit {
            return Ok(data);
        }
    }
    let data = api::graph(repo, number, &token).await?;
    let (name_c, data_c) = (name, data.clone());
    let _ = tokio::task::spawn_blocking(move || {
        cache::cache_put(&name_c, &data_c);
    })
    .await;
    Ok(data)
}

/// REST sub-issues for the opt-in sidebar tree, parsed like the graph
/// pane's `issues/{n}/sub_issues` call. The app caches the result per
/// parent for the session.
pub async fn fetch_sub_issues(repo: &str, number: u64) -> Result<Vec<model::IssueData>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::sub_issues(repo, number, &token).await
}

/// Changed files for one PR, cached like single issues.
pub async fn fetch_diff(repo: &str, number: u64, force: bool) -> Result<Vec<model::DiffFile>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let name = cache::Key::Diff { repo: repo.to_string(), number }.name();
    if !force {
        let name_c = name.clone();
        let hit: Option<Vec<model::DiffFile>> =
            tokio::task::spawn_blocking(move || cache::cache_get(&name_c))
                .await
                .map_err(|e| format!("cache read failed: {e}"))?;
        if let Some(files) = hit {
            return Ok(files);
        }
    }
    let files = api::pull_files(repo, number, &token).await?;
    let (name_c, files_c) = (name, files.clone());
    let _ = tokio::task::spawn_blocking(move || {
        cache::cache_put(&name_c, &files_c);
    })
    .await;
    Ok(files)
}

/// Post a thread comment on an issue/PR. Failures are strings for the
/// status line, never panics; auth and repo resolve first so misuse
/// never reaches the network.
pub async fn post_thread_comment(repo: &str, number: u64, body: &str) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_comment(repo, number, body, &token).await
}

/// Close (`open` false) or reopen an issue or PR. Same error contract
/// as `post_thread_comment`.
pub async fn set_state(repo: &str, number: u64, open: bool) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::set_state(repo, number, open, &token).await
}

/// Post a single-shot line comment on a PR file. Same error contract
/// as `post_thread_comment`.
pub async fn post_line_comment(repo: &str, number: u64, body: &str, path: &str, line: u64) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_review_thread(repo, number, body, path, line, &token).await
}

/// Submit the viewer's pending review on a PR so pending line comments
/// go live. Same error contract as `post_thread_comment`.
pub async fn submit_pending_review(repo: &str, number: u64) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::submit_pending_review(repo, number, &token).await
}

/// Reply inside an existing review thread: `comment_id` is the
/// numeric REST id of a comment in the thread (`in_reply_to`).
pub async fn post_thread_reply(repo: &str, number: u64, comment_id: u64, body: &str) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_thread_reply(repo, number, comment_id, body, &token).await
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

    fn row(number: u64, is_pr: bool, state: &str, window_at: Option<String>) -> Row {
        Row {
            issue: IssueData { number, is_pr, state: state.to_string(), title: format!("t{number}"), ..Default::default() },
            statuses: vec![("Board".to_string(), Some("Todo".to_string()))],
            detail: model::BoardDetail::default(),
            complete: false,
            window_at,
        }
    }

    fn numbers(rows: &[Row]) -> Vec<u64> {
        rows.iter().map(|r| r.issue.number).collect()
    }

    fn with_temp_data<T>(body: impl FnOnce() -> T) -> T {
        let _guard = cache::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        let out = body();
        match saved {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        out
    }

    /// Parity pin for the sidebar rows: a cold walk, a persisted round
    /// trip, a change-only merge over the persisted rows, and startup
    /// hydration, the way `fetch_recent_sections` chains them.
    #[test]
    fn sidebar_rows_survive_walk_delta_and_hydration() {
        with_temp_data(|| {
            let (repo, hours, show) = ("o/r", 24, [true; 4]);
            let now = chrono::Utc::now();
            let recent = Some((now - chrono::TimeDelta::hours(1)).to_rfc3339());
            let stale = Some((now - chrono::TimeDelta::hours(30)).to_rfc3339());
            assert!(cached_sections(repo, hours, show).is_none(), "no sync yet hydrates nothing");

            // Cold walk: rows regroup by list; a cached row out of the window never joins.
            let fresh = vec![
                row(3, false, "OPEN", None),
                row(1, true, "OPEN", None),
                row(4, false, "CLOSED", recent.clone()),
                row(2, true, "MERGED", recent.clone()),
            ];
            let walked = api::merge_rows(fresh.clone(), vec![row(9, true, "MERGED", stale)], now, hours);
            assert_eq!(numbers(&walked), vec![1, 2, 3, 4]);
            let sync = store::Sync { started_at: now.to_rfc3339(), hours, show };
            save_sections(repo, &sync, &walked, fresh);
            let hydrated = cached_sections(repo, hours, show).expect("fresh sync hydrates");
            assert_eq!(numbers(&hydrated), vec![1, 2, 3, 4], "persisted order is sidebar order");
            assert_eq!(hydrated[0].statuses, walked[0].statuses, "row fields round trip");

            // Change-only merge: #1 retitled stays in place, #3 moves list,
            // #6 is new, #7 is a closed (unmerged) PR no list shows.
            let (last, cached) = load_sections(repo, hours, show).expect("cached rows");
            assert!(api::delta_since(Some(&last.started_at), now, false).is_some(), "a fresh sync takes the delta path");
            let mut retitled = row(1, true, "OPEN", None);
            retitled.issue.title = "renamed".to_string();
            let delta = vec![row(3, false, "CLOSED", recent.clone()), retitled, row(6, true, "OPEN", None), row(7, true, "CLOSED", None)];
            let merged = api::merge_delta(delta.clone(), cached, now, hours, show);
            assert_eq!(numbers(&merged), vec![6, 1, 2, 3, 4]);
            save_sections(repo, &last, &merged, delta);
            let hydrated = cached_sections(repo, hours, show).expect("hydrates after delta");
            assert_eq!(numbers(&hydrated), vec![6, 1, 2, 3, 4]);
            assert_eq!(hydrated[1].issue.title, "renamed");
            assert_eq!(hydrated[3].issue.state, "CLOSED");
        });
    }

    /// The store holds the repo's rows, not one window's: reads apply
    /// the list mask and window, and a sync only stands in for windows
    /// and masks its walk covered.
    #[test]
    fn stored_rows_filter_by_mask_and_window_on_read() {
        with_temp_data(|| {
            let repo = "o/r";
            let now = chrono::Utc::now();
            let at = |h: i64| Some((now - chrono::TimeDelta::hours(h)).to_rfc3339());
            let rows = vec![row(1, true, "OPEN", None), row(2, true, "MERGED", at(30)), row(3, true, "MERGED", at(2)), row(4, false, "CLOSED", at(2))];
            let sync = store::Sync { started_at: now.to_rfc3339(), hours: 168, show: [true; 4] };
            save_sections(repo, &sync, &rows, vec![row(5, true, "CLOSED", None)]);
            assert_eq!(numbers(&cached_sections(repo, 168, [true; 4]).unwrap()), vec![1, 2, 3, 4], "stored order; the closed PR is in no list");
            assert_eq!(numbers(&cached_sections(repo, 24, [true; 4]).unwrap()), vec![1, 3, 4], "aged-out merged PR drops");
            assert_eq!(numbers(&cached_sections(repo, 24, [true, false, true, false]).unwrap()), vec![1], "hidden lists drop");
            assert!(cached_sections(repo, 720, [true; 4]).is_none(), "a wider window than the walk is a miss");
            let narrow = store::Sync { started_at: now.to_rfc3339(), hours: 24, show: [true, false, true, true] };
            save_sections(repo, &narrow, &rows[..1], Vec::new());
            assert!(cached_sections(repo, 24, [true; 4]).is_none(), "a list the walk hid is a miss");
            assert_eq!(numbers(&cached_sections(repo, 24, [true, false, false, true]).unwrap()), vec![1, 4]);
        });
    }

    #[tokio::test]
    async fn fetches_fail_before_the_network_without_a_repo() {
        // Empty repo errors before any token lookup or request, so these
        // hold with or without auth configured.
        assert!(fetch_number("", 1, false, None).await.is_err());
        assert!(fetch_recent_sections("", 24, [true; 4], Refresh::Delta, None).await.is_err());
        assert!(search_repo("", "bug", None).await.is_err());
        assert!(fetch_repo_meta("").await.is_err());
        assert!(fetch_board("").await.is_err());
        assert!(fetch_board_detail("", 1).await.is_err());
        assert!(fetch_diff("", 1, false).await.is_err());
        assert!(fetch_graph("", 1, false).await.is_err());
        assert!(fetch_sub_issues("", 1).await.is_err());
    }
}
