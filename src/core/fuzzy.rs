//! Fuzzy scoring for the Ctrl+P finder: a case-insensitive subsequence
//! match with bonuses for contiguous runs, word starts and a number
//! prefix. Pure: text in, score and matched positions out.

const CONTIGUOUS: i32 = 8;
const WORD_START: i32 = 6;
/// A query that is a number prefix of the leading `#number` beats any
/// other kind of match; the exact number beats a longer one.
const NUMBER_PREFIX: i32 = 1000;
const NUMBER_EXACT: i32 = 100;
const NONE: i32 = i32::MIN / 2;

/// A successful match. `indices` are char positions in the scored text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Match {
    pub(crate) score: i32,
    pub(crate) indices: Vec<usize>,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn word_start(text: &[char], at: usize) -> bool {
    match at.checked_sub(1).map(|prev| text[prev]) {
        None => true,
        Some(prev) => !prev.is_alphanumeric() || (prev.is_lowercase() && text[at].is_uppercase()),
    }
}

/// Match of a digit query against the leading `#number` of `text`.
fn number_prefix(query: &[char], text: &[char]) -> Option<Match> {
    let digits = match query {
        ['#', rest @ ..] => rest,
        all => all,
    };
    if digits.is_empty() || !digits.iter().all(char::is_ascii_digit) || text.first() != Some(&'#') {
        return None;
    }
    let number: Vec<char> = text[1..].iter().copied().take_while(char::is_ascii_digit).collect();
    if !number.starts_with(digits) {
        return None;
    }
    let exact = number.len() == digits.len();
    Some(Match {
        score: NUMBER_PREFIX + if exact { NUMBER_EXACT } else { 0 },
        indices: (1..=digits.len()).collect(),
    })
}

/// Best alignment of `query` as a subsequence of `text`, or `None`.
/// Whitespace in the query is ignored. An empty query matches with a
/// zero score and no indices.
pub(crate) fn score(query: &str, text: &str) -> Option<Match> {
    let query: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).collect();
    if query.is_empty() {
        return Some(Match { score: 0, indices: vec![] });
    }
    let text: Vec<char> = text.chars().collect();
    let (rows, cols) = (query.len(), text.len());
    let prefix = number_prefix(&query, &text);
    if rows > cols {
        return prefix;
    }
    let want: Vec<char> = query.iter().map(|c| fold(*c)).collect();
    let have: Vec<char> = text.iter().map(|c| fold(*c)).collect();
    let mut best = vec![vec![NONE; cols]; rows];
    let mut from = vec![vec![0usize; cols]; rows];
    for i in 0..rows {
        // Best earlier cell reachable with a gap, as `score + column`.
        let mut gapped: Option<(i32, usize)> = None;
        for j in 0..cols {
            if i > 0 && j >= 2 && best[i - 1][j - 2] > NONE {
                let value = best[i - 1][j - 2] + (j - 2) as i32;
                if gapped.is_none_or(|(top, _)| value > top) {
                    gapped = Some((value, j - 2));
                }
            }
            if have[j] != want[i] {
                continue;
            }
            let gain = 1 + if word_start(&text, j) { WORD_START } else { 0 };
            if i == 0 {
                best[0][j] = gain;
                continue;
            }
            if j >= 1 && best[i - 1][j - 1] > NONE {
                best[i][j] = best[i - 1][j - 1] + gain + CONTIGUOUS;
                from[i][j] = j - 1;
            }
            if let Some((top, k)) = gapped {
                let value = top - (j as i32 - 1) + gain;
                if value > best[i][j] {
                    best[i][j] = value;
                    from[i][j] = k;
                }
            }
        }
    }
    let last = (0..cols).filter(|j| best[rows - 1][*j] > NONE).max_by_key(|j| (best[rows - 1][*j], cols - *j));
    let fuzzy = last.map(|mut j| {
        let score = best[rows - 1][j];
        let mut indices = vec![j];
        for i in (1..rows).rev() {
            j = from[i][j];
            indices.push(j);
        }
        indices.reverse();
        Match { score, indices }
    });
    match (prefix, fuzzy) {
        (Some(p), Some(f)) => Some(if p.score >= f.score { p } else { f }),
        (p, f) => p.or(f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rank<'a>(query: &str, items: &[&'a str]) -> Vec<&'a str> {
        let mut hits: Vec<(i32, &str)> =
            items.iter().filter_map(|t| score(query, t).map(|m| (m.score, *t))).collect();
        hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        hits.into_iter().map(|(_, t)| t).collect()
    }

    #[test]
    fn empty_query_matches_everything_with_no_indices() {
        assert_eq!(score("", "#1 anything"), Some(Match { score: 0, indices: vec![] }));
        assert_eq!(score("  ", "#1 anything").map(|m| m.indices), Some(vec![]));
    }

    #[test]
    fn no_subsequence_is_none() {
        assert_eq!(score("zzz", "#1 Fix flaky auth probe"), None);
        assert_eq!(score("probe auth x", "#1 auth probe"), None);
        assert_eq!(score("longer than text", "#1 a"), None);
    }

    #[test]
    fn indices_point_at_the_matched_chars() {
        let m = score("fix", "#241 Fix flaky").expect("match");
        assert_eq!(m.indices, vec![5, 6, 7]);
        let m = score("ap", "#1 auth probe").expect("match");
        assert_eq!(m.indices, vec![3, 8]);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("AUTH", "#1 auth probe").is_some());
        assert!(score("auth", "#1 AUTH PROBE").is_some());
    }

    #[test]
    fn contiguous_runs_rank_above_scattered_letters() {
        let spread = "#4 p r o b e spread";
        assert_eq!(rank("probe", &[spread, "#5 Probe"]), vec!["#5 Probe", spread]);
    }

    #[test]
    fn equal_scores_keep_candidate_order() {
        let items = ["#1 Fix flaky auth probe", "#2 Auth probe retries", "#3 Probe for scopes"];
        assert_eq!(rank("probe", &items), items.to_vec());
    }

    #[test]
    fn word_start_beats_mid_word() {
        assert_eq!(rank("pr", &["#1 sleep rest", "#2 Pull request"]).first(), Some(&"#2 Pull request"));
    }

    #[test]
    fn number_prefix_beats_title_matches() {
        let items = ["#7 Version 24 notes", "#241 Fix", "#24 Fix"];
        assert_eq!(rank("24", &items), vec!["#24 Fix", "#241 Fix", "#7 Version 24 notes"]);
        assert_eq!(score("#24", "#241 Fix").map(|m| m.indices), Some(vec![1, 2]));
        assert_eq!(score("241", "#241 Fix").map(|m| m.indices), Some(vec![1, 2, 3]));
    }

    #[test]
    fn number_query_inside_a_number_is_only_a_fuzzy_match() {
        let inside = score("41", "#241 Fix").expect("fuzzy");
        assert!(inside.score < NUMBER_PREFIX);
    }

    #[test]
    fn spaces_in_the_query_are_ignored() {
        assert!(score("auth probe", "#1 Fix flaky auth probe").is_some());
    }

    #[test]
    fn unicode_text_is_char_indexed() {
        let m = score("é", "#1 Café ünïcode").expect("match");
        assert_eq!(m.indices, vec![6]);
        let m = score("ünï", "#1 Café ünïcode").expect("match");
        assert_eq!(m.indices, vec![8, 9, 10]);
        assert_eq!(score("日本", "#1 日本語 title").map(|m| m.indices), Some(vec![3, 4]));
    }
}
