//! Data access over HTTPS (`api` + `cache` + `store`).
//!
//! Missing auth or repo is a status-line error, never silent behavior.

mod backfill;

use crate::{api, cache, model, store};
use crate::model::IssueData;
use crate::model::ItemRef;

pub use backfill::{backfill_step, BackfillStep};
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
    number: ItemRef,
    force: bool,
    progress: Option<std::sync::mpsc::Sender<(ItemRef, IssueEvent)>>,
) -> Result<IssueBundle, String> {
    require_repo(repo)?;
    let token = require_token()?;
    if !force {
        let repo_c = repo.to_string();
        let hit = tokio::task::spawn_blocking(move || stored_bundle(&repo_c, number))
            .await
            .map_err(|e| format!("store read failed: {e}"))?;
        if let Some(bundle) = hit {
            return Ok(bundle);
        }
    }
    let report = |done: usize, total: usize| {
        if let Some(tx) = &progress {
            let _ = tx.send((number, IssueEvent::Progress { done, total }));
        }
    };
    let fetched = api::issue(
        repo,
        number.number,
        &token,
        Some(&report as &(dyn Fn(usize, usize) + Send + Sync)),
    )
    .await;
    let repo_c = repo.to_string();
    match &fetched {
        Ok(bundle) => {
            let bundle_c = bundle.clone();
            let _ = tokio::task::spawn_blocking(move || save_bundle(&repo_c, number, &bundle_c)).await;
        }
        Err(e) if is_not_found(e) => {
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut store) = store::open(&repo_c) {
                    let _ = store.drop_item(number);
                }
            })
            .await;
        }
        Err(_) => {}
    }
    fetched
}

/// A 404 from the item fetch: deleted or transferred.
fn is_not_found(error: &str) -> bool {
    error.contains("404 Not Found")
}

/// The stored issue bundle when it is fresh against the latest sync.
/// Project statuses come from the synced row, not the stored copy.
/// Blocking: call from the blocking pool.
pub(crate) fn stored_bundle(repo: &str, number: ItemRef) -> Option<IssueBundle> {
    let store = store::open(repo).ok()?;
    let (data, statuses): IssueBundle = store.fresh_detail(store::Detail::Issue, number, false)?;
    Some((data, store.item_statuses(number).unwrap_or(statuses)))
}

/// The sidebar row of a fetched bundle. Fields the bundle cannot carry
/// (CI rollup, sub-issue parent and summary, merge time) and an empty
/// status lookup come from `stored`, so a partial fetch never blanks a
/// good synced row. The merge time falls back to `updated_at`.
fn row_from_bundle(bundle: &IssueBundle, stored: Option<&api::Row>) -> api::Row {
    let (data, statuses) = bundle;
    let mut issue = data.clone();
    issue.body.clear();
    issue.comments.clear();
    issue.review_threads.clear();
    issue.paired.clear();
    let detail = model::BoardDetail {
        created: data.created_at.get(..10).unwrap_or("").to_owned(),
        updated: data.updated_at.get(..10).unwrap_or("").to_owned(),
        comments: data.comments.len(),
    };
    let window_at = match (data.is_pr, data.state.as_str()) {
        (true, "MERGED") => Some(stored.and_then(|r| r.window_at.clone()).unwrap_or_else(|| data.updated_at.clone())),
        (false, "CLOSED") => Some(data.updated_at.clone()),
        _ => None,
    };
    let mut statuses = statuses.clone();
    if let Some(old) = stored {
        issue.checks = issue.checks.or(old.issue.checks);
        if issue.number.source == model::Source::GitHub {
            issue.parent = old.issue.parent;
        }
        issue.sub_summary = old.issue.sub_summary;
        if issue.number.source == model::Source::GitHub {
            issue.links = old.issue.links.clone();
        }
        if issue.head_ref_name.is_empty() {
            issue.head_ref_name = old.issue.head_ref_name.clone();
        }
        if issue.jira_refs.is_empty() {
            issue.jira_refs = old.issue.jira_refs.clone();
        }
        if statuses.is_empty() {
            statuses = old.statuses.clone();
        }
    }
    api::Row { issue, statuses, detail, complete: false, window_at }
}

/// Store a fetched issue bundle at its own `updated_at`, and its row
/// into `items` newer-only so the item joins the local copy. Best
/// effort. Blocking.
pub(crate) fn save_bundle(repo: &str, number: ItemRef, bundle: &IssueBundle) {
    if let Ok(mut store) = store::open(repo) {
        let row = row_from_bundle(bundle, store.row(number).as_ref());
        let _ = store.upsert_rows_newer(&[row]);
        let _ = store.put_detail(store::Detail::Issue, number, &bundle.0.updated_at, bundle);
    }
}

/// Stored diff payload when fresh, else `None`. Blocking.
fn stored_detail<T: serde::de::DeserializeOwned>(
    repo: &str,
    kind: store::Detail,
    number: ItemRef,
) -> Option<T> {
    store::open(repo).ok()?.fresh_detail(kind, number, false)
}

/// Store a diff payload at the item's synced `updated_at`; an
/// item no sync listed stores nothing, since an open could not serve it.
/// Best effort. Blocking.
fn save_detail<T: serde::Serialize>(repo: &str, kind: store::Detail, number: ItemRef, value: &T) {
    if let Ok(store) = store::open(repo) {
        if let Some(updated_at) = store.item_updated_at(number) {
            let _ = store.put_detail(kind, number, &updated_at, value);
        }
    }
}

