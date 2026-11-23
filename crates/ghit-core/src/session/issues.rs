//! Item details and sub-issue loads: fetched payloads plus the channels their fetches land on.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use super::Session;
use crate::github;
use crate::model::{self, IssueData, ItemRef};

/// Issues held by the session: fetched payloads plus the fetch channel
/// triplet (sender, receiver, in-flight set) and per-issue page progress.
pub(crate) struct IssueStore {
    pub(crate) data: HashMap<ItemRef, IssueData>,
    pub(crate) statuses: HashMap<ItemRef, Vec<(String, Option<String>)>>,
    pub(crate) partial: HashSet<ItemRef>,
    pub(crate) tx: Sender<(ItemRef, github::IssueEvent)>,
    pub(crate) rx: Receiver<(ItemRef, github::IssueEvent)>,
    pub(crate) pending: HashSet<ItemRef>,
    /// Comment-page progress per issue: pages done vs total. Cleared when
    /// the terminal result lands.
    pub(crate) progress: HashMap<ItemRef, (usize, usize)>,
}

impl IssueStore {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { data: HashMap::new(), statuses: HashMap::new(), partial: HashSet::new(),
            tx, rx, pending: HashSet::new(), progress: HashMap::new() }
    }
}

impl Default for IssueStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A sub-issue fetch result: the parent and its children.
pub(crate) type SubIssueResult = (ItemRef, Result<Vec<IssueData>, String>);

/// REST sub-issue loads for the opt-in sidebar tree: per-parent session
/// state (loading or loaded children) plus the fetch channel.
pub(crate) struct SubIssueLoads {
    pub(crate) state: HashMap<ItemRef, model::LazyChildren>,
    pub(crate) tx: Sender<SubIssueResult>,
    pub(crate) rx: Receiver<SubIssueResult>,
}

impl SubIssueLoads {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { state: HashMap::new(), tx, rx }
    }
}

impl Default for SubIssueLoads {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Put an evicted item back with the payloads the undo held.
    pub fn restore_item(
        &mut self,
        n: ItemRef,
        data: Option<IssueData>,
        statuses: Option<Vec<(String, Option<String>)>>,
    ) {
        if let Some(data) = data {
            self.issues.data.insert(n, data);
        }
        if let Some(statuses) = statuses {
            self.issues.statuses.insert(n, statuses);
        }
    }
}
