//! Sidebar panes: the issue list, refs, filters and history.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::app::{App, FilterItem};
use crate::keymap::{Action, Context};
use ghit_core::model::{self, IssueData};
use crate::theme;
use super::{controls, hints};

use super::{section_title, spinning, loading_body, footer_line};
use super::main_pane::wrap_lines;
use ghit_core::model::ItemRef;

/// Split `text` into plain spans plus highlighted spans for each query
/// hit, so `/` search visibly marks the matched characters.
fn highlighted_spans(text: String, query: &str, base: Style) -> Vec<Span<'static>> {
    use crate::theme::{amber, ink};
    let hit = Style::new().fg(ink()).bg(amber()).bold();
    let ranges = ghit_core::core::match_ranges(&text, query);
    if ranges.is_empty() {
        return vec![Span::styled(text, base)];
    }
    let mut spans = vec![];
    let mut last = 0;
    for (s, e) in ranges {
        if s > last {
            spans.push(Span::styled(text[last..s].to_string(), base));
        }
        spans.push(Span::styled(text[s..e].to_string(), hit));
        last = e;
    }
    if last < text.len() {
        spans.push(Span::styled(text[last..].to_string(), base));
    }
    spans
}

/// Detail lines an expanded sidebar row adds below its summary: the full
/// title wrapped to the pane width, every label, author and state, and the
/// created/updated dates. Indented like the summary line.
fn expanded_row_lines(d: &IssueData, inner_width: usize) -> Vec<Line<'static>> {
    use crate::theme::{amber, muted, paper, state_color};
    const INDENT: &str = "    ";
    let width = inner_width.saturating_sub(INDENT.len()).max(8);
    let indent = |lines: Vec<Line<'static>>| -> Vec<Line<'static>> {
        lines
            .into_iter()
            .map(|line| {
                let mut spans = vec![Span::styled(INDENT, Style::new())];
                spans.extend(line.spans);
                Line::from(spans)
            })
            .collect()
    };
    let mut out = indent(wrap_lines(
        vec![Line::from(Span::styled(d.title.clone(), Style::new().fg(paper())))],
        width,
    ));
    let (labels, label_style) = if d.labels.is_empty() {
        ("no labels".to_string(), Style::new().fg(muted()))
    } else {
        let names = d.labels.iter().map(|l| format!("● {}", l.name)).collect::<Vec<_>>().join("  ");
        (names, Style::new().fg(amber()))
    };
    out.extend(indent(wrap_lines(
        vec![Line::from(Span::styled(labels, label_style))],
        width,
    )));
    out.push(Line::from(vec![
        Span::styled(INDENT, Style::new()),
        Span::styled(format!("by {} · ", d.author.login), Style::new().fg(paper())),
        Span::styled(d.state.clone(), Style::new().fg(state_color(&d.state)).bold()),
    ]));
    let created = d.created_at.get(..10).unwrap_or(&d.created_at);
    let updated = d.updated_at.get(..10).unwrap_or(&d.updated_at);
    out.push(Line::from(vec![
        Span::styled(INDENT, Style::new()),
        Span::styled(format!("created {created}"), Style::new().fg(muted())),
    ]));
    out.push(Line::from(vec![
        Span::styled(INDENT, Style::new()),
        Span::styled(format!("updated {updated}"), Style::new().fg(muted())),
    ]));
    out
}

/// Compact CI glyph for a PR row or header: the head-commit rollup in
/// its theme color, or `None` when the PR runs no checks (or the state
/// is unknown). Issues never carry a rollup, so they render nothing.
pub(super) fn check_span(d: &IssueData) -> Option<Span<'static>> {
    if !d.is_pr {
        return None;
    }
    let checks = d.checks?;
    let glyph = crate::theme::check_glyph(&checks);
    if glyph.is_empty() {
        return None;
    }
    Some(Span::styled(
        format!(" {glyph}"),
        Style::new().fg(crate::theme::check_color(&checks)).bold(),
    ))
}

/// Border chrome for one sidebar pane: the top title and bottom footer,
/// plus the Row fields that decide which segments its rows draw. The
/// `checks` flag gates the PR check glyph; Issues rows never carry one.
struct SectionFrame {
    title: Line<'static>,
    footer: Line<'static>,
    fields: model::RowFields,
    checks: bool,
    loading: bool,
    /// Label and body height for the loading body.
    loading_label: String,
    body_height: usize,
    /// Shown when the pane has no rows and nothing is loading.
    empty: Line<'static>,
}

