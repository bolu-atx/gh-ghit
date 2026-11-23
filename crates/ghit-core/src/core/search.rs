//! Content search: where a query matches in one line of text.

/// Smart case: a query with any uppercase letter matches case-sensitively.
pub(crate) fn case_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

fn fold(c: char, sensitive: bool) -> char {
    if sensitive { c } else { c.to_lowercase().next().unwrap_or(c) }
}

/// Non-overlapping matches of `query` in `line`, as half-open ranges of
/// char indices (not bytes), left to right. An empty query matches nothing.
pub fn find_ranges(line: &str, query: &str) -> Vec<(usize, usize)> {
    let sensitive = case_sensitive(query);
    let needle: Vec<char> = query.chars().map(|c| fold(c, sensitive)).collect();
    if needle.is_empty() {
        return vec![];
    }
    let hay: Vec<char> = line.chars().map(|c| fold(c, sensitive)).collect();
    let mut found = vec![];
    let mut at = 0;
    while at + needle.len() <= hay.len() {
        if hay[at..at + needle.len()] == needle[..] {
            found.push((at, at + needle.len()));
            at += needle.len();
        } else {
            at += 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_query_ignores_case() {
        assert_eq!(find_ranges("Foo foo FOO", "foo"), vec![(0, 3), (4, 7), (8, 11)]);
    }

    #[test]
    fn any_uppercase_makes_it_case_sensitive() {
        assert_eq!(find_ranges("Foo foo FOO", "Foo"), vec![(0, 3)]);
        assert_eq!(find_ranges("Foo foo FOO", "FOO"), vec![(8, 11)]);
    }

    #[test]
    fn several_matches_per_line_do_not_overlap() {
        assert_eq!(find_ranges("aaaa", "aa"), vec![(0, 2), (2, 4)]);
        assert_eq!(find_ranges("abab ab", "ab"), vec![(0, 2), (2, 4), (5, 7)]);
    }

    #[test]
    fn indices_count_chars_not_bytes() {
        assert_eq!(find_ranges("é日本 日本", "日本"), vec![(1, 3), (4, 6)]);
        assert_eq!(find_ranges("ÉCOLE école", "école"), vec![(0, 5), (6, 11)]);
    }

    #[test]
    fn empty_query_or_line_matches_nothing() {
        assert!(find_ranges("abc", "").is_empty());
        assert!(find_ranges("", "a").is_empty());
        assert!(find_ranges("ab", "abc").is_empty());
    }
}
