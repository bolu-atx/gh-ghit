//! Item details and sub-issue loads: fetched payloads plus the channels their fetches land on.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use crate::github;
use crate::model::{self, IssueData, ItemRef};

/// Issues held by the session: fetched payloads plus the fetch channel
/// triplet (sender, receiver, in-flight set) and per-issue page progress.
pub struct IssueStore {
    pub data: HashMap<ItemRef, IssueData>,
    pub statuses: HashMap<ItemRef, Vec<(String, Option<String>)>>,
    pub partial: HashSet<ItemRef>,
    pub tx: Sender<(ItemRef, github::IssueEvent)>,
    pub rx: Receiver<(ItemRef, github::IssueEvent)>,
    pub pending: HashSet<ItemRef>,
    /// Comment-page progress per issue: pages done vs total. Cleared when
    /// the terminal result lands.
    pub progress: HashMap<ItemRef, (usize, usize)>,
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
pub type SubIssueResult = (ItemRef, Result<Vec<IssueData>, String>);

/// REST sub-issue loads for the opt-in sidebar tree: per-parent session
/// state (loading or loaded children) plus the fetch channel.
pub struct SubIssueLoads {
    pub state: HashMap<ItemRef, model::LazyChildren>,
    pub tx: Sender<SubIssueResult>,
    pub rx: Receiver<SubIssueResult>,
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
