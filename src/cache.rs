use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