/// Narrowest merged/closed window a sync walk covers. The walk serves
/// every instance of the repo whatever its own window, so it reaches back
/// as far as the widest bounded window (7 days). Merged and closed lists
/// still retire at that cutoff, so open lists keep the row budget.
const SYNC_MIN_HOURS: u64 = 168;

/// How long a sync lease lasts without renewal: a crashed holder's lease
/// frees after this.
const LEASE_TTL_SECS: i64 = 60;

/// How often every process renews its lease, or tries to take a free one.
const LEASE_RETRY: std::time::Duration = std::time::Duration::from_secs(15);

/// How long the UI thread waits on another process's store lock before
/// it skips this lease try or read.
const LINK_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// Window a sync walk covers: the given instance window, widened to
/// `SYNC_MIN_HOURS`. 0 (all time) stays unbounded.
pub(crate) fn sync_hours(window: u64) -> u64 {
    if window == 0 { 0 } else { window.max(SYNC_MIN_HOURS) }
}

/// Every stored item of `repo` for the finder, newest update first. Empty
/// when the repo has no store file yet (never creates one). Local read only.
pub fn finder_candidates(repo: &str) -> Vec<store::Candidate> {
    if !store::path(repo).is_some_and(|p| p.exists()) {
        return vec![];
    }
    store::open(repo).and_then(|s| s.finder_candidates()).unwrap_or_default()
}

/// The last sync start and every stored row, unordered by list. Empty when
/// the store is new or unreadable. Blocking: call from the blocking pool.
fn load_rows(repo: &str) -> (Option<String>, Vec<api::Row>) {
    let Ok(store) = store::open(repo) else { return (None, Vec::new()) };
    (store.sync_started_at(), store.rows().unwrap_or_default())
}

/// Upsert `rows` and record `started_at` as the last sync start. Best
/// effort: the next sync rewrites. Blocking.
fn save_rows(repo: &str, started_at: &str, rows: &[api::Row]) {
    if let Ok(mut store) = store::open(repo) {
        let _ = store.write(started_at, rows);
    }
}

/// `stored` with each `fetched` row replacing the stored row of its number.
fn overlay(stored: Vec<api::Row>, fetched: &[api::Row]) -> Vec<api::Row> {
    let fresh: std::collections::HashSet<ItemRef> =
        fetched.iter().map(|r| r.issue.number).collect();
    stored
        .into_iter()
        .filter(|r| !fresh.contains(&r.issue.number))
        .chain(fetched.iter().cloned())
        .collect()
}

/// REST rows lack the CI rollup, project status and sub-issue parent:
/// each fetched row keeps its stored row's `checks`, `statuses`, `parent`,
/// `links` and, when REST sends none, `sub_summary`. The status pass refreshes them.
fn keep_stored_fields(stored: &[api::Row], fetched: &mut [api::Row]) {
    let old: std::collections::HashMap<ItemRef, &api::Row> =
        stored.iter().map(|r| (r.issue.number, r)).collect();
    for row in fetched {
        if let Some(prev) = old.get(&row.issue.number) {
            row.issue.checks = prev.issue.checks;
            row.statuses = prev.statuses.clone();
            row.issue.parent = prev.issue.parent;
            row.issue.links = prev.issue.links.clone();
            if row.issue.head_ref_name.is_empty() {
                row.issue.head_ref_name = prev.issue.head_ref_name.clone();
            }
            if row.issue.jira_refs.is_empty() {
                row.issue.jira_refs = prev.issue.jira_refs.clone();
            }
            row.issue.sub_summary = row.issue.sub_summary.or(prev.issue.sub_summary);
        }
    }
}

/// How a sections refresh runs: a change-only REST read (`Delta`), the same
/// plus a CI/project status refresh of the open rows (`Status`), or a
/// full walk of every list (`Full`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Refresh {
    #[default]
    Delta,
    Status,
    Full,
}

/// Sync the repo's recent rows into its store, then return the rows this
/// instance shows: open + windowed merged PRs, open + windowed closed
/// issues, filtered by `hours` and the list mask `show` (open PRs, merged
/// PRs, open issues, closed issues).
///
/// The sync itself is per repo: it covers all four lists and
/// `sync_hours(hours)`, so every instance's window and mask filter the
/// same stored rows. Only the store's sync lease holder calls this. A
/// sync of any age refreshes through one change-only REST read
/// (`api::changed_rows`); `Refresh::Full` or no sync walks the lists
/// head-first and stops on overlap with the stored rows.
/// `progress` re-tags the page counts onto the caller's existing section
/// channel. `Refresh::Status` also refetches CI and project status of the
/// open rows (best effort) after its change-only search.
pub async fn fetch_recent_sections(
    repo: &str,
    hours: u64,
    show: [bool; 4],
    refresh: Refresh,
    progress: Option<std::sync::mpsc::Sender<SectionEvent>>,
) -> Result<Vec<api::Row>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let started = chrono::Utc::now();
    let read_repo = repo.to_string();
    let (last, stored) = tokio::task::spawn_blocking(move || load_rows(&read_repo)).await.unwrap_or_default();
    let since = api::delta_since(last.as_deref(), refresh == Refresh::Full);
    let mut walked = false;
    let mut capped = false;
    let fetched = match since {
        Some(since) => {
            let mut rows = api::changed_rows(repo, since, &token).await?;
            keep_stored_fields(&stored, &mut rows);
            rows
        }
        None => {
            let stored_numbers: Option<std::collections::HashSet<ItemRef>> =
                (!stored.is_empty()).then(|| stored.iter().map(|r| r.issue.number).collect());
            let report = |done: usize, total: usize| {
                if let Some(tx) = &progress {
                    let _ = tx.send(SectionEvent::Progress { done, total });
                }
            };
            let fresh = api::sections(
                repo,
                sync_hours(hours),
                &token,
                stored_numbers.as_ref(),
                [true; 4],
                Some(&report as &(dyn Fn(usize, usize) + Send + Sync)),
            )
            .await?;
            walked = true;
            capped = fresh.capped;
            fresh.rows
        }
    };
    let mut changed: std::collections::HashSet<ItemRef> =
        fetched.iter().map(|r| r.issue.number).collect();
    let mut rows = overlay(stored, &fetched);
    // A walk just read every status; otherwise refresh the open rows'.
    if refresh == Refresh::Status && !walked {
        let targets = api::status_targets(&rows);
        if !targets.is_empty() {
            if let Ok(updates) = api::fetch_statuses(repo, &targets, &token).await {
                changed.extend(updates.iter().map(|u| u.number));
                api::apply_statuses(&mut rows, updates);
            }
        }
    }
    let saved: Vec<api::Row> = rows.iter().filter(|r| changed.contains(&r.issue.number)).cloned().collect();
    let save_repo = repo.to_string();
    let started = started.to_rfc3339();
    let _ = tokio::task::spawn_blocking(move || save_rows(&save_repo, &started, &saved)).await;
    let shown = api::visible_rows(rows, chrono::Utc::now(), hours, show);
    if capped || shown.len() >= api::MAX_SECTION_ROWS {
        if let Some(tx) = &progress {
            let _ = tx.send(SectionEvent::Capped);
        }
    }
    Ok(shown)
}

