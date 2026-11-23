//! The map filter dialog's actions: move the focused row, flip it, flip
//! a switch by its own key, reset. The map rebuilds when its filter
//! changes, so a change shows on the next frame; the layout wakes with
//! the surviving nodes in place. The filter is saved per repo.

use std::collections::BTreeSet;

use crate::app::{App, Entry};
use ghit_core::model::{LinkKind, MapFilter, MapScope};

/// One focusable row of the map filter dialog, in dialog order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilterRow {
    Scope,
    Closed,
    Prs,
    Hubs,
    Link(LinkKind),
    Milestone(String),
    Label(String),
}

impl App {
    pub(crate) fn open_map_filter(&mut self) {
        self.open(Entry::MapFilter(0));
    }

    /// The dialog's rows: the fixed switches, then one chip per milestone
    /// and per label, sorted. Chips come from the stored items, plus any
    /// the filter holds, so a chosen chip stays visible to clear.
    pub(crate) fn map_filter_rows(&self) -> Vec<FilterRow> {
        let filter = &self.map.filter;
        let mut milestones: BTreeSet<&str> = filter.milestones.iter().map(String::as_str).collect();
        let mut labels: BTreeSet<&str> = filter.labels.iter().map(String::as_str).collect();
        for item in &self.map.items {
            milestones.extend(item.milestone.as_deref());
            labels.extend(item.labels.iter().map(String::as_str));
        }
        let mut rows = vec![
            FilterRow::Scope,
            FilterRow::Closed,
            FilterRow::Prs,
            FilterRow::Hubs,
            FilterRow::Link(LinkKind::Blocks),
            FilterRow::Link(LinkKind::Parent),
            FilterRow::Link(LinkKind::Closes),
        ];
        rows.extend(milestones.into_iter().map(|title| FilterRow::Milestone(title.to_string())));
        rows.extend(labels.into_iter().map(|name| FilterRow::Label(name.to_string())));
        rows
    }

    /// Move the focus `delta` rows, wrapping at the ends.
    pub(crate) fn map_filter_step(&mut self, delta: isize) {
        let Some(focus) = self.map_filter_focus() else { return };
        let rows = self.map_filter_rows().len() as isize;
        self.set_map_filter_focus((focus as isize + delta).rem_euclid(rows) as usize);
    }

    /// Flip the focused row.
    pub(crate) fn map_filter_toggle(&mut self) {
        let Some(focus) = self.map_filter_focus() else { return };
        if let Some(row) = self.map_filter_rows().into_iter().nth(focus) {
            self.map_filter_flip(row);
        }
    }

    /// Flip one row: the scope swaps, a switch or chip turns on or off.
    pub(crate) fn map_filter_flip(&mut self, row: FilterRow) {
        let mut filter = self.map.filter.clone();
        match row {
            FilterRow::Scope => return self.map_toggle_scope(),
            FilterRow::Closed => filter.closed = !filter.closed,
            FilterRow::Prs => filter.prs = !filter.prs,
            FilterRow::Hubs => filter.hubs = !filter.hubs,
            FilterRow::Link(kind) => {
                let shown = filter.link_mut(kind);
                *shown = !*shown;
            }
            FilterRow::Milestone(title) => {
                if !filter.milestones.remove(&title) {
                    filter.milestones.insert(title);
                }
            }
            FilterRow::Label(name) => {
                if !filter.labels.remove(&name) {
                    filter.labels.insert(name);
                }
            }
        }
        self.apply_map_filter(filter);
    }

    /// Show `scope`; a click on the scope option already shown does nothing.
    pub(crate) fn map_filter_scope(&mut self, scope: MapScope) {
        if self.map.filter.scope != scope {
            self.map_toggle_scope();
        }
    }

    /// Back to the default filter, keeping the scope.
    pub(crate) fn map_filter_reset(&mut self) {
        self.apply_map_filter(MapFilter { scope: self.map.filter.scope, ..MapFilter::default() });
        self.notify("map filter reset".to_string());
    }

    fn apply_map_filter(&mut self, filter: MapFilter) {
        self.map.filter = filter;
        self.persist_prefs();
    }
}
