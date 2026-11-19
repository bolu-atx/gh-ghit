//! Local copy of each repo: one SQLite file per GitHub host and
//! `owner/repo`, shared by every clone, worktree and running ghit.
//!
//! The file is a rebuildable copy, so a schema version mismatch drops
//! every table and starts empty. No migrations.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::api::Row;
use crate::cache;

const SCHEMA_VERSION: &str = "2";

/// How long a write waits for another process's lock before it fails.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE items (
    number INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    title TEXT NOT NULL,
    author TEXT NOT NULL,
    labels TEXT NOT NULL,
    assignees TEXT NOT NULL,
    milestone TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    merged_at TEXT,
    closed_at TEXT,
    checks TEXT,
    project_status TEXT NOT NULL,
    window_at TEXT,
    position INTEGER,
    row_json TEXT NOT NULL
);
CREATE TABLE details (
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (number, kind)
);
";

/// The last sync: its start (RFC 3339), which the next change-only
/// search reaches back to, and the window and list mask it walked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sync {
    pub started_at: String,
    pub hours: u64,
    pub show: [bool; 4],
}

impl Sync {
    /// True when this sync's walk fetched every row the given window and
    /// list mask show: a window no wider (0 is unbounded) and no list
    /// it hid.
    pub fn covers(&self, hours: u64, show: [bool; 4]) -> bool {
        let window = self.hours == 0 || (hours != 0 && hours <= self.hours);
        window && show.iter().zip(self.show).all(|(want, had)| !want || had)
    }
}

/// What the finder needs of one stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub number: u64,
    pub is_pr: bool,
    pub state: String,
    pub title: String,
    pub author: String,
    pub labels: Vec<String>,
}

/// One open repo store.
pub struct Store {
    conn: Connection,
}

/// `<data root>/store/<host>/<owner>/<repo>.sqlite`, lowercased. `None`
/// for a slug that is not a plain `owner/repo`.
pub fn path(repo: &str) -> Option<PathBuf> {
    let (owner, name) = repo.split_once('/')?;
    let plain = |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\']);
    if !plain(owner) || !plain(name) {
        return None;
    }
    let file = format!("{}.sqlite", name.to_lowercase());
    Some(cache::base_dir().join("store").join(cache::host()).join(owner.to_lowercase()).join(file))
}

/// Open (or create) the store for `repo` under the current host.
pub fn open(repo: &str) -> Result<Store, String> {
    let path = path(repo).ok_or_else(|| format!("store: not an owner/repo slug: {repo}"))?;
    if let Some(dir) = path.parent() {
        if !cache::ensure_private_dir(dir) {
            return Err(format!("store: cannot create {}", dir.display()));
        }
    }
    open_at(&path)
}

/// Open the store file at `path` in WAL mode, rebuilding it when its
/// schema version differs.
fn open_at(path: &Path) -> Result<Store, String> {
    let fail = |e: rusqlite::Error| format!("store: {e}");
    let mut conn = Connection::open(path).map_err(fail)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(fail)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0)).map_err(fail)?;
    if schema_version(&conn).as_deref() != Some(SCHEMA_VERSION) {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        // Another process may have rebuilt it while this one waited.
        if schema_version(&tx).as_deref() != Some(SCHEMA_VERSION) {
            let tables: Vec<String> = {
                let mut stmt = tx
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
                    .map_err(fail)?;
                let names = stmt.query_map([], |r| r.get(0)).map_err(fail)?;
                names.collect::<Result<_, _>>().map_err(fail)?
            };
            for table in tables {
                tx.execute_batch(&format!("DROP TABLE \"{}\"", table.replace('"', "\"\""))).map_err(fail)?;
            }
            tx.execute_batch(SCHEMA).map_err(fail)?;
            tx.execute("INSERT INTO meta (key, value) VALUES ('schema_version', ?1)", [SCHEMA_VERSION])
                .map_err(fail)?;
        }
        tx.commit().map_err(fail)?;
    }
    Ok(Store { conn })
}

/// Stored schema version; `None` when the file is new or not ours.
fn schema_version(conn: &Connection) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

