//! Commands that start board, diff and write work on the shared runtime.

use super::{ComposeTarget, Session};
use crate::core::Kind;
use crate::fetch_queue::View;
use crate::runtime::spawn_bg;
use crate::{github, model};
use crate::model::ItemRef;

impl Session {
    fn repo_arg(&self) -> String {
        self.repo_flag.clone().or_else(|| self.repo_slug.clone()).unwrap_or_default()
    }

    /// Start the board fetch for `owner`; the result lands as a board event.
    pub fn load_board(&mut self, owner: String, view: &View) {
        self.board.loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        self.board.rx = Some(rx);
        self.enqueue(Kind::Other, None, async move {
            let _ = tx.send(github::fetch_board(&owner).await);
        }, view);
    }

    /// Drop the board cards. A fetch still in flight is ignored when it lands.
    pub fn close_board(&mut self) {
        self.board.data = None;
        self.board.loading = false;
    }

    /// Fetch missing board card details together without blocking.
    pub fn load_board_details(&mut self, view: &View) {
        let Some(board) = &self.board.data else { return };
        let mut cards = Vec::new();
        for item in &board.items {
            let (Some(n), Some(repo)) = (item.number, item.repo.clone()) else { continue };
            if self.board.details.contains_key(&n) || self.board.detail_pending.contains(&n) { continue; }
            if let Some(d) = self.issues.data.get(&n) {
                self.board.details.insert(n, model::BoardDetail::from_issue(d));
                continue;
            }
            self.board.detail_pending.insert(n);
            cards.push((repo, n));
        }
        if cards.is_empty() { return; }
        let tx = self.board.detail_tx.clone();
        self.enqueue(Kind::Other, None, async move {
            match github::fetch_board_details(&cards).await {
                Ok(details) => {
                    let mut missing = Vec::new();
                    for ((repo, n), detail) in cards.into_iter().zip(details) {
                        match detail {
                            Some(detail) => { let _ = tx.send((n, Ok(detail))); }
                            None => missing.push((repo, n)),
                        }
                    }
                    for (repo, n) in missing {
                        let _ = tx.send((n, github::fetch_board_detail(&repo, n).await));
                    }
                }
                Err(e) => for (_, n) in cards { let _ = tx.send((n, Err(e.clone()))); },
            }
        }, view);
    }

    /// Queue a diff fetch for PR `number`. No-op for issues, cached rows,
    /// and rows already in flight.
    pub fn load_diff(&mut self, number: ItemRef, force: bool, view: &View) {
        let Some(d) = self.issues.data.get(&number) else { return };
        if !d.is_pr {
            return;
        }
        if !force && self.diff.files.contains_key(&number) {
            return;
        }
        if !self.diff.pending.insert(number) {
            return;
        }
        let tx = self.diff.tx.clone();
        let repo = self.repo_arg();
        self.enqueue(Kind::Diff, Some(number), async move {
            let _ = tx.send((number, github::fetch_diff(&repo, number, force).await));
        }, view);
    }

    /// Queue one fetch of the head side of `names` in PR `n`.
    pub fn load_head_files(&mut self, n: ItemRef, names: Vec<String>, view: &View) {
        self.diff.heads_pending.extend(names.iter().map(|name| (n, name.clone())));
        let tx = self.diff.heads_tx.clone();
        let repo = self.repo_arg();
        self.enqueue(Kind::Diff, Some(n), async move {
            let result = github::fetch_head_files(&repo, n, names.clone()).await;
            let _ = tx.send((n, names, result));
        }, view);
    }

    /// Post a drafted comment on a background thread.
    pub fn send_draft(&mut self, number: ItemRef, target: ComposeTarget, body: String) {
        let repo = self.repo_arg();
        // A REST reply names a numeric comment id, not the GraphQL
        // thread id the cursor carries: resolve it from the loaded
        // thread before sending.
        let reply_comment_id = match &target {
            ComposeTarget::Reply { thread_id } => self
                .issues
                .data
                .get(&number)
                .and_then(|d| model::thread_reply_comment_id(&d.review_threads, thread_id)),
            _ => None,
        };
        let tx = self.sends.tx.clone();
        spawn_bg(async move {
            let result = match &target {
                ComposeTarget::Thread => github::post_thread_comment(&repo, number, &body).await,
                ComposeTarget::Line { path, line } => github::post_line_comment(&repo, number, &body, path, *line).await,
                ComposeTarget::Reply { .. } => match reply_comment_id {
                    Some(comment_id) => github::post_thread_reply(&repo, number, comment_id, &body).await,
                    None => Err("reply needs a review comment: refresh the PR to reload its threads".to_string()),
                },
            };
            let _ = tx.send((number, result));
        });
    }

    /// Submit the viewer's pending review of PR `n`. False while one is
    /// already in flight for it.
    pub fn submit_review(&mut self, n: ItemRef) -> bool {
        if !self.sends.submitting.insert(n) {
            return false;
        }
        let repo = self.repo_arg();
        let tx = self.sends.review_tx.clone();
        spawn_bg(async move {
            let result = github::submit_pending_review(&repo, n).await;
            let _ = tx.send((n, result));
        });
        true
    }

    /// Close (`open` false) or reopen an item on a background thread.
    pub fn set_state(&mut self, number: ItemRef, open: bool) {
        let repo = self.repo_arg();
        let tx = self.sends.state_tx.clone();
        spawn_bg(async move {
            let result = github::set_state(&repo, number, open).await;
            let _ = tx.send((number, open, result));
        });
    }
}
