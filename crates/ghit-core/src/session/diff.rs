//! Diff data: fetched file lists per PR, head-side file bodies and their channels.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use crate::model::{DiffFile, ItemRef};

/// One diff fetch result: the PR and its files.
pub type DiffResult = (ItemRef, Result<Vec<DiffFile>, String>);

/// One head-files fetch result: the PR, the names asked for, and the bodies.
pub type HeadFiles = (
    ItemRef,
    Vec<String>,
    Result<Vec<(String, Result<String, String>)>, String>,
);

pub struct DiffData {
    /// Files per PR, widened to the front end's context setting.
    pub files: HashMap<ItemRef, Vec<DiffFile>>,
    /// Patches as GitHub sent them.
    pub raw: HashMap<ItemRef, Vec<DiffFile>>,
    /// Head-side file lines per (PR, filename), for widening. `None`
    /// marks a failed fetch, so it is not retried this session.
    pub heads: HashMap<(ItemRef, String), Option<Vec<String>>>,
    pub heads_pending: HashSet<(ItemRef, String)>,
    pub pending: HashSet<ItemRef>,
    pub tx: Sender<DiffResult>,
    pub rx: Receiver<DiffResult>,
    pub heads_tx: Sender<HeadFiles>,
    pub heads_rx: Receiver<HeadFiles>,
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
