use super::client::{client, graphql_auth_failure, graphql_url, post_json};
use crate::model::{BoardData, BoardItem, Label};
use serde_json::{json, Map, Value};

fn query(kind: &str) -> String {
    let content = r#"
        __typename
        ... on Issue {
            number title url body
            labels(first: 100) { nodes { name } }
            milestone { title }
            repository { nameWithOwner }
            assignees(first: 10) { nodes { login } }
        }
        ... on PullRequest {
            number title url body isDraft
            labels(first: 100) { nodes { name } }
            milestone { title }
            repository { nameWithOwner }
            assignees(first: 10) { nodes { login } }
        }
        ... on DraftIssue {
            title body
            assignees(first: 10) { nodes { login } }
        }
    "#;
    format!(
        r#"query($owner: String!, $after: String) {{
            {kind}(login: $owner) {{
                projectsV2(first: 1) {{
                    nodes {{
                        title number url
                        fields(first: 20) {{ nodes {{
                            ... on ProjectV2SingleSelectField {{ name options {{ name }} }}
                        }} }}
                        items(first: 100, after: $after) {{
                            pageInfo {{ hasNextPage endCursor }}
                            nodes {{
                                fieldValueByName(name: "Status") {{
                                    ... on ProjectV2ItemFieldSingleSelectValue {{ name }}
                                }}
                                content {{ {content} }}
                            }}
                        }}
                    }}
                }}
            }}
        }}"#
    )
}