/// One pane (Issues or PRs): pinned, requested, then recent rows with a
/// thin divider row between non-empty groups. Everything is owned, so the
/// returned list outlives the `&App` borrow and the caller can persist its
/// `ListState` back into `App` for scroll continuity. Also returns the
/// drawn height of each row, which `render` publishes for hit-testing.
fn section_list(
    frame: SectionFrame,
    rows: &[ghit_core::model::SideRow],
    app: &App,
    inner_width: usize,
    mark: Option<ItemRef>,
    focused: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> (List<'static>, Vec<usize>) {
    use crate::theme::{amber, ink, muted, paper, state_color, RowState};
    use ghit_core::model::SideRow;
    let query = app.sidebar.search.clone();
    let mut items: Vec<ListItem<'static>> = vec![ListItem::new(Line::from(""))];
    for row in rows {
        if let SideRow::Loading { guide } = row {
            items.push(ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{guide} loading …"), Style::new().fg(muted())),
            ])));
            continue;
        }
        let Some(n) = row.number() else {
            let SideRow::Divider(label) = row else {
                continue;
            };
            // Labeled rule: the section name rides inside the divider.
            let head = format!("─ {label} ");
            let fill = inner_width.max(8).saturating_sub(head.chars().count());
            items.push(ListItem::new(Line::from(Span::styled(
                format!("{head}{}", "─".repeat(fill)),
                Style::new().bg(ink()).fg(muted()),
            ))));
            continue;
        };
        // The selection marker follows `current` on every pane: the
        // sidebar stays coupled to what the main pane displays, even
        // when Main holds focus (e.g. the Diff tab). Focus still shows
        // through the border, title, and row shade. The wheel hover
        // never draws the marker. Hover stays a transient preview in
        // state (j/k continue from it); the ListState cursor underneath
        // is untouched, so tabbing away and back restores the exact row
        // and scroll position.
        let sel = app.nav.current == Some(n) || mark == Some(n);
        let state = if sel {
            RowState::Active
        } else if app.nav.hover == Some(n) {
            RowState::Hover
        } else {
            RowState::Plain
        };
        let (marker, marker_style) = theme::row_marker(state, focused, "█ ");
        let (title_text, labels, updated, status, author, milestone) =
            match app.session.item(n) {
            Some(d) => {
                let st = app.session.statuses(n).map(|s| s.to_vec()).unwrap_or_default();
                let pinned = app.session.is_pinned(n);
                let status_icon = crate::theme::column_icon(&model::status_text(&st));
                let (_, labels, updated, status, author, milestone) =
                    model::sidebar_lines(d, &st, now, 38, pinned, &frame.fields, status_icon);
                (model::slug(&d.title, 30), labels, updated, status, author, milestone)
            }
            None => Default::default(),
        };
        // Nerd Font Font Awesome fa-thumb-tack marks pinned rows. Same two
        // cells as the marker it replaces, so sidebar hit-testing is untouched.
        let pin = if app.session.is_pinned(n) { theme::nf("\u{f08d} ", "• ") } else { "" };
        let data = app.session.item(n);
        let outside = matches!(row, SideRow::Tree { outside: true, .. });
        let num_style = if outside {
            Style::new().fg(muted())
        } else {
            Style::new()
                .fg(data.map(|d| state_color(&d.state)).unwrap_or_else(muted))
                .bold()
        };
        let title_style = if outside {
            Style::new().fg(muted())
        } else if sel {
            Style::new().fg(paper()).bold()
        } else {
            Style::new().fg(paper())
        };
        let mut first = vec![
            Span::styled(marker, marker_style),
            Span::styled(pin, Style::new().fg(amber())),
        ];
        if let SideRow::Tree { guide, has_children, folded, .. } = row {
            let glyph = if *has_children { if *folded { "▸" } else { "▾" } } else { " " };
            first.push(Span::styled(format!("{guide}{glyph} "), Style::new().fg(muted())));
        }
        first.extend(highlighted_spans(
            format!("{n} ", n = app.session.label(n)),
            &query,
            num_style,
        ));
        first.extend(highlighted_spans(title_text, &query, title_style));
        if outside {
            first.push(Span::styled(" · outside query", Style::new().fg(muted())));
        }
        if matches!(row, SideRow::Tree { .. }) {
            // GraphQL ships `{0, 0}` for every leaf: only real parents badge.
            if let Some(summary) = data.and_then(|d| d.sub_summary.as_ref()).filter(|s| s.total > 0) {
                first.push(Span::styled(format!(" {}/{}", summary.completed, summary.total),
                    Style::new().fg(muted())));
            }
        }
        // PR head-commit CI rollup: one glyph, no text, so hit-testing
        // and row height are untouched. The Row fields toggle gates it.
        if frame.checks {
            if let Some(d) = data {
                if let Some(check) = check_span(d) {
                    first.push(check);
                }
            }
        }
        // Second line: labels · relative updated · project status, plus
        // the author and milestone when their fields are on. Empty
        // segments collapse; the status keeps the board color and glyph.
        let status_color = crate::theme::column_color(&status);
        let mut second = vec![Span::raw("    ")];
        let mut first_segment = true;
        for (text, style) in [
            (labels, Style::new().fg(muted())),
            (updated, Style::new().fg(muted())),
            (status, Style::new().fg(status_color)),
            (author, Style::new().fg(paper())),
            (milestone, Style::new().fg(amber())),
        ] {
            if text.is_empty() {
                continue;
            }
            if !first_segment {
                second.push(Span::styled(" · ", Style::new().fg(muted())));
            }
            first_segment = false;
            second.push(Span::styled(text, style));
        }
        let mut lines = vec![Line::from(first), Line::from(second)];
        if app.diff.expanded_rows.contains(&n) {
            if let Some(d) = data {
                lines.extend(expanded_row_lines(d, inner_width));
            }
        }
        lines.push(Line::from(""));
        items.push(ListItem::new(lines));
    }
    let heights: Vec<usize> = items[1..].iter().map(ListItem::height).collect();
    if rows.is_empty() && frame.loading {
        items.extend(loading_body(&frame.loading_label, app.tick, frame.body_height).into_iter().map(ListItem::new));
    } else if rows.is_empty() && !frame.empty.spans.is_empty() {
        items.push(ListItem::new(frame.empty.clone()));
    }
    let list = List::new(items)
        .block(theme::pane_block(focused).title(frame.title).title_bottom(frame.footer))
        .highlight_style(theme::row_highlight(focused));
    (list, heights)
}

