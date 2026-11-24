//! Test-only doors into the session state: the sub-structs are private, so
//! tests that set up or inspect them go through these views.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::diff::HeadFiles;
use super::sends::SendResult;
use super::Session;
use crate::fetch_queue::Fetches;
use crate::github;
use crate::model::{BoardData, BoardDetail, DiffFile, IssueData, ItemRef, LazyChildren};

/// One issue-store fetch event.
type IssueEvent = (ItemRef, github::IssueEvent);

/// The item cache and its fetch state.
pub struct IssuesTest<'a> {
    pub data: &'a mut HashMap<ItemRef, IssueData>,
    pub statuses: &'a mut HashMap<ItemRef, Vec<(String, Option<String>)>>,
    pub partial: &'a mut HashSet<ItemRef>,
    pub pending: &'a mut HashSet<ItemRef>,
    pub progress: &'a mut HashMap<ItemRef, (usize, usize)>,
    pub tx: &'a Sender<IssueEvent>,
}

/// The listing feed and its cadence.
pub struct SectionsTest<'a> {
    pub rx: &'a mut Option<Receiver<github::SectionEvent>>,
    pub merged_window_hours: &'a mut u64,
    pub replace: &'a mut bool,
    pub last_fetch: &'a mut Instant,
    pub progress: &'a mut Option<(usize, usize)>,
    pub auto_refresh: &'a mut bool,
    pub auto_refresh_interval: &'a mut Duration,
    pub cap_notified: &'a mut bool,
    pub probe: &'a mut bool,
    pub refresh: &'a mut github::Refresh,
    pub sync_rx: &'a mut Option<Receiver<github::SectionEvent>>,
    pub last_sync: &'a mut Instant,
    pub last_status: &'a mut Instant,
    pub last_full: &'a mut Instant,
    pub link: &'a mut Option<github::StoreLink>,
    pub backfill: BackfillTest<'a>,
}

/// The backfill pacing and its page in flight.
pub struct BackfillTest<'a> {
    pub rx: &'a mut Option<Receiver<Result<github::BackfillStep, String>>>,
    pub months: &'a mut u32,
}

/// Diff payloads and their fetch state.
pub struct DiffTest<'a> {
    pub files: &'a mut HashMap<ItemRef, Vec<DiffFile>>,
    pub raw: &'a mut HashMap<ItemRef, Vec<DiffFile>>,
    pub heads: &'a mut HashMap<(ItemRef, String), Option<Vec<String>>>,
    pub heads_pending: &'a mut HashSet<(ItemRef, String)>,
    pub pending: &'a mut HashSet<ItemRef>,
    pub tx: &'a Sender<(ItemRef, Result<Vec<DiffFile>, String>)>,
    pub heads_tx: &'a Sender<HeadFiles>,
}

/// The board payload, card details and the fetch in flight.
pub struct BoardTest<'a> {
    pub data: &'a mut Option<BoardData>,
    pub loading: &'a mut bool,
    pub rx: &'a mut Option<Receiver<Result<BoardData, String>>>,
    pub details: &'a mut HashMap<ItemRef, BoardDetail>,
    pub detail_pending: &'a mut HashSet<ItemRef>,
}

/// Auth bookkeeping.
pub struct AuthTest<'a> {
    pub rx: &'a mut Option<Receiver<Result<Vec<String>, String>>>,
    pub bootstrap_rx: &'a mut Option<Receiver<Option<String>>>,
    pub reauth_requested: &'a mut bool,
    pub dismissed: &'a mut bool,
    pub degraded: &'a mut bool,
}

/// The refs the open item shows.
pub struct RefsTest<'a> {
    pub numbers: &'a mut Vec<ItemRef>,
    pub paired: &'a mut Vec<ItemRef>,
}

/// The pins, both for lookups and for listing.
pub struct PinsTest<'a> {
    pub set: &'a mut HashSet<ItemRef>,
    pub order: &'a mut Vec<ItemRef>,
}

/// Sub-issue loads by parent.
pub struct SubsTest<'a> {
    pub state: &'a mut HashMap<ItemRef, LazyChildren>,
}

/// Writes in flight.
pub struct SendsTest<'a> {
    pub submitting: &'a mut HashSet<ItemRef>,
    pub tx: &'a Sender<SendResult>,
    pub review_tx: &'a Sender<SendResult>,
}

