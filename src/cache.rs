use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const CACHE_TTL_SECS: u64 = 1800; // 30 minutes

fn base_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("ghit")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local").join("share").join("ghit")
    } else {
        PathBuf::from(".ghit-data")
    }
}

fn cache_path(name: &str) -> PathBuf {
    base_dir().join("cache").join(format!("{name}.json"))
}

pub fn cache_get<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    let raw = std::fs::read_to_string(cache_path(name)).ok()?;
    let payload: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let fetched = payload.get("fetched_at")?.as_u64()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(fetched) >= CACHE_TTL_SECS {
        return None;
    }
    serde_json::from_value(payload.get("data")?.clone()).ok()
}

pub fn cache_put<T: serde::Serialize>(name: &str, data: &T) {
    let p = cache_path(name);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let payload = serde_json::json!({"fetched_at": now, "data": data});
    let _ = std::fs::write(p, serde_json::to_string(&payload).unwrap_or_default());
}

fn host() -> String {
    std::env::var("GH_HOST")
        .unwrap_or_else(|_| "github.com".to_owned())
        .to_lowercase()
        .replace('/', "__")
}

fn slug(repo: &str) -> String {
    repo.to_lowercase().replace('/', "__")
}

/// Prefs key for a repo-scoped query: host plus lowercased slug, so a
/// GitHub Enterprise host never shares Format prefs with github.com.
pub fn prefs_repo_key(repo: &str) -> String {
    format!("{}/{}", host(), repo.to_lowercase())
}

/// Prefs key for a board-owner filter: host plus lowercased owner, for
/// the same cross-host reason as `prefs_repo_key`.
pub fn prefs_owner_key(owner: &str) -> String {
    format!("{}/{}", host(), owner.to_lowercase())
}

/// Typed cache key. Every variant includes the host so a GitHub
/// Enterprise repo never shares entries with a github.com repo of the
/// same slug.
pub enum Key {
    Issue { repo: String, number: u64 },
    Status { repo: String, number: u64 },
    Comments { repo: String, number: u64 },
    Diff { repo: String, number: u64 },
    Graph { repo: String, number: u64 },
    Sections { repo: String, hours: u64 },
    Board { owner: String },
    Labels { repo: String },
    Milestones { repo: String },
}

impl Key {
    pub fn name(&self) -> String {
        let h = host();
        match self {
            Key::Issue { repo, number } => format!("issue-{h}-{}-{number}", slug(repo)),
            Key::Status { repo, number } => format!("status-{h}-{}-{number}", slug(repo)),
            Key::Comments { repo, number } => format!("comments-{h}-{}-{number}", slug(repo)),
            Key::Diff { repo, number } => format!("diff-{h}-{}-{number}", slug(repo)),
            Key::Graph { repo, number } => format!("graph-{h}-{}-{number}", slug(repo)),
            Key::Sections { repo, hours } => format!("sections-{h}-{}-{hours}", slug(repo)),
            Key::Board { owner } => format!("board-{h}-{}", owner.to_lowercase()),
            Key::Labels { repo } => format!("labels-{h}-{}", slug(repo)),
            Key::Milestones { repo } => format!("milestones-{h}-{}", slug(repo)),
        }
    }
}

pub fn cache_drop(name: &str) {
    let _ = std::fs::remove_file(cache_path(name));
}

fn pin_path(repo: &str) -> PathBuf {
    base_dir().join(format!("pinned-{}.txt", repo.replace('/', "__")))
}

fn theme_path() -> PathBuf {
    base_dir().join("theme")
}

/// Last-selected theme name, plain text. `None` means never chosen.
pub fn load_theme() -> Option<String> {
    let raw = std::fs::read_to_string(theme_path()).ok()?;
    let name = raw.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

pub fn save_theme(name: &str) {
    let d = base_dir();
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let _ = std::fs::write(theme_path(), format!("{name}\n"));
}

fn prefs_path() -> PathBuf {
    base_dir().join("prefs.json")
}

fn default_window() -> u64 {
    crate::core::DEFAULT_WINDOW_HOURS
}

fn default_true() -> bool {
    true
}

/// Board filter saved per board owner: the text row plus the selected
/// milestone pills. Milestones serialize as a sorted list; `None` is the
/// "(no milestone)" pill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedBoardFilter {
    #[serde(default)]
    pub filter: String,
    #[serde(default)]
    pub milestones: Vec<Option<String>>,
}

/// Sidebar filter saved per repo: the selected label and milestone
/// pills. `None` in `milestones` is the "(no milestone)" pill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedSidebarFilter {
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub milestones: Vec<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQuery {
    pub name: String,
    pub query: String,
}