/// Stored rows this window and list mask show, in sidebar order; `None`
/// before the first sync or when the store is unreadable. Startup
/// hydration reads through here so the sidebar paints previous state
/// before the background refresh lands. Blocking.
pub fn cached_sections(repo: &str, hours: u64, show: [bool; 4]) -> Option<Vec<api::Row>> {
    let store = store::open(repo).ok()?;
    stored_view(&store, hours, show)
}

fn stored_view(store: &store::Store, hours: u64, show: [bool; 4]) -> Option<Vec<api::Row>> {
    store.sync_stamp()?;
    Some(api::visible_rows(store.rows().ok()?, chrono::Utc::now(), hours, show))
}

/// One process's tie to its repo store: the sync lease and the change
/// watch. Every ghit process of a repo shares one sync stream. The lease
/// holder alone fetches listings; every process notices the holder's
/// writes through `changed` and re-reads its own view. Dropping the link
/// releases a held lease, so another process takes over on its next try.
pub struct StoreLink {
    store: store::Store,
    holder: String,
    holding: bool,
    tried: std::time::Instant,
    version: i64,
    seen: Option<String>,
}

impl StoreLink {
    /// Open the link for `repo` and try to take the lease. `None` when the
    /// store cannot open; the caller then syncs on its own.
    pub fn open(repo: &str) -> Option<Self> {
        let store = store::open(repo).ok()?;
        store.set_busy_timeout(LINK_BUSY_TIMEOUT).ok()?;
        Some(Self::new(store, holder_id()))
    }

    fn new(store: store::Store, holder: String) -> Self {
        let version = store.data_version().unwrap_or_default();
        let seen = store.sync_stamp();
        let mut link = Self { store, holder, holding: false, tried: std::time::Instant::now(), version, seen };
        link.claim(false, chrono::Utc::now().timestamp());
        link
    }

    /// True while this process holds the sync lease.
    pub fn holding(&self) -> bool {
        self.holding
    }

    /// Renew the lease, or try to take a free one, every `LEASE_RETRY`.
    pub fn tick(&mut self, now: std::time::Instant) {
        if now.duration_since(self.tried) >= LEASE_RETRY {
            self.tried = now;
            self.claim(false, chrono::Utc::now().timestamp());
        }
    }

    /// Take the lease even from a live holder: a manual refresh syncs
    /// from here, and the old holder stops on its next renewal.
    pub fn take_over(&mut self) {
        self.claim(true, chrono::Utc::now().timestamp());
    }

    /// A failed try (a lock held past the busy timeout) keeps the last
    /// answer; the next try settles it.
    fn claim(&mut self, force: bool, now: i64) {
        if let Ok(holding) = self.store.claim_lease(&self.holder, now, LEASE_TTL_SECS, force) {
            self.holding = holding;
        }
    }

    /// True once per new sync written by any process since the last call.
    /// `PRAGMA data_version` gates it, so an idle store costs one pragma;
    /// lease renewals move that counter but not the sync start.
    pub fn changed(&mut self) -> bool {
        let Ok(version) = self.store.data_version() else { return false };
        if version == self.version {
            return false;
        }
        self.version = version;
        let stamp = self.store.sync_stamp();
        if stamp == self.seen {
            return false;
        }
        self.seen = stamp;
        true
    }

    /// Stored rows this window and list mask show (see `cached_sections`).
    pub fn rows(&self, hours: u64, show: [bool; 4]) -> Option<Vec<api::Row>> {
        stored_view(&self.store, hours, show)
    }

    /// Every stored row, whatever the window and mask; `None` before the
    /// first sync. Local `/` queries filter these.
    pub fn all_rows(&self) -> Option<Vec<api::Row>> {
        self.store.sync_stamp()?;
        self.store.rows().ok()
    }

