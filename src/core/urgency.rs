//! Fetch urgency: which background request starts first.

/// What a request fetches. The scheduler sorts by `urgency` of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Full issue or PR details.
    Details,
    Diff,
    Graph,
    /// The sidebar listing. One request carries the issues and the PRs.
    Sections,
    /// Labels, milestones, sub-issue children, the board.
    Other,
}

/// Start order, highest first. The sections fetch is one API call for
/// both sidebar panes, so Issues and PRs share one tier: the combined
/// request runs at the Issues tier and the PRs ride along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The issue or PR in the main pane, and its visible diff or graph.
    Current,
    /// The sidebar sections.
    Sections,
    /// Everything else: refs, pins, history, hidden tabs, children, meta.
    Background,
}

/// Tier of one request. `is_current` means the request belongs to the
/// issue in the main pane. `tab_shown` means the Diff or Graph tab is up.
pub fn urgency(kind: Kind, is_current: bool, tab_shown: bool) -> Tier {
    match kind {
        Kind::Details if is_current => Tier::Current,
        Kind::Diff | Kind::Graph if is_current && tab_shown => Tier::Current,
        Kind::Sections => Tier::Sections,
        Kind::Details | Kind::Diff | Kind::Graph | Kind::Other => Tier::Background,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_details_come_first() {
        assert_eq!(urgency(Kind::Details, true, false), Tier::Current);
        assert_eq!(urgency(Kind::Details, false, false), Tier::Background);
    }

    #[test]
    fn diff_and_graph_need_the_tab() {
        for kind in [Kind::Diff, Kind::Graph] {
            assert_eq!(urgency(kind, true, true), Tier::Current);
            assert_eq!(urgency(kind, true, false), Tier::Background);
            assert_eq!(urgency(kind, false, true), Tier::Background);
        }
    }

    #[test]
    fn sections_sit_between() {
        assert_eq!(urgency(Kind::Sections, false, false), Tier::Sections);
        assert!(Tier::Current < Tier::Sections && Tier::Sections < Tier::Background);
        assert_eq!(urgency(Kind::Other, true, true), Tier::Background);
    }
}