/// Cross-session Format dialog state, one file for every repo and board.
/// Missing or unreadable files load as defaults; unknown fields are
/// ignored so newer files still parse. Every field carries a serde
/// default so older files missing new keys keep working.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_true")]
    pub show_right: bool,
    #[serde(default)]
    pub board_cards: crate::model::CardFields,
    #[serde(default)]
    pub board_filters: std::collections::HashMap<String, SavedBoardFilter>,
    #[serde(default)]
    pub row_issue: crate::model::RowFields,
    #[serde(default)]
    pub row_pr: crate::model::RowFields,
    #[serde(default = "default_true")]
    pub show_open_prs: bool,
    #[serde(default = "default_true")]
    pub show_merged_prs: bool,
    #[serde(default = "default_true")]
    pub show_open_issues: bool,
    #[serde(default = "default_true")]
    pub show_closed_issues: bool,
    #[serde(default = "default_window")]
    pub merged_window_hours: u64,
    #[serde(default)]
    pub queries: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub saved_queries: Vec<SavedQuery>,
    #[serde(default)]
    pub sidebar_filters: std::collections::HashMap<String, SavedSidebarFilter>,
    #[serde(default)]
    pub group_by: crate::model::GroupBy,
    #[serde(default)]
    pub sub_issue_tree: bool,
    #[serde(default)]
    pub sort: crate::core::SideSort,
    #[serde(default)]
    pub sort_desc: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { show_sidebar: true, show_right: true, board_cards: crate::model::CardFields::default(),
            board_filters: std::collections::HashMap::new(),
            row_issue: crate::model::RowFields::default(),
            row_pr: crate::model::RowFields::default(),
            show_open_prs: true, show_merged_prs: true,
            show_open_issues: true, show_closed_issues: true,
            merged_window_hours: default_window(),
            queries: std::collections::HashMap::new(),
            saved_queries: Vec::new(),
            sidebar_filters: std::collections::HashMap::new(),
            group_by: crate::model::GroupBy::None,
            sub_issue_tree: false,
            sort: crate::core::SideSort::Default,
            sort_desc: false }
    }
}

