//! Compose drafts: cursor editing, `$EDITOR`, send and poll.

use std::io::Stdout;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::ComposeTarget;
use crate::app::App;
use ghit_core::model;
use crate::app::auth_failure;
use ghit_core::model::ItemRef;

/// Title naming what a draft answers; a thread comment adds the item's
/// title when it is loaded.
pub(crate) fn target_title(
    target: &ComposeTarget, label: &str, item_title: Option<&str>,
) -> String {
    match target {
        ComposeTarget::Thread => match item_title {
            Some(title) => format!("Comment on {label}  {title}"),
            None => format!("Comment on {label}"),
        },
        ComposeTarget::Line { path, line } => format!("Line comment  {path}:{line}"),
        ComposeTarget::Reply { .. } => {
            format!("Reply on {label}")
        }
    }
}

/// The compose modal: a draft plus where it goes. `cursor` is a char
/// index into `body` where typing lands; `sending` locks editing while
/// the background submit is in flight. `kill` is the readline kill ring
/// (one slot): kills stash text here, `Ctrl+Y` pastes it back.
pub(crate) struct ComposeState {
    pub(crate) number: ItemRef,
    pub(crate) target: ComposeTarget,
    pub(crate) body: String,
    pub(crate) cursor: usize,
    pub(crate) sending: bool,
    pub(crate) kill: String,
    pub(crate) view: ComposeView,
}

/// How the modal shows the draft: Write or Preview, the first visual row
/// (or preview line) in view, and the cursor `scroll` last followed.
/// Render follows the cursor only when it moved since then, so the wheel
/// can scroll away from it.
#[derive(Clone, Copy, Default)]
pub(crate) struct ComposeView {
    pub(crate) preview: bool,
    pub(crate) scroll: usize,
    pub(crate) followed: Option<usize>,
}

/// Compose slot: the `$EDITOR` handoff flag. Sends live in the session.
#[derive(Default)]
pub(crate) struct ComposeSlot {
    /// Set by input; the event loop suspends the TUI and runs `$EDITOR`
    /// on the draft outside it, then lands the edited text.
    pub(crate) editor_requested: bool,
}

impl App {
    /// The compose modal is up, whether drafting or sending.
    pub(crate) fn compose_open(&self) -> bool {
        self.draft().is_some()
    }

    /// Open the compose modal, deriving the target from context: on the
    /// Diff tab over a unified hunk row with a new-side line it drafts a
    /// line comment, everywhere else a thread comment. Split diffs fall
    /// back to a thread comment since paired rows carry no single line.
    pub(crate) fn open_compose(&mut self) {
        let Some(n) = self.nav.current else {
            self.notify("view an issue first".to_string());
            return;
        };
        if !self.session.has_item(n) {
            self.notify("still loading: wait for the issue first".to_string());
            return;
        }
        let target = self.compose_target().unwrap_or(ComposeTarget::Thread);
        self.open_draft(ComposeState { number: n, target, body: String::new(), cursor: 0, sending: false, kill: String::new(), view: ComposeView::default() });
    }

    /// Comment target for the active diff line, or `None` for a plain
    /// thread comment: not a PR diff, split layout, no file under the
    /// cursor, or no new-side line at the cursor. A cursor inside a
    /// rendered review-thread block answers a reply to that thread.
    fn compose_target(&self) -> Option<ComposeTarget> {
        if !self.visual_diff() || self.diff.split {
            return None;
        }
        let n = self.nav.current?;
        let files = self.session.diff_files(n)?;
        let fi = self.diff_file_at_cursor()?;
        let file = files.get(fi)?;
        let abs = self.diff.line?;
        let vis = abs.checked_sub(self.hits.body_base as usize)?;
        let header = self.hits.body_rows.iter().enumerate()
            .find_map(|(i, line)| (model::header_file_index(line, files) == Some(fi)).then_some(i))?;
        if vis <= header {
            return None;
        }
        let section: Vec<String> = self.hits.body_rows[header + 1..].to_vec();
        let cursor = vis - (header + 1);
        let d = self.session.item(n)?;
        if let Some(thread_id) = model::thread_at_cursor(&section, cursor, file, &d.review_threads, crate::theme::comment_marker()) {
            return Some(ComposeTarget::Reply { thread_id });
        }
        let line = model::line_target_at_cursor(&section, cursor, &file.patch)?;
        Some(ComposeTarget::Line { path: file.filename.clone(), line })
    }

    /// Discard the draft, whatever it holds. Quiet: Esc means gone.
    pub(crate) fn discard_compose(&mut self) {
        self.close(crate::app::Slot::Compose);
    }

    /// Byte offset of a char index, clamped to the end.
    fn compose_byte(body: &str, cursor: usize) -> usize {
        body.char_indices().nth(cursor).map(|(b, _)| b).unwrap_or(body.len())
    }

