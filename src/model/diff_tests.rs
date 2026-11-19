//! Tests for model::diff.

use super::*;
use super::diff::patch_new_lines;

#[test]
fn diff_lines_tag_hunks_adds_and_dels() {
    let patch = "@@ -1,2 +1,2 @@\n context\n-old\n+new";
    let kinds: Vec<DiffKind> = diff_lines(patch).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, vec![DiffKind::Hunk, DiffKind::Context, DiffKind::Del, DiffKind::Add]);
    assert!(diff_lines("").is_empty());
}

#[test]
fn split_rows_pair_changes_and_keep_context() {
    let rows = split_rows("@@ -1,3 +1,3 @@\n ctx\n-old\n+new\ntrail");
    assert_eq!(rows[0], SplitRow::Hunk("@@ -1,3 +1,3 @@".to_string()));
    assert_eq!(rows[1], SplitRow::Pair { left: " ctx".to_string(), right: " ctx".to_string(), old: Some(1), new: Some(1) });
    assert_eq!(rows[2], SplitRow::Pair { left: "-old".to_string(), right: "+new".to_string(), old: Some(2), new: Some(2) });
    assert_eq!(rows[3], SplitRow::Pair { left: "trail".to_string(), right: "trail".to_string(), old: Some(3), new: Some(3) });
}

#[test]
fn split_rows_leave_unpaired_lines_alone() {
    let rows = split_rows("@@ -1 +1 @@\n-a\n-b\n+c");
    assert_eq!(rows[1], SplitRow::Pair { left: "-a".to_string(), right: "+c".to_string(), old: Some(1), new: Some(1) });
    assert_eq!(rows[2], SplitRow::LeftOnly { text: "-b".to_string(), kind: DiffKind::Del, line: Some(2) });
    assert!(split_rows("").is_empty());
}

#[test]
fn patch_line_numbers_tracks_both_sides_across_hunks() {
    let numbers = patch_line_numbers("@@ -1,2 +10,3 @@\n ctx\n-old\n+new\n+more\n@@ -5,1 +15,1 @@\n tail");
    assert_eq!(numbers, vec![
        (None, None),
        (Some(1), Some(10)),
        (Some(2), None),
        (None, Some(11)),
        (None, Some(12)),
        (None, None),
        (Some(5), Some(15)),
    ]);
    // `\ No newline` markers carry no line on either side.
    let numbers = patch_line_numbers("@@ -1 +1 @@\n-old\n+new\n\\ No newline at end of file");
    assert_eq!(numbers, vec![(None, None), (Some(1), None), (None, Some(1)), (None, None)]);
}