/// Load the Format dialog prefs. Garbage or a missing file means
/// defaults, never an error.
pub fn load_prefs() -> Prefs {
    std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_prefs(prefs: &Prefs) {
    let d = base_dir();
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let _ = std::fs::write(
        prefs_path(),
        serde_json::to_string(prefs).unwrap_or_default(),
    );
}

pub fn load_pins(repo: &str) -> Vec<u64> {
    std::fs::read_to_string(pin_path(repo))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect()
}

pub fn save_pins(repo: &str, numbers: &[u64]) {
    let d = base_dir();
    if std::fs::create_dir_all(&d).is_err() {
        return;
    }
    let body = numbers.iter().map(|n| n.to_string()).collect::<Vec<_>>().join("\n") + "\n";
    let _ = std::fs::write(pin_path(repo), body);
}

/// Serializes tests that mutate `XDG_DATA_HOME`. Shared with `theme`
/// tests so parallel runners never steal each other's data dir.
/// A panicking test poisons the mutex. Take the guard anyway: each test
/// resets the env vars it guards, so one failure does not cascade into
/// every later env test.
#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_queries_round_trip_and_default_for_old_prefs() {
        let old: Prefs = serde_json::from_str("{}").unwrap();
        assert!(old.saved_queries.is_empty());
        let mut prefs = old;
        prefs.saved_queries.push(SavedQuery { name: "bugs".into(), query: "label:bug".into() });
        let decoded: Prefs = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(decoded.saved_queries, prefs.saved_queries);
    }

    #[test]
    fn keys_include_host_and_differ_across_hosts() {
        let _guard = test_env_lock();
        let saved = std::env::var("GH_HOST").ok();
        let variants = |repo: &str| {
            [
                Key::Issue { repo: repo.to_string(), number: 7 }.name(),
                Key::Status { repo: repo.to_string(), number: 7 }.name(),
                Key::Comments { repo: repo.to_string(), number: 7 }.name(),
                Key::Diff { repo: repo.to_string(), number: 7 }.name(),
                Key::Graph { repo: repo.to_string(), number: 7 }.name(),
                Key::Sections { repo: repo.to_string(), hours: 24 }.name(),
                Key::Board { owner: "octo".to_string() }.name(),
                Key::Labels { repo: repo.to_string() }.name(),
                Key::Milestones { repo: repo.to_string() }.name(),
            ]
        };
        std::env::set_var("GH_HOST", "github.com");
        let dotcom = variants("o/r");
        for name in &dotcom {
            assert!(name.contains("github.com"), "key includes host: {name}");
        }
        let distinct: std::collections::HashSet<&str> =
            dotcom.iter().map(|s| s.as_str()).collect();
        assert_eq!(distinct.len(), dotcom.len(), "variants differ from each other");
        std::env::set_var("GH_HOST", "ghe.corp");
        let ghe = variants("o/r");
        for name in &ghe {
            assert!(name.contains("ghe.corp"), "key includes host: {name}");
        }
        for (a, b) in dotcom.iter().zip(ghe.iter()) {
            assert_ne!(a, b, "same slug differs across hosts");
        }
        match saved {
            Some(v) => std::env::set_var("GH_HOST", v),
            None => std::env::remove_var("GH_HOST"),
        }
    }

    #[test]
    fn prefs_keys_include_host_and_differ_across_hosts() {
        let _guard = test_env_lock();
        let saved = std::env::var("GH_HOST").ok();
        std::env::set_var("GH_HOST", "github.com");
        let repo = prefs_repo_key("Acme/R");
        let owner = prefs_owner_key("Acme");
        assert_eq!(repo, "github.com/acme/r", "repo key carries host and slug");
        assert_eq!(owner, "github.com/acme", "board key carries host and owner");
        std::env::set_var("GH_HOST", "ghe.corp");
        assert_ne!(prefs_repo_key("Acme/R"), repo, "same repo differs across hosts");
        assert_ne!(prefs_owner_key("Acme"), owner, "same owner differs across hosts");
        match saved {
            Some(v) => std::env::set_var("GH_HOST", v),
            None => std::env::remove_var("GH_HOST"),
        }
    }

    #[test]
    fn prefs_roundtrip_then_missing_and_garbage_load_defaults() {
        let _guard = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved_xdg = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        // Missing file loads defaults.
        let fresh = load_prefs();
        assert_eq!(fresh.merged_window_hours, 24);
        assert!(fresh.show_open_prs && fresh.show_merged_prs);
        assert!(fresh.show_open_issues && fresh.show_closed_issues);
        assert_eq!(fresh.board_cards, crate::model::CardFields::default());
        // A round trip keeps toggles, per-owner filters, and per-repo queries.
        let mut prefs = Prefs::default();
        prefs.board_cards.milestone = true;
        prefs.row_pr.checks = false;
        prefs.show_merged_prs = false;
        prefs.merged_window_hours = 1;
        prefs.group_by = crate::model::GroupBy::Milestone;
        prefs.board_filters.insert(
            "o".to_string(),
            SavedBoardFilter {
                filter: "bug".to_string(),
                milestones: vec![Some("v1".to_string()), None],
            },
        );
        prefs.queries.insert("o/r".to_string(), "label:bug".to_string());
        save_prefs(&prefs);
        let back = load_prefs();
        assert!(back.board_cards.milestone);
        assert!(!back.row_pr.checks);
        assert!(!back.show_merged_prs);
        assert_eq!(back.merged_window_hours, 1);
        assert_eq!(back.group_by, crate::model::GroupBy::Milestone);
        assert_eq!(back.board_filters["o"].filter, "bug");
        assert_eq!(
            back.board_filters["o"].milestones,
            vec![Some("v1".to_string()), None]
        );
        assert_eq!(back.queries["o/r"], "label:bug");
        // Garbage loads defaults, and unknown fields are ignored.
        std::fs::write(super::prefs_path(), "{not json").unwrap();
        assert_eq!(load_prefs().merged_window_hours, 24);
        std::fs::write(super::prefs_path(), r#"{"future_key": 1, "merged_window_hours": 168}"#)
            .unwrap();
        let partial = load_prefs();
        assert_eq!(partial.merged_window_hours, 168);
        assert!(partial.show_open_prs);
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn pins_roundtrip() {
        let _guard = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved_xdg = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        save_pins("o/r", &[7, 9]);
        assert_eq!(load_pins("o/r"), vec![7, 9]);
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn cache_roundtrip_and_expiry() {
        let _guard = test_env_lock();
        let dir = tempfile::tempdir().unwrap();
        let saved_xdg = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        cache_put("k1", &serde_json::json!({"a": 1}));
        assert_eq!(cache_get::<serde_json::Value>("k1").unwrap(), serde_json::json!({"a": 1}));
        // Backdate to force expiry.
        let p = cache_path("k1");
        let mut payload: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        payload["fetched_at"] = serde_json::json!(now - CACHE_TTL_SECS - 1);
        std::fs::write(&p, serde_json::to_string(&payload).unwrap()).unwrap();
        assert!(cache_get::<serde_json::Value>("k1").is_none());
        match saved_xdg {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
    }
}
