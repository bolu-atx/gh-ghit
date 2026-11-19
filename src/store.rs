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

const SCHEMA_VERSION: &str = "1";

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
}