impl Session {
    /// Test fixture: the cached payload of `n`, mutable.
    pub fn item_mut(&mut self, n: ItemRef) -> Option<&mut IssueData> {
        self.issues.data.get_mut(&n)
    }

    /// Test fixture: the board payload, mutable.
    pub fn board_mut(&mut self) -> Option<&mut BoardData> {
        self.board.data.as_mut()
    }

    /// Test fixture: the item cache.
    pub fn issues_test(&mut self) -> IssuesTest<'_> {
        IssuesTest {
            data: &mut self.issues.data,
            statuses: &mut self.issues.statuses,
            partial: &mut self.issues.partial,
            pending: &mut self.issues.pending,
            progress: &mut self.issues.progress,
            tx: &self.issues.tx,
        }
    }

    /// Test fixture: the listing feed.
    pub fn sections_test(&mut self) -> SectionsTest<'_> {
        SectionsTest {
            rx: &mut self.sections.rx,
            merged_window_hours: &mut self.sections.merged_window_hours,
            replace: &mut self.sections.replace,
            last_fetch: &mut self.sections.last_fetch,
            progress: &mut self.sections.progress,
            auto_refresh: &mut self.sections.auto_refresh,
            auto_refresh_interval: &mut self.sections.auto_refresh_interval,
            cap_notified: &mut self.sections.cap_notified,
            probe: &mut self.sections.probe,
            refresh: &mut self.sections.refresh,
            sync_rx: &mut self.sections.sync_rx,
            last_sync: &mut self.sections.last_sync,
            last_status: &mut self.sections.last_status,
            last_full: &mut self.sections.last_full,
            link: &mut self.sections.link,
            backfill: BackfillTest {
                rx: &mut self.sections.backfill.rx,
                months: &mut self.sections.backfill.months,
            },
        }
    }

    /// Test fixture: diff payloads.
    pub fn diff_test(&mut self) -> DiffTest<'_> {
        DiffTest {
            files: &mut self.diff.files,
            raw: &mut self.diff.raw,
            heads: &mut self.diff.heads,
            heads_pending: &mut self.diff.heads_pending,
            pending: &mut self.diff.pending,
            tx: &self.diff.tx,
            heads_tx: &self.diff.heads_tx,
        }
    }

    /// Test fixture: the board payload.
    pub fn board_test(&mut self) -> BoardTest<'_> {
        BoardTest {
            data: &mut self.board.data,
            loading: &mut self.board.loading,
            rx: &mut self.board.rx,
            details: &mut self.board.details,
            detail_pending: &mut self.board.detail_pending,
        }
    }

    /// Test fixture: the auth bookkeeping.
    pub fn auth_test(&mut self) -> AuthTest<'_> {
        AuthTest {
            rx: &mut self.auth.rx,
            bootstrap_rx: &mut self.auth.bootstrap_rx,
            reauth_requested: &mut self.auth.reauth_requested,
            dismissed: &mut self.auth.dismissed,
            degraded: &mut self.auth.degraded,
        }
    }

    /// Test fixture: the refs of the open item.
    pub fn refs_test(&mut self) -> RefsTest<'_> {
        RefsTest {
            numbers: &mut self.refs.numbers,
            paired: &mut self.refs.paired,
        }
    }

    /// Test fixture: the pins.
    pub fn pins_test(&mut self) -> PinsTest<'_> {
        PinsTest {
            set: &mut self.pins.set,
            order: &mut self.pins.order,
        }
    }

    /// Test fixture: sub-issue loads.
    pub fn subs_test(&mut self) -> SubsTest<'_> {
        SubsTest {
            state: &mut self.subs.state,
        }
    }

    /// Test fixture: the writes in flight.
    pub fn sends_test(&mut self) -> SendsTest<'_> {
        SendsTest {
            submitting: &mut self.sends.submitting,
            tx: &self.sends.tx,
            review_tx: &self.sends.review_tx,
        }
    }

    /// Test fixture: the fetch queue and its run count.
    pub fn fetches_test(&mut self) -> &mut Fetches {
        &mut self.fetches
    }

    /// Test fixture: set the repo the session works on.
    pub fn set_repo_slug(&mut self, slug: &str) {
        self.repo_slug = Some(slug.to_string());
    }

    /// Test fixture: the items asked for at startup.
    pub fn set_initial_numbers(&mut self, numbers: Vec<ItemRef>) {
        self.initial_numbers = numbers;
    }
}
