use super::client::{client, get, graphql_auth_failure, graphql_url, post_json, rest_base};
use crate::model::{self, GraphData, GraphEdgeKind, GraphLinkedPr, GraphNode, Label};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Labels plus project status: valid on both Issues and PullRequests, so
/// every graph node shape reuses it.
const GRAPH_META: &str = r#"labels(first: 5) { nodes { name } }
 projectItems(first: 3) { nodes { status: fieldValueByName(name: "Status") {
 ... on ProjectV2ItemFieldSingleSelectValue { name } } } }"#;

/// The dependency star around one issue. Blocker connections stay behind
/// `with_blockers` so a token or repo without those fields can retry
/// without them and derive reference edges from the body instead.
fn graph_query(with_blockers: bool) -> String {
    let mut q = String::from(
        "query($owner: String!, $repo: String!, $num: Int!) { repository(owner: $owner, name: $repo) { issue(number: $num) { number title state body ",
    );
    q.push_str(GRAPH_META);
    q.push_str(" parent { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } ");
    // Sub-issues are not here: they load through REST
    // `issues/{n}/sub_issues` in `graph` (confirmed equivalent, #117).
    if with_blockers {
        for conn in ["blockedBy", "blocking"] {
            q.push_str(&format!("{conn}(first: 10) {{ nodes {{ number title state "));
            q.push_str(GRAPH_META);
            q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
        }
    }
    q.push_str("trackedIssues(first: 5) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
    q.push_str("trackedInIssues(first: 5) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" closedByPullRequestsReferences(first: 3, includeClosedPrs: true) { nodes { number state } } } totalCount } ");
    q.push_str("closedByPullRequestsReferences(first: 5, includeClosedPrs: true) { nodes { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } totalCount } ");
    q.push_str("timelineItems(first: 20, itemTypes: [CROSS_REFERENCED_EVENT]) { nodes { __typename ... on CrossReferencedEvent { willCloseTarget source { __typename ... on Issue { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } ... on PullRequest { number title state ");
    q.push_str(GRAPH_META);
    q.push_str(" } } } } } } } }");
    q
}

/// One aliased detail lookup per body-referenced number, for the fallback
/// path that resolves `#` refs without blocker fields.
fn graph_detail_query(owner: &str, name: &str, numbers: &[u64]) -> String {
    let mut fields = String::new();
    for (i, n) in numbers.iter().enumerate() {
        fields.push_str(&format!(
            "r{i}: repository(owner: {}, name: {}) {{ issueOrPullRequest(number: {n}) {{ __typename ... on Issue {{ number title state {GRAPH_META} }} ... on PullRequest {{ number title state {GRAPH_META} }} }} }} ",
            json!(owner),
            json!(name)
        ));
    }
    format!("query {{ {fields} }}")
}

fn parse_graph_labels(v: &Value) -> Vec<Label> {
    v.pointer("/labels/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| {
            l.get("name")?.as_str().map(|s| Label { name: s.to_owned() })
        })
        .collect()
}