fn side_section_title(name: &str, detail: &str, icon: &str, active: bool,
    tag: Option<&str>) -> Line<'static> {
    let mut title = section_title(name, detail, icon, active);
    if let Some(tag) = tag {
        title.spans.insert(2, Span::styled(format!("[{tag}] "),
            Style::new().fg(theme::amber())));
    }
    title
}

/// Position of `current` in display-row order, +1 for the top spacer.
fn selected_index(rows: &[model::SideRow], current: Option<ItemRef>) -> Option<usize> {
    current
        .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
        .map(|p| p + 1)
}

pub(super) fn render_sidebar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let issue_rows = app.issue_rows();
    let pr_rows = app.pr_rows();
    let pane = app.pane_loading();
    app.hits.side_issue_area = panes[0];
    app.hits.side_pr_area = panes[1];
    app.hits.side_issue_rows = issue_rows.clone();
    app.hits.side_pr_rows = pr_rows.clone();
    let inner = |a: Rect| (a.width as usize).saturating_sub(2);
    let now = chrono::Utc::now();
    let issues_focused = app.pane_focused(ghit_core::core::Pane::Issues);
    let issue_title = side_section_title("Issues", &app.side_subtitle(false), theme::nf("\u{f06a}", ""), issues_focused, app.query_tag());
    controls::side_controls(app, panes[0], &issue_title, false, ghit_core::core::Pane::Issues);
    let (il, issue_heights) = section_list(
        SectionFrame {
            // Nerd Font Font Awesome fa-exclamation-circle.
            title: spinning(issue_title, pane.sidebar_spinner, app.tick),
            footer: footer_line(&app.side_footer(false)),
            fields: app.sidebar.row_issue,
            checks: app.sidebar.row_issue.checks,
            loading: pane.sidebar,
            loading_label: app.pane_loading_label(false),
            body_height: usize::from(panes[0].height.saturating_sub(3)),
            empty: if pane.sidebar_spinner && !pane.sidebar { Line::from("") } else { hints::tip("  No issues. Press ", Context::Global, Action::OpenFormat, " to filter.") },
        },
        &issue_rows,
        app,
        inner(panes[0]),
        None,
        issues_focused,
        now,
    );
    app.hits.side_issue_heights = issue_heights;
    let mut istate = std::mem::take(&mut app.sidebar.issue_state);
    istate.select(selected_index(&issue_rows, app.effective(&issue_rows)));
    f.render_stateful_widget(il, panes[0], &mut istate);
    app.sidebar.issue_state = istate;
    let prs_focused = app.pane_focused(ghit_core::core::Pane::PRs);
    let pr_title = side_section_title("Pull requests", &app.side_subtitle(true), theme::nf("\u{f126}", ""), prs_focused, app.query_tag());
    controls::side_controls(app, panes[1], &pr_title, true, ghit_core::core::Pane::PRs);
    let (pl, pr_heights) = section_list(
        SectionFrame {
            // Nerd Font Font Awesome fa-code-fork.
            title: spinning(pr_title, pane.sidebar_spinner, app.tick),
            footer: footer_line(&app.side_footer(true)),
            fields: app.sidebar.row_pr,
            checks: app.sidebar.row_pr.checks,
            loading: pane.sidebar,
            loading_label: app.pane_loading_label(true),
            body_height: usize::from(panes[1].height.saturating_sub(3)),
            empty: if pane.sidebar_spinner && !pane.sidebar { Line::from("") } else { hints::tip("  No pull requests. Press ", Context::Global, Action::OpenFormat, " to filter.") },
        },
        &pr_rows,
        app,
        inner(panes[1]),
        None,
        prs_focused,
        now,
    );
    app.hits.side_pr_heights = pr_heights;
    let mut pstate = std::mem::take(&mut app.sidebar.pr_state);
    pstate.select(selected_index(&pr_rows, app.effective(&pr_rows)));
    f.render_stateful_widget(pl, panes[1], &mut pstate);
    app.sidebar.pr_state = pstate;
}

