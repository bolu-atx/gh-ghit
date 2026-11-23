//! Session: the UI-free domain state of one ghit run and the commands over it.
//!
//! A front end sends commands (methods) and drains events from `poll` once
//! per frame. Background fetches land on the channels the session owns, so
//! `poll` is the only place their results touch the state. No widget type
//! crosses this boundary.

mod auth;
mod backfill;
mod board;
mod commands;
mod diff;
mod issues;
mod listing;
mod sections;
mod sends;

#[cfg(test)]
mod events_tests;

use std::collections::HashSet;
use std::time::Instant;

pub use auth::AuthState;
pub use backfill::{BACKFILL_DELAY, BackfillFeed};
pub use board::{BoardFeed, DetailResult};
pub use diff::{DiffData, DiffResult, HeadFiles};
pub use listing::{Held, Landed, PollCtx};
pub use issues::{IssueStore, SubIssueLoads, SubIssueResult};
pub use sends::{ComposeTarget, SendResult, Sends, StateResult};
pub use sections::{
    FULL_REFRESH_INTERVAL, RECENT_REFRESH_INTERVAL, RECENT_REFRESH_MAX_INTERVAL, STATUS_REFRESH_INTERVAL, SectionFeed,
    auto_refresh_interval_default, refresh_interval_from, refresh_plan,
};

use crate::backend::IssueBackend;
use crate::core::Kind;
use crate::fetch_queue::{Fetches, View};
use crate::model::{LazyChildren, ItemRef, Source};
use crate::runtime::{rt, spawn_bg};
use crate::{core, github};

/// User pins: the set for lookups and the order they list in.
#[derive(Default)]
pub struct Pins {
    pub set: HashSet<ItemRef>,
    pub order: Vec<ItemRef>,
}

/// Items the open issue refers to: mentions, and the will-close
/// counterparts listed apart from them.
#[derive(Default)]
pub struct Refs {
    pub numbers: Vec<ItemRef>,
    /// Kept disjoint from `numbers` so a row has one behavior.
    pub paired: Vec<ItemRef>,
}

/// What a poll landed. The front end turns these into view changes.
#[derive(Debug)]
pub enum SessionEvent {
    /// The repo resolved. `numbers` are the items asked for at startup.
    Bootstrapped { numbers: Vec<ItemRef> },
    /// The auth probe finished: the missing scopes, or the failure.
    AuthChecked(Result<Vec<String>, String>),
    /// Window-listing rows landed in the store. File `landed.filed` under
    /// recent and drop `landed.gone`; a user fetch (`auto` false) with
    /// nothing open opens the last filed row.
    RowsLanded { landed: Landed, auto: bool },
    /// The stored listing changed while a query shows: re-run the query.
    StoreChanged,
    /// An auto-refresh started a listing fetch.
    SectionsStarted,
    /// A backfill step landed.
    BackfillLanded,
    SectionsProgress,
    /// The listing hit the row cap. Sent once per fetch setting.
    SectionsCapped,
    /// The change probe found nothing new.
    SectionsUnchanged,
    /// A listing landed (or failed). The session already reset its feed.
    SectionsDone { result: Result<Vec<github::Row>, String>, replace: bool, auto: bool },
    /// The lease holder's quiet sync failed.
    SyncFailed(String),
    /// The board payload landed in `board.data`.
    BoardLoaded,
    BoardFailed(String),
    /// A card detail landed in `board.details`.
    BoardDetail(ItemRef),
    BoardDetailFailed(String),
    /// A PR diff landed in `diff.files`.
    DiffLoaded(ItemRef),
    DiffFailed(String),
    /// Head-side file bodies for a PR landed; its diff wants re-widening.
    HeadFilesLanded(ItemRef),
    HeadFilesFailed { number: ItemRef, error: String },
    DraftSent { number: ItemRef, result: Result<(), String> },
    ReviewSubmitted { number: ItemRef, result: Result<(), String> },
    StateChanged { number: ItemRef, open: bool, result: Result<(), String> },
    IssueProgress(ItemRef),
    /// The full fetch landed in the store.
    IssueLoaded(ItemRef),
    IssueFailed { number: ItemRef, error: String },
    /// A parent's children landed in the store.
    SubIssuesLoaded(ItemRef),
    SubIssuesFailed(String),
}

