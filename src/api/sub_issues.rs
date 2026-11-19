//! REST sub-issues of one parent, for the opt-in sidebar tree.

use serde_json::Value;

use super::client::{client, get, rest_base};
use crate::model::{self, Label};

/// Sub-issues for the opt-in sidebar tree. The tree shows every child,
/// so it pages once at the REST maximum.
pub async fn sub_issues(
    repo: &str, number: u64, bearer: &str,
) -> Result<Vec<model::IssueData>, String> {
    let url = format!("{}/repos/{repo}/issues/{number}/sub_issues?per_page=100", rest_base());
    let value = get(&client(), &url, bearer).await?;
    Ok(parse_sub_issues(&value))
}

/// Sub-issue rows out of a REST `issues/{n}/sub_issues` payload.
/// Malformed entries skip, like every other list parser.
fn parse_sub_issues(value: &Value) -> Vec<model::IssueData> {
    value.as_array().into_iter().flatten().filter_map(sub_issue_row).collect()
}

/// One REST sub-issue as a sidebar row: number, title, state and labels,
/// which is all a dim tree row renders. REST state is lowercase; rows
/// carry the GraphQL uppercase form. The full fetch fills the rest when
/// the row opens.
fn sub_issue_row(v: &Value) -> Option<model::IssueData> {
    let labels = v.get("labels").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|l| l.get("name").and_then(Value::as_str).or_else(|| l.as_str()).map(|s| Label { name: s.to_owned() }))
        .collect();
    Some(model::IssueData {
        number: v.get("number")?.as_u64()?,
        title: v.get("title").and_then(Value::as_str).unwrap_or("").to_owned(),
        state: v.get("state").and_then(Value::as_str).unwrap_or("").to_uppercase(),
        labels,
        ..model::IssueData::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rest_sub_issues_parse_into_rows() {
        let rows = parse_sub_issues(&json!([
            {"number": 8, "title": "child", "state": "open", "labels": [{"name": "bug"}]},
            {"number": 9, "title": "done", "state": "closed", "labels": []},
            {"title": "no number"},
            null,
        ]));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].number, &*rows[0].title, &*rows[0].state), (8, "child", "OPEN"));
        assert_eq!(rows[0].labels[0].name, "bug");
        assert_eq!(rows[1].state, "CLOSED");
        assert!(parse_sub_issues(&json!({"not": "a list"})).is_empty());
    }
}
