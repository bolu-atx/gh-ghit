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
#[cfg(test)]
pub(crate) fn test_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_roundtrip() {
        let _guard = test_env_lock().lock().unwrap();
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
        let _guard = test_env_lock().lock().unwrap();
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