fn parse_item(node: &Value) -> BoardItem {
    let status = node
        .pointer("/fieldValueByName/name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(content) = node.get("content").filter(|v| v.is_object()) else {
        return BoardItem {
            title: String::new(),
            number: None,
            status,
            labels: vec![],
            repo: None,
            milestone: None,
            content: Value::Null,
        };
    };
    let typename = content.get("__typename").and_then(Value::as_str).unwrap_or("");
    let title = content.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let number = content.get("number").and_then(Value::as_u64);
    let labels = content
        .pointer("/labels/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| l.get("name")?.as_str().map(|s| Label { name: s.to_string() }))
        .collect();
    let repo = content
        .pointer("/repository/nameWithOwner")
        .and_then(Value::as_str)
        .map(str::to_string);
    let milestone = content
        .pointer("/milestone/title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let assignees: Vec<Value> = content
        .pointer("/assignees/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| a.get("login")?.as_str().map(|login| json!({"login": login})))
        .collect();
    let body = content.get("body").and_then(Value::as_str).unwrap_or("");
    // Keys are the ones card rendering reads off `content`, plus
    // `repository`, which the old mapping read to fill `BoardItem::repo`.
    let mut slim = Map::new();
    slim.insert("type".to_string(), Value::String(typename.to_string()));
    if let Some(url) = content.get("url").and_then(Value::as_str) {
        slim.insert("url".to_string(), Value::String(url.to_string()));
    }
    slim.insert("body".to_string(), Value::String(body.to_string()));
    if typename == "PullRequest" {
        slim.insert(
            "isDraft".to_string(),
            Value::Bool(content.get("isDraft").and_then(Value::as_bool).unwrap_or(false)),
        );
    }
    if let Some(repo) = &repo {
        slim.insert("repository".to_string(), Value::String(repo.clone()));
    }
    slim.insert("assignees".to_string(), Value::Array(assignees));
    BoardItem {
        title,
        number,
        status,
        labels,
        repo,
        milestone,
        content: Value::Object(slim),
    }
}

fn project_node<'a>(page: &'a Value, kind: &str) -> Option<&'a Value> {
    page.pointer(&format!("/data/{kind}/projectsV2/nodes/0"))
        .filter(|v| v.is_object())
}

fn parse_board_pages(owner: &str, pages: &[Value], kind: &str) -> Result<BoardData, String> {
    for page in pages {
        if let Some(e) = graphql_auth_failure(page) {
            return Err(e);
        }
    }
    let first_page = pages.first().ok_or_else(|| format!("no Projects board found for {owner}"))?;
    let project = project_node(first_page, kind)
        .ok_or_else(|| format!("no Projects board found for {owner}"))?;
    let title = project.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let number = project.get("number").and_then(Value::as_i64).unwrap_or(0);
    let url = project
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/orgs/{owner}/projects/{number}"));
    let mut order = Vec::new();
    if let Some(fields) = project.pointer("/fields/nodes").and_then(Value::as_array) {
        for field in fields {
            if field.get("name").and_then(Value::as_str) == Some("Status") {
                if let Some(options) = field.get("options").and_then(Value::as_array) {
                    for option in options {
                        if let Some(name) = option.get("name").and_then(Value::as_str) {
                            order.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    let mut items = Vec::new();
    for page in pages {
        let Some(project) = project_node(page, kind) else { continue };
        if let Some(nodes) = project.pointer("/items/nodes").and_then(Value::as_array) {
            for node in nodes {
                items.push(parse_item(node));
            }
        }
    }
    Ok(BoardData { title, order, items, url })
}

async fn fetch_kind(kind: &str, owner: &str, bearer: &str) -> Result<Option<BoardData>, String> {
    let url = graphql_url();
    let http = client();
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let value = post_json(
            &http,
            &url,
            bearer,
            &json!({"query": query(kind), "variables": {"owner": owner, "after": cursor}}),
        )
        .await?;
        if let Some(e) = graphql_auth_failure(&value) {
            return Err(e);
        }
        let owner_found = value
            .pointer(&format!("/data/{kind}"))
            .is_some_and(|v| v.is_object());
        if !owner_found {
            if pages.is_empty() {
                return Ok(None);
            }
            return Err(format!(
                "GitHub GraphQL: {}",
                value.get("errors").unwrap_or(&Value::Null)
            ));
        }
        let page_info = project_node(&value, kind)
            .and_then(|p| p.pointer("/items/pageInfo"));
        let more = page_info
            .and_then(|info| info.get("hasNextPage"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let next = page_info
            .and_then(|info| info.get("endCursor"))
            .and_then(Value::as_str)
            .map(str::to_string);
        pages.push(value);
        match (more, next) {
            (true, Some(next)) => cursor = Some(next),
            _ => break,
        }
    }
    parse_board_pages(owner, &pages, kind).map(Some)
}

/// First Projects v2 board for an org or user login.
///
/// The column order comes from the `Status` single-select field's
/// options, in the order the project defines them. That is the same
/// order the old `gh project field-list` mapping read.
pub async fn fetch_board(owner: &str, bearer: &str) -> Result<BoardData, String> {
    if let Some(board) = fetch_kind("organization", owner, bearer).await? {
        return Ok(board);
    }
    if let Some(board) = fetch_kind("user", owner, bearer).await? {
        return Ok(board);
    }
    Err(format!("no Projects board found for {owner}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Label;

    fn expected_board() -> BoardData {
        BoardData {
            title: "Roadmap".to_string(),
            order: vec!["Todo".to_string(), "In Progress".to_string(), "Done".to_string()],
            url: "https://github.com/orgs/o/projects/2".to_string(),
            items: vec![
                BoardItem {
                    title: "Fix auth".to_string(),
                    number: Some(101),
                    status: Some("Todo".to_string()),
                    labels: vec![Label { name: "bug".to_string() }],
                    repo: Some("o/r".to_string()),
                    milestone: Some("v1".to_string()),
                    content: json!({
                        "type": "Issue", "url": "https://github.com/o/r/issues/101",
                        "body": "First line\n\nMore details", "repository": "o/r",
                        "assignees": [{"login": "amy"}]
                    }),
                },
                BoardItem {
                    title: "Ship board".to_string(),
                    number: Some(102),
                    status: Some("Done".to_string()),
                    labels: vec![],
                    repo: Some("o/r".to_string()),
                    milestone: None,
                    content: json!({
                        "type": "PullRequest", "url": "https://github.com/o/r/pull/102",
                        "body": "", "repository": "o/r", "assignees": [],
                        "isDraft": true
                    }),
                },
                BoardItem {
                    title: "Draft idea".to_string(),
                    number: None,
                    status: Some("In Progress".to_string()),
                    labels: vec![],
                    repo: None,
                    milestone: None,
                    content: json!({
                        "type": "DraftIssue", "body": "Draft body",
                        "assignees": [{"login": "bo"}]
                    }),
                },
            ],
        }
    }

    fn page(items: Vec<Value>, more: bool, cursor: Option<&str>) -> Value {
        json!({"data": {"organization": {"projectsV2": {"nodes": [{
            "title": "Roadmap",
            "number": 2,
            "url": "https://github.com/orgs/o/projects/2",
            "fields": {"nodes": [
                {"name": "Priority", "options": [{"name": "High"}]},
                {"name": "Status", "options": [
                    {"name": "Todo"}, {"name": "In Progress"}, {"name": "Done"}
                ]}
            ]},
            "items": {"pageInfo": {"hasNextPage": more, "endCursor": cursor}, "nodes": items}
        }]}}}})
    }

    #[test]
    fn graphql_fixture_maps_to_the_gh_board() {
        let page1 = page(
            vec![
                json!({
                    "fieldValueByName": {"name": "Todo"},
                    "content": {
                        "__typename": "Issue", "number": 101, "title": "Fix auth",
                        "url": "https://github.com/o/r/issues/101",
                        "body": "First line\n\nMore details",
                        "labels": {"nodes": [{"name": "bug"}]},
                        "milestone": {"title": "v1"},
                        "repository": {"nameWithOwner": "o/r"},
                        "assignees": {"nodes": [{"login": "amy"}]}
                    }
                }),
                json!({
                    "fieldValueByName": {"name": "Done"},
                    "content": {
                        "__typename": "PullRequest", "number": 102, "title": "Ship board",
                        "url": "https://github.com/o/r/pull/102", "body": "",
                        "isDraft": true,
                        "labels": {"nodes": []}, "milestone": null,
                        "repository": {"nameWithOwner": "o/r"},
                        "assignees": {"nodes": []}
                    }
                }),
            ],
            true,
            Some("cursor-1"),
        );
        let page2 = page(
            vec![json!({
                "fieldValueByName": {"name": "In Progress"},
                "content": {
                    "__typename": "DraftIssue", "title": "Draft idea", "body": "Draft body",
                    "assignees": {"nodes": [{"login": "bo"}]}
                }
            })],
            false,
            None,
        );
        let board = parse_board_pages("o", &[page1, page2], "organization").unwrap();
        assert_eq!(
            serde_json::to_value(&board).unwrap(),
            serde_json::to_value(expected_board()).unwrap()
        );
    }

    #[test]
    fn board_query_covers_org_shape_and_paging() {
        let q = query("organization");
        for field in [
            "organization(login: $owner)",
            "projectsV2(first: 1)",
            "items(first: 100, after: $after)",
            "pageInfo",
            "ProjectV2SingleSelectField",
            "fieldValueByName(name: \"Status\")",
            "nameWithOwner",
            "isDraft",
        ] {
            assert!(q.contains(field), "missing: {field}");
        }
        assert!(query("user").contains("user(login: $owner)"));
    }

    #[test]
    fn board_without_a_project_errors_like_the_gh_path() {
        let value = json!({"data": {"organization": {"projectsV2": {"nodes": []}}}});
        let err = parse_board_pages("o", &[value], "organization").unwrap_err();
        assert_eq!(err, "no Projects board found for o");
    }

    #[test]
    fn board_auth_failure_is_classified() {
        let denied = json!({
            "data": {"organization": null},
            "errors": [{
                "type": "FORBIDDEN",
                "message": "Resource not accessible by personal access token"
            }]
        });
        let err = parse_board_pages("o", &[denied], "organization").unwrap_err();
        assert!(crate::api::client::is_auth_error(&err), "auth-class: {err}");
        assert!(err.contains("gh auth refresh"), "hint: {err}");
    }
}
