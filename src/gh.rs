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

const FIELDS: &str = "number,title,state,author,labels,assignees,createdAt,url,body,comments";

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
                labels: content
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| {
                        l.get("name")
                            .and_then(|n| n.as_str())
                            .map(|n| crate::model::Label { name: n.to_string() })
                    })
                    .collect(),
                content,
            }
        })
        .collect();

    let data = BoardData { number: num, title, order, items };
    cache::cache_put(&key, &data);
    Ok(data)
}
