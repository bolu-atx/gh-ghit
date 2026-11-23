//! Tests for model::sidebar.

use super::*;
use super::sidebar::relative_updated;

use super::test_support::sample_issue;
use crate::model::ItemRef;

fn at(stamp: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(stamp).unwrap().with_timezone(&chrono::Utc)
}

#[test]
fn relative_time_buckets_at_each_boundary() {
    let now = at("2026-11-10T12:00:00Z");
    assert_eq!(relative_updated("2026-11-10T12:00:00Z", now), "0 min ago");
    assert_eq!(relative_updated("2026-11-10T11:00:01Z", now), "59 min ago");
    assert_eq!(relative_updated("2026-11-10T11:00:00Z", now), "1 hr ago");
    assert_eq!(relative_updated("2026-11-09T12:00:01Z", now), "23 hr ago");
    assert_eq!(relative_updated("2026-11-09T12:00:00Z", now), "1 day ago");
    assert_eq!(relative_updated("2026-10-11T12:00:01Z", now), "29 day ago");
    assert_eq!(relative_updated("2026-10-11T12:00:00Z", now), "1 m ago");
    assert_eq!(relative_updated("2025-11-16T12:00:00Z", now), "11 m ago");
    assert_eq!(relative_updated("2025-11-15T12:00:00Z", now), "11/15/25");
    // Future stamps clamp; unparseable stamps collapse the segment.
    assert_eq!(relative_updated("2026-11-10T12:00:01Z", now), "0 min ago");
    assert_eq!(relative_updated("not a date", now), "");
}

#[test]
fn relative_time_old_date_reads_mm_dd_yy() {
    let now = at("2026-11-10T12:00:00Z");
    assert_eq!(relative_updated("2020-01-05T00:00:00Z", now), "01/05/20");
}

#[test]
fn sidebar_row_joins_labels_date_and_status() {
    let d = sample_issue();
    let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
    let now = at("2026-01-07T00:00:00Z");
    let (first, labels, updated, status, _, _) =
        sidebar_lines(&d, &st, now, 40, false, &RowFields::default(), "\u{25cb}");
    assert!(first.starts_with("#1 some title here"), "{first}");
    assert_eq!(labels, "● needs-triage");
    assert_eq!(updated, "2 day ago");
    assert_eq!(status, "\u{25cb} Todo");
    let line = format!("{labels} · {updated} · {status}");
    assert!(line.chars().count() <= 40, "{line}");
}

#[test]
fn sidebar_labels_truncate_before_date_and_status() {
    let labels = vec![
        Label { name: "alpha".to_string() },
        Label { name: "beta".to_string() },
    ];
    let st = vec![("Roadmap".to_string(), Some("Todo".to_string()))];
    let now = at("2026-01-07T00:00:00Z");
    let issue = IssueData {
        number: ItemRef::github(7),
        title: "t".to_string(),
        labels,
        updated_at: "2026-01-05T00:00:00Z".to_string(),
        ..sample_issue()
    };
    let (_, kept, updated, status, _, _) =
        sidebar_lines(&issue, &st, now, 30, false, &RowFields::default(), "\u{25cb}");
    assert_eq!(kept, "● alpha", "only the first chip fits");
    assert_eq!(updated, "2 day ago");
    assert!(status.ends_with("Todo"));
}

#[test]
fn sidebar_segments_collapse_when_empty() {
    let now = at("2026-01-07T00:00:00Z");
    let issue = IssueData {
        number: ItemRef::github(7),
        title: "t".to_string(),
        labels: vec![],
        updated_at: String::new(),
        ..sample_issue()
    };
    let (_, labels, updated, status, author, milestone) =
        sidebar_lines(&issue, &[], now, 40, false, &RowFields::default(), "");
    let got = (labels.as_str(), updated.as_str(), status.as_str(), author.as_str(),
        milestone.as_str());
    assert_eq!(got, ("", "", "", "", ""));
}

#[test]
fn pinned_marker() {
    let now = at("2026-01-07T00:00:00Z");
    let issue = IssueData {
        number: ItemRef::github(7),
        title: "t".to_string(),
        labels: vec![],
        updated_at: String::new(),
        ..sample_issue()
    };
    let (first, _, _, _, _, _) =
        sidebar_lines(&issue, &[], now, 40, true, &RowFields::default(), "");
    assert!(first.contains("📌"));
    let (plain, _, _, _, _, _) =
        sidebar_lines(&issue, &[], now, 40, false, &RowFields::default(), "");
    assert!(!plain.contains("📌"));
}