impl Store {
    /// The last sync, or `None` before the first one.
    pub fn last_sync(&self) -> Option<Sync> {
        let get = |key: &str| -> Option<String> {
            self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
        };
        let show_text = get("sync_show")?;
        let flags: Vec<bool> = show_text.chars().map(|c| c == '1').collect();
        Some(Sync {
            started_at: get("sync_started_at")?,
            hours: get("sync_hours")?.parse().ok()?,
            show: flags.try_into().ok()?,
        })
    }

    /// Every stored row: rows of the last sidebar result in its order,
    /// then the rest, newest number first.
    pub fn rows(&self) -> Result<Vec<Row>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self
            .conn
            .prepare("SELECT row_json FROM items ORDER BY position IS NULL, position, number DESC")
            .map_err(fail)?;
        let raw = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(fail)?;
        let mut rows = Vec::new();
        for json in raw {
            // A row that no longer parses is skipped; the next sync rewrites it.
            if let Ok(row) = serde_json::from_str(&json.map_err(fail)?) {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// Every stored item as a finder candidate, most recently updated first.
    pub fn finder_candidates(&self) -> Result<Vec<Candidate>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self
            .conn
            .prepare("SELECT number, kind, state, title, author, labels FROM items ORDER BY updated_at DESC, number DESC")
            .map_err(fail)?;
        let rows = stmt
            .query_map([], |r| {
                let labels: String = r.get(5)?;
                Ok(Candidate {
                    number: r.get::<_, i64>(0)? as u64,
                    is_pr: r.get::<_, String>(1)? == "pr",
                    state: r.get(2)?,
                    title: r.get(3)?,
                    author: r.get(4)?,
                    labels: serde_json::from_str(&labels).unwrap_or_default(),
                })
            })
            .map_err(fail)?;
        rows.collect::<Result<_, _>>().map_err(fail)
    }

    /// One transaction: upsert `listed` (the sidebar result, positioned in
    /// its order) and `other` (fetched rows no list shows now, unpositioned),
    /// clear the old positions, and record `sync`.
    pub fn write(&mut self, sync: &Sync, listed: &[Row], other: &[Row]) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        tx.execute("UPDATE items SET position = NULL", []).map_err(fail)?;
        {
            let mut upsert = tx.prepare(UPSERT).map_err(fail)?;
            let positioned = listed.iter().enumerate().map(|(i, row)| (row, Some(i as i64)));
            for (row, position) in positioned.chain(other.iter().map(|row| (row, None))) {
                let issue = &row.issue;
                let names = |list: Vec<&str>| serde_json::to_string(&list).unwrap_or_default();
                let merged_at = (issue.is_pr && issue.state == "MERGED").then(|| row.window_at.clone()).flatten();
                upsert
                    .execute(params![
                        issue.number as i64,
                        if issue.is_pr { "pr" } else { "issue" },
                        issue.state,
                        issue.title,
                        issue.author.login,
                        names(issue.labels.iter().map(|l| l.name.as_str()).collect()),
                        names(issue.assignees.iter().map(|a| a.login.as_str()).collect()),
                        issue.milestone,
                        issue.created_at,
                        issue.updated_at,
                        merged_at,
                        issue.checks.map(|c| format!("{c:?}")),
                        serde_json::to_string(&row.statuses).unwrap_or_default(),
                        row.window_at,
                        position,
                        serde_json::to_string(row).map_err(|e| format!("store: {e}"))?,
                    ])
                    .map_err(fail)?;
            }
        }
        let show: String = sync.show.iter().map(|s| if *s { '1' } else { '0' }).collect();
        for (key, value) in [("sync_started_at", sync.started_at.clone()), ("sync_hours", sync.hours.to_string()), ("sync_show", show)] {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(fail)?;
        }
        tx.commit().map_err(fail)
    }
}

/// The detail payloads a store keeps per item number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Issue body, comments and review threads, with project statuses.
    Issue,
    /// Changed files of a PR.
    Diff,
    /// Dependency graph.
    Graph,
}

