//! DEPRECATED `gh` CLI subprocess backend.
//!
//! All issue/PR fetching moved to the HTTPS client in `api.rs`, called
//! through `github.rs`. Only the project board still lives here until it is
//! ported to GraphQL — do not add new callers.

use crate::cache;
use crate::model::{BoardData, BoardItem};

fn gh(args: &[&str]) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run gh: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().next().unwrap_or("gh failed").trim().to_string();
        return Err(if first.is_empty() { "gh failed".to_string() } else { first });
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    serde_json::from_str(&text).map_err(|e| format!("bad gh output: {e}"))
}

/// Labels live on the item itself; fall back to the content payload.
/// Accepts `["name"]` or `[{"name": …}]` shapes and skips empty lists so a
/// present-but-empty item list still falls through to the content labels.
fn parse_labels(item: &serde_json::Value) -> Vec<crate::model::Label> {
    let content = item.get("content");
    for src in [item.get("labels"), content.and_then(|c| c.get("labels"))] {
        if let Some(names) = src.and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|l| {
                    l.as_str().map(str::to_string).or_else(|| {
                        l.get("name").and_then(|n| n.as_str()).map(str::to_string)
                    })
                })
                .collect::<Vec<_>>()
        }) {
            if !names.is_empty() {
                return names
                    .into_iter()
                    .map(|name| crate::model::Label { name })
                    .collect();
            }
        }
    }
    vec![]
}

/// Board data for an org/user login: title, status order, cards.
///
/// Deprecated: remove with the GraphQL board port; `app` calls it through
/// one shim so the call site stays visible.
#[deprecated(note = "port the board to GraphQL in api.rs, then remove")]
pub fn fetch_board(owner: &str) -> Result<BoardData, String> {
    let key = format!("board-{}", owner.to_lowercase());
    if let Some(cached) = cache::cache_get::<BoardData>(&key) {
        return Ok(cached);
    }
    let num_s;
    let projects = gh(&["project", "list", "--owner", owner, "--limit", "1", "--format", "json"])?;
    let list = projects.get("projects").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let Some(first) = list.first() else {
        return Err(format!("no Projects board found for {owner}"));
    };
    let num = first.get("number").and_then(|n| n.as_i64()).unwrap_or(0);
    let title = first.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
    num_s = num.to_string();

    let fields = gh(&["project", "field-list", &num_s, "--owner", owner, "--format", "json"])?;
    let mut order: Vec<String> = vec![];
    for f in fields.get("fields").and_then(|f| f.as_array()).cloned().unwrap_or_default() {
        if f.get("name").and_then(|n| n.as_str()) == Some("Status") {
            for o in f.get("options").and_then(|o| o.as_array()).cloned().unwrap_or_default() {
                if let Some(name) = o.get("name").and_then(|n| n.as_str()) {
                    order.push(name.to_string());
                }
            }
        }
    }
    let items_v = gh(&["project", "item-list", &num_s, "--owner", owner, "--limit", "100", "--format", "json"])?;
    let items_raw = items_v.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    let items: Vec<BoardItem> = items_raw
        .iter()
        .map(|c| {
            let content = c.get("content").cloned().unwrap_or(serde_json::Value::Null);
            BoardItem {
                id: c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                title: c.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                number: c
                    .get("number")
                    .and_then(|v| v.as_u64())
                    .or_else(|| content.get("number").and_then(|v| v.as_u64())),
                status: c.get("status").and_then(|v| v.as_str()).map(|s| s.to_string()),
                labels: parse_labels(c),
                repo: content
                    .get("repository")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                content,
            }
        })
        .collect();

    let data = BoardData { number: num, title, order, items };
    cache::cache_put(&key, &data);
    Ok(data)
}
