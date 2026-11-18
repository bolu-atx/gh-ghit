//! Issue payload parsing: review threads, will-close refs, CI rollup, comment pages.

use super::*;

/// Numeric REST id of a thread's first comment: the `in_reply_to`
/// target for a REST reply inside that thread. `None` when the thread
/// carries no comment with a numeric id (for example an old cache).
pub fn thread_reply_comment_id(threads: &[ReviewThread], thread_id: &str) -> Option<u64> {
    threads
        .iter()
        .find(|t| t.id == thread_id)?
        .comments
        .first()?
        .database_id
}

pub fn parse_review_threads(value: &serde_json::Value) -> Vec<ReviewThread> {
    let nodes = value.pointer("/reviewThreads/nodes").and_then(serde_json::Value::as_array);
    let mut threads = Vec::new();
    for node in nodes.into_iter().flatten() {
        if !node.is_object() { continue; }
        let mut thread = node.clone();
        thread["comments"] = node.pointer("/comments/nodes").cloned().unwrap_or_else(|| serde_json::json!([]));
        if let Ok(thread) = serde_json::from_value::<ReviewThread>(thread) { threads.push(thread); }
    }
    threads.sort_by_key(|thread| thread.is_resolved);
    threads
}

/// Parse the will-close connection of one issue or PR payload. Issues
/// carry closing PRs under `closedByPullRequestsReferences`; PRs carry
/// closing issues under `closingIssuesReferences`. Missing or malformed
/// nodes yield an empty list.
pub fn paired_refs(value: &serde_json::Value, is_pr: bool) -> Vec<PairedRef> {
    let field = if is_pr {
        "closingIssuesReferences"
    } else {
        "closedByPullRequestsReferences"
    };
    let nodes = value
        .pointer(&format!("/{field}/nodes"))
        .and_then(serde_json::Value::as_array);
    let mut out = Vec::new();
    for node in nodes.into_iter().flatten() {
        let Some(number) = node.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        out.push(PairedRef {
            number,
            state: node.get("state").and_then(serde_json::Value::as_str).unwrap_or("").to_owned(),
        });
    }
    out
}

/// Merge independently fetched pages in page order.
pub fn merge_comment_pages(mut pages: Vec<(usize, Vec<Comment>)>) -> Vec<Comment> {
    pages.sort_by_key(|(page, _)| *page);
    pages.into_iter().flat_map(|(_, comments)| comments).collect()
}

/// Parse the head-commit rollup of one sections-query PR node. Missing
/// commits or a missing/null rollup yield `None`; an unrecognized state
/// yields `Unknown`, never an error.
pub fn parse_check_state(value: &serde_json::Value) -> Option<CheckState> {
    let state = value
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(serde_json::Value::as_str)?;
    match state {
        "SUCCESS" => Some(CheckState::Pass),
        "FAILURE" | "ERROR" => Some(CheckState::Fail),
        "PENDING" | "EXPECTED" => Some(CheckState::Pending),
        _ => Some(CheckState::Unknown),
    }
}