    /// Every stored item and link, for the dependency map, with the item
    /// generation they were read at. `None` when `seen` is that generation
    /// (one cheap query) or the store fails to read; a failure leaves the
    /// caller's `seen` alone, so the next call retries.
    pub fn map_data(&self, seen: Option<i64>) -> Option<(i64, Vec<model::MapItem>, Vec<model::MapLink>)> {
        // Read the generation first: a write during the read only costs one more reread.
        let generation = self.store.items_generation().ok()?;
        if seen == Some(generation) {
            return None;
        }
        let items = self.store.map_items().ok()?;
        let links = self.store.edges().ok()?.into_iter().map(|edge| model::MapLink {
            src: edge.src,
            dst: edge.dst,
            kind: match edge.kind {
                store::EdgeKind::Blocks => model::LinkKind::Blocks,
                store::EdgeKind::Parent => model::LinkKind::Parent,
                store::EdgeKind::Closes => model::LinkKind::Closes,
                store::EdgeKind::Relates => model::LinkKind::Relates,
            },
            label: edge.label,
        }).collect();
        Some((generation, items, links))
    }

    /// The stored row of one item and the body of its stored detail, of
    /// any age, for a map card. Reads only the local copy.
    pub fn item_card(&self, number: ItemRef) -> Option<(api::Row, Option<String>)> {
        let row = self.store.row(number)?;
        let body = self.store.stored_detail::<IssueBundle>(store::Detail::Issue, number);
        Some((row, body.map(|(issue, _)| issue.body)))
    }

    /// Stamp of the row and stored detail a map card shows for `number`.
    pub fn card_stamp(&self, number: ItemRef) -> Option<String> {
        self.store.card_stamp(number)
    }

    /// Store GitHub search hits (newer-only, statuses kept) and return the
    /// stored version of every hit in GitHub's order, so the view shows what
    /// a later open reads. Falls back to the hits themselves when the store fails.
    pub fn save_search(&mut self, hits: Vec<api::Row>) -> Vec<api::Row> {
        if self.store.upsert_search(&hits).is_err() {
            return hits;
        }
        let numbers: Vec<ItemRef> = hits.iter().map(|row| row.issue.number).collect();
        let mut rows = self.store.rows_by_number(&numbers);
        rows.sort_by_key(|row| numbers.iter().position(|n| *n == row.issue.number));
        rows
    }

    /// The stored rows with these numbers, most recently updated first.
    pub fn rows_by_number(&self, numbers: &[ItemRef]) -> Vec<api::Row> {
        self.store.rows_by_number(numbers)
    }

    /// Pull requests linked to `number` by a stored closes edge.
    pub fn linked_prs(&self, number: ItemRef) -> Vec<model::PairedRef> {
        self.store.linked_prs(number)
    }

    /// Which backfill walks finished: the open ones, then the closed ones.
    pub fn backfill_done(&self) -> (bool, bool) {
        let done = |conns: &[api::Conn]| conns.iter().all(|c| self.store.backfill_conn(c.name()).done);
        (done(&[api::Conn::PrsOpen, api::Conn::IssuesOpen]), done(&[api::Conn::PrsClosed, api::Conn::IssuesClosed]))
    }
}

impl Drop for StoreLink {
    fn drop(&mut self) {
        if self.holding {
            let _ = self.store.release_lease(&self.holder);
        }
    }
}

/// Lease holder name: host and pid, plus a per-process sequence so two
/// links in one process (tests) stay distinct.
fn holder_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let host = std::env::var("HOSTNAME").or_else(|_| std::env::var("HOST")).unwrap_or_else(|_| "localhost".to_string());
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{host}:{}:{seq}", std::process::id())
}

/// Typed cache keys, re-exported so UI layers never touch `cache`.
pub use crate::cache::Key;

/// Format-dialog prefs, re-exported so UI layers never touch `cache`.
pub use crate::cache::{SavedBoardFilter, SavedQuery, SavedSidebarFilter, load_prefs, prefs_owner_key, prefs_repo_key, save_prefs};

/// Map physics prefs, re-exported so UI layers never touch `cache`.
pub use crate::cache::{load_physics, save_physics};

/// Drop the named entries. `refresh_current` routes through here so UI
/// layers never touch `cache` directly.
pub fn invalidate(keys: &[Key]) {
    for key in keys {
        cache::cache_drop(&key.name());
    }
}

/// Pinned-issue passthroughs so UI layers never touch `cache` directly.
pub fn load_pins(repo: &str) -> Vec<ItemRef> {
    cache::load_pins(repo)
}

/// Pinned-issue passthroughs so UI layers never touch `cache` directly.
pub fn save_pins(repo: &str, numbers: &[ItemRef]) {
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
pub async fn fetch_board_details(
    cards: &[(String, ItemRef)],
) -> Result<Vec<Option<model::BoardDetail>>, String> {
    let token = require_token()?;
    api::board_details(
        &cards
            .iter()
            .map(|(repo, item)| (repo.clone(), item.number))
            .collect::<Vec<_>>(),
        &token,
    )
    .await
}

/// Per-card detail: dates plus comment count.
pub async fn fetch_board_detail(repo: &str, number: ItemRef) -> Result<model::BoardDetail, String> {
    require_repo(repo)?;
    let token = require_token()?;
    let repo_c = repo.to_string();
    if let Ok(Some((data, _))) = tokio::task::spawn_blocking(move || stored_bundle(&repo_c, number)).await {
        return Ok(model::BoardDetail::from_issue(&data));
    }
    let bundle = api::issue(repo, number.number, &token, None).await?;
    let (repo_c, bundle_c) = (repo.to_string(), bundle.clone());
    let _ = tokio::task::spawn_blocking(move || save_bundle(&repo_c, number, &bundle_c)).await;
    let data = bundle.0;
    Ok(model::BoardDetail::from_issue(&data))
}

/// REST sub-issues for the opt-in sidebar tree. The app caches the
/// result per parent for the session.
pub async fn fetch_sub_issues(
    repo: &str,
    number: ItemRef,
) -> Result<Vec<model::IssueData>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::sub_issues(repo, number.number, &token).await
}

