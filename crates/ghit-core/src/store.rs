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
use crate::model::{IssueData, ItemRef, MapItem, PairedRef, Source};

const SCHEMA_VERSION: &str = "6";

/// How long a write waits for another process's lock before it fails.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE items (
    source TEXT NOT NULL,
    number INTEGER NOT NULL,
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
    row_json TEXT NOT NULL,
    PRIMARY KEY (source, number)
);
CREATE TABLE details (
    source TEXT NOT NULL,
    number INTEGER NOT NULL,
    kind TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (source, number, kind)
);
CREATE TABLE backfill (
    name TEXT PRIMARY KEY,
    cursor TEXT,
    done INTEGER NOT NULL DEFAULT 0,
    fetched INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE edges (
    src_source TEXT NOT NULL,
    src INTEGER NOT NULL,
    dst_source TEXT NOT NULL,
    dst INTEGER NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY (dst_source, dst, src_source, src, kind)
);
CREATE INDEX edges_src ON edges (src_source, src);
";

/// The sync lease and the last sync start, one row per repo store.
/// `holder` names the process that syncs the repo; its claim lasts until
/// `lease_expires` (unix seconds) unless it renews. `started_at` (RFC
/// 3339) is the start of the last sync, which the next change-only search
/// reaches back to.
const SYNC_SCHEMA: &str = "
CREATE TABLE sync_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    started_at TEXT,
    holder TEXT NOT NULL DEFAULT '',
    lease_expires INTEGER NOT NULL DEFAULT 0
);
INSERT INTO sync_state (id) VALUES (1);
";

/// What the finder needs of one stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub number: ItemRef,
    pub is_pr: bool,
    pub state: String,
    pub title: String,
    pub author: String,
    pub labels: Vec<String>,
}

/// What a link between two items means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// `src` blocks `dst`.
    Blocks,
    /// `src` is the parent of `dst`.
    Parent,
    /// The pull request `src` closes the issue `dst`.
    Closes,
}

impl EdgeKind {
    fn name(self) -> &'static str {
        match self {
            EdgeKind::Blocks => "blocks",
            EdgeKind::Parent => "parent",
            EdgeKind::Closes => "closes",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [EdgeKind::Blocks, EdgeKind::Parent, EdgeKind::Closes].into_iter().find(|kind| kind.name() == name)
    }
}

/// One link between two stored item numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub src: ItemRef,
    pub dst: ItemRef,
    pub kind: EdgeKind,
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
            tx.execute_batch(SYNC_SCHEMA).map_err(fail)?;
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
    /// Every stored row, newest number first. The sidebar derives list
    /// order from row data (`api::visible_rows`).
    pub fn rows(&self) -> Result<Vec<Row>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self.conn
            .prepare("SELECT row_json FROM items ORDER BY source, number DESC").map_err(fail)?;
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

    /// The stored row of one item.
    pub fn row(&self, number: ItemRef) -> Option<Row> {
        let json: String = self
            .conn
            .query_row(
                "SELECT row_json FROM items WHERE source = ?1 AND number = ?2",
                params![number.source.as_str(), number.number as i64],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    /// Upsert `rows` newer-only: a stored row at the same or a later
    /// `updated_at` stays as it is. Leaves `sync_state` untouched.
    pub fn upsert_rows_newer(&mut self, rows: &[Row]) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        upsert_rows(&tx, &upsert_newer_sql(), rows)?;
        tx.commit().map_err(fail)
    }

    /// Every stored link between items.
    pub fn edges(&self) -> Result<Vec<Edge>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self.conn.prepare(
            "SELECT src_source, src, dst_source, dst, kind FROM edges
             ORDER BY dst_source, dst, src_source, src, kind",
        ).map_err(fail)?;
        let edges = stmt.query_map([], read_edge).map_err(fail)?;
        Ok(edges.collect::<Result<Vec<_>, _>>().map_err(fail)?.into_iter().flatten().collect())
    }

    /// Pull requests linked to `number` by a closes edge, with state.
    pub fn linked_prs(&self, number: ItemRef) -> Vec<PairedRef> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT src FROM edges WHERE dst_source = ?1 AND dst = ?2
             AND src_source = 'github' AND kind = 'closes' ORDER BY src",
        ) else {
            return vec![];
        };
        let Ok(rows) = stmt.query_map(
            params![number.source.as_str(), number.number as i64],
            |row| row.get::<_, i64>(0),
        ) else {
            return vec![];
        };
        rows.filter_map(Result::ok)
            .map(|src| {
                let item = ItemRef::github(src as u64);
                PairedRef {
                    number: item,
                    state: self.row(item).map_or(String::new(), |row| row.issue.state),
                }
            })
            .collect()
    }

    /// Every stored item, as the dependency map needs it.
    pub fn map_items(&self) -> Result<Vec<MapItem>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self.conn
            .prepare("SELECT source, number, kind, state, title, milestone, labels FROM items
                      ORDER BY source, number")
            .map_err(fail)?;
        let items = stmt
            .query_map([], |r| {
                let labels: String = r.get(6)?;
                Ok(MapItem {
                    number: read_item(r, 0)?,
                    is_pr: r.get::<_, String>(2)? == "pr",
                    state: r.get(3)?,
                    title: r.get(4)?,
                    milestone: r.get(5)?,
                    labels: serde_json::from_str(&labels).unwrap_or_default(),
                })
            })
            .map_err(fail)?;
        items.collect::<Result<_, _>>().map_err(fail)
    }

    /// Every stored item as a finder candidate, most recently updated first.
    pub fn finder_candidates(&self) -> Result<Vec<Candidate>, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let mut stmt = self
            .conn
            .prepare("SELECT source, number, kind, state, title, author, labels FROM items
                      ORDER BY updated_at DESC, source, number DESC")
            .map_err(fail)?;
        let rows = stmt
            .query_map([], |r| {
                let labels: String = r.get(6)?;
                Ok(Candidate {
                    number: read_item(r, 0)?,
                    is_pr: r.get::<_, String>(2)? == "pr",
                    state: r.get(3)?,
                    title: r.get(4)?,
                    author: r.get(5)?,
                    labels: serde_json::from_str(&labels).unwrap_or_default(),
                })
            })
            .map_err(fail)?;
        rows.collect::<Result<_, _>>().map_err(fail)
    }

    /// One transaction: upsert `rows` and record `started_at` as the last
    /// sync start. Rows it leaves out keep their stored data.
    pub fn write(&mut self, started_at: &str, rows: &[Row]) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        upsert_rows(&tx, UPSERT, rows)?;
        tx.execute("UPDATE sync_state SET started_at = ?1 WHERE id = 1", [started_at]).map_err(fail)?;
        tx.commit().map_err(fail)
    }
}

