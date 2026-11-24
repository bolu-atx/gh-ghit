//! Section feed: the recent/search listing channel with its refresh cadence and sync state.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::Session;
use crate::github;

/// Background section feed: the recent/search listing channel plus its
/// refresh cadence, auto-refresh state, page progress, replace-vs-append
/// mode, and the merged-PR window.
pub(crate) struct SectionFeed {
    pub(crate) rx: Option<Receiver<github::SectionEvent>>,
    pub(crate) last_fetch: Instant,
    pub(crate) auto_refresh_interval: Duration,
    pub(crate) auto_refresh: bool,
    /// The next spawned fetch runs the change probe first.
    pub(crate) probe: bool,
    /// How the next spawned fetch runs: change-only, with a status
    /// refresh, or a full walk of every list.
    pub(crate) refresh: github::Refresh,
    /// When the last status refresh or full walk started.
    pub(crate) last_status: Instant,
    /// When the last full walk started.
    pub(crate) last_full: Instant,
    /// The lease holder's listing sync while a query shows: it writes the
    /// store and lands nothing, so the query result stays. Its own clock
    /// paces it, apart from the query's refresh.
    pub(crate) sync_rx: Option<Receiver<github::SectionEvent>>,
    pub(crate) last_sync: Instant,
    /// Section page progress: rows fetched vs the row cap, while pages
    /// remain. Cleared when the terminal result lands.
    pub(crate) progress: Option<(usize, usize)>,
    /// The next section payload replaces (search) instead of appending.
    pub(crate) replace: bool,
    pub(crate) merged_window_hours: u64,
    /// A cap notice already showed for this fetch setting, so the periodic
    /// auto-refresh never repeats it.
    pub(crate) cap_notified: bool,
    /// The repo store's sync lease and change watch, opened at startup.
    /// `None` (no repo, or the store cannot open) syncs on its own.
    pub(crate) link: Option<github::StoreLink>,
    /// Store backfill pacing and progress.
    pub(crate) backfill: super::backfill::BackfillFeed,
}

impl SectionFeed {
    pub fn new() -> Self {
        Self { rx: None, last_fetch: Instant::now(),
            auto_refresh_interval: auto_refresh_interval_default(),
            auto_refresh: false, probe: false, refresh: github::Refresh::Delta,
            last_status: Instant::now(), last_full: Instant::now(),
            sync_rx: None, last_sync: Instant::now(),
            progress: None, replace: false, merged_window_hours: 24,
            cap_notified: false, link: None,
            backfill: super::backfill::BackfillFeed::new() }
    }
}

impl Default for SectionFeed {
    fn default() -> Self {
        Self::new()
    }
}


pub const RECENT_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
pub const RECENT_REFRESH_MAX_INTERVAL: Duration = Duration::from_secs(60);
/// CI and project-status changes leave `updated_at` alone, so neither the
/// probe nor the change-only search sees them. Past this span since the
/// last status refresh, an auto-refresh re-reads the status of the open
/// rows, probe or no probe.
pub const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
/// Backstop: past this span since the last full walk, an auto-refresh
/// walks every list so rows the change-only search missed converge.
pub const FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(1800);

/// Which refresh an auto-refresh runs, given the time since the last
/// status refresh and since the last full walk.
pub fn refresh_plan(since_status: Duration, since_full: Duration) -> github::Refresh {
    if since_full >= FULL_REFRESH_INTERVAL {
        github::Refresh::Full
    } else if since_status >= STATUS_REFRESH_INTERVAL {
        github::Refresh::Status
    } else {
        github::Refresh::Delta
    }
}

/// Auto-refresh cadence, overridable for deterministic runs: the e2e
/// harness stretches it past any scenario lifetime so each scenario
/// issues exactly one sections fetch per boot instead of hammering
/// the API every 30s. Zero or garbage falls back to the default.
pub fn auto_refresh_interval_default() -> Duration {
    refresh_interval_from(std::env::var("GHIT_AUTO_REFRESH_SECS").ok().as_deref())
}

/// Parse the override value. Pure, so tests never mutate the process
/// env, which `App::new` in parallel tests also reads.
pub fn refresh_interval_from(raw: Option<&str>) -> Duration {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .map(Duration::from_secs)
        .unwrap_or(RECENT_REFRESH_INTERVAL)
}

impl Session {
    /// Set the merged/closed window the listing reads, in hours; 0 is all
    /// time.
    pub fn set_merged_window(&mut self, hours: u64) {
        self.sections.merged_window_hours = hours;
    }

    /// Set the backfill age limit in months from the prefs; 0 is all
    /// history.
    pub fn set_backfill_months(&mut self, months: u32) {
        self.sections.backfill.months = months;
    }

    /// The next listing fetch may announce the row cap again.
    pub fn reset_cap_notice(&mut self) {
        self.sections.cap_notified = false;
    }

    /// A search is still in flight when the listing takes over: drop it,
    /// so its rows cannot replace the ones the local view lands.
    pub fn cancel_pending_search(&mut self) {
        if !std::mem::take(&mut self.sections.replace) {
            return;
        }
        self.sections.rx = None;
        self.sections.progress = None;
        self.sections.auto_refresh = false;
    }

    /// Local rows are landing: drop any listing fetch in flight and
    /// restart the cadence, so nothing lands over them.
    pub fn settle_local_rows(&mut self) {
        self.sections.rx = None;
        self.sections.progress = None;
        self.sections.replace = false;
        self.sections.auto_refresh = false;
        self.sections.last_fetch = Instant::now();
    }

    /// The next listing fetch walks every list instead of probing first.
    pub fn request_full_listing(&mut self) {
        self.sections.refresh = github::Refresh::Full;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_refresh_interval_env_override() {
        assert_eq!(refresh_interval_from(None), RECENT_REFRESH_INTERVAL);
        assert_eq!(refresh_interval_from(Some("3600")), Duration::from_secs(3600));
        for bad in ["0", "nope", ""] {
            assert_eq!(refresh_interval_from(Some(bad)), RECENT_REFRESH_INTERVAL, "reject {bad}");
        }
    }

    #[test]
    fn refresh_plan_picks_delta_status_or_full() {
        let secs = Duration::from_secs;
        assert_eq!(refresh_plan(secs(30), secs(30)), github::Refresh::Delta);
        assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL - secs(1), secs(60)), github::Refresh::Delta);
        assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL, secs(600)), github::Refresh::Status);
        assert_eq!(refresh_plan(secs(10), FULL_REFRESH_INTERVAL - secs(1)), github::Refresh::Delta);
        assert_eq!(refresh_plan(STATUS_REFRESH_INTERVAL, FULL_REFRESH_INTERVAL), github::Refresh::Full);
        assert_eq!(refresh_plan(secs(0), FULL_REFRESH_INTERVAL), github::Refresh::Full);
    }
}