#[test]
fn cursor_maps_guttered_rows_to_new_side_lines() {
    // Rows render with a line-number gutter, exactly as `ui` emits.
    let patch = "@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n+more\n tail";
    let section: Vec<String> = ["     │ @@ -1,3 +1,3 @@", "   1 │  ctx", "   2 │ -old", "   2 │ +new", "   3 │ +more", "   3 │  tail"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(line_target_at_cursor(&section, 0, patch), None, "hunk header has no line");
    assert_eq!(line_target_at_cursor(&section, 1, patch), Some(1));
    assert_eq!(line_target_at_cursor(&section, 2, patch), Some(1), "deletion keeps the line above");
    assert_eq!(line_target_at_cursor(&section, 3, patch), Some(2));
    assert_eq!(line_target_at_cursor(&section, 5, patch), Some(4));
}

#[test]
fn diff_summary_counts_files_and_lines() {
    let files = vec![
        DiffFile { filename: "a".into(), status: "modified".into(), additions: 10, deletions: 2, patch: String::new() },
        DiffFile { filename: "b".into(), status: "added".into(), additions: 5, deletions: 0, patch: String::new() },
    ];
    assert_eq!(diff_summary(&files), "2 files · +15 -2");
    assert_eq!(diff_summary(&files[..1]), "1 file · +10 -2");
}

#[test]
fn header_file_index_matches_longest_filename() {
    let files = vec![
        DiffFile { filename: "a.rs".into(), status: String::new(), additions: 0, deletions: 0, patch: String::new() },
        DiffFile { filename: "src/a.rs".into(), status: String::new(), additions: 0, deletions: 0, patch: String::new() },
    ];
    assert_eq!(header_file_index("▸ \u{f15c} src/a.rs   modified", &files), Some(1));
    assert_eq!(header_file_index("▾ \u{f15c} a.rs   added", &files), Some(0));
    assert_eq!(header_file_index("  -old", &files), None);
    assert_eq!(header_file_index("───", &files), None);
}

#[test]
fn matches_new_side_and_falls_back_for_missing_lines() {
    let file = DiffFile { filename: "a".into(), patch: "@@ -2,2 +4,2 @@\n-old\n+new\n context".into(), ..Default::default() };
    let mut thread = ReviewThread { path: "a".into(), line: Some(4), ..Default::default() };
    assert_eq!(thread_patch_row(&thread, &file), Some(2));
    thread.line = Some(2);
    assert_eq!(thread_patch_row(&thread, &file), None);
    thread.line = Some(4); thread.path = "b".into();
    assert_eq!(thread_patch_row(&thread, &file), None);
}
#[test]
fn cursor_maps_rendered_rows_to_new_side_lines() {
    // Patch rows render with a two-cell dent, exactly as `ui` emits.
    let patch = "@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n+more\n tail";
    let section: Vec<String> = ["  @@ -1,3 +1,3 @@", "   ctx", "  -old", "  +new", "  +more", "   tail"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(line_target_at_cursor(&section, 0, patch), None, "hunk header has no line");
    assert_eq!(line_target_at_cursor(&section, 1, patch), Some(1));
    assert_eq!(line_target_at_cursor(&section, 2, patch), Some(1), "deletion keeps the line above");
    assert_eq!(line_target_at_cursor(&section, 3, patch), Some(2));
    assert_eq!(line_target_at_cursor(&section, 5, patch), Some(4));
}
#[test]
fn cursor_inside_thread_block_answers_thread_id() {
    let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,2 +1,2 @@\n a\n+b".into(), ..Default::default() };
    let threads = vec![ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(2), ..Default::default() }];
    let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "  +b", "      \u{f075} line 2 · unresolved", "      reviewer"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(thread_at_cursor(&section, 0, &file, &threads), None, "hunk header");
    assert_eq!(thread_at_cursor(&section, 1, &file, &threads), None, "code row keeps line target");
    assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "own patch row keeps line target");
    assert_eq!(thread_at_cursor(&section, 3, &file, &threads), Some("t1".into()), "thread header");
    assert_eq!(thread_at_cursor(&section, 4, &file, &threads), Some("t1".into()), "author row");
}
#[test]
fn thread_headers_after_one_row_resolve_in_order() {
    let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,2 +1,2 @@\n a\n+b".into(), ..Default::default() };
    let threads = vec![
        ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(2), ..Default::default() },
        ReviewThread { id: "t2".into(), path: "a.rs".into(), line: Some(2), ..Default::default() },
    ];
    let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "  +b",
        "      \u{f075} line 2 · unresolved", "      \u{f075} line 2 · unresolved"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "own patch row");
    assert_eq!(thread_at_cursor(&section, 3, &file, &threads), Some("t1".into()));
    assert_eq!(thread_at_cursor(&section, 4, &file, &threads), Some("t2".into()));
}
#[test]
fn thread_without_a_patch_row_resolves_in_the_leading_zone() {
    let file = DiffFile { filename: "a.rs".into(), patch: "@@ -1,1 +1,1 @@\n a".into(), ..Default::default() };
    let threads = vec![ReviewThread { id: "t9".into(), path: "a.rs".into(), line: None, ..Default::default() }];
    let section: Vec<String> = ["    \u{f075} line unavailable · unresolved", "  @@ -1,1 +1,1 @@", "   a"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(thread_at_cursor(&section, 0, &file, &threads), Some("t9".into()));
    assert_eq!(thread_at_cursor(&section, 2, &file, &threads), None, "code row");
}
#[test]
fn cursor_skips_thread_and_blank_rows() {
    let patch = "@@ -1,2 +1,2 @@\n a\n+b";
    // Inline review threads, blanks, and wrapped continuations never
    // match a patch row, so the target stays on the last real row.
    let section: Vec<String> = ["  @@ -1,2 +1,2 @@", "   a", "      \u{f075} line 2 · unresolved", "      reviewer", "", "  +b"]
        .iter().map(|s| s.to_string()).collect();
    assert_eq!(line_target_at_cursor(&section, 4, patch), Some(1));
    assert_eq!(line_target_at_cursor(&section, 5, patch), Some(2));
}

fn numbered(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("l{i}")).collect()
}

#[test]
fn widen_patch_adds_context_from_the_head_file() {
    // Line 10 changed; GitHub sent 3 lines of context each side.
    let mut head = numbered(30);
    head[9] = "new".into();
    let patch = "@@ -7,7 +7,7 @@ fn f\n l7\n l8\n l9\n-old\n+new\n l11\n l12\n l13";
    let wide = widen_patch(patch, &head, 5);
    let want = "@@ -5,11 +5,11 @@\n l5\n l6\n l7\n l8\n l9\n-old\n+new\n l11\n l12\n l13\n l14\n l15";
    assert_eq!(wide, want);
    // 3 is what GitHub sent: the same rows come back.
    assert_eq!(diff_lines(&widen_patch(patch, &head, 3)).len(), diff_lines(patch).len());
}

#[test]
fn widen_patch_clamps_at_file_edges_and_merges_hunks() {
    let mut head = numbered(20);
    head[1] = "B".into();
    head[17] = "S".into();
    let patch = "@@ -1,5 +1,5 @@\n l1\n-b\n+B\n l3\n l4\n l5\n@@ -15,6 +15,6 @@\n l15\n l16\n l17\n-s\n+S\n l19\n l20";
    let all = widen_patch(patch, &head, usize::MAX);
    let rows = diff_lines(&all);
    assert_eq!(rows.iter().filter(|(k, _)| *k == DiffKind::Hunk).count(), 1, "windows touch, so hunks merge");
    assert!(all.starts_with("@@ -1,20 +1,20 @@"), "{all}");
    let news: Vec<Option<u64>> = patch_new_lines(&all);
    assert_eq!(news.last().copied().flatten(), Some(20));
    // A small context keeps the hunks apart.
    let five = widen_patch(patch, &head, 5);
    assert_eq!(diff_lines(&five).iter().filter(|(k, _)| *k == DiffKind::Hunk).count(), 2);
}

#[test]
fn widen_patch_keeps_new_files_and_bad_patches() {
    let added = "@@ -0,0 +1,2 @@\n+a\n+b";
    assert_eq!(widen_patch(added, &["a".into(), "b".into()], 25), "@@ -0,0 +1,2 @@\n+a\n+b");
    assert_eq!(widen_patch("garbage", &[], 25), "garbage");
}