impl Store {
    /// Start of the last Jira sync. It has its own `meta` key, apart from
    /// the GitHub sync start in `sync_state`.
    pub fn jira_last_sync(&self) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'jira_last_sync'", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    /// One transaction: upsert Jira `rows` and record `started_at` as the
    /// last Jira sync start. GitHub rows and the GitHub sync start stay.
    pub fn write_jira(&mut self, started_at: &str, rows: &[Row]) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        upsert_rows(&tx, UPSERT, rows)?;
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('jira_last_sync', ?1)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [started_at],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)
    }

    /// Store GitHub search hits. A hit only replaces a stored row with an
    /// older `updated_at`. REST hits carry no CI or project status, so a
    /// replaced row keeps the stored `checks`, statuses and parent.
    /// `sync_state` stays untouched, so no process sees a sync.
    pub fn upsert_search(&mut self, rows: &[Row]) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        for hit in rows {
            let stored: Option<String> = tx
                .query_row(
                    "SELECT row_json FROM items WHERE source = ?1 AND number = ?2",
                    params![hit.issue.number.source.as_str(), hit.issue.number.number as i64],
                    |r| r.get(0),
                )
                .optional()
                .map_err(fail)?;
            let mut row = hit.clone();
            if let Some(old) = stored.and_then(|json| serde_json::from_str::<Row>(&json).ok()) {
                if old.issue.updated_at >= row.issue.updated_at {
                    continue;
                }
                row.issue.checks = old.issue.checks;
                row.issue.parent = old.issue.parent;
                row.statuses = old.statuses;
            }
            upsert_rows(&tx, UPSERT, std::slice::from_ref(&row))?;
        }
        tx.commit().map_err(fail)
    }

    /// The stored rows with these numbers, most recently updated first.
    pub fn rows_by_number(&self, numbers: &[ItemRef]) -> Vec<Row> {
        let mut rows: Vec<Row> = self.rows().unwrap_or_default().into_iter()
            .filter(|row| numbers.contains(&row.issue.number))
            .collect();
        rows.sort_by(|a, b| (&b.issue.updated_at, b.issue.number).cmp(&(&a.issue.updated_at, a.issue.number)));
        rows
    }
}

fn read_item(row: &rusqlite::Row, column: usize) -> rusqlite::Result<ItemRef> {
    let tag: String = row.get(column)?;
    let source = Source::parse(&tag).ok_or_else(|| rusqlite::Error::InvalidColumnType(
        column, tag, rusqlite::types::Type::Text,
    ))?;
    Ok(ItemRef { source, number: row.get::<_, i64>(column + 1)? as u64 })
}

/// An edge row; `None` for a kind this build does not know.
fn read_edge(row: &rusqlite::Row) -> rusqlite::Result<Option<Edge>> {
    let kind: String = row.get(4)?;
    let (src, dst) = (read_item(row, 0)?, read_item(row, 2)?);
    Ok(EdgeKind::from_name(&kind).map(|kind| Edge { src, dst, kind }))
}

/// Upsert `rows` inside `tx` with the given upsert statement.
fn upsert_rows(tx: &rusqlite::Transaction, sql: &str, rows: &[Row]) -> Result<(), String> {
    let fail = |e: rusqlite::Error| format!("store: {e}");
    let mut upsert = tx.prepare(sql).map_err(fail)?;
    let mut changed = false;
    for row in rows {
        let issue = &row.issue;
        let names = |list: Vec<&str>| serde_json::to_string(&list).unwrap_or_default();
        let merged_at = (issue.is_pr && issue.state == "MERGED").then(|| row.window_at.clone()).flatten();
        let written = upsert
            .execute(params![
                issue.number.source.as_str(),
                issue.number.number as i64,
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
                serde_json::to_string(row).map_err(|e| format!("store: {e}"))?,
            ])
            .map_err(fail)?;
        // A newer-only upsert may leave the stored row alone; its edges stay too.
        if written > 0 {
            replace_edges(tx, issue).map_err(fail)?;
            changed = true;
        }
    }
    if changed {
        bump_generation(tx).map_err(fail)?;
    }
    Ok(())
}

/// Count one write to item rows or edges in the `meta` table.
fn bump_generation(tx: &rusqlite::Transaction) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('items_gen', '1')
         ON CONFLICT (key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
        [],
    )
    .map(|_| ())
}

/// Replace every edge into `issue` with the links its row carries.
/// Closes edges into a Jira issue are owned by the PR row that names
/// it, so a Jira row landing leaves them alone; a PR row replaces the
/// Jira closes edges it owns.
fn replace_edges(tx: &rusqlite::Transaction, issue: &IssueData) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM edges WHERE dst_source = ?1 AND dst = ?2
         AND NOT (dst_source = 'jira' AND kind = 'closes')",
        params![issue.number.source.as_str(), issue.number.number as i64],
    )?;
    if issue.is_pr {
        tx.execute(
            "DELETE FROM edges WHERE src_source = ?1 AND src = ?2
             AND dst_source = 'jira' AND kind = 'closes'",
            params![issue.number.source.as_str(), issue.number.number as i64],
        )?;
        for dst in &issue.jira_refs {
            tx.execute(
                "INSERT OR IGNORE INTO edges (src_source, src, dst_source, dst, kind)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![issue.number.source.as_str(), issue.number.number as i64,
                    dst.source.as_str(), dst.number as i64, EdgeKind::Closes.name()],
            )?;
        }
    }
    let links = issue.parent.map(|parent| (parent, EdgeKind::Parent))
        .into_iter()
        .chain(issue.links.blocked_by.iter().map(|&blocker| (blocker, EdgeKind::Blocks)))
        .chain(issue.links.closed_by.iter().map(|&pr| (pr, EdgeKind::Closes)));
    for (src, kind) in links {
        tx.execute(
            "INSERT OR IGNORE INTO edges (src_source, src, dst_source, dst, kind)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![src.source.as_str(), src.number as i64,
                issue.number.source.as_str(), issue.number.number as i64, kind.name()],
        )?;
    }
    Ok(())
}