/// Right pane: issues/PRs mentioned in the current body, with the same row
/// look as the left sidebar. Renders only when `refs_visible`; the caller
/// hides the column otherwise so the main pane keeps its width.
pub(super) fn render_refs(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let rows = app.refs_rows();
    app.hits.refs_area = area;
    app.hits.ref_rows = rows.clone();
    let inner = (area.width as usize).saturating_sub(2);
    let refs_focused = app.pane_focused(ghit_core::core::Pane::Refs);
    let (list, heights) = section_list(
        SectionFrame {
            // Nerd Font Font Awesome fa-link.
            title: spinning(section_title("Refs", &app.refs_subtitle(), theme::nf("\u{f0c1}", ""), refs_focused), app.pane_loading().refs, app.tick),
            footer: footer_line(&app.refs_footer()),
            fields: app.sidebar.row_issue,
            checks: app.sidebar.row_pr.checks,
            loading: false,
            loading_label: String::new(),
            body_height: 0,
            empty: Line::default(),
        },
        &rows,
        app,
        inner,
        app.refs.highlight,
        refs_focused,
        chrono::Utc::now(),
    );
    app.hits.ref_heights = heights;
    let mut state = std::mem::take(&mut app.refs.state);
    let sel = app.refs
        .highlight
        .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
        .map(|p| p + 1);
    state.select(sel);
    f.render_stateful_widget(list, area, &mut state);
    app.refs.state = state;
}