fn parse_graph_status(v: &Value) -> Option<String> {
    v.pointer("/projectItems/nodes")
        .and_then(Value::as_array)?
        .first()?
        .pointer("/status/name")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn parse_graph_prs(v: &Value) -> Vec<GraphLinkedPr> {
    v.pointer("/closedByPullRequestsReferences/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(GraphLinkedPr {
                number: p.get("number")?.as_u64()?,
                state: p.get("state").and_then(Value::as_str).unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// One issue-typed node: center, parent, sub-issues, blockers, tracked.
fn parse_graph_issue(v: &Value) -> Result<GraphNode, String> {
    let string = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(GraphNode {
        number: v.get("number").and_then(Value::as_u64).ok_or("graph node without a number")?,
        title: string("title"),
        state: string("state"),
        is_pr: false,
        labels: parse_graph_labels(v),
        project_status: parse_graph_status(v),
        linked_prs: parse_graph_prs(v),
    })
}

/// One polymorphic node: closing PRs and timeline sources carry
/// `__typename`; labeled the same, without nested closing references.
fn parse_graph_typed(v: &Value) -> Result<GraphNode, String> {
    let string = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    Ok(GraphNode {
        number: v.get("number").and_then(Value::as_u64).ok_or("graph node without a number")?,
        title: string("title"),
        state: string("state"),
        is_pr: v.get("__typename").and_then(Value::as_str).unwrap_or("") == "PullRequest",
        labels: parse_graph_labels(v),
        project_status: parse_graph_status(v),
        linked_prs: vec![],
    })
}

/// Neighbors in pane priority order: blockers, parent, children, closing
/// PRs, tracked, then timeline cross-references (`willCloseTarget` PRs
/// count as closing). First edge wins on duplicates via `build_graph`.
fn push_graph_conn(issue: &Value, ptr: &str, kind: GraphEdgeKind, out: &mut Vec<(GraphNode, GraphEdgeKind)>) {
    for n in issue.pointer(ptr).and_then(Value::as_array).into_iter().flatten() {
        if let Ok(node) = parse_graph_issue(n) {
            out.push((node, kind));
        }
    }
}

fn parse_graph_neighbors(issue: &Value) -> Vec<(GraphNode, GraphEdgeKind)> {
    let mut out: Vec<(GraphNode, GraphEdgeKind)> = vec![];
    push_graph_conn(issue, "/blocking/nodes", GraphEdgeKind::Blocks, &mut out);
    push_graph_conn(issue, "/blockedBy/nodes", GraphEdgeKind::BlockedBy, &mut out);
    if let Some(p) = issue.get("parent").filter(|v| v.is_object()) {
        if let Ok(node) = parse_graph_issue(p) {
            out.push((node, GraphEdgeKind::Parent));
        }
    }
    for n in issue
        .pointer("/closedByPullRequestsReferences/nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Ok(node) = parse_graph_typed(n) {
            out.push((node, GraphEdgeKind::Closes));
        }
    }
    push_graph_conn(issue, "/trackedIssues/nodes", GraphEdgeKind::References, &mut out);
    push_graph_conn(issue, "/trackedInIssues/nodes", GraphEdgeKind::ReferencedBy, &mut out);
    for t in issue.pointer("/timelineItems/nodes").and_then(Value::as_array).into_iter().flatten() {
        let will_close = t.get("willCloseTarget").and_then(Value::as_bool).unwrap_or(false);
        let Some(src) = t.get("source").filter(|v| v.is_object()) else { continue };
        if let Ok(node) = parse_graph_typed(src) {
            let kind = if will_close && node.is_pr {
                GraphEdgeKind::Closes
            } else {
                GraphEdgeKind::ReferencedBy
            };
            out.push((node, kind));
        }
    }
    out
}

fn graph_issue(value: &Value) -> Result<&Value, String> {
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
    value
        .pointer("/data/repository/issue")
        .filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))
}

/// The full graph: center plus every neighbor, capped by `build_graph`.
/// Test-only since `graph` splices REST sub-issues in before building.
#[cfg(test)]
fn parse_graph(value: &Value) -> Result<GraphData, String> {
    let (center, neighbors) = parse_graph_parts(value)?;
    Ok(model::build_graph(center, neighbors))
}

/// Center plus GraphQL neighbors, before the REST sub-issues join.
fn parse_graph_parts(value: &Value) -> Result<(GraphNode, Vec<(GraphNode, GraphEdgeKind)>), String> {
    let issue = graph_issue(value)?;
    let center = parse_graph_issue(issue)?;
    Ok((center, parse_graph_neighbors(issue)))
}