pub struct Session {
    pub issue_backend: IssueBackend,
    pub issues: IssueStore,
    pub sections: SectionFeed,
    pub subs: SubIssueLoads,
    pub auth: AuthState,
    pub refs: Refs,
    pub pins: Pins,
    pub board: BoardFeed,
    pub diff: DiffData,
    pub sends: Sends,
    pub repo_flag: Option<String>,
    pub repo_slug: Option<String>,
    pub initial_numbers: Vec<ItemRef>,
    /// Queue and run-count for every background fetch; see `fetch_queue`.
    pub fetches: Fetches,
}

impl Session {
    pub fn new(repo_flag: Option<String>) -> Self {
        Self {
            issue_backend: IssueBackend::GitHub,
            issues: IssueStore::new(),
            sections: SectionFeed::new(),
            subs: SubIssueLoads::new(),
            auth: AuthState::default(),
            refs: Refs::default(),
            pins: Pins::default(),
            board: BoardFeed::new(),
            diff: DiffData::new(),
            sends: Sends::new(),
            repo_flag,
            repo_slug: None,
            initial_numbers: vec![],
            fetches: Fetches::default(),
        }
    }

    /// The repo the session works on, lowercased: the flag wins over the
    /// checkout's remote.
    pub fn repo_key(&self) -> Option<String> {
        let r = self.repo_flag.clone().or_else(|| self.repo_slug.clone())?;
        Some(r.to_lowercase())
    }

    /// Startup: remember the requested items, resolve the repo off the
    /// async workers and probe the token. `poll` reports both.
    pub fn start(&mut self, numbers: &[ItemRef]) {
        self.initial_numbers = numbers.to_vec();
        let repo = self.repo_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.auth.bootstrap_rx = Some(rx);
        // `git` and `gh` subprocesses block: keep them off the async workers.
        rt().spawn_blocking(move || {
            let slug = github::resolve_repo(repo.as_deref());
            let _ = github::token();
            let _ = tx.send(slug);
        });
        // Early auth probe: one cheap round-trip validates the token so a
        // dead one prompts before the first real fetch fails into an empty
        // sidebar. Scopes need a repo and follow once it resolves.
        let (atx, arx) = std::sync::mpsc::channel();
        self.auth.rx = Some(arx);
        spawn_bg(async move {
            let _ = atx.send(github::check_auth().await);
        });
    }

    /// Queue a background fetch and start whatever fits. Tiers decide the
    /// start order (`core::urgency`); `fetch_queue` caps what runs at once.
    pub fn enqueue(
        &mut self,
        kind: Kind,
        number: Option<ItemRef>,
        task: impl std::future::Future<Output = ()> + Send + 'static,
        view: &View,
    ) {
        self.fetches.push(kind, number, Box::pin(task));
        self.pump(view);
    }

    /// Start queued fetches while slots are free. While the API quota is
    /// spent only the current issue's fetches start.
    pub fn pump(&mut self, view: &View) {
        self.fetches.start(view, github::rate_limited());
    }

    /// Remove every cached payload for `n` so a revisit fetches again.
    pub fn drop_cached(&mut self, n: ItemRef) {
        self.issues.partial.remove(&n);
        self.issues.progress.remove(&n);
        self.issues.data.remove(&n);
        self.issues.statuses.remove(&n);
        self.board.details.remove(&n);
        self.diff.files.remove(&n);
        self.diff.pending.remove(&n);
    }

    /// Queue the full fetch for `number`. No-op while one is in flight.
    pub fn load(&mut self, number: ItemRef, force: bool, view: &View) {
        if !self.issues.pending.insert(number) { return; }
        let tx = self.issues.tx.clone();
        let repo = self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default();
        let backend = self.issue_backend;
        let is_pr = self.is_github_pr(number);
        self.enqueue(Kind::Details, Some(number), async move {
            let result = if is_pr {
                github::fetch_number(&repo, number, force, Some(tx.clone())).await
            } else {
                backend.fetch_number(&repo, number, force, Some(tx.clone())).await
            };
            let _ = tx.send((number, github::IssueEvent::Done(result.map(Box::new))));
        }, view);
    }