/// Labels pane: milestones then labels, one row each, a checkbox on the
/// selected ones. It edits the same filter sets as the Format dialog and
/// frames like its sibling list panes.
pub(super) fn render_labels(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, teal, violet, RowState};
    let focused = app.pane_focused(ghit_core::core::Pane::Labels);
    let on_labels = app.sidebar.label_filter.len();
    let on_milestones = app.sidebar.milestone_filter.len();
    let items = app.filter_items();
    let milestones = items.iter().filter(|item| matches!(item, FilterItem::Milestone(_))).count();
    // Nerd Font Font Awesome fa-filter.
    let mut title = section_title("Filters", "", theme::nf("\u{f0b0}", ""), focused);
    if on_labels + on_milestones > 0 {
        title.spans[2] = Span::styled(format!("{} on", on_labels + on_milestones),
            Style::new().fg(amber()));
    }
    let footer = if items.is_empty() { String::new() } else {
        format!("milestones ({}/{milestones}) · labels ({on_labels}/{})",
            on_milestones, items.len() - milestones)
    };
    // Buttons sit on the bottom border in place of the counts footer.
    let mut buttons = vec![];
    // Only real milestones can be done; "(no milestone)" alone needs no button.
    if !app.sidebar.milestones.is_empty() {
        let label = if app.sidebar.hide_done_milestones { "show done" } else { "hide done" };
        buttons.push((label, Action::ToggleDoneMilestones));
    }
    if on_labels + on_milestones > 0 {
        buttons.push(("clear all", Action::ClearLabels));
    }
    let mut block = theme::pane_block(focused).title(title);
    if buttons.is_empty() {
        block = block.title_bottom(footer_line(&footer));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.hits.labels_area = area;
    if !buttons.is_empty() {
        let row = Rect::new(area.x + 1, area.bottom() - 1, area.width.saturating_sub(2), 1);
        controls::button_row(f, app, row, &buttons, ghit_core::core::Pane::Labels);
    }
    if items.is_empty() {
        let line = if app.sidebar.meta_rx.is_some() || !app.sidebar.meta_loaded {
            Line::styled("  loading …", Style::new().bg(ink()).fg(muted()))
        } else {
            Line::styled("  No labels or milestones in this repo", Style::new().bg(ink()).fg(muted()))
        };
        f.render_widget(Paragraph::new(line).style(theme::base()).wrap(Wrap { trim: false }), inner);
        return;
    }
    // Display lines: a group header before each group, `Some(index)` for
    // items, and `Some(NOTE)` for the hidden-milestones note.
    const NOTE: usize = usize::MAX;
    // Pills are the shown titles plus "(no milestone)".
    let hidden = (app.sidebar.milestones.len() + 1).saturating_sub(milestones);
    let mut lines: Vec<Option<usize>> = vec![None];
    if app.sidebar.hide_done_milestones && hidden > 0 && milestones > 0 {
        lines.push(Some(NOTE));
    }
    for index in 0..items.len() {
        if index == milestones {
            lines.push(None);
        }
        lines.push(Some(index));
    }
    let height = inner.height as usize;
    app.labels.cursor = app.labels.cursor.min(items.len() - 1);
    let cursor_line = lines.iter().position(|i| *i == Some(app.labels.cursor)).unwrap_or(0);
    let mut offset = app.labels.offset.min(lines.len().saturating_sub(height));
    if cursor_line < offset {
        offset = cursor_line;
    } else if cursor_line >= offset + height {
        offset = cursor_line + 1 - height;
    }
    // Keep the group header (and the note under it) with the group's first row.
    if offset == cursor_line {
        while offset > 0 && matches!(lines[offset - 1], None | Some(NOTE)) {
            offset -= 1;
        }
    }
    app.labels.offset = offset;
    for (row, line_index) in lines.iter().enumerate().skip(offset).take(height) {
        let rect = Rect::new(inner.x, inner.y + (row - offset) as u16, inner.width, 1);
        if *line_index == Some(NOTE) {
            let noun = if hidden == 1 { "milestone" } else { "milestones" };
            let line = Line::styled(format!("    {hidden} done {noun} hidden"), Style::new().bg(ink()).fg(muted()).italic());
            f.render_widget(Paragraph::new(line).style(theme::base()), rect);
            continue;
        }
        let Some(index) = *line_index else {
            // Nerd Font Font Awesome fa-flag / fa-tag.
            let (icon, name, accent) = if row == 0 {
                (theme::nf("\u{f024} ", ""), "Milestones", violet())
            } else {
                (theme::nf("\u{f02b} ", ""), "Labels", teal())
            };
            let line = Line::from(vec![
                Span::styled(format!("  {icon}"), Style::new().bg(ink()).fg(accent)),
                Span::styled(name, Style::new().bg(ink()).fg(muted()).bold()),
            ]);
            f.render_widget(Paragraph::new(line).style(theme::base()), rect);
            continue;
        };
        let on = app.filter_item_on(&items[index]);
        let (name, none) = match &items[index] {
            FilterItem::Milestone(Some(title)) => (title.clone(), false),
            FilterItem::Milestone(None) => ("(no milestone)".to_string(), true),
            FilterItem::Label(name) => (name.clone(), false),
        };
        let state = if index == app.labels.cursor { RowState::Active } else { RowState::Plain };
        let bg = theme::row_bg(state, focused);
        let (marker, marker_style) = theme::row_marker(state, focused, "█ ");
        // Nerd Font Font Awesome fa-check-square / fa-square-o, same as the
        // sidebar filter toggles.
        let name_style = if on {
            Style::new().bg(bg).fg(amber()).bold()
        } else if none {
            Style::new().bg(bg).fg(muted())
        } else {
            Style::new().bg(bg).fg(paper())
        };
        let percent = match &items[index] {
            FilterItem::Milestone(Some(title)) => app.sidebar.milestones.iter()
                .find(|m| &m.title == title).and_then(model::Milestone::percent),
            _ => None,
        };
        // Progress bar only when the row has room for a readable name.
        let bar = percent.filter(|_| inner.width >= 24).map(|p| progress_spans(p, bg));
        let bar_width = if bar.is_some() { PROGRESS_WIDTH } else { 0 };
        let room = (inner.width as usize).saturating_sub(5 + bar_width);
        let slug = model::slug(&name, room);
        let mut spans = vec![
            Span::styled(marker, marker_style.bg(bg)),
            Span::styled(if on { theme::nf("\u{f14a} ", "■ ") } else { theme::nf("\u{f096} ", "□ ") },
                Style::new().bg(bg).fg(if on { amber() } else { muted() })),
        ];
        if let Some(bar) = bar {
            let pad = room.saturating_sub(slug.chars().count());
            spans.push(Span::styled(slug, name_style));
            spans.push(Span::styled(" ".repeat(pad), Style::new().bg(bg)));
            spans.extend(bar);
        } else {
            spans.push(Span::styled(slug, name_style));
        }
        let line = Line::from(spans);
        f.render_widget(Paragraph::new(line).style(Style::new().bg(bg)), rect);
        app.hits.labels_rows.push((rect, index));
    }
}

