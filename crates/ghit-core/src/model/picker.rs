//! Picker lists: the label names and milestones the Format dialog and the
//! Labels pane offer, with the item sources that list each one.

use super::{Milestone, Source};

/// The item sources that list one label name or milestone. A name that
/// both sources list has no source marker.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NameSource {
    pub github: bool,
    pub jira: bool,
}

impl NameSource {
    /// The source an entry of `source` lists.
    pub const fn of(source: Source) -> Self {
        match source {
            Source::GitHub => Self { github: true, jira: false },
            Source::Jira => Self { github: false, jira: true },
        }
    }

    /// The one source that lists the name; `None` when both or neither do.
    pub const fn single(self) -> Option<Source> {
        match (self.github, self.jira) {
            (true, false) => Some(Source::GitHub),
            (false, true) => Some(Source::Jira),
            _ => None,
        }
    }

    const fn union(self, other: Self) -> Self {
        Self { github: self.github || other.github, jira: self.jira || other.jira }
    }
}

/// The picker lists of one repo: label names and milestones, each with the
/// sources that list it. `error` carries the failure of an arm whose lists
/// are missing, so the other arm's lists still show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoMeta {
    pub labels: Vec<(String, NameSource)>,
    pub milestones: Vec<(Milestone, NameSource)>,
    pub error: Option<String>,
}

impl RepoMeta {
    /// One source's lists, in the order that source lists them.
    pub fn side(source: Source, labels: Vec<String>, milestones: Vec<Milestone>) -> Self {
        let listed = NameSource::of(source);
        Self {
            labels: labels.into_iter().map(|name| (name, listed)).collect(),
            milestones: milestones.into_iter().map(|milestone| (milestone, listed)).collect(),
            error: None,
        }
    }

    /// `self`'s order first, then the names only `other` carries. A name in
    /// both lists once, tagged with both sources.
    pub fn merge(mut self, other: Self) -> Self {
        for (name, listed) in other.labels {
            match self.labels.iter_mut().find(|(known, _)| *known == name) {
                Some((_, known)) => *known = known.union(listed),
                None => self.labels.push((name, listed)),
            }
        }
        for (milestone, listed) in other.milestones {
            match self.milestones.iter_mut().find(|(known, _)| known.title == milestone.title) {
                Some((_, known)) => *known = known.union(listed),
                None => self.milestones.push((milestone, listed)),
            }
        }
        self
    }

    /// Record the failure of the arm whose lists are missing.
    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self
    }
}

#[cfg(test)]
#[path = "picker_tests.rs"]
mod tests;
