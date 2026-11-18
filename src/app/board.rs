//! Project board modal: open, filter, cursor and poll.

use std::collections::{HashMap, HashSet};

use ratatui::widgets::ListState;

use crate::app::{App, Entry, FormatEdit, Slot};
use crate::core::self;
use crate::{github, model};
use crate::model::BoardData;
use crate::app::auth_failure;
use crate::app::poll::kick_off_details;

/// Board modal state: the loaded project payload, its fetch channel,
/// per-column list states, cursor, text/milestone filters, and the card
/// detail cache plus its fetch triplet.
pub(crate) struct BoardView {
    pub(crate) data: Option<BoardData>,
    pub(crate) loading: bool,
    pub(crate) rx: Option<std::sync::mpsc::Receiver<Result<BoardData, String>>>,
    pub(crate) states: Vec<ListState>,
    pub(crate) expanded: Option<(usize, usize)>,
    /// Board text filter (`f` while the board is open): label, card title,
    /// or `#num`. Milestones filter through the pills below, never this
    /// text. Empty shows every card.
    pub(crate) filter: String,
    /// Board milestone pills: the selected milestones. Empty selects every
    /// card; `None` is the "(no milestone)" pill.
    pub(crate) milestones: HashSet<Option<String>>,
    /// Active widget in the board format dialog: 0 is the text row, then
    /// milestone pills in `model::board_milestones` order, then the card
    /// field rows in `model::CardFields::ORDER` order.
    pub(crate) filter_focus: usize,
    /// Milestone draft edited in the dialog; Enter applies it.
    pub(crate) milestone_draft: HashSet<Option<String>>,
    /// Collapsed-card fields toggled in the dialog Card fields section.
    pub(crate) fields: model::CardFields,
    pub(crate) col: usize,
    pub(crate) row: usize,
    pub(crate) details: HashMap<u64, model::BoardDetail>,
    pub(crate) detail_pending: HashSet<u64>,
    pub(crate) detail_tx: std::sync::mpsc::Sender<(u64, Result<model::BoardDetail, String>)>,
    pub(crate) detail_rx: std::sync::mpsc::Receiver<(u64, Result<model::BoardDetail, String>)>,
}

impl BoardView {
    pub(crate) fn new(
        detail_tx: std::sync::mpsc::Sender<(u64, Result<model::BoardDetail, String>)>,
        detail_rx: std::sync::mpsc::Receiver<(u64, Result<model::BoardDetail, String>)>,
    ) -> Self {
        Self { data: None, loading: false, rx: None, states: Vec::new(), expanded: None,
            filter: String::new(), milestones: HashSet::new(), filter_focus: 0,
            milestone_draft: HashSet::new(), fields: model::CardFields::default(),
            col: 0, row: 0, details: HashMap::new(),
            detail_pending: HashSet::new(), detail_tx, detail_rx }
    }
}