/// One REST sub-issue object as a graph node. REST state is lowercase,
/// graph state is the GraphQL uppercase form; REST carries no project
/// status or closing-PR list, so those stay empty — the same fields
/// every REST issue payload in this file already lacks.
fn sub_issue_node(v: &Value) -> Option<GraphNode> {
    let labels = v.get("labels").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|l| l.get("name").and_then(Value::as_str).or_else(|| l.as_str()).map(|s| Label { name: s.to_owned() }))
        .collect();
    Some(GraphNode {
        number: v.get("number")?.as_u64()?,
        title: v.get("title").and_then(Value::as_str).unwrap_or("").to_owned(),
        state: v.get("state").and_then(Value::as_str).unwrap_or("").to_uppercase(),
        is_pr: false,
        labels,
        project_status: None,
        linked_prs: vec![],
    })
}

/// Sub-issue nodes out of a REST `issues/{n}/sub_issues` payload.
/// Malformed entries skip, like every other list parser here.
fn parse_sub_issues(value: &Value) -> Vec<GraphNode> {
    value.as_array().into_iter().flatten().filter_map(sub_issue_node).collect()
}

/// Splice REST sub-issues into GraphQL neighbors at their pane
/// priority slot: after blockers and parent, before every other kind,
/// so a duplicate number still wins as a child (`build_graph` keeps
/// the first edge).
fn with_sub_issues(mut neighbors: Vec<(GraphNode, GraphEdgeKind)>, subs: Vec<GraphNode>) -> Vec<(GraphNode, GraphEdgeKind)> {
    let at = neighbors.iter()
        .take_while(|(_, k)| matches!(k, GraphEdgeKind::Blocks | GraphEdgeKind::BlockedBy | GraphEdgeKind::Parent))
        .count();
    let children = subs.into_iter().map(|n| (n, GraphEdgeKind::Child));
    neighbors.splice(at..at, children);
    neighbors
}

/// Sub-issues through REST `GET issues/{n}/sub_issues`, capped at the
/// old GraphQL `first: 10`. Ported in #117: probe
/// `gh api repos/bolu-atx/gh-ghit/issues/117/sub_issues` returns 200
/// with the same issue objects REST returns everywhere else.
async fn fetch_sub_issues(client: &reqwest::Client, base: &str, repo: &str, number: u64, bearer: &str) -> Result<Vec<GraphNode>, String> {
    let value = get(client, &format!("{base}/repos/{repo}/issues/{number}/sub_issues?per_page=10"), bearer).await?;
    Ok(parse_sub_issues(&value))
}

/// True when the fetch failed on the blocker connections (schema hides
/// them from some tokens): retry without them and derive references.
fn is_blocker_field_error(e: &str) -> bool {
    e.contains("blockedBy") || e.contains("blocking")
}

/// Center plus neighbors plus unresolved body refs from a blocker-free
/// payload.
type GraphFallbackParts = (GraphNode, Vec<(GraphNode, GraphEdgeKind)>, Vec<u64>);

/// Fallback center plus neighbors from a blocker-free payload, with the
/// body-referenced numbers still to resolve. Pure so tests pin it without
/// the network.
fn parse_graph_fallback_parts(value: &Value, repo: &str) -> Result<GraphFallbackParts, String> {
    let issue = graph_issue(value)?;
    let center = parse_graph_issue(issue)?;
    let neighbors = parse_graph_neighbors(issue);
    let known: HashSet<u64> = neighbors
        .iter()
        .map(|(n, _)| n.number)
        .chain([center.number])
        .collect();
    let body = issue.get("body").and_then(Value::as_str).unwrap_or("");
    let room = model::MAX_GRAPH_NODES.saturating_sub(1 + neighbors.len());
    let missing: Vec<u64> = model::referenced_numbers(body, Some(repo))
        .into_iter()
        .filter(|n| !known.contains(n))
        .take(room)
        .collect();
    Ok((center, neighbors, missing))
}