impl Detail {
    fn name(self) -> &'static str {
        match self {
            Detail::Issue => "issue",
            Detail::Diff => "diff",
            Detail::Graph => "graph",
        }
    }
}

/// True when an open serves a stored detail with no request: no force,
/// and the detail was fetched at the `updated_at` the latest sync holds.
/// An item no sync has listed (`current` is `None`) always fetches.
pub fn serve_stored(force: bool, stored: Option<&str>, current: Option<&str>) -> bool {
    !force && matches!((stored, current), (Some(a), Some(b)) if a == b)
}

impl Store {
    /// `updated_at` of the item as the latest sync listed it.
    pub fn item_updated_at(&self, number: u64) -> Option<String> {
        self.conn
            .query_row("SELECT updated_at FROM items WHERE number = ?1", [number as i64], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    /// Project statuses of the item as the latest sync listed them.
    pub fn item_statuses(&self, number: u64) -> Option<Vec<(String, Option<String>)>> {
        let json: String = self
            .conn
            .query_row("SELECT project_status FROM items WHERE number = ?1", [number as i64], |r| r.get(0))
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    /// The stored detail when it is fresh for an open (`serve_stored`).
    pub fn fresh_detail<T: serde::de::DeserializeOwned>(&self, kind: Detail, number: u64, force: bool) -> Option<T> {
        let (stored_at, payload): (String, String) = self
            .conn
            .query_row(
                "SELECT updated_at, payload FROM details WHERE number = ?1 AND kind = ?2",
                params![number as i64, kind.name()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()?;
        let current = self.item_updated_at(number);
        if !serve_stored(force, Some(&stored_at), current.as_deref()) {
            return None;
        }
        // A payload that no longer parses is a miss; the fetch rewrites it.
        serde_json::from_str(&payload).ok()
    }

    /// Store a detail with the `updated_at` it was fetched at.
    pub fn put_detail<T: serde::Serialize>(&self, kind: Detail, number: u64, updated_at: &str, value: &T) -> Result<(), String> {
        let payload = serde_json::to_string(value).map_err(|e| format!("store: {e}"))?;
        self.conn
            .execute(
                "INSERT INTO details (number, kind, updated_at, payload) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (number, kind) DO UPDATE SET updated_at = excluded.updated_at, payload = excluded.payload",
                params![number as i64, kind.name(), updated_at, payload],
            )
            .map(|_| ())
            .map_err(|e| format!("store: {e}"))
    }

    /// Drop an item and its details: it was deleted or transferred.
    pub fn drop_item(&mut self, number: u64) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        tx.execute("DELETE FROM items WHERE number = ?1", [number as i64]).map_err(fail)?;
        tx.execute("DELETE FROM details WHERE number = ?1", [number as i64]).map_err(fail)?;
        tx.commit().map_err(fail)
    }
}

/// `closed_at` stays NULL: the listing fields carry no `closedAt`.
const UPSERT: &str = "
INSERT INTO items (number, kind, state, title, author, labels, assignees, milestone,
    created_at, updated_at, merged_at, checks, project_status, window_at, position, row_json)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
ON CONFLICT (number) DO UPDATE SET
    kind = excluded.kind, state = excluded.state, title = excluded.title,
    author = excluded.author, labels = excluded.labels, assignees = excluded.assignees,
    milestone = excluded.milestone, created_at = excluded.created_at,
    updated_at = excluded.updated_at, merged_at = excluded.merged_at,
    checks = excluded.checks, project_status = excluded.project_status,
    window_at = excluded.window_at, position = excluded.position, row_json = excluded.row_json
";

#[cfg(test)]
mod tests {
    use super::*;

    fn row(number: u64, state: &str) -> Row {
        Row {
            issue: crate::model::IssueData { number, state: state.to_string(), ..Default::default() },
            statuses: vec![],
            detail: crate::model::BoardDetail::default(),
            complete: false,
            window_at: None,
        }
    }

    fn sync(started_at: &str) -> Sync {
        Sync { started_at: started_at.to_string(), hours: 24, show: [true, false, true, true] }
    }

    fn numbers(store: &Store) -> Vec<u64> {
        store.rows().unwrap().iter().map(|r| r.issue.number).collect()
    }

    #[test]
    fn finder_candidates_list_every_item_newest_update_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("s.sqlite")).unwrap();
        let mut a = row(1, "OPEN");
        a.issue.title = "old".into();
        a.issue.updated_at = "2026-01-01T00:00:00Z".into();
        a.issue.labels = vec![crate::model::Label { name: "bug".into() }];
        let mut b = row(2, "MERGED");
        b.issue.is_pr = true;
        b.issue.updated_at = "2026-02-01T00:00:00Z".into();
        store.write(&sync("2026-02-02T00:00:00Z"), &[], &[a, b]).unwrap();
        let got = store.finder_candidates().unwrap();
        assert_eq!(got.iter().map(|c| c.number).collect::<Vec<_>>(), vec![2, 1]);
        assert!(got[0].is_pr && got[0].state == "MERGED");
        assert_eq!((got[1].title.as_str(), got[1].labels.clone()), ("old", vec!["bug".to_string()]));
    }

    #[test]
    fn path_is_keyed_by_host_and_lowercased_slug() {
        let _guard = cache::test_env_lock();
        let saved = (std::env::var("XDG_DATA_HOME").ok(), std::env::var("GH_HOST").ok());
        std::env::set_var("XDG_DATA_HOME", "/data");
        std::env::set_var("GH_HOST", "GHE.Corp");
        let got = path("Acme/Widget");
        let rejected = [path("acme"), path("../x"), path("a/.."), path("a/b/c"), path("/x")];
        for (key, value) in [("XDG_DATA_HOME", saved.0), ("GH_HOST", saved.1)] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        assert_eq!(got, Some(PathBuf::from("/data/ghit/store/ghe.corp/acme/widget.sqlite")));
        assert!(rejected.iter().all(Option::is_none), "{rejected:?}");
    }

    #[test]
    fn writes_round_trip_rows_order_and_sync() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert!(store.last_sync().is_none() && numbers(&store).is_empty(), "a new store is empty");
        store.write(&sync("t1"), &[row(5, "OPEN"), row(2, "OPEN")], &[row(9, "CLOSED")]).unwrap();
        assert_eq!(numbers(&store), vec![5, 2, 9], "listed order first, then the rest");
        assert_eq!(store.last_sync(), Some(sync("t1")));
        // A later write repositions; rows it leaves out keep their data.
        store.write(&sync("t2"), &[row(2, "OPEN")], &[]).unwrap();
        assert_eq!(numbers(&store), vec![2, 9, 5]);
        assert_eq!(store.last_sync().unwrap().started_at, "t2");
        let wal: String = store.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(wal, "wal");
    }

    #[test]
    fn two_connections_see_each_others_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let mut first = open_at(&file).unwrap();
        let mut second = open_at(&file).unwrap();
        first.write(&sync("t1"), &[row(1, "OPEN")], &[]).unwrap();
        assert_eq!(numbers(&second), vec![1]);
        second.write(&sync("t2"), &[row(2, "OPEN"), row(1, "OPEN")], &[]).unwrap();
        assert_eq!(numbers(&first), vec![2, 1]);
        assert_eq!(first.last_sync().unwrap().started_at, "t2");
    }

