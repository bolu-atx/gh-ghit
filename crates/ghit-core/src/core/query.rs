//! Local answer to a `/` query: parse the GitHub search text into terms,
//! filter stored rows with them, and decide whether the store holds every
//! row the query could match. Any term the parser does not know makes the
//! whole query a server search.

use chrono::{DateTime, Months, NaiveDate, Utc};

use crate::model::IssueData;

/// One parsed search term. A query matches a row when every term does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Term {
    Open,
    /// Closed issues and PRs, merged ones included, as on GitHub.
    Closed,
    Merged,
    Issue,
    Pr,
    Label(String),
    NotLabel(String),
    Author(String),
    Assignee(String),
    Milestone(String),
    NoMilestone,
    /// `updated:>=DATE`: the row's last update is on or after this day.
    UpdatedSince(NaiveDate),
    /// A bare title word or quoted phrase, lowercased.
    Word(String),
}

/// A query every term of which the local filter understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalQuery(Vec<Term>);

/// Split on spaces outside double quotes. Quotes drop out of the token, so
/// `label:"good first"` is one token `label:good first`. The flag says the
/// token had quotes. `None` for an unterminated quote.
fn tokens(text: &str) -> Option<Vec<(String, bool)>> {
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut in_quote) = (String::new(), false, false);
    for c in text.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                quoted = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() || quoted {
                    out.push((std::mem::take(&mut cur), quoted));
                }
                quoted = false;
            }
            c => cur.push(c),
        }
    }
    if in_quote {
        return None;
    }
    if !cur.is_empty() || quoted {
        out.push((cur, quoted));
    }
    Some(out)
}

/// A lowercased qualifier value. Empty values, `@me` and an unquoted
/// comma (GitHub reads it as OR) are not local.
fn plain(value: &str, quoted: bool) -> Option<String> {
    let bad = value.is_empty() || value.starts_with('@') || (!quoted && value.contains(','));
    (!bad).then(|| value.to_lowercase())
}

fn term(token: &str, quoted: bool) -> Option<Term> {
    let Some((key, value)) = token.split_once(':') else {
        // A bare `-word` negates; the local filter has no such term.
        return (!token.is_empty() && !token.starts_with('-')).then(|| Term::Word(token.to_lowercase()));
    };
    Some(match key.to_lowercase().as_str() {
        "is" => match value.to_lowercase().as_str() {
            "open" => Term::Open,
            "closed" => Term::Closed,
            "merged" => Term::Merged,
            "issue" => Term::Issue,
            "pr" => Term::Pr,
            _ => return None,
        },
        "label" => Term::Label(plain(value, quoted)?),
        "-label" => Term::NotLabel(plain(value, quoted)?),
        "author" => Term::Author(plain(value, quoted)?),
        "assignee" => Term::Assignee(plain(value, quoted)?),
        "milestone" => Term::Milestone(plain(value, quoted)?),
        "no" if value.eq_ignore_ascii_case("milestone") => Term::NoMilestone,
        "updated" => Term::UpdatedSince(NaiveDate::parse_from_str(value.strip_prefix(">=")?, "%Y-%m-%d").ok()?),
        _ => return None,
    })
}

impl LocalQuery {
    /// `None` when any term is unknown to the local parser, or the text
    /// is empty.
    pub fn parse(text: &str) -> Option<Self> {
        let terms = tokens(text)?
            .into_iter()
            .map(|(token, quoted)| term(&token, quoted))
            .collect::<Option<Vec<_>>>()?;
        (!terms.is_empty()).then_some(Self(terms))
    }

    /// True when `issue` satisfies every term.
    pub fn matches(&self, issue: &IssueData) -> bool {
        self.0.iter().all(|term| term_matches(term, issue))
    }

    /// True when the store holds every row this query could match. Open
    /// rows are stored once the open backfill is done. Closed and merged
    /// rows are stored back to the age limit, so a query that can match
    /// them needs an `updated:` bound inside that limit. `open_done` and
    /// `closed_done` say which backfill walks finished.
    pub fn covered(&self, now: DateTime<Utc>, months: u32, open_done: bool, closed_done: bool) -> bool {
        if !open_done {
            return false;
        }
        if self.0.contains(&Term::Open) {
            return true;
        }
        if !closed_done {
            return false;
        }
        if months == 0 {
            return true;
        }
        let Some(cutoff) = now.checked_sub_months(Months::new(months)) else { return false };
        let bound = self.0.iter().filter_map(|term| match term {
            Term::UpdatedSince(date) => Some(*date),
            _ => None,
        });
        bound.max().is_some_and(|date| date >= cutoff.date_naive())
    }
}

