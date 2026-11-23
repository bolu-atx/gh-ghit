//! Store backfill scheduling: the sync-lease holder fills the store with
//! the repo's working set, one page per step, at the lowest fetch tier.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::app::App;
use ghit_core::{core, github};

/// Pause between two pages (and before the first), so the backfill never
/// bursts and the startup sync goes first.
pub(super) const BACKFILL_DELAY: Duration = Duration::from_secs(2);

/// Wait after a failed page before the next try.
const BACKFILL_RETRY: Duration = Duration::from_secs(60);

type StepResult = Result<github::BackfillStep, String>;

/// Backfill state of one process. The store holds the real progress; this
/// only paces the steps and feeds the status bar.
pub(crate) struct BackfillFeed {
    pub(super) rx: Option<Receiver<StepResult>>,
    next_at: Instant,
    /// A step ran and the backfill is not done: the status bar shows it.
    running: bool,
    done: bool,
    /// `GHIT_NO_BACKFILL` is set (the e2e harness sets it).
    off: bool,
    /// Age limit in months from the prefs; 0 = all history.
    pub(crate) months: u32,
    progress: Option<(usize, usize)>,
}

impl BackfillFeed {
    pub(crate) fn new() -> Self {
        Self {
            rx: None,
            next_at: Instant::now() + BACKFILL_DELAY,
            running: false,
            done: false,
            off: std::env::var_os("GHIT_NO_BACKFILL").is_some(),
            months: 12,
            progress: None,
        }
    }

    /// A step may start: only the lease holder, with no step in flight,
    /// no quota limit, and the pause since the last page over.
    fn due(&self, holder: bool, limited: bool, now: Instant) -> bool {
        holder && !self.off && !self.done && self.rx.is_none() && !limited && now >= self.next_at
    }

    /// Take a finished step's result and schedule the next one.
    fn land(&mut self, result: StepResult, now: Instant) {
        match result {
            Ok(step) => {
                self.progress = step.progress;
                self.done = step.done;
                self.running = !step.done;
                self.next_at = now + BACKFILL_DELAY.max(Duration::from_secs(step.pause_secs));
            }
            Err(_) => self.next_at = now + BACKFILL_RETRY,
        }
    }
}

impl App {
    /// True while this process holds the sync lease through a store link.
    /// Without a link there is no store to fill.
    fn backfill_holder(&self) -> bool {
        self.sections.link.as_ref().is_some_and(github::StoreLink::holding)
    }

    /// Status bar input: the backfill runs here, with its progress.
    pub(crate) fn backfill_state(&self) -> (bool, Option<(usize, usize)>) {
        let feed = &self.sections.backfill;
        (feed.running && self.backfill_holder(), feed.progress)
    }

    /// Per loop pass: land a finished step, then queue the next page when
    /// one is due. True when visible state changed.
    pub(super) fn drive_backfill(&mut self, now: Instant) -> bool {
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
                    self.sections.backfill.next_at = now + BACKFILL_RETRY;
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
        self.enqueue(core::Kind::Backfill, None, async move {
            let _ = tx.send(github::backfill_step(&repo, months).await);
        });
        landed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(progress: Option<(usize, usize)>, done: bool, pause_secs: u64) -> StepResult {
        Ok(github::BackfillStep { progress, done, pause_secs })
    }

    #[test]
    fn only_the_idle_lease_holder_is_due() {
        let mut feed = BackfillFeed::new();
        feed.off = false;
        let now = feed.next_at;
        assert!(feed.due(true, false, now));
        assert!(!feed.due(false, false, now), "a non-holder never backfills");
        assert!(!feed.due(true, true, now), "not while rate limited");
        assert!(!feed.due(true, false, now - Duration::from_millis(1)), "the delay holds it back");
        feed.off = true;
        assert!(!feed.due(true, false, now), "GHIT_NO_BACKFILL switches it off");
        feed.off = false;
        feed.rx = Some(std::sync::mpsc::channel().1);
        assert!(!feed.due(true, false, now), "one page in flight at a time");
    }

    #[test]
    fn landing_a_step_paces_the_next_and_stops_when_done() {
        let mut feed = BackfillFeed::new();
        let now = Instant::now();
        feed.land(step(Some((100, 500)), false, 0), now);
        assert!(feed.running && !feed.done && feed.progress == Some((100, 500)));
        assert_eq!(feed.next_at, now + BACKFILL_DELAY, "a small delay between pages");
        feed.land(step(Some((200, 500)), false, 120), now);
        assert_eq!(feed.next_at, now + Duration::from_secs(120), "a low quota waits for the reset");
        feed.land(Err("boom".into()), now);
        assert!(feed.running, "a failed page keeps the walk");
        assert_eq!(feed.next_at, now + BACKFILL_RETRY);
        feed.land(step(None, true, 0), now);
        assert!(feed.done && !feed.running && feed.progress.is_none());
        assert!(!feed.due(true, false, now + Duration::from_secs(3600)), "done never reruns");
    }
}