    #[test]
    fn schema_version_mismatch_rebuilds_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let mut store = open_at(&file).unwrap();
        store.write(&sync("t1"), &[row(1, "OPEN")], &[]).unwrap();
        store.conn.execute("UPDATE meta SET value = '0' WHERE key = 'schema_version'", []).unwrap();
        store.conn.execute_batch("CREATE TABLE leftover (x)").unwrap();
        drop(store);
        let store = open_at(&file).unwrap();
        assert!(store.last_sync().is_none() && numbers(&store).is_empty(), "mismatch drops the old copy");
        let leftover: i64 = store
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'leftover'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover, 0, "every old table goes");
        assert_eq!(schema_version(&store.conn).as_deref(), Some(SCHEMA_VERSION));
    }

    #[test]
    fn sync_covers_narrower_windows_and_fewer_lists_only() {
        let synced = Sync { started_at: String::new(), hours: 168, show: [true, true, false, true] };
        assert!(synced.covers(168, [true, true, false, true]));
        assert!(synced.covers(24, [true, false, false, false]), "narrower window, fewer lists");
        assert!(!synced.covers(720, [true; 4]), "wider window");
        assert!(!synced.covers(0, [true, false, false, false]), "unbounded window");
        assert!(!synced.covers(24, [false, false, true, false]), "a list it hid");
        let unbounded = Sync { hours: 0, ..synced };
        assert!(unbounded.covers(0, [true, false, false, false]) && unbounded.covers(24, [true, false, false, false]));
    }

    #[test]
    fn serve_stored_needs_equal_updated_at_and_no_force() {
        assert!(serve_stored(false, Some("t1"), Some("t1")));
        assert!(!serve_stored(false, Some("t1"), Some("t2")), "a newer sync refetches");
        assert!(!serve_stored(false, Some("t1"), None), "an unlisted item fetches");
        assert!(!serve_stored(false, None, Some("t1")), "nothing stored fetches");
        assert!(!serve_stored(true, Some("t1"), Some("t1")), "force refetches");
    }

    #[test]
    fn details_round_trip_per_kind_and_follow_the_item() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        let mut listed = row(5, "OPEN");
        listed.issue.updated_at = "t1".to_string();
        listed.statuses = vec![("Board".to_string(), Some("Todo".to_string()))];
        store.write(&sync("s"), &[listed], &[]).unwrap();
        assert_eq!(store.item_updated_at(5).as_deref(), Some("t1"));
        assert_eq!(store.item_statuses(5), Some(vec![("Board".to_string(), Some("Todo".to_string()))]));
        assert_eq!(store.item_updated_at(6), None);

        let issue = crate::model::IssueData { number: 5, body: "b".to_string(), updated_at: "t1".to_string(), ..Default::default() };
        let files = vec![crate::model::DiffFile { filename: "a.rs".to_string(), ..Default::default() }];
        let graph = crate::model::GraphData::default();
        store.put_detail(Detail::Issue, 5, "t1", &issue).unwrap();
        store.put_detail(Detail::Diff, 5, "t1", &files).unwrap();
        store.put_detail(Detail::Graph, 5, "t1", &graph).unwrap();
        assert_eq!(store.fresh_detail::<crate::model::IssueData>(Detail::Issue, 5, false).unwrap().body, "b");
        assert_eq!(store.fresh_detail::<Vec<crate::model::DiffFile>>(Detail::Diff, 5, false).unwrap()[0].filename, "a.rs");
        assert_eq!(store.fresh_detail::<crate::model::GraphData>(Detail::Graph, 5, false), Some(graph));
        assert!(store.fresh_detail::<crate::model::IssueData>(Detail::Issue, 5, true).is_none(), "force skips the store");

        // A sync that reports a newer updated_at makes every detail stale.
        let mut newer = row(5, "OPEN");
        newer.issue.updated_at = "t2".to_string();
        store.write(&sync("s2"), &[newer], &[]).unwrap();
        assert!(store.fresh_detail::<crate::model::IssueData>(Detail::Issue, 5, false).is_none());

        // A refetch replaces the stored copy.
        let fixed = crate::model::IssueData { body: "new".to_string(), updated_at: "t2".to_string(), ..issue.clone() };
        store.put_detail(Detail::Issue, 5, "t2", &fixed).unwrap();
        assert_eq!(store.fresh_detail::<crate::model::IssueData>(Detail::Issue, 5, false).unwrap().body, "new");

        // A 404 drops the item and its details.
        store.drop_item(5).unwrap();
        assert!(numbers(&store).is_empty() && store.item_updated_at(5).is_none());
        assert!(store.fresh_detail::<crate::model::IssueData>(Detail::Issue, 5, false).is_none());
        let left: i64 = store.conn.query_row("SELECT count(*) FROM details", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }
}