    /// Unknown GitHub numbers use the backend's issue-or-PR detail fetch.
    fn is_github_pr(&self, number: ItemRef) -> bool {
        number.source == Source::GitHub
            && self.issues.data.get(&number).is_some_and(|data| data.is_pr)
    }

    /// Read the saved pins of the repo, minus `exclude`, into `pins`.
    pub fn load_pins(&mut self, key: &str, exclude: &[ItemRef]) {
        let mut pins = github::load_pins(key);
        pins.retain(|n| !exclude.contains(n));
        self.pins.set = pins.iter().copied().collect();
        self.pins.order = pins;
    }

    /// Flip the pin on `number` and save the list. Returns whether it is
    /// pinned now; the error names why no pin can be saved.
    pub fn toggle_pin(&mut self, number: ItemRef) -> Result<bool, String> {
        let Some(key) = self.repo_key() else {
            return Err("pinning needs a repo: use -R or run inside one".to_string());
        };
        let loaded = github::load_pins(&key);
        let was_pinned = loaded.contains(&number);
        let pins = core::toggle_pin(&loaded, number);
        if was_pinned {
            self.pins.set.remove(&number);
        } else {
            self.pins.set.insert(number);
        }
        self.pins.order = pins.clone();
        github::save_pins(&key, &pins);
        Ok(!was_pinned)
    }

    /// True while this process holds the sync lease through a store link.
    /// Without a link there is no store to fill.
    pub fn backfill_holder(&self) -> bool {
        self.sections.link.as_ref().is_some_and(github::StoreLink::holding)
    }

    /// Status bar input: whether the backfill runs here, with its progress.
    pub fn backfill_state(&self) -> (bool, Option<(usize, usize)>) {
        let feed = &self.sections.backfill;
        (feed.running() && self.backfill_holder(), feed.progress())
    }

