//! Priority queue for background fetches. Requests wait here; at most
//! `MAX_RUNNING` run at once. The tier of a request is computed when the
//! scheduler picks, not when it is queued, so switching the current
//! issue moves its requests up and the old issue's requests down.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::core::{urgency, Kind, Tier};

/// Most fetches in flight at once.
pub(super) const MAX_RUNNING: usize = 4;

/// What the scheduler needs to know about the screen.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct View {
    /// The issue in the main pane, or the one about to be.
    pub current: Option<u64>,
    /// The Diff tab is up.
    pub tab_shown: bool,
}

struct Entry<J> {
    kind: Kind,
    number: Option<u64>,
    job: J,
}

/// Pending requests in arrival order. Generic over the job so tests can
/// use plain values.
pub(super) struct FetchQueue<J> {
    entries: Vec<Entry<J>>,
}

impl<J> Default for FetchQueue<J> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<J> FetchQueue<J> {
    /// Queue a request. A newer sections request replaces a queued older one.
    pub fn push(&mut self, kind: Kind, number: Option<u64>, job: J) {
        if kind == Kind::Sections {
            self.entries.retain(|e| e.kind != Kind::Sections);
        }
        self.entries.push(Entry { kind, number, job });
    }

    fn tier(entry: &Entry<J>, view: &View) -> Tier {
        let is_current = entry.number.is_some() && entry.number == view.current;
        urgency(entry.kind, is_current, view.tab_shown)
    }

    /// Take the next request to start: lowest tier first, oldest first
    /// inside a tier. While `limited`, only the current tier may start.
    pub fn next(&mut self, view: &View, limited: bool) -> Option<J> {
        let (index, _) = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, Self::tier(e, view)))
            .filter(|(_, tier)| !limited || *tier == Tier::Current)
            .min_by_key(|(_, tier)| *tier)?;
        Some(self.entries.remove(index).job)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Pop order under `view`, for tests.
    #[cfg(test)]
    pub fn order(&mut self, view: &View) -> Vec<(Kind, Option<u64>)> {
        let mut out = Vec::new();
        while let Some(i) = (0..self.entries.len()).min_by_key(|i| Self::tier(&self.entries[*i], view)) {
            let e = self.entries.remove(i);
            out.push((e.kind, e.number));
        }
        out
    }
}

type Task = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Frees its slot when the task ends, panics included.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// The queue plus the running count.
pub(super) struct Fetches {
    queue: FetchQueue<Task>,
    running: Arc<AtomicUsize>,
    pub limit: usize,
    /// Tests freeze the queue to inspect its order.
    #[cfg(test)]
    pub hold: bool,
    /// Tests run queued tasks on their own runtime, so other tests'
    /// fetches on the shared one cannot starve them.
    #[cfg(test)]
    pub runtime: Option<tokio::runtime::Handle>,
}

impl Default for Fetches {
    fn default() -> Self {
        Self {
            queue: FetchQueue::default(),
            running: Arc::new(AtomicUsize::new(0)),
            limit: MAX_RUNNING,
            #[cfg(test)]
            hold: false,
            #[cfg(test)]
            runtime: None,
        }
    }
}

impl Fetches {
    pub fn push(&mut self, kind: Kind, number: Option<u64>, task: Task) {
        self.queue.push(kind, number, task);
    }

    /// Start queued requests. Current-tier requests always start, so slow
    /// background fetches never delay what the main pane shows; the rest
    /// start while slots are free.
    pub fn start(&mut self, view: &View, limited: bool) {
        #[cfg(test)]
        if self.hold {
            return;
        }
        while let Some(task) = self.queue.next(view, true) {
            self.spawn(task);
        }
        while self.running.load(Ordering::Relaxed) < self.limit {
            let Some(task) = self.queue.next(view, limited) else { return };
            self.spawn(task);
        }
    }

    fn spawn(&mut self, task: Task) {
        self.running.fetch_add(1, Ordering::Relaxed);
        let slot = Slot(self.running.clone());
        let run = async move {
            let _slot = slot;
            task.await;
        };
        #[cfg(test)]
        if let Some(handle) = &self.runtime {
            handle.spawn(run);
            return;
        }
        super::spawn_bg(run);
    }