/// Resolve fallback `#` refs to display nodes in one aliased query.
///
/// GraphQL kept: batch win — one aliased query for every missing
/// number replaces one REST `issues/{n}` call per number, and it
/// carries the per-node Projects v2 status REST has no equivalent
/// for (see `sections`).
async fn resolve_graph_refs(
    client: &reqwest::Client,
    url: &str,
    bearer: &str,
    owner: &str,
    name: &str,
    numbers: &[u64],
) -> Result<Vec<GraphNode>, String> {
    if numbers.is_empty() {
        return Ok(vec![]);
    }
    let value = post_json(
        client,
        url,
        bearer,
        &json!({"query": graph_detail_query(owner, name, numbers), "variables": {}}),
    )
    .await?;
    let data = value.get("data").filter(|v| v.is_object()).ok_or_else(|| {
        format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null))
    })?;
    let mut out = vec![];
    for i in 0..numbers.len() {
        if let Some(raw) = data.pointer(&format!("/r{i}/issueOrPullRequest")).filter(|v| v.is_object()) {
            if let Ok(node) = parse_graph_typed(raw) {
                out.push(node);
            }
        }
    }
    Ok(out)
}

/// The dependency star around one issue: blockers, parent, sub-issues,
/// linked and closing PRs, cross-references. When the token or repo hides
/// the blocked-by/blocking fields the fetch retries without them and
/// derives reference edges from body `#` refs instead.
///
/// GraphQL kept for the star itself: batch win — one query replaces a
/// REST call per relation per node, and several relations have no
/// REST equivalent at all. Probes (#117): `issues/117/tracked` 404s
/// (no tracked-issues endpoint), the REST timeline has no
/// `willCloseTarget` (see `issue`), and every node carries Projects
/// v2 status (GraphQL-only, see `sections`). Sub-issues are the one
/// confirmed equivalent and load through REST below. Follow-up worth
/// probing: REST `issues/{n}/dependencies/blocked_by|blocking`
/// returned 200 on #117, but plain issue objects without project
/// status or closing-PR refs, so they stay in this query for now.
pub async fn graph(repo: &str, number: u64, bearer: &str) -> Result<GraphData, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let url = graphql_url();
    let client = client();
    let base = rest_base();
    let value = post_json(
        &client,
        &url,
        bearer,
        &json!({"query": graph_query(true), "variables": {"owner": owner, "repo": name, "num": number}}),
    )
    .await?;
    match parse_graph_parts(&value) {
        Ok((center, neighbors)) => {
            let subs = fetch_sub_issues(&client, &base, repo, number, bearer).await?;
            Ok(model::build_graph(center, with_sub_issues(neighbors, subs)))
        }
        Err(e) if is_blocker_field_error(&e) => {
            let value = post_json(
                &client,
                &url,
                bearer,
                &json!({"query": graph_query(false), "variables": {"owner": owner, "repo": name, "num": number}}),
            )
            .await?;
            let (center, mut neighbors, missing) = parse_graph_fallback_parts(&value, repo)?;
            for node in resolve_graph_refs(&client, &url, bearer, owner, name, &missing).await? {
                neighbors.push((node, GraphEdgeKind::References));
            }
            let subs = fetch_sub_issues(&client, &base, repo, number, bearer).await?;
            let mut data = model::build_graph(center, with_sub_issues(neighbors, subs));
            data.from_references = true;
            Ok(data)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph_node(n: u64, title: &str) -> Value {
        json!({"number": n, "title": title, "state": "OPEN",
            "labels": {"nodes": [{"name": "bug"}]},
            "projectItems": {"nodes": [{"status": {"name": "Todo"}}]},
            "closedByPullRequestsReferences": {"nodes": [{"number": 44, "state": "MERGED"}]}})
    }

    fn graph_payload() -> Value {
        json!({"data": {"repository": {"issue": {
            "number": 7, "title": "center", "state": "OPEN", "body": "see #9",
            "labels": {"nodes": []}, "projectItems": {"nodes": []},
            "closedByPullRequestsReferences": {"nodes": []},
            "parent": {"number": 1, "title": "parent", "state": "CLOSED",
                "labels": {"nodes": []}, "projectItems": {"nodes": []},
                "closedByPullRequestsReferences": {"nodes": []}},
            "blockedBy": {"nodes": [graph_node(9, "blocker")], "totalCount": 1},
            "blocking": {"nodes": [graph_node(10, "blocked")], "totalCount": 1},
            "trackedIssues": {"nodes": [], "totalCount": 0},
            "trackedInIssues": {"nodes": [], "totalCount": 0},
            "timelineItems": {"nodes": [
                {"__typename": "CrossReferencedEvent", "willCloseTarget": false,
                 "source": {"__typename": "Issue", "number": 11, "title": "ref",
                    "state": "OPEN", "labels": {"nodes": []}, "projectItems": {"nodes": []}}},
                {"__typename": "CrossReferencedEvent", "willCloseTarget": true,
                 "source": {"__typename": "PullRequest", "number": 12, "title": "fix",
                    "state": "OPEN", "labels": {"nodes": []}, "projectItems": {"nodes": []}}}
            ]}
        }}}})
    }

    #[test]
    fn graph_query_carries_every_relation() {
        let full = graph_query(true);
        for field in ["parent", "blockedBy", "blocking", "trackedIssues",
            "trackedInIssues", "closedByPullRequestsReferences", "timelineItems",
            "CROSS_REFERENCED_EVENT", "labels", "projectItems"]
        {
            assert!(full.contains(field), "missing {field}");
        }
        // Sub-issues load through REST `issues/{n}/sub_issues` (#117).
        assert!(!full.contains("subIssues"), "{full}");
        let slim = graph_query(false);
        assert!(!slim.contains("blockedBy"), "{slim}");
        assert!(!slim.contains("blocking"), "{slim}");
        assert!(!slim.contains("subIssues"), "{slim}");
    }

    #[test]
    fn graph_queries_have_balanced_braces() {
        for q in [graph_query(true), graph_query(false), graph_detail_query("o", "r", &[1, 2])] {
            let (open, close) = (
                q.chars().filter(|&c| c == '{').count(),
                q.chars().filter(|&c| c == '}').count(),
            );
            assert_eq!(open, close, "unbalanced braces in graph query: {q}");
        }
    }

    #[test]
    fn parses_full_graph_with_labels_status_and_prs() {
        let data = parse_graph(&graph_payload()).unwrap();
        assert_eq!(data.center, 7);
        assert!(!data.from_references);
        // Center + parent + blocker + blocked + 2 timeline = 6 nodes.
        // Sub-issues join from REST, not this payload (next test).
        assert_eq!(data.nodes.len(), 6);
        let kinds: Vec<GraphEdgeKind> = data.edges.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                GraphEdgeKind::Blocks,
                GraphEdgeKind::BlockedBy,
                GraphEdgeKind::Parent,
                GraphEdgeKind::ReferencedBy,
                GraphEdgeKind::Closes,
            ]
        );
        let blocker = data.nodes.iter().find(|n| n.number == 9).unwrap();
        assert_eq!(blocker.labels[0].name, "bug");
        assert_eq!(blocker.project_status.as_deref(), Some("Todo"));
        assert_eq!(blocker.linked_prs, vec![GraphLinkedPr { number: 44, state: "MERGED".to_string() }]);
        let closer = data.nodes.iter().find(|n| n.number == 12).unwrap();
        assert!(closer.is_pr);
        assert!(!data.nodes.iter().find(|n| n.number == 11).unwrap().is_pr);
        // The center blocks #10, so only it (and #7's edge) counts as blocking.
        assert!(model::graph_blocks_something(&data, 7));
        assert!(!model::graph_blocks_something(&data, 11));
        assert!(parse_graph(&json!({"errors": [{"message": "denied"}]})).is_err());
    }

    #[test]
    fn rest_sub_issues_parse_and_join_as_children() {
        let subs = parse_sub_issues(&json!([
            {"number": 8, "title": "child", "state": "open", "labels": [{"name": "bug"}]},
            {"number": 9, "title": "also blocker", "state": "closed", "labels": []},
            {"title": "no number"},
            null,
        ]));
        assert_eq!(subs.len(), 2);
        assert_eq!((subs[0].number, &*subs[0].title, &*subs[0].state), (8, "child", "OPEN"));
        assert_eq!(subs[0].labels[0].name, "bug");
        assert_eq!(subs[0].project_status, None);
        assert!(subs[0].linked_prs.is_empty());
        assert_eq!(subs[1].state, "CLOSED");
        assert!(parse_sub_issues(&json!({"not": "a list"})).is_empty());

        // Children splice after blockers/parent and win on duplicates:
        // #9 stays the blocker the GraphQL query already found.
        let (center, neighbors) = parse_graph_parts(&graph_payload()).unwrap();
        let data = model::build_graph(center, with_sub_issues(neighbors, subs));
        let kinds: Vec<GraphEdgeKind> = data.edges.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                GraphEdgeKind::Blocks,
                GraphEdgeKind::BlockedBy,
                GraphEdgeKind::Parent,
                GraphEdgeKind::Child,
                GraphEdgeKind::ReferencedBy,
                GraphEdgeKind::Closes,
            ]
        );
        let child = data.nodes.iter().find(|n| n.number == 8).unwrap();
        assert_eq!(child.title, "child");
        assert!(data.edges.iter().any(|e| e.kind == GraphEdgeKind::Child && e.to == 8));
    }

    #[test]
    fn blocker_field_errors_route_to_the_fallback() {
        assert!(is_blocker_field_error("GitHub GraphQL: {\"message\":\"Field 'blockedBy' doesn't exist\"}"));
        assert!(is_blocker_field_error("Field 'blocking' doesn't exist on type 'Issue'"));
        assert!(!is_blocker_field_error("GitHub GraphQL: null"));
        // No blocker connections in the payload: body refs become missing.
        let mut slim = graph_payload();
        let issue = slim.pointer_mut("/data/repository/issue").unwrap();
        issue.as_object_mut().unwrap().remove("blockedBy");
        issue.as_object_mut().unwrap().remove("blocking");
        issue["body"] = json!("follow up in #9 and #10, dup of #9");
        let (center, neighbors, missing) = parse_graph_fallback_parts(&slim, "o/r").unwrap();
        assert_eq!(center.number, 7);
        // Deduped in first-seen order.
        assert_eq!(missing, vec![9, 10]);
        assert!(neighbors.iter().any(|(n, _)| n.number == 1), "parent still resolves");
    }

    #[test]
    fn fallback_derives_reference_edges_from_body_refs() {
        let mut slim = graph_payload();
        let issue = slim.pointer_mut("/data/repository/issue").unwrap();
        issue.as_object_mut().unwrap().remove("blockedBy");
        issue.as_object_mut().unwrap().remove("blocking");
        // #7 is the center: only #9 stays missing.
        issue["body"] = json!("follow up in #9, self #7");
        let (center, neighbors, missing) = parse_graph_fallback_parts(&slim, "o/r").unwrap();
        assert_eq!(center.number, 7);
        assert_eq!(missing, vec![9]);
        // Resolved nodes join as References edges under the cap.
        let detail = json!({"data": {
            "r0": {"issueOrPullRequest": {"__typename": "Issue", "number": 9,
                "title": "nine", "state": "OPEN",
                "labels": {"nodes": []}, "projectItems": {"nodes": []}}}
        }});
        let data = detail.get("data").unwrap();
        let raw = data.pointer("/r0/issueOrPullRequest").unwrap();
        let node = parse_graph_typed(raw).unwrap();
        let mut all = neighbors;
        all.push((node, GraphEdgeKind::References));
        let mut data = model::build_graph(center, all);
        data.from_references = true;
        assert!(data.from_references);
        assert!(data.edges.iter().any(|e| e.kind == GraphEdgeKind::References && e.to == 9));
    }

    #[test]
    fn graph_detail_query_aliases_each_number() {
        let q = graph_detail_query("o", "r", &[9, 10]);
        assert!(q.contains("r0:") && q.contains("r1:"));
        assert!(q.contains("issueOrPullRequest(number: 9)"));
        assert!(q.contains("issueOrPullRequest(number: 10)"));
    }

}