    /// Per loop pass: land a finished backfill step, then queue the next
    /// page when one is due. True when visible state changed.
    pub fn drive_backfill(&mut self, now: Instant, view: &View) -> bool {
        use std::sync::mpsc::TryRecvError;
        let mut landed = false;
        if let Some(rx) = &self.sections.backfill.rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.sections.backfill.rx = None;
                    self.sections.backfill.land(result, now);
                    landed = true;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    self.sections.backfill.rx = None;
                    self.sections.backfill.retry_later(now);
                }
            }
        }
        let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) else { return landed };
        if !self.sections.backfill.due(self.backfill_holder(), github::rate_limited(), now) {
            return landed;
        }
        let months = self.sections.backfill.months;
        let (tx, rx) = std::sync::mpsc::channel();
        self.sections.backfill.rx = Some(rx);
        self.enqueue(Kind::Backfill, None, async move {
            let _ = tx.send(github::backfill_step(&repo, months).await);
        }, view);
        landed
    }

    /// Land everything the background fetches finished and report it.
    #[cfg(test)]
    fn poll(&mut self) -> Vec<SessionEvent> {
        let mut events = self.poll_feed();
        events.extend(self.poll_items());
        events
    }

    /// Land repo, auth and sections results. A front end that writes
    /// sidebar rows into `issues` between the two phases must call this
    /// first, so a full item fetch from `poll_items` wins over a row.
    fn poll_feed(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        self.poll_bootstrap(&mut events);
        self.poll_auth(&mut events);
        self.poll_sections(&mut events);
        events
    }

    /// Land full item fetches, sub-issue loads, diffs, the board and the
    /// results of writes.
    fn poll_items(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        self.poll_issues(&mut events);
        self.poll_subs(&mut events);
        self.poll_diffs(&mut events);
        self.poll_head_files(&mut events);
        self.poll_board(&mut events);
        self.poll_details(&mut events);
        self.poll_sends(&mut events);
        events
    }

    fn poll_bootstrap(&mut self, events: &mut Vec<SessionEvent>) {
        let Some(result) = self.auth.bootstrap_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.auth.bootstrap_rx = None;
        self.repo_slug = result;
        // Repo-scoped access probe, once: the boot probe only checks the
        // token itself, while scopes need a repo to read against.
        if !self.auth.repo_probed {
            self.auth.repo_probed = true;
            if let Some(repo) = self.repo_flag.clone().or_else(|| self.repo_slug.clone()) {
                let (tx, rx) = std::sync::mpsc::channel();
                self.auth.rx = Some(rx);
                spawn_bg(async move {
                    let _ = tx.send(github::check_repo_access(&repo).await);
                });
            }
        }
        events.push(SessionEvent::Bootstrapped { numbers: std::mem::take(&mut self.initial_numbers) });
    }

    fn poll_auth(&mut self, events: &mut Vec<SessionEvent>) {
        let Some(result) = self.auth.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.auth.rx = None;
        events.push(SessionEvent::AuthChecked(result));
    }

    fn poll_sections(&mut self, events: &mut Vec<SessionEvent>) {
        self.poll_quiet_sync(events);
        let Some(event) = self.sections.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        match event {
            github::SectionEvent::Progress { done, total } => {
                self.sections.progress = Some((done, total));
                events.push(SessionEvent::SectionsProgress);
            }
            // The fetch that hit the cap still lands its rows through Done;
            // the notice only has to name the cap, once per fetch setting.
            github::SectionEvent::Capped => {
                if !self.sections.cap_notified {
                    self.sections.cap_notified = true;
                    events.push(SessionEvent::SectionsCapped);
                }
            }
            github::SectionEvent::Unchanged => {
                self.sections.rx = None;
                self.sections.progress = None;
                if std::mem::take(&mut self.sections.auto_refresh) {
                    self.sections.auto_refresh_interval = auto_refresh_interval_default();
                }
                events.push(SessionEvent::SectionsUnchanged);
            }
            github::SectionEvent::Done(result) => {
                self.sections.rx = None;
                self.sections.progress = None;
                let auto = std::mem::take(&mut self.sections.auto_refresh);
                if auto {
                    self.sections.auto_refresh_interval = if result.is_ok() {
                        auto_refresh_interval_default()
                    } else {
                        (self.sections.auto_refresh_interval * 2).min(RECENT_REFRESH_MAX_INTERVAL)
                    };
                }
                let replace = std::mem::take(&mut self.sections.replace);
                events.push(SessionEvent::SectionsDone { result, replace, auto });
            }
        }
    }

    /// The holder's sync behind a query writes the store and lands no rows.
    /// A finished or dead sync frees the slot for the next cadence tick.
    fn poll_quiet_sync(&mut self, events: &mut Vec<SessionEvent>) {
        let Some(rx) = self.sections.sync_rx.as_ref() else { return };
        let mut finished = false;
        for event in rx.try_iter().collect::<Vec<_>>() {
            match event {
                github::SectionEvent::Done(result) => {
                    finished = true;
                    if let Err(e) = result { events.push(SessionEvent::SyncFailed(e)); }
                }
                github::SectionEvent::Unchanged => finished = true,
                _ => {}
            }
        }
        if finished || self.sections.sync_rx.as_ref().is_some_and(|rx| matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected))) {
            self.sections.sync_rx = None;
        }
    }

    fn poll_issues(&mut self, events: &mut Vec<SessionEvent>) {
        let ready: Vec<_> = self.issues.rx.try_iter().collect();
        for (number, event) in ready {
            match event {
                github::IssueEvent::Progress { done, total } => {
                    self.issues.progress.insert(number, (done, total));
                    events.push(SessionEvent::IssueProgress(number));
                }
                github::IssueEvent::Done(result) => {
                    self.issues.progress.remove(&number);
                    self.issues.pending.remove(&number);
                    match result.map(|bundle| *bundle) {
                        Ok((mut data, statuses)) => {
                            // Completing the full fetch never files the row:
                            // viewing a recent row must not relocate it to
                            // requested. Explicit actions file through
                            // `ensure_visited` at their own site.
                            self.issues.partial.remove(&number);
                            // REST `issues/{n}` has no parent link: keep the
                            // sub-issue fields the sections row already carried.
                            if let Some(old) = self.issues.data.get(&number) {
                                data.parent = data.parent.or(old.parent);
                                data.sub_summary = data.sub_summary.or(old.sub_summary);
                            }
                            self.issues.data.insert(number, data);
                            self.issues.statuses.insert(number, statuses);
                            events.push(SessionEvent::IssueLoaded(number));
                        }
                        Err(error) => events.push(SessionEvent::IssueFailed { number, error }),
                    }
                }
            }
        }
    }

    fn poll_subs(&mut self, events: &mut Vec<SessionEvent>) {
        let ready: Vec<_> = self.subs.rx.try_iter().collect();
        for (parent, result) in ready {
            match result {
                Ok(list) => {
                    let mut numbers = Vec::with_capacity(list.len());
                    for mut data in list {
                        numbers.push(data.number);
                        data.parent = Some(parent);
                        // An outside child is a slim row: opening it runs the
                        // full fetch. A number the query already fetched keeps
                        // its richer payload.
                        if !self.issues.data.contains_key(&data.number) {
                            self.issues.partial.insert(data.number);
                            self.issues.data.insert(data.number, data);
                        }
                    }
                    self.subs.state.insert(parent, LazyChildren::Loaded(numbers));
                    events.push(SessionEvent::SubIssuesLoaded(parent));
                }
                Err(e) => {
                    // Drop the state so the next explicit expand can retry.
                    self.subs.state.remove(&parent);
                    events.push(SessionEvent::SubIssuesFailed(e));
                }
            }
        }
    }

    fn poll_diffs(&mut self, events: &mut Vec<SessionEvent>) {
        let ready: Vec<_> = self.diff.rx.try_iter().collect();
        for (number, result) in ready {
            self.diff.pending.remove(&number);
            match result {
                Ok(files) => {
                    // A refetch may move the head: drop its old file bodies.
                    self.diff.heads.retain(|(i, _), _| *i != number);
                    self.diff.raw.insert(number, files.clone());
                    self.diff.files.insert(number, files);
                    events.push(SessionEvent::DiffLoaded(number));
                }
                Err(e) => events.push(SessionEvent::DiffFailed(e)),
            }
        }
    }

    fn poll_head_files(&mut self, events: &mut Vec<SessionEvent>) {
        let ready: Vec<HeadFiles> = self.diff.heads_rx.try_iter().collect();
        for (n, names, result) in ready {
            for name in &names {
                self.diff.heads_pending.remove(&(n, name.clone()));
            }
            match result {
                Ok(bodies) => {
                    for (name, body) in bodies {
                        let lines = body.ok().map(|b| b.lines().map(str::to_owned).collect());
                        self.diff.heads.insert((n, name), lines);
                    }
                    events.push(SessionEvent::HeadFilesLanded(n));
                }
                Err(error) => {
                    for name in names {
                        self.diff.heads.insert((n, name), None);
                    }
                    events.push(SessionEvent::HeadFilesFailed { number: n, error });
                }
            }
        }
    }

    fn poll_board(&mut self, events: &mut Vec<SessionEvent>) {
        if !self.board.loading {
            return;
        }
        let Some(res) = self.board.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.board.rx = None;
        self.board.loading = false;
        match res {
            Ok(data) => {
                self.board.data = Some(data);
                events.push(SessionEvent::BoardLoaded);
            }
            Err(e) => events.push(SessionEvent::BoardFailed(e)),
        }
    }

    fn poll_details(&mut self, events: &mut Vec<SessionEvent>) {
        let ready: Vec<_> = self.board.detail_rx.try_iter().collect();
        for (n, res) in ready {
            self.board.detail_pending.remove(&n);
            match res {
                Ok(d) => {
                    self.board.details.insert(n, d);
                    events.push(SessionEvent::BoardDetail(n));
                }
                Err(e) => events.push(SessionEvent::BoardDetailFailed(e)),
            }
        }
    }

    fn poll_sends(&mut self, events: &mut Vec<SessionEvent>) {
        for (number, result) in self.sends.rx.try_iter().collect::<Vec<_>>() {
            events.push(SessionEvent::DraftSent { number, result });
        }
        // The review lock frees as the result lands.
        for (number, result) in self.sends.review_rx.try_iter().collect::<Vec<_>>() {
            self.sends.submitting.remove(&number);
            events.push(SessionEvent::ReviewSubmitted { number, result });
        }
        for (number, open, result) in self.sends.state_rx.try_iter().collect::<Vec<_>>() {
            events.push(SessionEvent::StateChanged { number, open, result });
        }
    }
}