    /// Type a char at the cursor. Locked while sending so the in-flight
    /// body never changes under the request.
    pub(crate) fn compose_insert(&mut self, c: char) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        s.view.preview = false;
        let cursor = s.cursor.min(s.body.chars().count());
        let byte = Self::compose_byte(&s.body, cursor);
        s.body.insert(byte, c);
        s.cursor = cursor + 1;
    }

    /// Delete the char before the cursor.
    pub(crate) fn compose_backspace(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending || s.cursor == 0 {
            return;
        }
        s.view.preview = false;
        let cursor = s.cursor.min(s.body.chars().count());
        let end = Self::compose_byte(&s.body, cursor);
        let start = Self::compose_byte(&s.body, cursor - 1);
        s.body.drain(start..end);
        s.cursor = cursor - 1;
    }

    /// Step the cursor one char left or right.
    pub(crate) fn compose_step(&mut self, delta: isize) {
        let Some(s) = self.draft_mut() else { return };
        let len = s.body.chars().count();
        s.cursor = s.cursor.saturating_add_signed(delta).min(len);
    }

    /// Step the cursor one visual row up or down, keeping the column.
    /// Rows wrap at the width the modal last drew; before a first draw
    /// they are the logical lines.
    pub(crate) fn compose_line(&mut self, delta: isize) {
        let cells = match self.hits.compose_text.width {
            0 => usize::MAX,
            width => width as usize,
        };
        let Some(s) = self.draft_mut() else { return };
        let rows = ghit_core::core::text_rows(&s.body, cells);
        let (row, col) = ghit_core::core::cursor_pos(&rows, s.cursor.min(s.body.chars().count()));
        let next = row.saturating_add_signed(delta).min(rows.len() - 1);
        s.cursor = ghit_core::core::index_at(&rows, next, col);
    }

    /// Flip between Write and Preview (`Ctrl+R`).
    pub(crate) fn compose_toggle_preview(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        s.view = ComposeView { preview: !s.view.preview, scroll: 0, followed: None };
    }

    /// Scroll the draft or its preview by `delta` rows (wheel). Render
    /// clamps the offset to the content.
    pub(crate) fn compose_scroll(&mut self, delta: isize) {
        let Some(s) = self.draft_mut() else { return };
        s.view.scroll = s.view.scroll.saturating_add_signed(delta);
    }

    /// Put the cursor at the char under a click in the text area.
    pub(crate) fn compose_click(&mut self, column: u16, row: u16) {
        let (text, popup) = (self.hits.compose_text, self.hits.popup);
        let Some(s) = self.draft_mut() else { return };
        let inside = row >= text.y && row < text.bottom() && column >= popup.x && column < popup.right();
        if s.sending || s.view.preview || !inside {
            return;
        }
        let rows = ghit_core::core::text_rows(&s.body, text.width as usize);
        let visual = s.view.scroll + row.saturating_sub(text.y) as usize;
        s.cursor = ghit_core::core::index_at(&rows, visual, column.saturating_sub(text.x) as usize);
    }

    /// Line start of the cursor: the char after the previous `\n`, or 0.
    fn compose_line_start(body: &str, cursor: usize) -> usize {
        let head: String = body.chars().take(cursor.min(body.chars().count())).collect();
        head.rfind('\n').map(|b| head[..=b].chars().count()).unwrap_or(0)
    }

    /// Line end of the cursor: the next `\n`, or the body end.
    fn compose_line_end(body: &str, cursor: usize) -> usize {
        let cursor = cursor.min(body.chars().count());
        let mut end = cursor;
        for (i, ch) in body.chars().enumerate().skip(cursor) {
            if ch == '\n' {
                return i;
            }
            end = i + 1;
        }
        end
    }

    /// Word char for emacs word motions: letters, digits, `_`.
    fn compose_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Word start behind the cursor: skip whitespace, then word chars.
    fn compose_word_back_from(body: &str, cursor: usize) -> usize {
        let chars: Vec<char> = body.chars().collect();
        let mut i = cursor.min(chars.len());
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && Self::compose_word_char(chars[i - 1]) {
            i -= 1;
        }
        i
    }

    /// Word end ahead of the cursor: from mid-word to its end, else past
    /// the next word.
    fn compose_word_forward_from(body: &str, cursor: usize) -> usize {
        let chars: Vec<char> = body.chars().collect();
        let mut i = cursor.min(chars.len());
        if i < chars.len() && Self::compose_word_char(chars[i]) {
            while i < chars.len() && Self::compose_word_char(chars[i]) {
                i += 1;
            }
        } else {
            while i < chars.len() && !Self::compose_word_char(chars[i]) {
                i += 1;
            }
            while i < chars.len() && Self::compose_word_char(chars[i]) {
                i += 1;
            }
        }
        i
    }

    /// Drain `[start, end)` (char indices), stashing the cut text in the
    /// kill ring. Locked while sending so the in-flight body never changes.
    fn compose_cut(&mut self, start: usize, end: usize) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        let len = s.body.chars().count();
        let (start, end) = (start.min(len), end.min(len));
        if start >= end {
            return;
        }
        let killed: String = s.body.chars().skip(start).take(end - start).collect();
        let byte_start = Self::compose_byte(&s.body, start);
        let byte_end = Self::compose_byte(&s.body, end);
        s.body.drain(byte_start..byte_end);
        s.kill = killed;
        s.cursor = start;
    }

    /// Jump to the line start (`Ctrl+A`, `Home`).
    pub(crate) fn compose_home(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        s.cursor = Self::compose_line_start(&s.body, s.cursor);
    }

    /// Jump to the line end (`Ctrl+E`, `End`).
    pub(crate) fn compose_end(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        s.cursor = Self::compose_line_end(&s.body, s.cursor);
    }

    /// Jump one word back (`Alt+B`).
    pub(crate) fn compose_word_back(&mut self) {
        let (body, cursor, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_back_from(&body, cursor);
        if let Some(s) = self.draft_mut() {
            s.cursor = at;
        }
    }

    /// Jump one word forward (`Alt+F`).
    pub(crate) fn compose_word_forward(&mut self) {
        let (body, cursor, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_forward_from(&body, cursor);
        if let Some(s) = self.draft_mut() {
            s.cursor = at;
        }
    }

    /// Kill to the line end (`Ctrl+K`); at the end of a line the newline
    /// itself is the kill, joining the lines like bash does.
    pub(crate) fn compose_kill_to_end(&mut self) {
        let (body, cursor, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let len = body.chars().count();
        let cursor = cursor.min(len);
        let mut end = Self::compose_line_end(&body, cursor);
        if end == cursor && cursor < len {
            end = cursor + 1;
        }
        self.compose_cut(cursor, end);
    }

    /// Kill back to the line start (`Ctrl+U`).
    pub(crate) fn compose_kill_to_start(&mut self) {
        let (body, cursor, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let start = Self::compose_line_start(&body, cursor);
        self.compose_cut(start, cursor.min(body.chars().count()));
    }

    /// Delete the char ahead (`Ctrl+D`, `Delete`). A plain delete, not
    /// a kill: the kill ring keeps its text for yank. Never discards: an
    /// empty draft has nothing to delete.
    pub(crate) fn compose_delete_forward(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        let len = s.body.chars().count();
        let cursor = s.cursor.min(len);
        if cursor >= len {
            return;
        }
        let start = Self::compose_byte(&s.body, cursor);
        let end = Self::compose_byte(&s.body, cursor + 1);
        s.body.drain(start..end);
        s.cursor = cursor;
    }

    /// Delete the word behind (`Ctrl+W`).
    pub(crate) fn compose_delete_word_back(&mut self) {
        let (body, cursor, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let at = Self::compose_word_back_from(&body, cursor);
        self.compose_cut(at, cursor.min(body.chars().count()));
    }

    /// Delete the word ahead (`Alt+D`); the cursor stays put.
    pub(crate) fn compose_delete_word_forward(&mut self) {
        let (body, cursor, len, sending) = match self.draft() {
            Some(s) => (s.body.clone(), s.cursor, s.body.chars().count(), s.sending),
            None => return,
        };
        if sending {
            return;
        }
        let cursor = cursor.min(len);
        let end = Self::compose_word_forward_from(&body, cursor);
        if end == cursor {
            return;
        }
        let Some(s) = self.draft_mut() else { return };
        let killed: String = s.body.chars().skip(cursor).take(end - cursor).collect();
        let byte_start = Self::compose_byte(&s.body, cursor);
        let byte_end = Self::compose_byte(&s.body, end);
        s.body.drain(byte_start..byte_end);
        s.kill = killed;
        s.cursor = cursor;
    }

    /// Paste the kill ring back (`Ctrl+Y`).
    pub(crate) fn compose_yank(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending || s.kill.is_empty() {
            return;
        }
        let kill = s.kill.clone();
        let cursor = s.cursor.min(s.body.chars().count());
        let byte = Self::compose_byte(&s.body, cursor);
        s.body.insert_str(byte, &kill);
        s.cursor = cursor + kill.chars().count();
    }

    /// Swap the char before the cursor with the one under it (`Ctrl+T`);
    /// at the draft end that swaps the last two chars.
    pub(crate) fn compose_transpose(&mut self) {
        let Some(s) = self.draft_mut() else { return };
        if s.sending {
            return;
        }
        let chars: Vec<char> = s.body.chars().collect();
        let len = chars.len();
        if len < 2 {
            return;
        }
        let cursor = s.cursor.min(len);
        if cursor == 0 {
            return;
        }
        let (a, b) = if cursor >= len { (len - 2, len - 1) } else { (cursor - 1, cursor) };
        let mut next = chars;
        next.swap(a, b);
        s.body = next.into_iter().collect();
        s.cursor = if cursor >= len { len } else { cursor + 1 };
    }

    /// `$EDITOR` for the compose modal. Just the env var: no config
    /// plumbing, no fallback. `None` when unset or blank.
    pub(crate) fn compose_editor() -> Option<String> {
        std::env::var("EDITOR").ok().filter(|e| !e.trim().is_empty())
    }

    /// Arm the external editor run (`Ctrl+O` in the modal). With no
    /// `$EDITOR` there is nothing to run, so the draft stays and the
    /// toast names the fix.
    pub(crate) fn request_compose_editor(&mut self) {
        let sending = self.draft().map(|c| c.sending).unwrap_or(true);
        if sending {
            return;
        }
        if Self::compose_editor().is_none() {
            self.notify("no $EDITOR set: export EDITOR to edit comments outside ghit".to_string());
            return;
        }
        self.compose.editor_requested = true;
    }

    /// Land the external editor run: the file text replaces the draft
    /// verbatim and the cursor parks at the end.
    pub(crate) fn land_editor_text(&mut self, text: String, editor: &str) {
        let Some(c) = self.draft_mut() else { return };
        c.body = text;
        c.cursor = c.body.chars().count();
        self.notify(format!("edited in {editor}"));
    }

    /// Submit the draft on a background thread; the loop never blocks.
    /// Empty drafts stay open with a status-line error, never a send.
    pub(crate) fn submit_compose(&mut self) {
        let Some(c) = self.draft_mut() else { return };
        if c.sending {
            return;
        }
        if c.body.trim().is_empty() {
            self.notify("comment is empty: write something first".to_string());
            return;
        }
        c.sending = true;
        let (number, target, body) = (c.number, c.target.clone(), c.body.clone());
        self.session.send_draft(number, target, body);
        self.notify("sending comment…".to_string());
    }
}

/// Land a compose submit: success closes the modal, confirms on the
/// status line, and refetches the issue so the Comments tab shows the
/// new comment; failure keeps the draft and names the cause.
pub(super) fn land_draft_sent(app: &mut App, n: ItemRef, res: Result<(), String>) {
    match res {
        Ok(()) => {
            let pending = matches!(app.draft().map(|c| &c.target), Some(ComposeTarget::Line { .. } | ComposeTarget::Reply { .. }));
            app.close(crate::app::Slot::Compose);
            if pending {
                app.notify(format!(
                    "line comment pending on {n} — press ctrl-s to submit",
                    n = app.session.label(n)
                ));
            } else {
                app.notify(format!("comment posted to {n}", n = app.session.label(n)));
            }
            app.load_issue(n, true);
        }
        Err(e) => {
            if let Some(c) = app.draft_mut() {
                c.sending = false;
            }
            auth_failure(app, &e);
            app.notify(e);
        }
    }
}

/// Suspend the TUI, run `$EDITOR` on the draft, and land the result.
/// Mirrors `run_reauth`: the loop owns the terminal, so input only arms
/// `editor_requested` and the loop runs this with the terminal. The draft
/// stays put on every failure path, so no edit ever loses text.
pub(crate) fn run_compose_editor(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
    use crossterm::ExecutableCommand;
    app.compose.editor_requested = false;
    let Some(editor) = App::compose_editor() else {
        app.notify("no $EDITOR set: export EDITOR to edit comments outside ghit".to_string());
        return;
    };
    let drafting = app.draft().map(|c| !c.sending).unwrap_or(false);
    if !drafting {
        return;
    }
    // Stage the draft in a temp file so the editor opens real markdown.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut path = std::env::temp_dir();
    path.push(format!("ghit-comment-{}-{stamp}.md", std::process::id()));
    if std::fs::write(&path, &app.draft().expect("draft").body).is_err() {
        app.notify("editor needs a writable temp dir".to_string());
        return;
    }
    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let status = std::process::Command::new(&editor).arg(&path).status();
    let _ = enable_raw_mode();
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = terminal.clear();
    let text = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    match (status, text) {
        (Ok(s), Some(text)) if s.success() => app.land_editor_text(text, &editor),
        (Ok(_), _) => app.notify(format!("{editor} failed: draft kept")),
        (Err(e), _) => app.notify(format!("{editor} failed to start ({e}): draft kept")),
    }
}
