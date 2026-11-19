//! Store backfill queries: one GraphQL connection per call, 100 slim
//! nodes a page, newest update first. The walk of a connection stops at
//! the age limit or the last page.

use super::client::{client, graphql_auth_failure, graphql_url, parse_graphql_cost, post_json, RATE_LIMIT_FIELD};
use super::sections::{parse_node, ISSUE_FIELDS, PR_FIELDS};
use super::Row;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// The four connections of a repo's working set. Open items come first
/// and ignore the age limit; closed ones stop at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    PrsOpen,
    IssuesOpen,
    PrsClosed,
    IssuesClosed,
}

impl Conn {
    pub const ALL: [Conn; 4] = [Conn::PrsOpen, Conn::IssuesOpen, Conn::PrsClosed, Conn::IssuesClosed];

    /// Key of the connection's progress row in the store.
    pub fn name(self) -> &'static str {
        match self {
            Conn::PrsOpen => "prs_open",
            Conn::IssuesOpen => "issues_open",
            Conn::PrsClosed => "prs_closed",
            Conn::IssuesClosed => "issues_closed",
        }
    }

    fn is_pr(self) -> bool {
        matches!(self, Conn::PrsOpen | Conn::PrsClosed)
    }

    fn is_open(self) -> bool {
        matches!(self, Conn::PrsOpen | Conn::IssuesOpen)
    }
}

/// One parsed page of a connection.
#[derive(Debug)]
pub struct Page {
    pub rows: Vec<Row>,
    /// The connection's `totalCount`.
    pub total: u64,
    /// Cursor of the next page; `None` once the connection is done.
    pub next: Option<String>,
    pub done: bool,
    /// `(remaining points, reset unix second)` when the reply reports it.
    pub quota: Option<(u64, u64)>,
}

/// The query for one page of `conn`; `$after` is the stored cursor.
pub fn query(conn: Conn) -> String {
    let (field, states, fields) = match conn {
        Conn::PrsOpen => ("pullRequests", "OPEN", PR_FIELDS),
        Conn::PrsClosed => ("pullRequests", "[CLOSED, MERGED]", PR_FIELDS),
        Conn::IssuesOpen => ("issues", "OPEN", ISSUE_FIELDS),
        Conn::IssuesClosed => ("issues", "CLOSED", ISSUE_FIELDS),
    };
    format!(
        "query($owner: String!, $repo: String!, $after: String) {{\n repository(owner: $owner, name: $repo) {{\n {field}(first: 100, states: {states}, after: $after, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ totalCount nodes {{ {fields} }} pageInfo {{ hasNextPage endCursor }} }} }}\n {RATE_LIMIT_FIELD} }}"
    )
}

/// Oldest `updatedAt` the backfill keeps; `None` (0 months) keeps all.
pub fn age_cutoff(now: DateTime<Utc>, months: u32) -> Option<DateTime<Utc>> {
    if months == 0 {
        return None;
    }
    now.checked_sub_months(chrono::Months::new(months))
}

fn older_than(node: &Value, cutoff: DateTime<Utc>) -> bool {
    node.get("updatedAt")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| t.with_timezone(&Utc) < cutoff)
}

/// Rows of one reply. A closed connection drops nodes older than
/// `cutoff` and is done once its last node is (the list is updated-desc,
/// so no later page holds a newer one). An open one never stops early.
/// The window anchor matches the sync's: `mergedAt` for merged PRs,
/// `updatedAt` for closed issues, none otherwise.
pub fn parse_page(value: &Value, conn: Conn, cutoff: Option<DateTime<Utc>>) -> Result<Page, String> {
    if let Some(e) = graphql_auth_failure(value) {
        return Err(e);
    }
    let field = if conn.is_pr() { "pullRequests" } else { "issues" };
    let list = value
        .pointer(&format!("/data/repository/{field}"))
        .filter(|v| v.is_object())
        .ok_or_else(|| format!("GitHub GraphQL: {}", value.get("errors").unwrap_or(&Value::Null)))?;
    let nodes: &[Value] = list.get("nodes").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let limit = cutoff.filter(|_| !conn.is_open());
    let mut rows = Vec::new();
    for n in nodes {
        if limit.is_some_and(|c| older_than(n, c)) {
            continue;
        }
        let anchor = match (conn.is_pr(), n.get("state").and_then(Value::as_str)) {
            (true, Some("MERGED")) => Some("mergedAt"),
            (false, Some("CLOSED")) => Some("updatedAt"),
            _ => None,
        };
        let window_at = anchor.and_then(|k| n.get(k)?.as_str()).map(str::to_owned);
        rows.push(parse_node(n, conn.is_pr(), window_at)?);
    }
    let past_age = limit.is_some_and(|c| nodes.last().is_some_and(|n| older_than(n, c)));
    let info = list.get("pageInfo");
    let more = info.and_then(|v| v.get("hasNextPage")).and_then(Value::as_bool).unwrap_or(false);
    let cursor = info.and_then(|v| v.get("endCursor")).and_then(Value::as_str).map(str::to_owned);
    let done = past_age || !more || cursor.is_none();
    Ok(Page {
        rows,
        total: list.get("totalCount").and_then(Value::as_u64).unwrap_or(0),
        next: if done { None } else { cursor },
        done,
        quota: parse_graphql_cost(value).map(|(_, remaining, reset)| (remaining, reset)),
    })
}

