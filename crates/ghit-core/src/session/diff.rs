//! Diff data: fetched file lists per PR, head-side file bodies and their channels.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use super::Session;
use crate::model::{DiffFile, ItemRef};

/// One diff fetch result: the PR and its files.
pub(crate) type DiffResult = (ItemRef, Result<Vec<DiffFile>, String>);

/// One head-files fetch result: the PR, the names asked for, and the bodies.
pub(crate) type HeadFiles = (
    ItemRef,
    Vec<String>,
    Result<Vec<(String, Result<String, String>)>, String>,
);

pub(crate) struct DiffData {
    /// Files per PR, widened to the front end's context setting.
    pub(crate) files: HashMap<ItemRef, Vec<DiffFile>>,
    /// Patches as GitHub sent them.
    pub(crate) raw: HashMap<ItemRef, Vec<DiffFile>>,
    /// Head-side file lines per (PR, filename), for widening. `None`
    /// marks a failed fetch, so it is not retried this session.
    pub(crate) heads: HashMap<(ItemRef, String), Option<Vec<String>>>,
    pub(crate) heads_pending: HashSet<(ItemRef, String)>,
    pub(crate) pending: HashSet<ItemRef>,
    pub(crate) tx: Sender<DiffResult>,
    pub(crate) rx: Receiver<DiffResult>,
    pub(crate) heads_tx: Sender<HeadFiles>,
    pub(crate) heads_rx: Receiver<HeadFiles>,
}

impl DiffData {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (heads_tx, heads_rx) = std::sync::mpsc::channel();
        Self { files: HashMap::new(), raw: HashMap::new(), heads: HashMap::new(),
            heads_pending: HashSet::new(), pending: HashSet::new(), tx, rx, heads_tx, heads_rx }
    }
}

impl Default for DiffData {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Show `files` as PR `n`'s diff, replacing the previous widening.
    pub fn show_diff(&mut self, n: ItemRef, files: Vec<DiffFile>) {
        self.diff.files.insert(n, files);
    }
}
