//! Status bar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use super::hints;

use super::spinner_frame;

pub(super) fn render_bar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, base, ink, muted, paper};
    app.hits.bar_hints.clear();
    app.hits.repo = Rect::default();
    app.hits.source = Rect::default();
    if app.nav.focus == crate::app::Focus::Bar {
        let content = app.search.typing;
        let text = if content { &app.search.query } else if app.sidebar.searching { &app.sidebar.search } else { &app.bar };
        let prompt = if content || app.sidebar.searching { "/ " } else { "❯ " };
        let shown = if text.is_empty() {
            if content {
                "search this pane · smart case …".to_string()
            } else if app.sidebar.searching {
                format!(
                    "instant filter loaded: title, #n, label, author · sort {} (s/S) …",
                    app.sort_label()
                )
            } else {
                "type #123, o, q …".to_string()
            }
        } else {
            text.clone()
        };
        f.render_widget(Paragraph::new(Line::from(vec![
            Span::styled(prompt, Style::new().fg(amber()).bold()),
            Span::styled(shown, Style::new().fg(if text.is_empty() { muted() } else { paper() })),
        ])).style(base()), area);
        if content && !text.is_empty() {
            let counter = format!("{} ", app.search.counter());
            let w = counter.chars().count() as u16;
            if w + 4 < area.width {
                f.render_widget(
                    Paragraph::new(counter).style(Style::new().bg(ink()).fg(muted())),
                    Rect::new(area.right() - w, area.y, w, 1),
                );
            }
        }
        f.set_cursor_position((area.x + (2 + text.chars().count() as u16).min(area.width.saturating_sub(1)), area.y));
        return;
    }
    // Minimal shortcut hints, responsive: the longest tier that fits
    // the space left by the content wins. The spinner truncates
    // against the smallest tier, the most content-favoring reserve,
    // so live fetch feedback never loses to static hints.
    let (hint_rest, hint_essentials) = hints::bar_hints(app);
    let short_hint = hints::essentials_width(app);
    let left_width = if area.width > short_hint + 1 { area.width - short_hint - 1 } else { area.width };
    let mut segments: Vec<(String, bool)> = Vec::new();
    // Lasting auth warning first, so long project names never clip it.
    // It stays until a re-auth succeeds and names the `R` reopen key.
    if app.session.auth.degraded {
        segments.push(("⚠ auth degraded · R re-auth".to_string(), true));
    }
    let mut repo_segment = None;
    if let Some(repo) = app.session.repo_key().filter(|s| !s.is_empty()) {
        repo_segment = Some(segments.len());
        segments.push((format!("{}{repo}", crate::theme::nf("\u{f1d3} ", "")), false));
    }
    let mut source_segment = None;
    if !app.sidebar.query_active.trim().is_empty() {
        source_segment = Some(segments.len());
        segments.push((format!("{}{}", crate::theme::nf("\u{f002} ", ""), app.query_source()), false));
    }
    if let Some(statuses) = app.nav.current.and_then(|n| app.session.issues.statuses.get(&n)) {
        for (title, status) in statuses.iter().filter(|(title, _)| !title.is_empty()) {
            let suffix = status.as_ref().filter(|s| !s.is_empty()).map(|s| format!(" ({s})")).unwrap_or_default();
            segments.push((format!("{}{title}{suffix}", crate::theme::nf("\u{f0db} ", "")), false));
        }
    }
    // Loading spinner between project and message: muted, with
    // determinate `n/m` where known. Explicit messages win, so the
    // derivation already hides this under one; empty collapses per the
    // existing rule. The label truncates first so the short hint
    // never shrinks, keeping the 80-cell bar intact.
    let loads = ghit_core::core::loading(&app.fetch_state());
    if !loads.is_empty() {
        let label = ghit_core::core::loading_label(&loads);
        let used: usize = segments.iter().map(|(s, _)| s.chars().count()).sum::<usize>();
        let room = (left_width as usize).saturating_sub(used + segments.len() * 3 + 2);
        if room > 0 {
            let shown = ghit_core::core::toast_snippet(&label, room.max(1));
            let frame = spinner_frame(app.tick);
            segments.push((format!("{frame} {shown}"), false));
        }
    }
    if let Some(prefix) = app.pending_prefix {
        segments.push((format!("{prefix}…"), true));
    }
    if app.message_visible() && !app.message.is_empty() {
        segments.push((format!("{}{}", crate::theme::nf("\u{f071} ", ""), app.message), true));
    }
    let segment_rect = |at: usize| {
        let before: usize = segments[..at].iter().map(|(s, _)| s.chars().count() + 3).sum();
        let width = segments[at].0.chars().count() as u16;
        let x = area.x + before as u16;
        (x + width <= area.right()).then(|| Rect::new(x, area.y, width, 1))
    };
    if let Some(rect) = repo_segment.and_then(segment_rect) {
        app.hits.repo = rect;
    }
    if let Some(rect) = source_segment.and_then(segment_rect) {
        app.hits.source = rect;
    }
    let left_used: usize =
        segments.iter().map(|(s, _)| s.chars().count()).sum::<usize>() + segments.len().saturating_sub(1) * 3;
    let left_shown = (left_used as u16).min(area.width);
    let toast_is_undo = app.message_visible()
        && app.undo_live()
        && app.undo.as_ref().is_some_and(|u| u.message == app.message);
    app.hits.undo_toast = if toast_is_undo { Rect::new(area.x, area.y, left_shown, 1) } else { Rect::default() };
    // Content wins: hints only get the cells left beside it, with one
    // cell of breathing room. Long messages and heavy fetch labels evict
    // the context hints first, then `q`.
    let avail = (area.width as usize).saturating_sub(left_shown as usize + 1);
    let shown = hints::fit(&hint_rest, &hint_essentials, avail);
    let mut spans = Vec::new();
    for (i, (text, accent)) in segments.into_iter().enumerate() {
        if i > 0 { spans.push(Span::styled(" · ", Style::new().fg(muted()))); }
        spans.push(Span::styled(text, Style::new().fg(if accent { amber() } else { paper() })));
    }
    // The left rect stretches to the hint so the bar background
    // stays continuous; the content itself never reaches past
    // `left_shown`, the rest is `base()` fill.
    let hint_w = if shown.is_empty() { 0 } else { hints::line_width(&shown) as u16 };
    let left_rect_w = if shown.is_empty() { left_shown } else { area.width.saturating_sub(hint_w + 1) };
    f.render_widget(Paragraph::new(Line::from(spans)).style(base()), Rect::new(area.x, area.y, left_rect_w, area.height));
    if !shown.is_empty() {
        let x = area.x + area.width - hint_w;
        let (hint_line, rects) = hints::line(&shown, x, area.y);
        app.hits.bar_hints = rects;
        f.render_widget(
            Paragraph::new(hint_line).style(Style::new().bg(ink()).fg(muted())),
            Rect::new(x, area.y, hint_w, area.height),
        );
    }
}
