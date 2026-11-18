//! Diff tab text: unified and split views, review threads, file jumps.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Paragraph, Wrap,
};

use crate::app::App;
use crate::core::MAX_DIFF_FILES;
use crate::model::{self, IssueData};
use super::search::{HiddenSite, SearchAux};

/// Chars of the unified line gutter: a 4-wide number and ` │ `.
const GUTTER_CHARS: usize = 7;



/// Diff tab body: one ruled section per file, folded by default.
/// Issues get a short note, PRs without data yet get a loading line.
/// `width` is the pane content width; narrow panes fall back to unified
/// lines because two columns would fit nothing.
pub(super) fn diff_text(app: &App, d: &IssueData, width: usize) -> ratatui::text::Text<'static> {
    diff_text_aux(app, d, width, &mut SearchAux::default())
}

/// `diff_text` plus what content search needs: the gutter widths to skip
/// and the match counts of folded files.
pub(super) fn diff_text_aux(
    app: &App,
    d: &IssueData,
    width: usize,
    aux: &mut SearchAux,
) -> ratatui::text::Text<'static> {
    use crate::theme::{amber, green, ink, muted, paper, red};
    use model::DiffKind;
    if !d.is_pr {
        return ratatui::text::Text::from(Line::from(Span::styled(
            "Diff is only available for pull requests.",
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    let Some(files) = app.diff.files.get(&d.number) else {
        let note = if app.diff.pending.contains(&d.number) {
            "Fetching diff …"
        } else {
            "No diff loaded yet."
        };
        return ratatui::text::Text::from(Line::from(Span::styled(
            note,
            Style::new().bg(ink()).fg(muted()),
        )));
    };
    if files.is_empty() && d.review_threads.is_empty() {
        return ratatui::text::Text::from(Line::from(Span::styled(
            "No changed files.",
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    let split = app.diff.split && width >= 40;
    let cursor = app.diff.cursor.min(files.len().saturating_sub(1));
    let cursor_name = files.get(cursor).map(|f| f.filename.as_str()).unwrap_or("");
    // Nerd Font Font Awesome fa-file-text reuses the per-file vocabulary;
    // the status words keep their own icons and stay spelled out. The `❯`
    // separator drops its dots so the icon nets zero width and the summary
    // wraps exactly as before.
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "\u{f15c} {} · {} (s: {}) · ↑↓ lines · J/K files · space fold ❯ {}",
            model::diff_summary(files),
            if split { "split" } else { "unified" },
            if split { "unified" } else { "split" },
            cursor_name,
        ),
        Style::new().bg(ink()).fg(muted()),
    ))];
    let cursor_here = app.nav.pane == crate::core::Pane::Main;
    let rule = Span::styled("─".repeat(width.max(8)), Style::new().bg(ink()).fg(muted()));
    // Hunk rows sit two cells in so they read as belonging to the section.
    let dent = || Span::styled("  ", Style::new().bg(ink()));
    // Nerd Font status glyphs; the word stays so a missing font degrades
    // to tofu plus readable text.
    let status_icon = |status: &str| match status {
        "added" => "\u{f067} ",
        "removed" => "\u{f068} ",
        "modified" | "changed" => "\u{f040} ",
        "renamed" | "copied" => "\u{f0c5} ",
        "unchanged" => "\u{f00c} ",
        _ => "",
    };
    for (fi, f) in files.iter().take(MAX_DIFF_FILES).enumerate() {
        let expanded = app.diff.expanded_files.contains(&(d.number, f.filename.clone()));
        let at_cursor = cursor_here && cursor == fi;
        let mut head = Line::from(vec![
            Span::styled(
                if expanded { "▾ " } else { "▸ " }.to_string(),
                Style::new().bg(ink()).fg(if at_cursor { amber() } else { muted() }).bold(),
            ),
            Span::styled("\u{f15c} ", Style::new().bg(ink()).fg(paper())),
            Span::styled(f.filename.clone(), Style::new().bg(ink()).fg(paper()).bold()),
            Span::styled(
                format!("  {}{}  ", status_icon(&f.status), f.status),
                Style::new().bg(ink()).fg(muted()),
            ),
            Span::styled(format!("+{} ", f.additions), Style::new().bg(ink()).fg(green())),
            Span::styled(
                format!("-{} · {} changed", f.deletions, f.additions + f.deletions),
                Style::new().bg(ink()).fg(red()),
            ),
        ]);
        if !expanded && f.is_large() && f.has_patch() {
            head.spans.push(Span::styled(
                "  large diff · space to expand",
                Style::new().bg(ink()).fg(muted()),
            ));
        }
        if at_cursor {
            for span in head.spans.iter_mut() {
                span.style = span.style.patch(Style::new().bg(crate::theme::line_hl()));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(rule.clone()));
        lines.push(head);
        for thread in d.review_threads.iter().filter(|t| t.path == f.filename && model::thread_patch_row(t, f).is_none()) {
            append_review_thread(&mut lines, thread, false);
        }
        if !expanded {
            if app.search.active() {
                let count = model::diff_lines(&f.patch).iter()
                    .map(|(_, text)| crate::core::find_ranges(text, &app.search.query).len())
                    .sum();
                if count > 0 {
                    aux.hidden.push(HiddenSite { line: lines.len() - 1, file: fi, count });
                }
            }
            continue;
        }
        if model::diff_lines(&f.patch).is_empty() {
            lines.push(Line::from(vec![
                dent(),
                Span::styled(
                    "(binary or large file, no patch)",
                    Style::new().bg(ink()).fg(muted()).italic(),
                ),
            ]));
            continue;
        }
        if split {
            lines.extend(split_file_lines(f, d, width.saturating_sub(2)).into_iter().map(|mut l| {
                l.spans.insert(0, dent());
                l
            }));
        } else {
            let numbers = model::patch_line_numbers(&f.patch);
            for (row, (kind, text)) in model::diff_lines(&f.patch).into_iter().enumerate() {
                let style = match kind {
                    DiffKind::Hunk => Style::new().bg(ink()).fg(muted()).bold(),
                    DiffKind::Add => Style::new().bg(ink()).fg(green()),
                    DiffKind::Del => Style::new().bg(ink()).fg(red()),
                    DiffKind::Context => Style::new().bg(ink()).fg(paper()),
                };
                // New-side number for Adds/Context, old-side for Dels,
                // blank for hunk headers: the gutter `│` delimits it from
                // the code so cursor math can strip it back off.
                let (old, new) = numbers.get(row).copied().unwrap_or((None, None));
                let num = match kind {
                    DiffKind::Hunk => None,
                    DiffKind::Del => old,
                    _ => new,
                };
                aux.skips.insert(lines.len(), GUTTER_CHARS);
                lines.push(Line::from(vec![
                    Span::styled(diff_gutter_num(num), Style::new().bg(ink()).fg(muted())),
                    Span::styled(" │ ", Style::new().bg(ink()).fg(muted())),
                    Span::styled(text, style),
                ]));
                for thread in d.review_threads.iter().filter(|t| model::thread_patch_row(t, f) == Some(row)) { append_review_thread(&mut lines, thread, true); }
            }
        }
    }
    let mut extra_paths = Vec::new();
    for thread in &d.review_threads {
        if !files.iter().take(MAX_DIFF_FILES).any(|file| file.filename == thread.path)
            && !extra_paths.contains(&thread.path) {
            extra_paths.push(thread.path.clone());
        }
    }
    for path in extra_paths {
        lines.push(Line::from(""));
        lines.push(Line::from(rule.clone()));
        lines.push(Line::from(Span::styled(format!("  \u{f15c} {path}"), Style::new().bg(ink()).fg(paper()).bold())));
        append_review_threads(&mut lines, d, &path);
    }
    if files.len() > MAX_DIFF_FILES {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("… and {} more files", files.len() - MAX_DIFF_FILES),
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    ratatui::text::Text::from(lines)
}

/// Scan cap for `J`/`K` file jumps: windows probed in one direction until
/// the target header shows. Past this many wrapped rows the jump falls back
/// to visible-header/page motion in `App::jump_diff_file`.
const JUMP_SCAN_ROWS: u32 = 20000;

/// Absolute scroll offset of file `fi`'s header, measured by rendering the
/// same `diff_text` through the same `Paragraph` wrap the body uses, so a
/// `J`/`K` landing matches the live frame exactly. Searches outward from the
/// current viewport toward `delta` in small windows; `None` when the body
/// never rendered, the file is out of range, its header cannot fit on one
/// row (neither this probe nor the live render could match it), or the cap
/// hits.
pub(super) fn diff_header_offset(app: &App, fi: usize, delta: isize) -> Option<u16> {
    if delta == 0 {
        return None;
    }
    let n = app.nav.current?;
    let files = app.diff.files.get(&n)?;
    if fi >= files.len().min(MAX_DIFF_FILES) {
        return None;
    }
    let width = u32::from(app.hits.body_area.width);
    let height = u32::from(app.hits.body_area.height);
    if width == 0 || height == 0 {
        return None;
    }
    // A header row leads with `▸ `/`▾ ` plus an icon; when even the bare
    // filename cannot fit beside them, the header never matches.
    if files.get(fi).map(|f| f.filename.len() + 5 > width as usize).unwrap_or(true) {
        return None;
    }
    let d = app.issues.data.get(&n)?;
    let text = diff_text(app, d, width as usize);
    use ratatui::buffer::Buffer;
    use ratatui::widgets::Widget;
    let window = height.max(1) * 2;
    let area = Rect::new(0, 0, width as u16, window.min(u16::MAX as u32) as u16);
    let start = u32::from(app.nav.scroll);
    let mut scanned: u32 = 0;
    let mut k: u32 = 0;
    loop {
        let s = if delta > 0 {
            start.saturating_add(k.saturating_mul(window))
        } else {
            start.saturating_sub(k.saturating_mul(window))
        };
        let mut buf = Buffer::empty(area);
        Paragraph::new(text.clone())
            .wrap(Wrap { trim: false })
            .scroll((s.min(u16::MAX as u32) as u16, 0))
            .render(area, &mut buf);
        let mut blank = true;
        for y in 0..window as u16 {
            let mut row = String::new();
            for x in 0..width as u16 {
                row.push_str(buf[(x, y)].symbol());
            }
            if !row.trim().is_empty() {
                blank = false;
            }
            if model::header_file_index(&row, files) == Some(fi) {
                return Some(s.saturating_add(u32::from(y)) as u16);
            }
        }
        if delta < 0 && s == 0 {
            return None;
        }
        if delta > 0 && blank {
            return None;
        }
        scanned = scanned.saturating_add(window);
        if scanned > JUMP_SCAN_ROWS {
            return None;
        }
        k = k.saturating_add(1);
    }
}

fn append_review_threads(lines: &mut Vec<Line<'static>>, d: &IssueData, path: &str) {
    for thread in d.review_threads.iter().filter(|thread| thread.path == path) {
        append_review_thread(lines, thread, false);
    }
}

fn append_review_thread(lines: &mut Vec<Line<'static>>, thread: &model::ReviewThread, inline: bool) {
    let indent = if inline { "    " } else { "  " };
    let line = thread.line.map(|n| format!("line {n}")).unwrap_or_else(|| "line unavailable".to_string());
    let state = if thread.is_resolved { "resolved" } else { "unresolved" };
    let color = if thread.is_resolved { crate::theme::muted() } else { crate::theme::amber() };
    let pending = if thread.has_pending() { " · pending" } else { "" };
    lines.push(Line::from(Span::styled(format!("{indent}\u{f075} {line} · {state}{pending}"), Style::new().bg(crate::theme::ink()).fg(color).bold())));
    for comment in &thread.comments {
        let author = comment.author.as_ref().map(|a| a.login.as_str()).unwrap_or("unknown");
        let author_label = if comment.is_pending() { format!("{author} · pending") } else { author.to_string() };
        lines.push(Line::from(Span::styled(format!("{indent}  {author_label}"), Style::new().bg(crate::theme::ink()).fg(crate::theme::paper()).bold())));
        lines.extend(crate::theme::render_markdown(&comment.body).lines.into_iter().map(|mut line| {
            line.spans.insert(0, Span::raw(format!("{indent}  ")));
            Line::from(line.spans.into_iter().map(|span| Span::styled(span.content.into_owned(), span.style)).collect::<Vec<_>>())
        }));
    }
}

/// Right-aligned line number for a diff gutter, blank when the row
/// carries no number on its side. Width 4 covers real files; longer
/// numbers overflow plainly rather than shifting the code column.
fn diff_gutter_num(n: Option<u64>) -> String {
    match n {
        Some(v) => format!("{:>4}", v),
        None => "    ".to_string(),
    }
}

/// Fit `s` into exactly `w` cells: truncate, then pad with spaces.
fn fit_cell(s: &str, w: usize) -> String {
    let mut out: String = s.chars().take(w).collect();
    while out.chars().count() < w {
        out.push(' ');
    }
    out
}

/// Side-by-side rows for one file patch at `width` cells wide.
fn split_file_lines(file: &model::DiffFile, d: &IssueData, width: usize) -> Vec<Line<'static>> {
    use crate::theme::{green, ink, muted, paper, red};
    use model::{DiffKind, SplitRow};
    let sep = " │ ";
    let lw = width.saturating_sub(sep.len()) / 2;
    let rw = width.saturating_sub(sep.len() + lw);
    let mut lines = vec![Line::from(vec![
        Span::styled(fit_cell("     — old", lw), Style::new().bg(ink()).fg(muted())),
        Span::styled(sep, Style::new().bg(ink()).fg(muted())),
        Span::styled(fit_cell("     + new", rw), Style::new().bg(ink()).fg(muted())),
    ])];
    let patch = &file.patch;
    let mut patch_rows = model::diff_lines(patch).into_iter().enumerate().filter(|(_, (kind, _))| *kind != DiffKind::Del);
    for row in model::split_rows(patch) {
        let has_new = !matches!(row, SplitRow::LeftOnly { .. });
        let patch_row = if has_new { patch_rows.next().map(|(i, _)| i) } else { None };
        // Each side keeps its own gutter: old numbers left, new right.
        let cell = |num: Option<u64>, text: &str| format!("{} {}", diff_gutter_num(num), text);
        match row {
            SplitRow::Hunk(text) => lines.push(Line::from(Span::styled(
                fit_cell(&text, width),
                Style::new().bg(ink()).fg(muted()).bold(),
            ))),
            SplitRow::Pair { left, right, old, new } => {
                let change = left.starts_with('-') && right.starts_with('+');
                lines.push(Line::from(vec![
                    Span::styled(
                        fit_cell(&cell(old, &left), lw),
                        Style::new().bg(ink()).fg(if change { red() } else { paper() }),
                    ),
                    Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                    Span::styled(
                        fit_cell(&cell(new, &right), rw),
                        Style::new().bg(ink()).fg(if change { green() } else { paper() }),
                    ),
                ]));
            }
            SplitRow::LeftOnly { text, kind, line } => lines.push(Line::from(vec![
                Span::styled(
                    fit_cell(&cell(line, &text), lw),
                    Style::new().bg(ink()).fg(if kind == DiffKind::Del { red() } else { paper() }),
                ),
                Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                Span::styled(fit_cell("", rw), Style::new().bg(ink())),
            ])),
            SplitRow::RightOnly { text, kind, line } => lines.push(Line::from(vec![
                Span::styled(fit_cell("", lw), Style::new().bg(ink())),
                Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                Span::styled(
                    fit_cell(&cell(line, &text), rw),
                    Style::new().bg(ink()).fg(if kind == DiffKind::Add { green() } else { paper() }),
                ),
            ])),
        }
        if let Some(index) = patch_row {
            for thread in d.review_threads.iter().filter(|t| model::thread_patch_row(t, file) == Some(index)) { append_review_thread(&mut lines, thread, true); }
        }
    }
    lines
}
