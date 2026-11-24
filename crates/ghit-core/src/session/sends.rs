//! Writes to GitHub: comment drafts, pending-review submits and state changes.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender};

use crate::model::ItemRef;

/// Where a composed comment lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeTarget {
    /// Plain issue/PR thread comment.
    Thread,
    /// Single-shot PR line comment: file plus new-side line.
    Line { path: String, line: u64 },
    /// Reply inside an existing review thread.
    Reply { thread_id: String },
}

pub(crate) type SendResult = (ItemRef, Result<(), String>);
pub(crate) type StateResult = (ItemRef, bool, Result<(), String>);

/// Channels and in-flight sets of the writes.
pub(crate) struct Sends {
    pub(crate) tx: Sender<SendResult>,
    pub(crate) rx: Receiver<SendResult>,
    pub(crate) review_tx: Sender<SendResult>,
    pub(crate) review_rx: Receiver<SendResult>,
    /// PRs with a review submit in flight: one per PR.
    pub(crate) submitting: HashSet<ItemRef>,
    pub(crate) state_tx: Sender<StateResult>,
    pub(crate) state_rx: Receiver<StateResult>,
}

impl Sends {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (review_tx, review_rx) = std::sync::mpsc::channel();
        let (state_tx, state_rx) = std::sync::mpsc::channel();
        Self { tx, rx, review_tx, review_rx, submitting: HashSet::new(), state_tx, state_rx }
    }
}

impl Default for Sends {
    fn default() -> Self {
        Self::new()
    }
}
