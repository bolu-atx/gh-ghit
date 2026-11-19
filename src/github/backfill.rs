//! Store backfill: one page of the repo's working set per step. Progress
//! lives in the store, so every step resumes where the last one (of any
//! process) stopped.

use super::{require_repo, require_token};
use crate::api::{self, Conn};
use crate::store::{self, BackfillConn};

/// Below this many remaining GraphQL points the backfill waits for the
/// quota reset, well above the client's own low-points backoff.
const RESERVE_POINTS: u64 = 500;

/// What one step reports to the scheduler.
#[derive(Debug, PartialEq, Eq)]
pub struct BackfillStep {
    /// `(rows stored, rows expected)` across the connections.
    pub progress: Option<(usize, usize)>,
    /// Every connection is done: the backfill never runs again.
    pub done: bool,
    /// Seconds to wait before the next step because the quota is low.
    pub pause_secs: u64,
}

/// The first unfinished connection and its stored cursor.
fn next_conn(states: &[(Conn, BackfillConn)]) -> Option<(Conn, Option<String>)> {
    states.iter().find(|(_, s)| !s.done).map(|(c, s)| (*c, s.cursor.clone()))
}

/// Rows stored vs rows expected. A finished connection expects what it
/// stored (an age limit stops it short of `totalCount`); one that has not
/// started adds nothing yet.
fn progress(states: &[(Conn, BackfillConn)]) -> (usize, usize) {
    states.iter().fold((0, 0), |(d, t), (_, s)| {
        let expected = if s.done { s.fetched } else { s.total.max(s.fetched) };
        (d + s.fetched as usize, t + expected as usize)
    })
}

/// Seconds until the quota resets when the reply left fewer than
/// `RESERVE_POINTS`; 0 otherwise.
fn pause_secs(quota: Option<(u64, u64)>, now: u64) -> u64 {
    match quota {
        Some((remaining, reset)) if remaining < RESERVE_POINTS => reset.saturating_sub(now),
        _ => 0,
    }
}

fn load_states(repo: &str) -> Result<Vec<(Conn, BackfillConn)>, String> {
    let store = store::open(repo)?;
    Ok(Conn::ALL.iter().map(|c| (*c, store.backfill_conn(c.name()))).collect())
}

fn step_report(states: &[(Conn, BackfillConn)], pause_secs: u64) -> BackfillStep {
    let done = next_conn(states).is_none();
    BackfillStep { progress: (!done).then(|| progress(states)), done, pause_secs }
}

/// Fetch and store one page of the first unfinished connection. With
/// every connection done it makes no request. `months` is the age limit
/// (0 = all history).
pub async fn backfill_step(repo: &str, months: u32) -> Result<BackfillStep, String> {
    require_repo(repo)?;
    let read_repo = repo.to_string();
    let states = tokio::task::spawn_blocking(move || load_states(&read_repo)).await.map_err(|e| e.to_string())??;
    let Some((conn, cursor)) = next_conn(&states) else { return Ok(step_report(&states, 0)) };
    let token = require_token()?;
    let cutoff = api::age_cutoff(chrono::Utc::now(), months);
    let page = api::fetch_page(repo, conn, cursor.as_deref(), cutoff, &token).await?;
    let pause = pause_secs(page.quota, chrono::Utc::now().timestamp().max(0) as u64);
    let save_repo = repo.to_string();
    let states = tokio::task::spawn_blocking(move || {
        let mut store = store::open(&save_repo)?;
        // False: another process advanced this connection first; its
        // progress stands and the next step reads it.
        store.backfill_save(conn.name(), cursor.as_deref(), &page.rows, page.next.as_deref(), page.done, page.total)?;
        load_states(&save_repo)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(step_report(&states, pause))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(cursor: Option<&str>, done: bool, fetched: u64, total: u64) -> BackfillConn {
        BackfillConn { cursor: cursor.map(str::to_owned), done, fetched, total }
    }

    fn all(states: [BackfillConn; 4]) -> Vec<(Conn, BackfillConn)> {
        Conn::ALL.into_iter().zip(states).collect()
    }

    #[test]
    fn next_step_resumes_from_the_stored_cursor() {
        let fresh = all(Default::default());
        assert_eq!(next_conn(&fresh), Some((Conn::PrsOpen, None)));
        let mid = all([state(None, true, 10, 10), state(Some("abc"), false, 100, 250), state(None, false, 0, 0), state(None, false, 0, 0)]);
        assert_eq!(next_conn(&mid), Some((Conn::IssuesOpen, Some("abc".to_string()))));
    }

    #[test]
    fn finished_backfill_has_no_next_step_and_hides_progress() {
        let done = all([state(None, true, 1, 1), state(None, true, 2, 9), state(None, true, 3, 3), state(None, true, 4, 4)]);
        assert_eq!(next_conn(&done), None);
        assert_eq!(step_report(&done, 0), BackfillStep { progress: None, done: true, pause_secs: 0 });
    }

    #[test]
    fn progress_sums_stored_over_expected() {
        let mid = all([state(None, true, 10, 30), state(Some("a"), false, 100, 250), state(None, false, 0, 0), state(None, false, 0, 0)]);
        assert_eq!(progress(&mid), (110, 260), "done connection expects what it stored");
        assert_eq!(step_report(&mid, 0).progress, Some((110, 260)));
    }

    #[test]
    fn low_quota_pauses_until_reset() {
        assert_eq!(pause_secs(Some((RESERVE_POINTS - 1, 1_100)), 1_000), 100);
        assert_eq!(pause_secs(Some((RESERVE_POINTS, 1_100)), 1_000), 0);
        assert_eq!(pause_secs(Some((10, 900)), 1_000), 0, "reset already passed");
        assert_eq!(pause_secs(None, 1_000), 0);
    }
}