/// Changed files for one PR, stored like single issues.
pub async fn fetch_diff(
    repo: &str,
    number: ItemRef,
    force: bool,
) -> Result<Vec<model::DiffFile>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    if !force {
        let repo_c = repo.to_string();
        let hit: Option<Vec<model::DiffFile>> =
            tokio::task::spawn_blocking(move || stored_detail(&repo_c, store::Detail::Diff, number))
                .await
                .map_err(|e| format!("store read failed: {e}"))?;
        if let Some(files) = hit {
            return Ok(files);
        }
    }
    let files = api::pull_files(repo, number.number, &token).await?;
    let (repo_c, files_c) = (repo.to_string(), files.clone());
    let _ = tokio::task::spawn_blocking(move || save_detail(&repo_c, store::Detail::Diff, number, &files_c)).await;
    Ok(files)
}

/// Head-side bodies of `names` in PR `number`, for diff context. Not
/// stored: the app keeps them for the session only.
pub async fn fetch_head_files(
    repo: &str,
    number: ItemRef,
    names: Vec<String>,
) -> Result<Vec<(String, Result<String, String>)>, String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::head_files(repo, number.number, names, &token).await
}

/// Post a thread comment on an issue/PR. Failures are strings for the
/// status line, never panics; auth and repo resolve first so misuse
/// never reaches the network.
pub async fn post_thread_comment(repo: &str, number: ItemRef, body: &str) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_comment(repo, number.number, body, &token).await
}

/// Close (`open` false) or reopen an issue or PR. Same error contract
/// as `post_thread_comment`.
pub async fn set_state(repo: &str, number: ItemRef, open: bool) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::set_state(repo, number.number, open, &token).await
}

/// Post a single-shot line comment on a PR file. Same error contract
/// as `post_thread_comment`.
pub async fn post_line_comment(
    repo: &str,
    number: ItemRef,
    body: &str,
    path: &str,
    line: u64,
) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_review_thread(repo, number.number, body, path, line, &token).await
}

/// Submit the viewer's pending review on a PR so pending line comments
/// go live. Same error contract as `post_thread_comment`.
pub async fn submit_pending_review(repo: &str, number: ItemRef) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::submit_pending_review(repo, number.number, &token).await
}