/// The detail payloads a store keeps per item number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Issue body, comments and review threads, with project statuses.
    Issue,
    /// Changed files of a PR.
    Diff,
}

impl Detail {
    fn name(self) -> &'static str {
        match self {
            Detail::Issue => "issue",
            Detail::Diff => "diff",
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
    pub fn item_updated_at(&self, number: ItemRef) -> Option<String> {
        self.conn
            .query_row(
                "SELECT updated_at FROM items WHERE source = ?1 AND number = ?2",
                params![number.source.as_str(), number.number as i64],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Project statuses of the item as the latest sync listed them.
    pub fn item_statuses(&self, number: ItemRef) -> Option<Vec<(String, Option<String>)>> {
        let json: String = self
            .conn
            .query_row(
                "SELECT project_status FROM items WHERE source = ?1 AND number = ?2",
                params![number.source.as_str(), number.number as i64],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()?;
        serde_json::from_str(&json).ok()
    }

    /// The stored detail when it is fresh for an open (`serve_stored`).
    pub fn fresh_detail<T: serde::de::DeserializeOwned>(
        &self,
        kind: Detail,
        number: ItemRef,
        force: bool,
    ) -> Option<T> {
        let (stored_at, payload) = self.detail_payload(kind, number)?;
        let current = self.item_updated_at(number);
        if !serve_stored(force, Some(&stored_at), current.as_deref()) {
            return None;
        }
        // A payload that no longer parses is a miss; the fetch rewrites it.
        serde_json::from_str(&payload).ok()
    }

    /// The stored detail whatever its age, for views that only show it.
    pub fn stored_detail<T>(&self, kind: Detail, number: ItemRef) -> Option<T>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_str(&self.detail_payload(kind, number)?.1).ok()
    }

    /// `updated_at` and raw payload of a stored detail.
    fn detail_payload(&self, kind: Detail, number: ItemRef) -> Option<(String, String)> {
        self.conn
            .query_row(
                "SELECT updated_at, payload FROM details
                 WHERE source = ?1 AND number = ?2 AND kind = ?3",
                params![number.source.as_str(), number.number as i64, kind.name()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Store a detail with the `updated_at` it was fetched at.
    pub fn put_detail<T: serde::Serialize>(
        &self,
        kind: Detail,
        number: ItemRef,
        updated_at: &str,
        value: &T,
    ) -> Result<(), String> {
        let payload = serde_json::to_string(value).map_err(|e| format!("store: {e}"))?;
        self.conn
            .execute(
                "INSERT INTO details (source, number, kind, updated_at, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (source, number, kind) DO UPDATE
                 SET updated_at = excluded.updated_at, payload = excluded.payload",
                params![number.source.as_str(), number.number as i64, kind.name(), updated_at, payload],
            )
            .map(|_| ())
            .map_err(|e| format!("store: {e}"))
    }

    /// Drop an item and its details: it was deleted or transferred.
    pub fn drop_item(&mut self, number: ItemRef) -> Result<(), String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        tx.execute(
            "DELETE FROM items WHERE source = ?1 AND number = ?2",
            params![number.source.as_str(), number.number as i64],
        ).map_err(fail)?;
        tx.execute(
            "DELETE FROM details WHERE source = ?1 AND number = ?2",
            params![number.source.as_str(), number.number as i64],
        ).map_err(fail)?;
        tx.execute(
            "DELETE FROM edges WHERE dst_source = ?1 AND dst = ?2",
            params![number.source.as_str(), number.number as i64],
        ).map_err(fail)?;
        bump_generation(&tx).map_err(fail)?;
        tx.commit().map_err(fail)
    }
}

/// Sync lease and change watch. Every ghit process of a repo shares one
/// sync stream: the lease holder alone fetches listings, and every
/// process watches the file for the holder's writes.
impl Store {
    /// Start of the last sync, or `None` before the first one.
    pub fn sync_started_at(&self) -> Option<String> {
        self.conn
            .query_row("SELECT started_at FROM sync_state WHERE id = 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    /// A stamp that moves when either source finishes a sync; `None`
    /// before the first sync of both.
    pub fn sync_stamp(&self) -> Option<String> {
        let (github, jira) = (self.sync_started_at(), self.jira_last_sync());
        if github.is_none() && jira.is_none() {
            return None;
        }
        Some(format!("{}|{}", github.unwrap_or_default(), jira.unwrap_or_default()))
    }

    /// Take or renew the lease for `holder` until `now + ttl` (unix
    /// seconds). It succeeds when `holder` already holds it or the last
    /// claim expired; `force` takes it from a live holder too. True when
    /// `holder` holds the lease afterwards.
    pub fn claim_lease(&self, holder: &str, now: i64, ttl: i64, force: bool) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "UPDATE sync_state SET holder = ?1, lease_expires = ?2
                 WHERE id = 1 AND (?4 OR holder = ?1 OR lease_expires <= ?3)",
                params![holder, now + ttl, now, force],
            )
            .map_err(|e| format!("store: {e}"))?;
        Ok(changed == 1)
    }

    /// Give the lease up at once, so another process takes it on its next
    /// try instead of waiting out the expiry. A no-op unless `holder`
    /// holds it.
    pub fn release_lease(&self, holder: &str) -> Result<(), String> {
        self.conn
            .execute("UPDATE sync_state SET lease_expires = 0 WHERE id = 1 AND holder = ?1", [holder])
            .map(|_| ())
            .map_err(|e| format!("store: {e}"))
    }

    /// SQLite's change counter for this connection: it moves whenever
    /// another connection (any process) commits to the file.
    pub fn data_version(&self) -> Result<i64, String> {
        self.conn.query_row("PRAGMA data_version", [], |r| r.get(0)).map_err(|e| format!("store: {e}"))
    }

    /// Count of writes to item rows and edges, any connection. Lease
    /// renewals and detail writes leave it alone; search hits move it.
    pub fn items_generation(&self) -> Result<i64, String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = 'items_gen'", [], |r| r.get::<_, String>(0))
            .optional()
            .map(|v| v.and_then(|v| v.parse().ok()).unwrap_or(0))
            .map_err(|e| format!("store: {e}"))
    }

    /// Stamp of what a map card shows for one item: the row's and the
    /// stored issue detail's `updated_at`. It moves when either is rewritten.
    pub fn card_stamp(&self, number: ItemRef) -> Option<String> {
        let detail: Option<String> = self
            .conn
            .query_row(
                "SELECT updated_at FROM details
                 WHERE source = ?1 AND number = ?2 AND kind = 'issue'",
                params![number.source.as_str(), number.number as i64],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        Some(format!("{}|{}", self.item_updated_at(number)?, detail.unwrap_or_default()))
    }

    /// How long this connection waits for another process's lock.
    pub fn set_busy_timeout(&self, timeout: Duration) -> Result<(), String> {
        self.conn.busy_timeout(timeout).map_err(|e| format!("store: {e}"))
    }
}

/// Backfill upsert: a row the sync wrote at the same or a later
/// `updated_at` stays as it is. Same-time rows keep the sync's version
/// because a status refresh changes statuses without moving `updated_at`.
fn upsert_newer_sql() -> String {
    format!("{} WHERE excluded.updated_at > items.updated_at", UPSERT.trim_end())
}

/// Resume state of one backfill connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillConn {
    /// `endCursor` of the last stored page; `None` before the first page.
    pub cursor: Option<String>,
    pub done: bool,
    /// Rows stored so far.
    pub fetched: u64,
    /// The connection's `totalCount` at the last page (an upper bound
    /// when an age limit stops the walk early).
    pub total: u64,
}

/// Backfill progress, kept in the store so any process resumes it.
impl Store {
    /// State of the named connection; the default before its first page.
    pub fn backfill_conn(&self, name: &str) -> BackfillConn {
        self.conn
            .query_row("SELECT cursor, done, fetched, total FROM backfill WHERE name = ?1", [name], |r| {
                Ok(BackfillConn {
                    cursor: r.get(0)?,
                    done: r.get::<_, i64>(1)? != 0,
                    fetched: r.get::<_, i64>(2)? as u64,
                    total: r.get::<_, i64>(3)? as u64,
                })
            })
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// One transaction: store a fetched page and move the connection to
    /// `next`. Rows only replace a stored row with an older `updated_at`;
    /// `sync_state` stays untouched, so other processes see no sync.
    /// False (nothing stored) when the stored cursor is no longer
    /// `expected`: another process advanced the walk first.
    pub fn backfill_save(
        &mut self,
        name: &str,
        expected: Option<&str>,
        rows: &[Row],
        next: Option<&str>,
        done: bool,
        total: u64,
    ) -> Result<bool, String> {
        let fail = |e: rusqlite::Error| format!("store: {e}");
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(fail)?;
        let stored = {
            let mut stmt = tx.prepare("SELECT cursor, fetched FROM backfill WHERE name = ?1").map_err(fail)?;
            stmt.query_row([name], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)))
                .optional()
                .map_err(fail)?
        };
        let (cursor, fetched) = stored.unwrap_or((None, 0));
        if cursor.as_deref() != expected {
            return Ok(false);
        }
        upsert_rows(&tx, &upsert_newer_sql(), rows)?;
        tx.execute(
            "INSERT INTO backfill (name, cursor, done, fetched, total) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO UPDATE SET cursor = excluded.cursor, done = excluded.done,
                 fetched = excluded.fetched, total = excluded.total",
            params![name, next, done, fetched + rows.len() as i64, total as i64],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)?;
        Ok(true)
    }
}

/// `closed_at` stays NULL: the listing fields carry no `closedAt`.
const UPSERT: &str = "
INSERT INTO items (source, number, kind, state, title, author, labels, assignees, milestone,
    created_at, updated_at, merged_at, checks, project_status, window_at, row_json)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
ON CONFLICT (source, number) DO UPDATE SET
    kind = excluded.kind, state = excluded.state, title = excluded.title,
    author = excluded.author, labels = excluded.labels, assignees = excluded.assignees,
    milestone = excluded.milestone, created_at = excluded.created_at,
    updated_at = excluded.updated_at, merged_at = excluded.merged_at,
    checks = excluded.checks, project_status = excluded.project_status,
    window_at = excluded.window_at, row_json = excluded.row_json
";

#[cfg(test)]
mod demo_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn row(number: ItemRef, state: &str) -> Row {
        Row {
            issue: crate::model::IssueData { number, state: state.to_string(), ..Default::default() },
            statuses: vec![],
            detail: crate::model::BoardDetail::default(),
            complete: false,
            window_at: None,
        }
    }

    fn numbers(store: &Store) -> Vec<ItemRef> {
        store.rows().unwrap().iter().map(|r| r.issue.number).collect()
    }

    #[test]
    fn item_columns_store_source_tags_and_sort_numbers_numerically() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("numbers.sqlite")).unwrap();
        let github = [ItemRef::github(2), ItemRef::github(10)];
        let jira = github.map(|item| ItemRef { source: Source::Jira, ..item });
        let rows: Vec<_> = github.into_iter().chain(jira).map(|item| row(item, "OPEN")).collect();
        store.write("sync", &rows).unwrap();
        let mut stmt = store.conn.prepare(
            "SELECT source, typeof(number) FROM items ORDER BY source, number",
        ).unwrap();
        let columns: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(columns, vec![
            ("github".into(), "integer".into()), ("github".into(), "integer".into()),
            ("jira".into(), "integer".into()), ("jira".into(), "integer".into()),
        ]);
        assert_eq!(numbers(&store), vec![github[1], github[0], jira[1], jira[0]]);
        let map: Vec<_> = store.map_items().unwrap().iter().map(|item| item.number).collect();
        assert_eq!(map, vec![github[0], github[1], jira[0], jira[1]]);
        let finder: Vec<_> = store.finder_candidates().unwrap().iter()
            .map(|item| item.number).collect();
        assert_eq!(finder, numbers(&store));
    }

