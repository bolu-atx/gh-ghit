//! A `/` query as JQL: the terms Jira shares with the GitHub search, and
//! the points where a query asks for PRs only.

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

use crate::core::query::tokens;

use super::search::quote;

/// One translated term.
enum Clause {
    /// A JQL condition.
    Where(String),
    /// A term Jira matches without a condition (`is:issue`).
    Any,
    /// A PR-only term: Jira has no rows for it.
    Pr,
}

/// A qualifier value Jira can compare. Empty values and `@me` are
/// GitHub-only, and an unquoted comma is GitHub's OR.
fn value_text(value: &str, quoted: bool) -> Option<&str> {
    let bad = value.is_empty() || value.starts_with('@') || (!quoted && value.contains(','));
    (!bad).then_some(value)
}

fn clause(token: &str, quoted: bool) -> Option<Clause> {
    let Some((key, value)) = token.split_once(':') else {
        // A bare word is full-text. `-word` negates, which this
        // translation does not cover.
        if token.is_empty() || token.starts_with('-') {
            return None;
        }
        return Some(Clause::Where(format!("text ~ {}", quote(token))));
    };
    let value = value_text(value, quoted)?;
    Some(match key.to_ascii_lowercase().as_str() {
        "is" => match value.to_ascii_lowercase().as_str() {
            "open" => Clause::Where("statusCategory != Done".to_string()),
            "closed" => Clause::Where("statusCategory = Done".to_string()),
            "issue" => Clause::Any,
            "pr" | "merged" => Clause::Pr,
            _ => return None,
        },
        "label" => Clause::Where(format!("labels = {}", quote(value))),
        "-label" => Clause::Where(format!("labels != {}", quote(value))),
        "author" => Clause::Where(format!("reporter = {}", quote(value))),
        "assignee" => Clause::Where(format!("assignee = {}", quote(value))),
        "milestone" => Clause::Where(format!("fixVersion = {}", quote(value))),
        "no" if value.eq_ignore_ascii_case("milestone") => {
            Clause::Where("fixVersion is EMPTY".to_string())
        }
        "updated" => Clause::Where(format!("updated >= {}", quote(value.strip_prefix(">=")?))),
        _ => return None,
    })
}

/// The JQL of a `/` query on `project`, or `None` when Jira sits the
/// search out: an empty query, a PR-only term, or a term with no JQL
/// equivalent.
pub(crate) fn from_search(project: &str, text: &str) -> Option<String> {
    let terms = tokens(text)?;
    if terms.is_empty() {
        return None;
    }
    let mut clauses: Vec<String> = Vec::new();
    for (token, quoted) in terms {
        match clause(&token, quoted)? {
            Clause::Where(sql) => clauses.push(sql),
            Clause::Any => {}
            Clause::Pr => return None,
        }
    }
    let project = quote(project);
    if clauses.is_empty() {
        return Some(format!("project = {project} ORDER BY updated DESC"));
    }
    Some(format!("project = {project} AND {} ORDER BY updated DESC", clauses.join(" AND ")))
}