/// Reply inside an existing review thread: `comment_id` is the
/// numeric REST id of a comment in the thread (`in_reply_to`).
pub async fn post_thread_reply(
    repo: &str,
    number: ItemRef,
    comment_id: u64,
    body: &str,
) -> Result<(), String> {
    require_repo(repo)?;
    let token = require_token()?;
    api::add_thread_reply(repo, number.number, comment_id, body, &token).await
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

    fn row(number: ItemRef, is_pr: bool, state: &str, window_at: Option<String>) -> Row {
        Row {
            issue: IssueData {
                number,
                is_pr,
                state: state.to_string(),
                title: format!("t{number}", number = number.number),
                ..Default::default()
            },
            statuses: vec![("Board".to_string(), Some("Todo".to_string()))],
            detail: model::BoardDetail::default(),
            complete: false,
            window_at,
        }
    }

    fn numbers(rows: &[Row]) -> Vec<ItemRef> {
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

    /// A row created and last updated `hours_ago`; merged PRs and closed
    /// issues anchor their window there too, as the listing parse does.
    fn dated(
        number: ItemRef,
        is_pr: bool,
        state: &str,
        now: chrono::DateTime<chrono::Utc>,
        hours_ago: i64,
    ) -> Row {
        let at = (now - chrono::TimeDelta::hours(hours_ago))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let windowed = matches!((is_pr, state), (true, "MERGED") | (false, "CLOSED"));
        let mut row = row(number, is_pr, state, windowed.then(|| at.clone()));
        row.issue.created_at = at.clone();
        row.issue.updated_at = at;
        row
    }

    /// The map rereads on a row or search write, not on a lease renewal,
    /// and a failed read leaves its generation unseen so it retries.
    #[test]
    fn map_data_rereads_only_after_item_writes_and_retries_failures() {
        with_temp_data(|| {
            let repo = "o/r";
            let mut map = StoreLink::open(repo).expect("store opens");
            let mut other = StoreLink::open(repo).expect("store opens");
            let first = map.map_data(None).expect("the first read loads");
            let seen = Some(first.0);
            assert!(map.map_data(seen).is_none(), "an unchanged store is not reread");

            other.claim(false, chrono::Utc::now().timestamp() + 1000);
            assert!(map.map_data(seen).is_none(), "a lease renewal is not a data change");

            save_rows(repo, "t1", &[row(ItemRef::github(1), true, "OPEN", None)]);
            let (generation, items, _) = map.map_data(seen).expect("a row write rereads");
            assert_eq!(items.len(), 1);
            let seen = Some(generation);
            assert!(map.map_data(seen).is_none());

            map.save_search(vec![row(ItemRef::github(2), false, "OPEN", None)]);
            let (generation, items, _) = map.map_data(seen).expect("a search write on the map's own connection rereads");
            assert_eq!(items.len(), 2);
            let seen = Some(generation);

            let file = store::path(repo).unwrap();
            let raw = rusqlite::Connection::open(&file).unwrap();
            raw.execute_batch("ALTER TABLE edges RENAME TO edges_off").unwrap();
            save_rows(repo, "t2", &[row(ItemRef::github(3), false, "OPEN", None)]);
            raw.execute_batch("UPDATE meta SET value = value + 1 WHERE key = 'items_gen'").unwrap();
            assert!(map.map_data(seen).is_none(), "a failed read returns nothing");
            raw.execute_batch("ALTER TABLE edges_off RENAME TO edges").unwrap();
            assert!(map.map_data(seen).is_some(), "the next call retries the same generation");
        })
    }

    #[test]
    fn rest_rows_keep_stored_checks_statuses_and_parent() {
        let mut prev = row(ItemRef::github(1), true, "OPEN", None);
        prev.issue.checks = Some(model::CheckState::Pass);
        prev.issue.parent = Some(ItemRef::github(9));
        prev.issue.sub_summary = Some(model::SubSummary { total: 2, completed: 1 });
        let mut rest = row(ItemRef::github(1), true, "MERGED", None);
        rest.statuses = vec![];
        let mut new = row(ItemRef::github(2), false, "OPEN", None);
        new.statuses = vec![];
        let mut fetched = vec![rest, new];
        keep_stored_fields(&[prev.clone()], &mut fetched);
        assert_eq!(fetched[0].issue.state, "MERGED", "REST owns the state");
        assert_eq!(fetched[0].issue.checks, Some(model::CheckState::Pass));
        assert_eq!(fetched[0].issue.parent, Some(ItemRef::github(9)));
        assert_eq!(fetched[0].issue.sub_summary, prev.issue.sub_summary);
        assert_eq!(fetched[0].statuses, prev.statuses);
        assert!(fetched[1].statuses.is_empty() && fetched[1].issue.checks.is_none(), "unstored rows stay bare");
    }

    /// Parity pin for the sidebar rows: a cold walk, a persisted round
    /// trip, a change-only sync over the persisted rows, and startup
    /// hydration, the way `fetch_recent_sections` chains them. Since #244
    /// the store keeps no list positions: order derives from row dates, so
    /// the rows carry dates, and a sync upserts only the rows it fetched.
    #[test]
    fn sidebar_rows_survive_walk_delta_and_hydration() {
        with_temp_data(|| {
            let (repo, hours, show) = ("o/r", 24, [true; 4]);
            let now = chrono::Utc::now();
            assert!(cached_sections(repo, hours, show).is_none(), "no sync yet hydrates nothing");

            // Cold walk: rows regroup by list; a stored row out of the window never shows.
            let walked = vec![
                dated(ItemRef::github(3), false, "OPEN", now, 5),
                dated(ItemRef::github(1), true, "OPEN", now, 4),
                dated(ItemRef::github(4), false, "CLOSED", now, 2),
                dated(ItemRef::github(2), true, "MERGED", now, 1),
                dated(ItemRef::github(9), true, "MERGED", now, 30),
            ];
            save_rows(repo, &now.to_rfc3339(), &walked);
            let hydrated = cached_sections(repo, hours, show).expect("a sync hydrates");
            assert_eq!(
                numbers(&hydrated),
                vec![
                    ItemRef::github(1),
                    ItemRef::github(2),
                    ItemRef::github(3),
                    ItemRef::github(4)
                ],
                "derived order is sidebar order"
            );
            assert_eq!(
                hydrated[0].statuses, walked[1].statuses,
                "row fields round trip"
            );

            // Change-only sync: #1 retitled stays in place, #3 moves list,
            // #6 is new, #7 is a closed (unmerged) PR no list shows.
            let (last, stored) = load_rows(repo);
            assert!(api::delta_since(last.as_deref(), false).is_some(), "a fresh sync takes the delta path");
            let mut retitled = walked[1].clone();
            retitled.issue.title = "renamed".to_string();
            let mut closed = dated(ItemRef::github(3), false, "CLOSED", now, 0);
            closed.issue.created_at = walked[0].issue.created_at.clone();
            let delta = vec![
                closed,
                retitled,
                dated(ItemRef::github(6), true, "OPEN", now, 0),
                dated(ItemRef::github(7), true, "CLOSED", now, 0),
            ];
            let shown = api::visible_rows(overlay(stored, &delta), now, hours, show);
            assert_eq!(
                numbers(&shown),
                vec![
                    ItemRef::github(6),
                    ItemRef::github(1),
                    ItemRef::github(2),
                    ItemRef::github(3),
                    ItemRef::github(4)
                ]
            );
            save_rows(repo, last.as_deref().unwrap(), &delta);
            let hydrated = cached_sections(repo, hours, show).expect("hydrates after delta");
            assert_eq!(
                numbers(&hydrated),
                vec![
                    ItemRef::github(6),
                    ItemRef::github(1),
                    ItemRef::github(2),
                    ItemRef::github(3),
                    ItemRef::github(4)
                ],
                "the store reads back what the sync returned"
            );
            assert_eq!(hydrated[1].issue.title, "renamed");
            assert_eq!(hydrated[3].issue.state, "CLOSED");
        });
    }

    /// The store holds the repo's rows, not one window's: reads apply
    /// the list mask and window, whatever window or mask synced them.
    #[test]
    fn stored_rows_filter_by_mask_and_window_on_read() {
        with_temp_data(|| {
            let repo = "o/r";
            let now = chrono::Utc::now();
            let rows = vec![
                dated(ItemRef::github(1), true, "OPEN", now, 1),
                dated(ItemRef::github(2), true, "MERGED", now, 30),
                dated(ItemRef::github(3), true, "MERGED", now, 2),
                dated(ItemRef::github(4), false, "CLOSED", now, 2),
                dated(ItemRef::github(5), true, "CLOSED", now, 1),
            ];
            save_rows(repo, &now.to_rfc3339(), &rows);
            assert_eq!(
                numbers(&cached_sections(repo, 168, [true; 4]).unwrap()),
                vec![
                    ItemRef::github(1),
                    ItemRef::github(3),
                    ItemRef::github(2),
                    ItemRef::github(4)
                ],
                "the closed PR is in no list"
            );
            assert_eq!(
                numbers(&cached_sections(repo, 24, [true; 4]).unwrap()),
                vec![ItemRef::github(1), ItemRef::github(3), ItemRef::github(4)],
                "aged-out merged PR drops"
            );
            assert_eq!(
                numbers(&cached_sections(repo, 24, [true, false, true, false]).unwrap()),
                vec![ItemRef::github(1)],
                "hidden lists drop"
            );
            assert_eq!(
                numbers(&cached_sections(repo, 0, [false, true, false, false]).unwrap()),
                vec![ItemRef::github(3), ItemRef::github(2)],
                "all time reads every stored row"
            );
        });
    }

    #[test]
    fn sync_walk_covers_at_least_a_week() {
        assert_eq!(sync_hours(1), SYNC_MIN_HOURS);
        assert_eq!(sync_hours(24), SYNC_MIN_HOURS);
        assert_eq!(sync_hours(168), 168);
        assert_eq!(sync_hours(720), 720);
        assert_eq!(sync_hours(0), 0, "all time stays unbounded");
    }

    /// Two ghit processes on one repo with different windows: one lease
    /// holder syncs, both see each sync once, neither view clobbers the
    /// other, and a lapsed or released lease hands over.
    #[test]
    fn two_processes_share_one_sync_without_clobbering_views() {
        with_temp_data(|| {
            let repo = "o/r";
            let now = chrono::Utc::now();
            let t = now.timestamp();
            // A: 24h window, every list. B: 7d window, PRs only.
            let (a_view, b_view) = ((24, [true; 4]), (168, [true, true, false, false]));
            let mut a = StoreLink::open(repo).expect("store opens");
            let mut b = StoreLink::open(repo).expect("store opens");
            assert!(a.holding() && !b.holding(), "only one process holds the lease");

            // A syncs the whole repo, whatever its own view.
            save_rows(
                repo,
                &now.to_rfc3339(),
                &[
                    dated(ItemRef::github(1), true, "OPEN", now, 3),
                    dated(ItemRef::github(2), true, "MERGED", now, 30),
                    dated(ItemRef::github(3), false, "OPEN", now, 2),
                    dated(ItemRef::github(4), false, "CLOSED", now, 50),
                ],
            );
            assert!(
                a.changed() && b.changed(),
                "every process sees the new sync"
            );
            assert!(!b.changed(), "once per sync");
            assert_eq!(
                numbers(&a.rows(a_view.0, a_view.1).unwrap()),
                vec![ItemRef::github(1), ItemRef::github(3)]
            );
            assert_eq!(
                numbers(&b.rows(b_view.0, b_view.1).unwrap()),
                vec![ItemRef::github(1), ItemRef::github(2)]
            );
            a.claim(false, t);
            assert!(a.holding() && !b.changed(), "a lease renewal is not a new sync");

            // A stops renewing (a crash): B takes over once the lease expires.
            b.claim(false, t + LEASE_TTL_SECS - 1);
            assert!(!b.holding(), "a live lease refuses B");
            b.claim(false, t + LEASE_TTL_SECS);
            assert!(b.holding(), "an expired lease hands over");
            a.claim(false, t + LEASE_TTL_SECS + 1);
            assert!(!a.holding(), "A learns it lost the lease");

            // B's sync keeps A's view whole: it covers the repo, not B's view.
            save_rows(
                repo,
                &chrono::Utc::now().to_rfc3339(),
                &[dated(ItemRef::github(5), false, "OPEN", now, 0)],
            );
            assert!(a.changed());
            assert_eq!(
                numbers(&a.rows(a_view.0, a_view.1).unwrap()),
                vec![ItemRef::github(1), ItemRef::github(5), ItemRef::github(3)]
            );
            assert_eq!(
                numbers(&b.rows(b_view.0, b_view.1).unwrap()),
                vec![ItemRef::github(1), ItemRef::github(2)]
            );

            // A quitting holder releases: the other takes over on its next try.
            drop(b);
            a.claim(false, t + LEASE_TTL_SECS + 2);
            assert!(a.holding(), "a released lease is free before its expiry");
        });
    }

    #[tokio::test]
    async fn fetches_fail_before_the_network_without_a_repo() {
        // Empty repo errors before any token lookup or request, so these
        // hold with or without auth configured.
        assert!(fetch_number("", ItemRef::github(1), false, None).await.is_err());
        assert!(fetch_recent_sections("", 24, [true; 4], Refresh::Delta, None).await.is_err());
        assert!(search_repo("", "bug", None).await.is_err());
        assert!(fetch_repo_meta("").await.is_err());
        assert!(fetch_board("").await.is_err());
        assert!(fetch_board_detail("", ItemRef::github(1)).await.is_err());
        assert!(fetch_diff("", ItemRef::github(1), false).await.is_err());
        assert!(fetch_sub_issues("", ItemRef::github(1)).await.is_err());
    }

    /// A stored detail whose `updated_at` matches the synced row opens
    /// with no request (the unit-test client panics on any), and its
    /// project statuses come from the synced row.
    #[test]
    fn unchanged_item_opens_from_the_store_with_zero_requests() {
        let _guard = cache::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved = (std::env::var("XDG_DATA_HOME").ok(), std::env::var("GH_TOKEN").ok());
        std::env::set_var("XDG_DATA_HOME", dir.path());
        std::env::set_var("GH_TOKEN", "test");
        let mut listed = row(ItemRef::github(5), false, "OPEN", None);
        listed.issue.updated_at = "t1".to_string();
        save_rows("o/r", &chrono::Utc::now().to_rfc3339(), &[listed]);
        let data = IssueData {
            number: ItemRef::github(5),
            body: "stored body".to_string(),
            updated_at: "t1".to_string(),
            ..Default::default()
        };
        save_bundle(
            "o/r",
            ItemRef::github(5),
            &(data, vec![("Board".to_string(), Some("Old".to_string()))]),
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let (got, statuses) = runtime
            .block_on(fetch_number("o/r", ItemRef::github(5), false, None))
            .unwrap();
        for (key, value) in [("XDG_DATA_HOME", saved.0), ("GH_TOKEN", saved.1)] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        assert_eq!(got.body, "stored body");
        assert_eq!(statuses, vec![("Board".to_string(), Some("Todo".to_string()))], "statuses come from the sync");
    }

    /// Run `f` with an isolated data dir and a token (HTTP panics in tests).
    fn with_store_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = cache::test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved = (std::env::var("XDG_DATA_HOME").ok(), std::env::var("GH_TOKEN").ok());
        std::env::set_var("XDG_DATA_HOME", dir.path());
        std::env::set_var("GH_TOKEN", "test");
        let out = f();
        for (key, value) in [("XDG_DATA_HOME", saved.0), ("GH_TOKEN", saved.1)] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        out
    }

    fn fetched(number: ItemRef, is_pr: bool, state: &str, updated: &str) -> IssueBundle {
        let data = IssueData {
            number,
            is_pr,
            state: state.to_string(),
            title: format!("t{number}", number = number.number),
            body: "b".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: updated.to_string(),
            ..Default::default()
        };
        (data, vec![])
    }

    /// An opened item no sync listed joins `items`; its second open and
    /// its diff then uses the store.
    #[test]
    fn an_opened_unlisted_item_joins_the_store() {
        with_store_env(|| {
            let bundle = fetched(ItemRef::github(7), true, "MERGED", "2026-02-01T00:00:00Z");
            save_bundle("o/r", ItemRef::github(7), &bundle);
            let row = store::open("o/r")
                .unwrap()
                .row(ItemRef::github(7))
                .expect("row stored");
            assert_eq!(row.issue.state, "MERGED");
            assert_eq!(row.issue.body, "", "rows carry no body");
            assert_eq!(row.window_at.as_deref(), Some("2026-02-01T00:00:00Z"));
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let (got, _) = runtime
                .block_on(fetch_number("o/r", ItemRef::github(7), false, None))
                .unwrap();
            assert_eq!(got.body, "b", "second open makes zero requests");
            save_detail("o/r", store::Detail::Diff, ItemRef::github(7), &vec![1u8]);
            assert_eq!(
                stored_detail::<Vec<u8>>("o/r", store::Detail::Diff, ItemRef::github(7)),
                Some(vec![1])
            );
        });
    }

    #[test]
    fn an_older_fetched_row_does_not_overwrite() {
        with_store_env(|| {
            let mut listed = row(ItemRef::github(5), false, "OPEN", None);
            listed.issue.updated_at = "2026-03-01T00:00:00Z".to_string();
            save_rows("o/r", "s", &[listed]);
            save_bundle(
                "o/r",
                ItemRef::github(5),
                &fetched(ItemRef::github(5), false, "CLOSED", "2026-02-01T00:00:00Z"),
            );
            assert_eq!(
                store::open("o/r")
                    .unwrap()
                    .row(ItemRef::github(5))
                    .unwrap()
                    .issue
                    .state,
                "OPEN"
            );
        });
    }

    /// A fetched row without CI, statuses, parent or summary keeps the
    /// stored row's.
    #[test]
    fn a_fetched_row_keeps_stored_checks_and_statuses() {
        with_store_env(|| {
            let mut listed = row(ItemRef::github(5), true, "OPEN", None);
            listed.issue.updated_at = "2026-02-01T00:00:00Z".to_string();
            listed.issue.checks = Some(model::CheckState::Pass);
            listed.issue.parent = Some(ItemRef::github(2));
            save_rows("o/r", "s", &[listed]);
            save_bundle(
                "o/r",
                ItemRef::github(5),
                &fetched(ItemRef::github(5), true, "OPEN", "2026-03-01T00:00:00Z"),
            );
            let got = store::open("o/r").unwrap().row(ItemRef::github(5)).unwrap();
            assert_eq!(got.issue.updated_at, "2026-03-01T00:00:00Z", "the newer fetch landed");
            assert_eq!(got.issue.checks, Some(model::CheckState::Pass));
            assert_eq!(got.issue.parent, Some(ItemRef::github(2)));
            assert_eq!(got.statuses, vec![("Board".to_string(), Some("Todo".to_string()))]);
        });
    }

    #[test]
    fn only_a_404_is_not_found() {
        assert!(is_not_found("GitHub request failed: 404 Not Found for https://api.github.com/repos/o/r/issues/9: {}"));
        assert!(!is_not_found("GitHub request failed: 500 for https://api.github.com/repos/o/r/issues/404"));
    }
}