impl App {
    /// Modal opens instantly with a skeleton; the network fetch lands in
    /// the background.
    pub(crate) fn open_board(&mut self) {
        if board_open(self) {
            return;
        }
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone());
        let Some(owner) = github::board_owner(repo.as_deref()) else {
            self.notify("board needs a repo: use -R or run inside one".to_string());
            return;
        };
        self.open(Entry::Board);
        self.board.loading = true;
        self.board.col = 0;
        self.board.row = 0;
        self.restore_board_filter(&owner);
        let (tx, rx) = std::sync::mpsc::channel();
        self.board.rx = Some(rx);
        self.enqueue(core::Kind::Other, None, async move {
            let _ = tx.send(github::fetch_board(&owner).await);
        });
    }

    /// Restore the saved text filter and milestone pills for a board
    /// owner. A fresh open looks like last time; an owner without saved
    /// state starts unfiltered.
    pub(crate) fn restore_board_filter(&mut self, owner: &str) {
        match github::load_prefs().board_filters.get(&github::prefs_owner_key(owner)) {
            Some(saved) => {
                self.board.filter = saved.filter.clone();
                self.board.milestones = saved.milestones.iter().cloned().collect();
            }
            None => {
                self.board.filter.clear();
                self.board.milestones.clear();
            }
        }
        self.prune_board_milestones();
    }

    /// Drop selected milestone pills that no longer exist on the board
    /// and clamp the dialog focus to the widget count. A renamed or
    /// deleted milestone would otherwise hide every lane with no pill
    /// to deselect it by. Persists the pruned set when prefs are live.
    pub(crate) fn prune_board_milestones(&mut self) {
        let Some(board) = &self.board.data else { return };
        let live: std::collections::HashSet<Option<String>> =
            model::board_milestones(&board.items).into_iter().collect();
        self.board.milestones.retain(|m| live.contains(m));
        self.board.milestone_draft.retain(|m| live.contains(m));
        let widgets = self.board_pills().len() + 1 + model::CardFields::ORDER.len();
        self.board.filter_focus = self.board.filter_focus.min(widgets.saturating_sub(1));
        self.persist_prefs();
    }

    /// Close the board and drop its cards. A fetch still in flight is
    /// ignored when it lands. The filter stays.
    pub(crate) fn close_board(&mut self) {
        self.close(Slot::Board);
        self.board.data = None;
        self.board.loading = false;
    }

    /// Open the format dialog against the board cards. It opens in
    /// navigate mode with the active text and pills prefilled; the board
    /// stays up behind it.
    pub(crate) fn open_board_filter(&mut self) {
        self.sidebar.query_draft = self.board.filter.clone();
        self.board.milestone_draft = self.board.milestones.clone();
        self.board.filter_focus = 0;
        self.open(Entry::Format(FormatEdit::None));
    }

    /// Apply the dialog drafts and keep the cursor inside the narrowed
    /// grid. Positions move under a new filter: never keep a stale
    /// expansion.
    pub(crate) fn apply_board_filter(&mut self) {
        self.board.filter = self.sidebar.query_draft.trim().to_string();
        self.board.milestones = self.board.milestone_draft.clone();
        self.board.expanded = None;
        self.clamp_board_cursor();
        self.persist_prefs();
    }

    pub(crate) fn clear_board_filter(&mut self) {
        self.board.filter.clear();
        self.board.milestones.clear();
        self.board.expanded = None;
        self.clamp_board_cursor();
        self.persist_prefs();
    }

    /// True when any board filter is active: text or milestone pills.
    pub(crate) fn board_filtered(&self) -> bool {
        !self.board.filter.trim().is_empty() || !self.board.milestones.is_empty()
    }

    /// Milestone pills for the dialog, in display order.
    pub(crate) fn board_pills(&self) -> Vec<Option<String>> {
        match &self.board.data {
            Some(board) => model::board_milestones(&board.items),
            None => vec![],
        }
    }

    /// Step the dialog focus across the text row, pills, and card field
    /// rows, wrapping.
    pub(crate) fn board_filter_step(&mut self, delta: isize) {
        let n = self.board_pills().len() + 1 + model::CardFields::ORDER.len();
        self.board.filter_focus = (self.board.filter_focus as isize + delta).rem_euclid(n as isize) as usize;
    }

    /// Flip one Card fields checkbox by its dialog row and persist it.
    /// Card fields apply live; the text and milestone drafts still need
    /// Enter.
    pub(crate) fn toggle_board_card_field(&mut self, index: usize) {
        if self.board.fields.toggle(index) {
            self.persist_prefs();
        }
    }

    fn clamp_board_cursor(&mut self) {
        let Some(board) = &self.board.data else { return };
        let cols = board_columns(board, &self.board.filter, &self.board.milestones);
        let col = self.board.col.min(cols.len().saturating_sub(1));
        let rows = cols.get(col).map(|(_, cards)| cards.len()).unwrap_or(0);
        self.board.col = col;
        self.board.row = self.board.row.min(rows.saturating_sub(1));
    }
}

pub(crate) fn board_open(app: &App) -> bool {
    app.is_open(Slot::Board)
}

/// Board columns narrowed by the active filter: milestone pills select
/// (empty selects all) plus the text query over title, label, `#num`.
/// An empty filter returns every card.
pub(crate) fn board_columns<'a>(
    board: &'a BoardData,
    filter: &str,
    milestones: &HashSet<Option<String>>,
) -> Vec<(String, Vec<&'a model::BoardItem>)> {
    let cols = model::board_columns(&board.items, &board.order);
    if filter.trim().is_empty() && milestones.is_empty() {
        return cols;
    }
    cols.into_iter()
        .map(|(name, cards)| {
            let cards = cards
                .into_iter()
                .filter(|c| {
                    if !milestones.is_empty() && !milestones.contains(&c.milestone) {
                        return false;
                    }
                    let labels: Vec<&str> = c.labels.iter().map(|l| l.name.as_str()).collect();
                    core::matches_board_filter(c.number, &c.title, &labels, filter)
                })
                .collect();
            (name, cards)
        })
        .collect()
}

pub(crate) fn board_move(app: &mut App, dx: isize, dy: isize) {
    app.board.expanded = None;
    let Some(board) = &app.board.data else { return };
    let cols = board_columns(board, &app.board.filter, &app.board.milestones);
    if cols.is_empty() {
        return;
    }
    let col = (app.board.col as isize + dx).clamp(0, cols.len() as isize - 1) as usize;
    let rows = cols[col].1.len();
    (app.board.col, app.board.row) =
        core::board_cursor(cols.len(), rows, app.board.col, app.board.row, dx, dy);
}

pub(crate) fn board_selected_number(app: &App) -> Option<u64> {
    let board = app.board.data.as_ref()?;
    let cols = board_columns(board, &app.board.filter, &app.board.milestones);
    cols.get(app.board.col)?.1.get(app.board.row)?.number
}

pub(crate) fn poll_board(app: &mut App) -> bool {
    if !app.board.loading {
        return false;
    }
    let res = app.board.rx.as_ref().and_then(|rx| rx.try_recv().ok());
    let Some(res) = res else { return false };
    app.board.rx = None;
    app.board.loading = false;
    match res {
        Ok(data) => {
            app.board.data = Some(data);
            app.board.col = 0;
            app.board.row = 0;
            app.board.states.clear();
            app.board.expanded = None;
            app.prune_board_milestones();
            kick_off_details(app);
        }
        Err(e) => { auth_failure(app, &e); app.notify(e); }
    }
    true
}