/// Fetch one page of `conn` after `cursor`.
pub async fn fetch_page(
    repo: &str,
    conn: Conn,
    cursor: Option<&str>,
    cutoff: Option<DateTime<Utc>>,
    bearer: &str,
) -> Result<Page, String> {
    let (owner, name) = repo.split_once('/').ok_or("repository must be owner/repo")?;
    let body = json!({"query": query(conn), "variables": {"owner": owner, "repo": name, "after": cursor}});
    let value = post_json(&client(), &graphql_url(), bearer, &body).await?;
    parse_page(&value, conn, cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn node(number: u64, state: &str, updated: &str) -> Value {
        json!({"number": number, "title": "t", "state": state, "createdAt": "2020-01-01T00:00:00Z",
               "updatedAt": updated, "mergedAt": updated, "url": "u"})
    }

    fn reply(field: &str, nodes: Vec<Value>, more: bool) -> Value {
        json!({"data": {"repository": {field: {"totalCount": 42, "nodes": nodes,
            "pageInfo": {"hasNextPage": more, "endCursor": "c1"}}},
            "rateLimit": {"cost": 2, "remaining": 4000, "resetAt": "2026-11-18T13:00:00Z"}}})
    }

    #[test]
    fn query_is_slim_ordered_paged_and_costed() {
        for conn in Conn::ALL {
            let q = query(conn);
            assert!(q.contains("first: 100") && q.contains("after: $after"), "{conn:?}");
            assert!(q.contains("orderBy: {field: UPDATED_AT, direction: DESC}"));
            assert!(q.contains(RATE_LIMIT_FIELD) && q.contains("totalCount") && q.contains("endCursor"));
        }
        assert!(query(Conn::PrsOpen).contains("pullRequests(first: 100, states: OPEN"));
        assert!(query(Conn::PrsClosed).contains("states: [CLOSED, MERGED]"));
        assert!(query(Conn::IssuesClosed).contains("issues(first: 100, states: CLOSED"));
        assert!(query(Conn::PrsOpen).contains("statusCheckRollup") && !query(Conn::IssuesOpen).contains("statusCheckRollup"));
        assert!(query(Conn::IssuesOpen).contains("subIssuesSummary"), "same fields as the sync");
    }

    #[test]
    fn connection_names_are_distinct() {
        let mut names: Vec<_> = Conn::ALL.iter().map(|c| c.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 4);
    }

    #[test]
    fn age_cutoff_counts_months_and_zero_means_all() {
        let now = at("2026-11-18T00:00:00Z");
        assert_eq!(age_cutoff(now, 12), Some(at("2025-11-18T00:00:00Z")));
        assert_eq!(age_cutoff(now, 0), None);
    }

    #[test]
    fn closed_connection_stops_at_the_age_limit() {
        let cutoff = Some(at("2025-11-18T00:00:00Z"));
        let v = reply("issues", vec![node(3, "CLOSED", "2026-01-01T00:00:00Z"), node(2, "CLOSED", "2025-06-01T00:00:00Z")], true);
        let page = parse_page(&v, Conn::IssuesClosed, cutoff).unwrap();
        assert_eq!(page.rows.iter().map(|r| r.issue.number).collect::<Vec<_>>(), vec![3], "older node dropped");
        assert!(page.done && page.next.is_none(), "the last node is past the limit");
        // Without a limit the same page continues.
        let page = parse_page(&v, Conn::IssuesClosed, None).unwrap();
        assert_eq!((page.rows.len(), page.done, page.next.as_deref()), (2, false, Some("c1")));
    }

    #[test]
    fn open_connection_ignores_the_age_limit() {
        let cutoff = Some(at("2025-11-18T00:00:00Z"));
        let v = reply("pullRequests", vec![node(1, "OPEN", "2020-01-01T00:00:00Z")], true);
        let page = parse_page(&v, Conn::PrsOpen, cutoff).unwrap();
        assert_eq!((page.rows.len(), page.done, page.next.as_deref()), (1, false, Some("c1")));
        assert_eq!(page.total, 42);
        assert_eq!(page.quota, Some((4000, 1_795_006_800)));
    }

    #[test]
    fn last_page_is_done_and_anchors_match_the_sync() {
        let v = reply("pullRequests", vec![node(5, "MERGED", "2026-02-01T00:00:00Z"), node(4, "CLOSED", "2026-01-01T00:00:00Z")], false);
        let page = parse_page(&v, Conn::PrsClosed, None).unwrap();
        assert!(page.done && page.next.is_none());
        assert_eq!(page.rows[0].window_at.as_deref(), Some("2026-02-01T00:00:00Z"));
        assert_eq!(page.rows[1].window_at, None, "closed, unmerged PRs have no list");
    }

    #[test]
    fn missing_connection_is_an_error() {
        assert!(parse_page(&json!({"errors": [{"message": "boom"}]}), Conn::IssuesOpen, None).is_err());
    }
}