/// Cells a milestone progress bar takes: a space, five bar cells, and a
/// right-aligned percentage (" ▰▰▰▱▱ 60%").
const PROGRESS_WIDTH: usize = 11;

/// A milestone's share of closed items as a five-cell bar and a percent.
fn progress_spans(percent: u32, bg: ratatui::style::Color) -> Vec<Span<'static>> {
    use crate::theme::{muted, teal};
    let filled = ((percent as usize * 5) + 50) / 100;
    vec![
        Span::styled(format!(" {}", "▰".repeat(filled)), Style::new().bg(bg).fg(teal())),
        Span::styled("▱".repeat(5 - filled), Style::new().bg(bg).fg(muted())),
        Span::styled(format!("{percent:>4}%"), Style::new().bg(bg).fg(muted())),
    ]
}

/// Last visits under Refs: up to four back entries above the current
/// issue, bottom-aligned. A click on a row `k` steps up walks back `k`
/// steps through `go_back_n`, so Ctrl+I replays the skipped entries.
pub(super) fn render_history(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    // Nerd Font Font Awesome fa-history.
    let focused = app.pane_focused(ghit_core::core::Pane::History);
    let block = theme::pane_block(focused).title(section_title("History", "", theme::nf("\u{f1da}", ""), focused));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.hits.history_area = area;
    let cursor = app.history_steps();
    let mut numbers: Vec<ItemRef> = app.nav.history.iter().rev().take(4).copied().collect();
    numbers.reverse();
    if let Some(current) = app.nav.current {
        numbers.push(current);
    }
    let total = numbers.len();
    let skip = total.saturating_sub(inner.height as usize);
    for (index, number) in numbers.into_iter().enumerate().skip(skip) {
        let row = Rect::new(inner.x, inner.y + (index - skip) as u16, inner.width, 1);
        let steps = total - index - 1;
        // `▌`, not the sidebar's `█`: the list selection marker stays
        // unique on screen (e2e counts it), and "current" is not a
        // list selection.
        let state = if steps == cursor { theme::RowState::Active } else { theme::RowState::Plain };
        let (marker, marker_style) = theme::row_marker(state, focused, "▌ ");
        let bg = theme::row_bg(state, focused);
        let head = app.session.label(number);
        let room = (inner.width as usize).saturating_sub(2 + head.chars().count() + 1);
        let title = app.session.item(number)
            .filter(|issue| !issue.title.is_empty() && room > 1)
            .map(|issue| format!(" {}", model::slug(&issue.title, room)))
            .unwrap_or_default();
        let line = Line::from(vec![
            Span::styled(marker, marker_style.bg(bg)),
            Span::styled(head, Style::new().bg(bg).fg(theme::paper())),
            Span::styled(title, Style::new().bg(bg).fg(theme::muted())),
        ]);
        f.render_widget(Paragraph::new(line).style(Style::new().bg(bg)), row);
        app.hits.history_rows.push((row, steps));
    }
}
