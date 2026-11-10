use crate::cache;
use crate::model::{BoardData, BoardItem, IssueData};

const STATUS_QUERY: &str = r#"
query($owner: String!, $repo: String!, $num: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $num) {
      projectItems(first: 10) {
        nodes {
          project { title }
          status: fieldValueByName(name: "Status") {
            __typename
            ... on ProjectV2ItemFieldSingleSelectValue { name }
          }
        }
      }
    }
  }
}
"#;

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

fn gh_issue_raw(repo: &str, number: u64, fields: &str, kind: &str) -> Result<serde_json::Value, String> {
    let num = number.to_string();
    let mut owned: Vec<String> = vec![];
    if !repo.is_empty() {
        owned.push("-R".to_string());
        owned.push(repo.to_string());
    }
    let mut full: Vec<&str> = vec![kind, "view", &num];
    if !owned.is_empty() {
        full.push(&owned[0]);
        full.push(&owned[1]);
    }
    full.push("--json");
    full.push(fields);
    gh(&full)
}

const FIELDS: &str = "number,title,state,author,labels,assignees,createdAt,updatedAt,url,body,comments";

/// Fetch an issue; fall back to PR view when the number is a pull request.
pub fn gh_issue_json(repo: &str, number: u64) -> Result<IssueData, String> {
    let v = gh_issue_raw(repo, number, FIELDS, "issue").or_else(|e| {
        if e.contains("Could not resolve") {
            gh_issue_raw(repo, number, FIELDS, "pr")
        } else {
            Err(e)
        }
    })?;
    let mut d: IssueData =
        serde_json::from_value(v).map_err(|e| format!("bad issue payload: {e}"))?;
    d.is_pr = d.url.contains("/pull/");
    Ok(d)
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

/// `[(project title, status name|None)]` — `[]` when unknown or unset.
pub fn gh_project_statuses(
    owner_repo: &str,
    number: u64,
    is_pr: bool,
) -> Vec<(String, Option<String>)> {
    let Some((owner, repo)) = owner_repo.split_once('/') else {
        return vec![];
    };
    let query = STATUS_QUERY.replace(
        "issue(number:",
        if is_pr { "pullRequest(number:" } else { "issue(number:" },
    );
    let num = number.to_string();
    let out = std::process::Command::new("gh")
        .args([
            "api",
            "graphql",
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("repo={repo}"),
            "-F",
            &format!("num={num}"),
            "-f",
            &format!("query={query}"),
        ])
        .output();
    let Ok(out) = out else { return vec![] };
    if !out.status.success() {
        return vec![];
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    let key = if is_pr { "pullRequest" } else { "issue" };
    let nodes = v
        .pointer(&format!("/data/repository/{key}/projectItems/nodes"))
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    nodes
        .iter()
        .map(|n| {
            let title = n
                .pointer("/project/title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let status = n
                .get("status")
                .and_then(|s| s.get("name"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            (title, status)
        })
        .collect()
}

/// `(number, merged_at)` for PRs in `state`: `open` rows carry `None`.
fn gh_pr_rows(repo: &str, state: &str) -> Vec<(u64, Option<String>)> {
    let mut args: Vec<&str> = vec!["pr", "list", "--state", state, "--limit", "50", "--json", "number,mergedAt"];
    let owned;
    if !repo.is_empty() {
        owned = vec!["-R".to_string(), repo.to_string()];
        args.push(&owned[0]);
        args.push(&owned[1]);
    }
    let v = gh(&args).unwrap_or(serde_json::Value::Null);
    v.as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|p| {
            let n = p.get("number").and_then(|n| n.as_u64())?;
            let merged = p.get("mergedAt").and_then(|m| m.as_str()).map(|s| s.to_string());
            Some((n, merged))
        })
        .collect()
}

/// Open PR numbers for the sidebar. Empty when offline: the section still
/// shows clicked and fetched PRs.
pub fn gh_open_prs(repo: &str) -> Vec<u64> {
    gh_pr_rows(repo, "open").into_iter().map(|(n, _)| n).collect()
}

/// Keep merged rows whose `mergedAt` falls inside the window. Pure so the
/// cutoff is testable with a fixed clock.
pub fn filter_recent(
    rows: &[(u64, Option<String>)],
    now: chrono::DateTime<chrono::Utc>,
    hours: u64,
) -> Vec<u64> {
    let cutoff = now - chrono::Duration::hours(hours as i64);
    rows.iter()
        .filter_map(|(n, m)| {
            let ts = m.as_deref()?;
            let when = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
            (when.with_timezone(&chrono::Utc) >= cutoff).then_some(*n)
        })
        .collect()
}

/// Recently merged PR numbers inside the last `hours` hours.
pub fn gh_recently_merged_prs(repo: &str, hours: u64) -> Vec<u64> {
    filter_recent(&gh_pr_rows(repo, "merged"), chrono::Utc::now(), hours)
}

/// Board data for an org/user login: title, status order, cards.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(h: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 10)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
            .and_utc()
    }

    #[test]
    fn merged_window_keeps_only_fresh_merges() {
        let rows = vec![
            (1, Some("2026-11-10T11:30:00Z".to_string())), // 30 min ago
            (2, Some("2026-11-09T12:00:00Z".to_string())), // yesterday
            (3, Some("2026-11-05T12:00:00Z".to_string())), // five days back
            (4, None),                                     // still open
        ];
        let now = utc(12, 0);
        assert_eq!(filter_recent(&rows, now, 1), vec![1]);
        assert_eq!(filter_recent(&rows, now, 24), vec![1, 2]);
        assert_eq!(filter_recent(&rows, now, 168), vec![1, 2, 3]);
    }
}