    #[cfg(test)]
    pub fn queue_mut(&mut self) -> &mut FetchQueue<Task> {
        &mut self.queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(current: Option<u64>, tab_shown: bool) -> View {
        View { current, tab_shown }
    }

    fn drain(q: &mut FetchQueue<u32>, v: &View, limited: bool) -> Vec<u32> {
        std::iter::from_fn(|| q.next(v, limited)).collect()
    }

    #[test]
    fn higher_tier_starts_first_then_arrival_order() {
        let mut q = FetchQueue::default();
        q.push(Kind::Other, None, 1);
        q.push(Kind::Details, Some(9), 2);
        q.push(Kind::Sections, None, 3);
        q.push(Kind::Details, Some(5), 4);
        q.push(Kind::Details, Some(5), 5);
        assert_eq!(drain(&mut q, &view(Some(5), false), false), vec![4, 5, 3, 1, 2]);
    }

    #[test]
    fn switching_current_reorders_pending_requests() {
        let mut q = FetchQueue::default();
        q.push(Kind::Details, Some(1), 1);
        q.push(Kind::Details, Some(2), 2);
        q.push(Kind::Sections, None, 3);
        assert_eq!(q.next(&view(Some(1), false), false), Some(1));
        let mut q = FetchQueue::default();
        q.push(Kind::Details, Some(1), 1);
        q.push(Kind::Details, Some(2), 2);
        q.push(Kind::Sections, None, 3);
        // The user moves to #2: its request jumps ahead, #1 drops behind sections.
        assert_eq!(drain(&mut q, &view(Some(2), false), false), vec![2, 3, 1]);
    }

    #[test]
    fn hidden_tab_fetches_wait_until_the_tab_shows() {
        let mut q = FetchQueue::default();
        q.push(Kind::Diff, Some(7), 1);
        q.push(Kind::Sections, None, 2);
        assert_eq!(q.next(&view(Some(7), false), false), Some(2));
        q.push(Kind::Sections, None, 3);
        assert_eq!(q.next(&view(Some(7), true), false), Some(1));
    }

    #[test]
    fn limited_mode_starts_only_the_current_tier() {
        let mut q = FetchQueue::default();
        q.push(Kind::Sections, None, 1);
        q.push(Kind::Details, Some(4), 2);
        q.push(Kind::Other, None, 3);
        let v = view(Some(4), false);
        assert_eq!(drain(&mut q, &v, true), vec![2]);
        assert_eq!(q.len(), 2, "lower tiers keep waiting");
        assert_eq!(drain(&mut q, &v, false), vec![1, 3], "they start once the limit clears");
    }

    #[test]
    fn newer_sections_request_replaces_the_queued_one() {
        let mut q = FetchQueue::default();
        q.push(Kind::Sections, None, 1);
        q.push(Kind::Sections, None, 2);
        assert_eq!(drain(&mut q, &view(None, false), false), vec![2]);
    }

    #[test]
    fn concurrency_cap_holds_and_slots_free_on_completion() {
        let own = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let mut f = Fetches { runtime: Some(own.handle().clone()), ..Fetches::default() };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for n in 0..6u64 {
            let rx = rx.clone();
            f.push(Kind::Other, Some(n), Box::pin(async move {
                let rx = rx.clone();
                let _ = tokio::task::spawn_blocking(move || rx.lock().unwrap().recv()).await;
            }));
        }
        f.start(&view(None, false), false);
        assert_eq!(f.running.load(Ordering::Relaxed), MAX_RUNNING);
        assert_eq!(f.queue.len(), 6 - MAX_RUNNING);
        drop(tx);
        for _ in 0..500 {
            f.start(&view(None, false), false);
            if f.queue.len() == 0 && f.running.load(Ordering::Relaxed) == 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("queue never drained");
    }

    #[test]
    fn current_issue_starts_even_when_background_fills_every_slot() {
        let own = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let mut f = Fetches { runtime: Some(own.handle().clone()), ..Fetches::default() };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let blocked = |rx: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>| -> Task {
            Box::pin(async move {
                let _ = tokio::task::spawn_blocking(move || rx.lock().unwrap().recv()).await;
            })
        };
        for n in 0..MAX_RUNNING as u64 {
            f.push(Kind::Other, Some(n), blocked(rx.clone()));
        }
        f.start(&view(Some(42), false), false);
        assert_eq!(f.running.load(Ordering::Relaxed), MAX_RUNNING);
        f.push(Kind::Details, Some(42), blocked(rx.clone()));
        f.start(&view(Some(42), false), false);
        assert_eq!(f.queue.len(), 0, "the current issue never waits for a slot");
        assert_eq!(f.running.load(Ordering::Relaxed), MAX_RUNNING + 1);
        drop(tx);
    }

    #[test]
    fn tests_drop_fetches_instead_of_spawning_them() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        let mut f = Fetches::default();
        f.push(Kind::Other, None, Box::pin(async move { flag.store(true, Ordering::Relaxed) }));
        f.start(&view(None, false), false);
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!ran.load(Ordering::Relaxed), "a unit test started a real fetch");
        assert_eq!(f.running.load(Ordering::Relaxed), 0);
    }
}
