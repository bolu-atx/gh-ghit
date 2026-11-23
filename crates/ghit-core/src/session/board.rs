//! Board data: the project board, its card details and the fetches that fill them.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

use crate::model::{BoardData, BoardDetail, ItemRef};

/// One card-detail fetch result.
pub type DetailResult = (ItemRef, Result<BoardDetail, String>);

/// The board payload and its in-flight fetch, plus the per-item card
/// details (filled from listings, issues or a batch fetch).
pub struct BoardFeed {
    pub data: Option<BoardData>,
    pub loading: bool,
    pub rx: Option<Receiver<Result<BoardData, String>>>,
    pub details: HashMap<ItemRef, BoardDetail>,
    pub detail_pending: HashSet<ItemRef>,
    pub detail_tx: Sender<DetailResult>,
    pub detail_rx: Receiver<DetailResult>,
}

impl BoardFeed {
    pub fn new() -> Self {
        let (detail_tx, detail_rx) = std::sync::mpsc::channel();
        Self { data: None, loading: false, rx: None, details: HashMap::new(),
            detail_pending: HashSet::new(), detail_tx, detail_rx }
    }
}

impl Default for BoardFeed {
    fn default() -> Self {
        Self::new()
    }
}