fn term_matches(term: &Term, issue: &IssueData) -> bool {
    let has_label = |name: &str| issue.labels.iter().any(|l| l.name.to_lowercase() == name);
    match term {
        Term::Open => issue.state == "OPEN",
        Term::Closed => matches!(issue.state.as_str(), "CLOSED" | "MERGED"),
        Term::Merged => issue.is_pr && issue.state == "MERGED",
        Term::Issue => !issue.is_pr,
        Term::Pr => issue.is_pr,
        Term::Label(name) => has_label(name),
        Term::NotLabel(name) => !has_label(name),
        Term::Author(login) => issue.author.login.to_lowercase() == *login,
        Term::Assignee(login) => issue.assignees.iter().any(|a| a.login.to_lowercase() == *login),
        Term::Milestone(title) => issue.milestone.as_ref().is_some_and(|m| m.to_lowercase() == *title),
        Term::NoMilestone => issue.milestone.is_none(),
        Term::UpdatedSince(date) => issue
            .updated_at
            .get(..10)
            .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
            .is_some_and(|day| day >= *date),
        Term::Word(word) => issue.title.to_lowercase().contains(word.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Author, Label};

    fn query(text: &str) -> LocalQuery {
        LocalQuery::parse(text).expect("local")
    }

    fn issue(title: &str) -> IssueData {
        IssueData {
            title: title.to_string(),
            state: "OPEN".to_string(),
            author: Author { login: "Bo".to_string() },
            updated_at: "2026-11-10T08:00:00Z".to_string(),
            ..Default::default()
        }
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_each_qualifier() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(
            query("is:open is:Closed is:merged is:issue is:pr").0,
            vec![Term::Open, Term::Closed, Term::Merged, Term::Issue, Term::Pr]
        );
        assert_eq!(
            query("label:Bug -label:wip author:Bo assignee:amy milestone:v1 no:milestone updated:>=2026-11-09").0,
            vec![
                Term::Label("bug".into()), Term::NotLabel("wip".into()), Term::Author("bo".into()),
                Term::Assignee("amy".into()), Term::Milestone("v1".into()), Term::NoMilestone,
                Term::UpdatedSince(day),
            ]
        );
        assert_eq!(
            query("Crash  on").0,
            vec![Term::Word("crash".into()), Term::Word("on".into())]
        );
    }

    #[test]
    fn quoted_values_keep_their_spaces() {
        assert_eq!(query(r#"label:"Good First Issue" -label:"help wanted""#).0, vec![
            Term::Label("good first issue".into()), Term::NotLabel("help wanted".into()),
        ]);
        assert_eq!(query(r#""two words""#).0, vec![Term::Word("two words".into())]);
        assert_eq!(query(r#"milestone:"v1, final""#).0, vec![Term::Milestone("v1, final".into())]);
    }

    #[test]
    fn unknown_terms_send_the_query_to_the_server() {
        for text in [
            "in:body crash", "reviewed-by:amy", "review:approved", "sort:updated", "is:draft", "-author:bo",
            "-is:open", "no:label", "assignee:@me", "label:a,b", "updated:<2026-11-09", "updated:>2026-11-09",
            "updated:>=2026-11", "-crash", r#"label:"open"#, "", "   ", "base:main label:bug",
        ] {
            assert!(LocalQuery::parse(text).is_none(), "{text:?} must go to the server");
        }
    }

    #[test]
    fn filters_each_qualifier() {
        let mut row = issue("Crash on start");
        row.labels = vec![Label { name: "Bug".into() }, Label { name: "good first issue".into() }];
        row.assignees = vec![Author { login: "Amy".into() }];
        row.milestone = Some("V1".into());
        let yes = [
            "is:open", "is:issue", "label:bug", "label:\"good first issue\"", "-label:wip", "author:bo",
            "assignee:amy", "milestone:v1", "updated:>=2026-11-10", "crash", "START crash",
        ];
        let no = [
            "is:closed", "is:merged", "is:pr", "label:wip", "-label:bug", "author:amy", "assignee:bo",
            "milestone:v2", "no:milestone", "updated:>=2026-11-11", "crash missing",
        ];
        for text in yes {
            assert!(query(text).matches(&row), "{text} should match");
        }
        for text in no {
            assert!(!query(text).matches(&row), "{text} should not match");
        }
        assert!(query("no:milestone").matches(&issue("x")));
    }

    #[test]
    fn state_terms_follow_github() {
        let mut merged = issue("m");
        merged.is_pr = true;
        merged.state = "MERGED".into();
        let mut closed = issue("c");
        closed.state = "CLOSED".into();
        assert!(query("is:merged is:pr").matches(&merged) && query("is:closed").matches(&merged));
        assert!(!query("is:open").matches(&merged) && !query("is:merged").matches(&closed));
        assert!(query("is:closed is:issue").matches(&closed));
    }

    #[test]
    fn combined_terms_all_apply() {
        let row = issue("Crash on start");
        assert!(query("is:open author:bo crash").matches(&row));
        assert!(!query("is:open author:amy crash").matches(&row));
    }

    #[test]
    fn horizon_decides_between_store_and_server() {
        let now = at("2026-11-18T00:00:00Z");
        let cov = |text: &str, months, open, closed| query(text).covered(now, months, open, closed);
        // Open rows are always stored once the open walks finished.
        assert!(cov("is:open label:bug", 12, true, false));
        assert!(!cov("is:open label:bug", 12, false, false), "backfill still running");
        // Closed rows need a bound inside the horizon (cutoff 2025-11-18).
        assert!(cov("is:closed updated:>=2025-11-18", 12, true, true));
        assert!(cov("is:merged updated:>=2026-01-01", 12, true, true));
        assert!(!cov("is:closed updated:>=2025-11-17", 12, true, true), "bound reaches past the horizon");
        assert!(!cov("is:closed", 12, true, true), "no bound reaches all history");
        assert!(!cov("label:bug", 12, true, true), "no state term can match old closed rows");
        assert!(!cov("is:closed updated:>=2026-01-01", 12, true, false), "closed walks unfinished");
        assert!(cov("is:closed updated:>=2025-11-17 updated:>=2026-01-01", 12, true, true), "the latest bound counts");
        // Zero months backfills all history: any query is covered.
        assert!(cov("is:closed", 0, true, true));
    }
}