    #[test]
    fn sources_keep_rows_details_edges_and_lookup_results_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sources.sqlite");
        let mut store = open_at(&path).unwrap();
        let github = ItemRef::github(45);
        let jira = ItemRef {
            source: crate::model::Source::Jira,
            number: 45,
        };
        let mut gh_row = row(github, "OPEN");
        gh_row.issue.title = "GitHub".into();
        let mut jira_row = row(jira, "OPEN");
        jira_row.issue.title = "Jira".into();
        jira_row.issue.links.blocked_by = vec![github];
        store.write("sync", &[gh_row, jira_row]).unwrap();
        store
            .put_detail(Detail::Issue, github, "", &"github body")
            .unwrap();
        store
            .put_detail(Detail::Issue, jira, "", &"jira body")
            .unwrap();
        drop(store);

        let mut store = open_at(&path).unwrap();
        assert_eq!(store.row(github).unwrap().issue.title, "GitHub");
        assert_eq!(store.row(jira).unwrap().issue.title, "Jira");
        assert_eq!(
            store
                .stored_detail::<String>(Detail::Issue, github)
                .unwrap(),
            "github body"
        );
        assert_eq!(
            store.stored_detail::<String>(Detail::Issue, jira).unwrap(),
            "jira body"
        );
        assert_eq!(store.finder_candidates().unwrap().len(), 2);
        assert_eq!(store.map_items().unwrap().len(), 2);
        assert_eq!(store.rows_by_number(&[jira]).len(), 1);
        assert_eq!(
            store.edges().unwrap(),
            vec![Edge {
                src: github,
                dst: jira,
                kind: EdgeKind::Blocks
            }]
        );
        store.drop_item(jira).unwrap();
        assert!(store.row(github).is_some());
        assert!(store.row(jira).is_none());
        assert!(store.edges().unwrap().is_empty());
        assert!(store
            .stored_detail::<String>(Detail::Issue, github)
            .is_some());
        assert!(store.stored_detail::<String>(Detail::Issue, jira).is_none());
    }

    #[test]
    fn pr_jira_refs_write_closes_edges_that_survive_jira_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("jira.sqlite")).unwrap();
        let jira = ItemRef { source: Source::Jira, number: 123 };
        let mut pr = row(ItemRef::github(9), "MERGED");
        pr.issue.is_pr = true;
        pr.issue.jira_refs = vec![jira];
        store.write("g1", &[pr.clone(), row(jira, "OPEN")]).unwrap();
        let edge = Edge { src: ItemRef::github(9), dst: jira, kind: EdgeKind::Closes };
        assert_eq!(store.edges().unwrap(), vec![edge]);
        assert_eq!(
            store.linked_prs(jira),
            vec![PairedRef { number: ItemRef::github(9), state: "MERGED".to_string() }]
        );
        // A Jira sync landing its own row leaves the PR-owned edge alone.
        store.write_jira("j1", &[row(jira, "OPEN")]).unwrap();
        assert_eq!(store.edges().unwrap(), vec![edge]);
        // A PR row that no longer names the key replaces its edges.
        pr.issue.jira_refs = vec![];
        store.write("g2", &[pr]).unwrap();
        assert!(store.edges().unwrap().is_empty());
        assert!(store.linked_prs(jira).is_empty());
    }

    #[test]
    fn finder_candidates_list_every_item_newest_update_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("s.sqlite")).unwrap();
        let mut a = row(ItemRef::github(1), "OPEN");
        a.issue.title = "old".into();
        a.issue.updated_at = "2026-01-01T00:00:00Z".into();
        a.issue.labels = vec![crate::model::Label { name: "bug".into() }];
        let mut b = row(ItemRef::github(2), "MERGED");
        b.issue.is_pr = true;
        b.issue.updated_at = "2026-02-01T00:00:00Z".into();
        store.write("2026-02-02T00:00:00Z", &[a, b]).unwrap();
        let got = store.finder_candidates().unwrap();
        assert_eq!(
            got.iter().map(|c| c.number).collect::<Vec<_>>(),
            vec![ItemRef::github(2), ItemRef::github(1)]
        );
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
    fn writes_round_trip_rows_and_sync_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert!(
            store.sync_started_at().is_none() && numbers(&store).is_empty(),
            "a new store is empty"
        );
        store
            .write(
                "t1",
                &[
                    row(ItemRef::github(5), "OPEN"),
                    row(ItemRef::github(2), "OPEN"),
                    row(ItemRef::github(9), "CLOSED"),
                ],
            )
            .unwrap();
        assert_eq!(
            numbers(&store),
            vec![ItemRef::github(9), ItemRef::github(5), ItemRef::github(2)]
        );
        assert_eq!(store.sync_started_at().as_deref(), Some("t1"));
        // A later write upserts; rows it leaves out keep their data.
        let mut retitled = row(ItemRef::github(2), "OPEN");
        retitled.issue.title = "renamed".to_string();
        store.write("t2", &[retitled]).unwrap();
        let rows = store.rows().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .find(|r| r.issue.number == ItemRef::github(2))
                .unwrap()
                .issue
                .title,
            "renamed"
        );
        assert_eq!(store.sync_started_at().as_deref(), Some("t2"));
        let wal: String = store.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(wal, "wal");
    }

    #[test]
    fn jira_syncs_keep_their_own_time_and_leave_github_rows_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert!(store.sync_stamp().is_none() && store.jira_last_sync().is_none());
        let jira = |number| ItemRef { source: Source::Jira, number };
        store.write("g1", &[row(ItemRef::github(1), "OPEN")]).unwrap();
        let after_github = store.sync_stamp();
        store.write_jira("j1", &[row(jira(1), "OPEN"), row(jira(2), "CLOSED")]).unwrap();
        assert_eq!(store.jira_last_sync().as_deref(), Some("j1"));
        assert_eq!(store.sync_started_at().as_deref(), Some("g1"), "GitHub sync time stays");
        assert_ne!(store.sync_stamp(), after_github, "a Jira sync moves the stamp");
        assert_eq!(numbers(&store).len(), 3, "both sources share the file");
        store.write("g2", &[row(ItemRef::github(1), "CLOSED")]).unwrap();
        assert_eq!(store.jira_last_sync().as_deref(), Some("j1"), "a GitHub sync leaves Jira time");
        assert!(store.row(jira(1)).is_some() && store.row(ItemRef::github(1)).is_some());
        store.write_jira("j2", &[]).unwrap();
        assert_eq!(store.jira_last_sync().as_deref(), Some("j2"));
    }

    #[test]
    fn two_connections_see_each_others_writes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let mut first = open_at(&file).unwrap();
        let mut second = open_at(&file).unwrap();
        first
            .write("t1", &[row(ItemRef::github(1), "OPEN")])
            .unwrap();
        assert_eq!(numbers(&second), vec![ItemRef::github(1)]);
        second
            .write("t2", &[row(ItemRef::github(2), "OPEN")])
            .unwrap();
        assert_eq!(
            numbers(&first),
            vec![ItemRef::github(2), ItemRef::github(1)]
        );
        assert_eq!(first.sync_started_at().as_deref(), Some("t2"));
    }

    #[test]
    fn lease_has_one_holder_until_it_expires_or_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let (a, b) = (open_at(&file).unwrap(), open_at(&file).unwrap());
        assert!(a.claim_lease("a", 100, 60, false).unwrap(), "a free lease is taken");
        assert!(!b.claim_lease("b", 101, 60, false).unwrap(), "a live lease refuses others");
        assert!(a.claim_lease("a", 130, 60, false).unwrap(), "the holder renews");
        assert!(!b.claim_lease("b", 189, 60, false).unwrap(), "renewal pushed the expiry to 190");
        assert!(b.claim_lease("b", 190, 60, false).unwrap(), "an expired lease hands over");
        assert!(!a.claim_lease("a", 191, 60, false).unwrap(), "the old holder lost it");
        a.release_lease("a").unwrap();
        assert!(!a.claim_lease("a", 192, 60, false).unwrap(), "only the holder can release");
        b.release_lease("b").unwrap();
        assert!(a.claim_lease("a", 192, 60, false).unwrap(), "a released lease is free at once");
        assert!(b.claim_lease("b", 193, 60, true).unwrap(), "force takes a live lease");
        assert!(!a.claim_lease("a", 194, 60, false).unwrap());
    }

    #[test]
    fn items_generation_counts_row_edge_and_search_writes_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert_eq!(store.items_generation().unwrap(), 0);
        store
            .write("t1", &[row(ItemRef::github(1), "OPEN")])
            .unwrap();
        assert_eq!(store.items_generation().unwrap(), 1, "a sync write counts");
        store.claim_lease("a", 100, 60, false).unwrap();
        store.put_detail(Detail::Issue, ItemRef::github(1), "u", &"body").unwrap();
        assert_eq!(store.items_generation().unwrap(), 1, "lease and detail writes do not");
        store.upsert_search(&[row(ItemRef::github(1), "OPEN")]).unwrap();
        assert_eq!(store.items_generation().unwrap(), 1, "a search hit that changes no row does not");
        let mut newer = row(ItemRef::github(2), "OPEN");
        newer.issue.updated_at = "2".into();
        store.upsert_search(&[newer]).unwrap();
        assert_eq!(store.items_generation().unwrap(), 2, "a stored search hit counts");
        store.drop_item(ItemRef::github(2)).unwrap();
        assert_eq!(store.items_generation().unwrap(), 3, "a drop counts");
    }

    #[test]
    fn card_stamp_moves_with_the_items_row_and_detail() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("r.sqlite")).unwrap();
        assert_eq!(store.card_stamp(ItemRef::github(1)), None);
        store
            .write("t1", &[row(ItemRef::github(1), "OPEN")])
            .unwrap();
        let bare = store.card_stamp(ItemRef::github(1));
        store
            .put_detail(Detail::Issue, ItemRef::github(1), "u", &"body")
            .unwrap();
        assert_ne!(
            store.card_stamp(ItemRef::github(1)),
            bare,
            "a stored detail moves it"
        );
        let other = store.card_stamp(ItemRef::github(1));
        store
            .write("t2", &[row(ItemRef::github(2), "OPEN")])
            .unwrap();
        assert_eq!(
            store.card_stamp(ItemRef::github(1)),
            other,
            "another item's write does not"
        );
    }

    #[test]
    fn data_version_moves_on_other_connections_commits_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let mut writer = open_at(&file).unwrap();
        let watcher = open_at(&file).unwrap();
        let (seen, own) = (watcher.data_version().unwrap(), writer.data_version().unwrap());
        writer.write("t1", &[row(ItemRef::github(1), "OPEN")]).unwrap();
        assert_ne!(watcher.data_version().unwrap(), seen, "another connection's commit moves it");
        assert_eq!(writer.data_version().unwrap(), own, "a connection's own commit does not");
    }

    #[test]
    fn schema_version_mismatch_rebuilds_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("r.sqlite");
        let mut store = open_at(&file).unwrap();
        store.write("t1", &[row(ItemRef::github(1), "OPEN")]).unwrap();
        store.conn.execute("UPDATE meta SET value = '0' WHERE key = 'schema_version'", []).unwrap();
        store.conn.execute_batch("CREATE TABLE leftover (x)").unwrap();
        drop(store);
        let store = open_at(&file).unwrap();
        assert!(store.sync_started_at().is_none() && numbers(&store).is_empty(), "mismatch drops the old copy");
        let leftover: i64 = store
            .conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'leftover'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover, 0, "every old table goes");
        assert_eq!(schema_version(&store.conn).as_deref(), Some(SCHEMA_VERSION));
        assert!(store.claim_lease("a", 1, 60, false).unwrap(), "the rebuilt lease row is free");
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
        let mut listed = row(ItemRef::github(5), "OPEN");
        listed.issue.updated_at = "t1".to_string();
        listed.statuses = vec![("Board".to_string(), Some("Todo".to_string()))];
        store.write("s", &[listed]).unwrap();
        assert_eq!(
            store.item_updated_at(ItemRef::github(5)).as_deref(),
            Some("t1")
        );
        assert_eq!(
            store.item_statuses(ItemRef::github(5)),
            Some(vec![("Board".to_string(), Some("Todo".to_string()))])
        );
        assert_eq!(store.item_updated_at(ItemRef::github(6)), None);

        let issue = crate::model::IssueData {
            number: ItemRef::github(5),
            body: "b".to_string(),
            updated_at: "t1".to_string(),
            ..Default::default()
        };
        let files = vec![crate::model::DiffFile {
            filename: "a.rs".to_string(),
            ..Default::default()
        }];
        store
            .put_detail(Detail::Issue, ItemRef::github(5), "t1", &issue)
            .unwrap();
        store
            .put_detail(Detail::Diff, ItemRef::github(5), "t1", &files)
            .unwrap();
        assert_eq!(
            store
                .fresh_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5), false)
                .unwrap()
                .body,
            "b"
        );
        assert_eq!(
            store
                .fresh_detail::<Vec<crate::model::DiffFile>>(
                    Detail::Diff,
                    ItemRef::github(5),
                    false
                )
                .unwrap()[0]
                .filename,
            "a.rs"
        );
        assert!(
            store
                .fresh_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5), true)
                .is_none(),
            "force skips the store"
        );

        // A sync that reports a newer updated_at makes every detail stale.
        let mut newer = row(ItemRef::github(5), "OPEN");
        newer.issue.updated_at = "t2".to_string();
        store.write("s2", &[newer]).unwrap();
        assert!(store
            .fresh_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5), false)
            .is_none());
        assert_eq!(
            store
                .stored_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5))
                .unwrap()
                .body,
            "b",
            "a view still reads it"
        );

        // A refetch replaces the stored copy.
        let fixed = crate::model::IssueData {
            body: "new".to_string(),
            updated_at: "t2".to_string(),
            ..issue.clone()
        };
        store
            .put_detail(Detail::Issue, ItemRef::github(5), "t2", &fixed)
            .unwrap();
        assert_eq!(
            store
                .fresh_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5), false)
                .unwrap()
                .body,
            "new"
        );

        // A 404 drops the item and its details.
        store.drop_item(ItemRef::github(5)).unwrap();
        assert!(numbers(&store).is_empty() && store.item_updated_at(ItemRef::github(5)).is_none());
        assert!(store
            .fresh_detail::<crate::model::IssueData>(Detail::Issue, ItemRef::github(5), false)
            .is_none());
        let left: i64 = store
            .conn
            .query_row("SELECT count(*) FROM details", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    fn dated(number: ItemRef, title: &str, updated: &str) -> Row {
        let mut r = row(number, "CLOSED");
        r.issue.title = title.into();
        r.issue.updated_at = updated.into();
        r
    }

    #[test]
    fn backfill_never_clobbers_a_newer_or_equal_synced_row() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("s.sqlite")).unwrap();
        store
            .write(
                "s1",
                &[
                    dated(ItemRef::github(1), "synced", "2026-05-01T00:00:00Z"),
                    dated(ItemRef::github(2), "synced", "2026-05-01T00:00:00Z"),
                ],
            )
            .unwrap();
        let page = [
            dated(ItemRef::github(1), "backfill", "2026-01-01T00:00:00Z"),
            dated(ItemRef::github(2), "backfill", "2026-05-01T00:00:00Z"),
            dated(ItemRef::github(3), "backfill", "2026-01-01T00:00:00Z"),
        ];
        assert!(store
            .backfill_save("issues_closed", None, &page, Some("c1"), false, 9)
            .unwrap());
        let titles: Vec<_> = store
            .rows()
            .unwrap()
            .iter()
            .map(|r| (r.issue.number, r.issue.title.clone()))
            .collect();
        assert_eq!(
            titles,
            vec![
                (ItemRef::github(3), "backfill".to_string()),
                (ItemRef::github(2), "synced".to_string()),
                (ItemRef::github(1), "synced".to_string())
            ]
        );
        // A backfilled row is replaced by a newer backfill page.
        assert!(store
            .backfill_save(
                "issues_closed",
                Some("c1"),
                &[dated(ItemRef::github(3), "newer", "2026-02-01T00:00:00Z")],
                None,
                true,
                9
            )
            .unwrap());
        assert_eq!(store.rows().unwrap()[0].issue.title, "newer");
        assert_eq!(store.sync_started_at().as_deref(), Some("s1"), "backfill leaves the sync start alone");
    }

    #[test]
    fn backfill_resumes_from_the_stored_cursor_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sqlite");
        let mut store = open_at(&path).unwrap();
        assert_eq!(store.backfill_conn("prs_open"), BackfillConn::default());
        assert!(store
            .backfill_save(
                "prs_open",
                None,
                &[dated(ItemRef::github(1), "a", "2026-01-01T00:00:00Z")],
                Some("c1"),
                false,
                5
            )
            .unwrap());
        drop(store);
        // Another process (a new connection) continues from the cursor.
        let mut store = open_at(&path).unwrap();
        let state = store.backfill_conn("prs_open");
        assert_eq!(
            (
                state.cursor.as_deref(),
                state.done,
                state.fetched,
                state.total
            ),
            (Some("c1"), false, 1, 5)
        );
        assert!(
            !store
                .backfill_save(
                    "prs_open",
                    None,
                    &[dated(ItemRef::github(2), "b", "2026-01-01T00:00:00Z")],
                    Some("c2"),
                    false,
                    5
                )
                .unwrap(),
            "a stale cursor stores nothing"
        );
        assert_eq!(numbers(&store), vec![ItemRef::github(1)]);
        assert!(store
            .backfill_save(
                "prs_open",
                Some("c1"),
                &[dated(ItemRef::github(2), "b", "2026-01-01T00:00:00Z")],
                None,
                true,
                5
            )
            .unwrap());
        let state = store.backfill_conn("prs_open");
        assert_eq!((state.cursor, state.done, state.fetched), (None, true, 2));
    }

    #[test]
    fn schema_rebuild_restarts_the_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sqlite");
        let mut store = open_at(&path).unwrap();
        store.backfill_save("prs_open", None, &[], None, true, 0).unwrap();
        store.conn.execute("UPDATE meta SET value = 'old' WHERE key = 'schema_version'", []).unwrap();
        drop(store);
        let store = open_at(&path).unwrap();
        assert!(!store.backfill_conn("prs_open").done);
    }
    fn linked(
        number: ItemRef,
        parent: Option<ItemRef>,
        blocked_by: &[ItemRef],
        closed_by: &[ItemRef],
    ) -> Row {
        let mut r = row(number, "OPEN");
        r.issue.parent = parent;
        r.issue.links = crate::model::IssueLinks { blocked_by: blocked_by.to_vec(), closed_by: closed_by.to_vec() };
        r
    }

    fn edge(src: ItemRef, dst: ItemRef, kind: EdgeKind) -> Edge {
        Edge { src, dst, kind }
    }

    #[test]
    fn edges_come_from_the_parsed_issue_row() {
        let node = serde_json::json!({
            "number": 7, "title": "t", "state": "OPEN", "createdAt": "c", "updatedAt": "u",
            "parent": {"number": 3},
            "blockedBy": {"nodes": [{"number": 4}]},
            "closedByPullRequestsReferences": {"nodes": [{"number": 9}]}
        });
        let parsed = crate::api::parse_node(&node, false, None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("e.sqlite")).unwrap();
        store.write("s1", &[parsed]).unwrap();
        assert_eq!(
            store.edges().unwrap(),
            [
                edge(ItemRef::github(3), ItemRef::github(7), EdgeKind::Parent),
                edge(ItemRef::github(4), ItemRef::github(7), EdgeKind::Blocks),
                edge(ItemRef::github(9), ItemRef::github(7), EdgeKind::Closes)
            ]
        );
    }

    #[test]
    fn rewriting_an_issue_replaces_its_edges() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("e.sqlite")).unwrap();
        let mut first = linked(
            ItemRef::github(7),
            None,
            &[ItemRef::github(4), ItemRef::github(5)],
            &[],
        );
        first.issue.updated_at = "t1".into();
        store
            .write(
                "s1",
                &[
                    first,
                    linked(ItemRef::github(8), None, &[ItemRef::github(4)], &[]),
                ],
            )
            .unwrap();
        let mut second = linked(ItemRef::github(7), None, &[ItemRef::github(5)], &[]);
        second.issue.updated_at = "t2".into();
        store.write("s2", &[second]).unwrap();
        assert_eq!(
            store.edges().unwrap(),
            [
                edge(ItemRef::github(5), ItemRef::github(7), EdgeKind::Blocks),
                edge(ItemRef::github(4), ItemRef::github(8), EdgeKind::Blocks)
            ]
        );

        // A backfill page that is not newer leaves row and edges alone.
        let mut stale = linked(ItemRef::github(7), None, &[], &[]);
        stale.issue.updated_at = "t1".into();
        store.backfill_save("c", None, &[stale], None, true, 1).unwrap();
        assert_eq!(store.edges().unwrap().len(), 2);

        // A newer backfill page replaces them.
        let mut newer = linked(ItemRef::github(7), Some(ItemRef::github(1)), &[], &[]);
        newer.issue.updated_at = "t3".into();
        store
            .backfill_save("d", None, &[newer], None, true, 1)
            .unwrap();
        assert_eq!(
            store.edges().unwrap(),
            [
                edge(ItemRef::github(1), ItemRef::github(7), EdgeKind::Parent),
                edge(ItemRef::github(4), ItemRef::github(8), EdgeKind::Blocks)
            ]
        );

        store.drop_item(ItemRef::github(8)).unwrap();
        assert_eq!(
            store.edges().unwrap(),
            [edge(
                ItemRef::github(1),
                ItemRef::github(7),
                EdgeKind::Parent
            )]
        );
    }

    #[test]
    fn map_items_carry_kind_state_title_milestone_and_labels() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_at(&dir.path().join("m.sqlite")).unwrap();
        let mut pr = row(ItemRef::github(4), "MERGED");
        pr.issue.is_pr = true;
        pr.issue.milestone = Some("v1".to_string());
        pr.issue.labels = vec![crate::model::Label { name: "bug".into() }, crate::model::Label { name: "perf".into() }];
        store.write("s1", &[row(ItemRef::github(3), "OPEN"), pr]).unwrap();
        let mut items = store.map_items().unwrap();
        items.sort_by_key(|item| item.number);
        assert_eq!(items.len(), 2);
        assert_eq!(
            (
                items[0].number,
                items[0].is_pr,
                &*items[0].state,
                items[0].milestone.as_deref()
            ),
            (ItemRef::github(3), false, "OPEN", None)
        );
        assert_eq!(
            (
                items[1].is_pr,
                &*items[1].state,
                items[1].milestone.as_deref()
            ),
            (true, "MERGED", Some("v1"))
        );
        assert_eq!(
            (items[0].labels.len(), items[1].labels.clone()),
            (0, vec!["bug".to_string(), "perf".to_string()])
        );
    }

    #[test]
    fn schema_bump_drops_stored_edges() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("e.sqlite");
        let mut store = open_at(&file).unwrap();
        store.write("s1", &[linked(ItemRef::github(2), None, &[ItemRef::github(1)], &[])]).unwrap();
        store.conn.execute("UPDATE meta SET value = '0' WHERE key = 'schema_version'", []).unwrap();
        drop(store);
        let store = open_at(&file).unwrap();
        assert!(store.edges().unwrap().is_empty());
    }
}
