//! Look-and-feel: all rendering. Reads `App`, never fetches.
//!
//! Colors come only from `theme::` functions so `t` repaints everything.
//! Hit areas (`body_rows`, sidebar rects, board cells) are recorded here
//! during `render`; `input` consumes them on the next event.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};

use crate::app::{App, Tab};
use crate::model::{self, IssueData};

pub(crate) fn render(f: &mut ratatui::Frame, app: &mut App) {
    use crate::theme::ink;
    f.render_widget(Block::default().style(Style::new().bg(ink())), f.area());
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    if app.refs_visible() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(44), Constraint::Min(1), Constraint::Length(36)])
            .split(root[0]);
        render_sidebar(f, cols[0], app);
        render_main(f, cols[1], app);
        render_refs(f, cols[2], app);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(44), Constraint::Min(1)])
            .split(root[0]);
        render_sidebar(f, cols[0], app);
        render_main(f, cols[1], app);
    }
    app.capture_body_rows(f.buffer_mut());
    // Diff tab bookkeeping from the fresh capture. Only the Diff tab
    // renders `▸ `/`▾ ` markers. Headers match by filename (longest wins)
    // so wrapped rows and scrolled-off files resolve exactly; every other
    // row inherits the previous header's file, pre-header rows map to None.
    if app.visual_diff() {
        let header_rows: Vec<(usize, usize)> = {
            let empty: Vec<model::DiffFile> = vec![];
            let files = app
                .current
                .and_then(|n| app.diffs.get(&n))
                .map(|fs| fs.as_slice())
                .unwrap_or(&empty);
            app.body_rows
                .iter()
                .enumerate()
                .filter_map(|(i, line)| model::header_file_index(line, files).map(|fi| (i, fi)))
                .collect()
        };
        app.diff_headers = header_rows.iter().map(|(i, _)| *i).collect();
        let mut row_files: Vec<Option<usize>> = vec![None; app.body_rows.len()];
        let mut cur: Option<usize> = None;
        let mut hi = 0;
        for (i, slot) in row_files.iter_mut().enumerate() {
            while hi < header_rows.len() && header_rows[hi].0 == i {
                cur = Some(header_rows[hi].1);
                hi += 1;
            }
            *slot = cur;
        }
        app.diff_row_files = row_files;
        // Resolve the active line against the fresh capture: track the
        // owning file, and unfold a collapsed file landed on by input.
        // The lazy init anchor never expands: opening a Diff stays folded.
        match app.diff_line {
            None => {
                if let Some(&h) = app.diff_headers.first() {
                    app.diff_line = Some(app.body_base as usize + h);
                }
            }
            Some(abs) => {
                let vis = abs.saturating_sub(app.body_base as usize);
                let landed = app.diff_row_files.get(vis).copied().flatten();
                if let Some(f) = landed {
                    app.diff_cursor = f;
                    if app.diff_touched {
                        let key = app.current.and_then(|n| {
                            app.diffs
                                .get(&n)
                                .and_then(|fs| fs.get(f))
                                .map(|file| (n, file.filename.clone()))
                        });
                        if let Some(k) = key {
                            app.expanded_files.insert(k);
                        }
                    }
                }
                if vis < app.diff_row_files.len() {
                    app.diff_touched = false;
                }
            }
        }
        // Active-line highlight, painted on the buffer after the paragraph
        // so wrapped rows light up exactly. Same amber/ink vocabulary as
        // the refs mention highlight.
        if let Some(abs) = app.diff_line {
            use crate::theme::{amber, ink};
            let vis = abs.saturating_sub(app.scroll as usize);
            if vis < app.body_area.height as usize {
                let y = app.body_area.y + vis as u16;
                let buf = f.buffer_mut();
                for x in app.body_area.x..app.body_area.x + app.body_area.width {
                    buf[(x, y)].set_style(Style::new().bg(amber()).fg(ink()));
                }
            }
        }
    } else {
        app.diff_headers = vec![];
        app.diff_row_files = vec![];
    }

    app.bar_area = root[1];
    render_bar(f, root[1], app);

    if crate::app::board_open(app) {
        dim_background(f, 0.35);
        render_board(f, f.area(), app);
    }
    if app.filter_open {
        dim_background(f, 0.35);
        render_filter(f, f.area(), app);
    }
    if app.shortcuts_open {
        dim_background(f, 0.35);
        render_shortcuts(f, f.area());
    }
    if app.compose_open() {
        dim_background(f, 0.35);
        render_compose(f, f.area(), app);
    }
    if app.auth_prompt.is_some() {
        dim_background(f, 0.35);
        render_auth(f, f.area(), app);
    }
    paint_click_flash(f, app);
}

/// Click flash: invert the last-clicked rect for `CLICK_FLASH_MS` so the
/// press reads instantly, even before its fetch lands. Reversed style
/// only, no colors, so every theme flashes. Style-only and painted after
/// the frame capture above, so `body_rows` hit areas stay intact;
/// an expired flash clears here, queuing nothing.
fn paint_click_flash(f: &mut ratatui::Frame, app: &mut App) {
    if !app.flash_visible() {
        app.last_click = None;
        return;
    }
    let Some((rect, _)) = app.last_click else { return };
    let area = f.area();
    let x0 = rect.x.max(area.x);
    let y0 = rect.y.max(area.y);
    let x1 = (rect.x + rect.width).min(area.x + area.width);
    let y1 = (rect.y + rect.height).min(area.y + area.height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let buf = f.buffer_mut();
    for y in y0..y1 {
        for x in x0..x1 {
            buf[(x, y)].set_style(Style::new().reversed());
        }
    }
}

/// Auth-scope prompt (`gh` token dead or under-scoped): names the cause,
/// the missing scopes, and the exact re-auth command. `y` suspends the
/// TUI and runs it; `n` dismisses to a status-line hint.
fn render_auth(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, ink, muted, paper, teal, base};
    let Some(p) = app.auth_prompt.as_ref() else { return };
    let popup = centered(area, 80, 60);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(amber()))
        .style(base())
        .title(" GitHub auth ");
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let mut lines = vec![
        Line::from(Span::styled(p.cause.clone(), Style::new().bg(ink()).fg(paper()))),
        Line::from(""),
    ];
    if !p.scopes_known {
        lines.push(Line::from(Span::styled(
            "scopes could not be read from this failure — the error above names the fix",
            Style::new().bg(ink()).fg(muted()),
        )));
    } else if p.missing.is_empty() {
        lines.push(Line::from(Span::styled(
            "the token is expired, revoked, or invalid",
            Style::new().bg(ink()).fg(muted()),
        )));
    } else {
        lines.push(Line::from(vec![
            Span::styled("missing scopes: ", Style::new().bg(ink()).fg(muted())),
            Span::styled(p.missing.join(", "), Style::new().bg(ink()).fg(amber()).bold()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("needed: ", Style::new().bg(ink()).fg(muted())),
            Span::styled(crate::api::REQUIRED_SCOPES.join(", "), Style::new().bg(ink()).fg(teal())),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        crate::api::reauth_command(),
        Style::new().bg(ink()).fg(paper()),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "y re-auth now · n dismiss (nothing is fetched until auth works)",
        Style::new().bg(ink()).fg(muted()),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_shortcuts(f: &mut ratatui::Frame, area: Rect) {
    use crate::theme::{amber, base, ink, muted, paper, teal};
    let width = area.width.min(76);
    let height = area.height.min(26);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base()).title(" Shortcuts ");
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let body = [
        "Navigation", "  tab panes · j/k select · ↑/↓ scroll · page up/down",
        "  1/2 details/visual · e refs · n/N search matches",
        "  ctrl-o back · ctrl-i forward", "",
        "Actions", "  : command · / find · f filter · b board · w window",
        "  o open · r refresh · p pin · y yank · t theme · q quit · c comment",
        "  a sort · A reverse · x evict", "",
        "Board", "  h/l columns · j/k cards · space expand · enter open · f filter", "",
        "Diff", "  ↑/↓ lines · space expand · s split", "",
        "Esc / ? / q close",
    ];
    // Credit footer (display-only: the modal swallows all mouse input,
    // so the URL below is never clickable). Banner lines stay within
    // the popup inner width (76 - 2 borders).
    let banner = [
        "  ___ _   _ ___ _____",
        " / __| | | |_ _|_   _|",
        "| (_ | |_| || |  | |",
        " \\___|____|___|  |_|",
    ];
    let body_style = Style::new().bg(ink()).fg(paper());
    let mut lines: Vec<Line> = body.into_iter().map(|s| Line::styled(s, body_style)).collect();
    lines.push(Line::from(""));
    for b in banner {
        lines.push(Line::styled(b, Style::new().bg(ink()).fg(amber())));
    }
    lines.push(Line::styled("created by bolu-atx", Style::new().bg(ink()).fg(muted())));
    lines.push(Line::styled(
        "https://github.com/bolu-atx/gh-ghit",
        Style::new().bg(ink()).fg(teal()).underlined(),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Filter dialog (`f`). Over the board it is a form: a text row plus one
/// milestone pill per milestone on the cards; otherwise it holds the
/// recent window, open/closed toggles, and a GitHub search query. The
/// title names the target.
fn render_filter(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if crate::app::board_open(app) {
        render_board_filter(f, area, app);
        return;
    }
    use crate::theme::{amber, ink, muted, paper, base};
    let popup = centered(area, 62, 52);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        // Nerd Font Font Awesome fa-filter.
        .title(Line::from(vec![
            Span::styled(" \u{f0b0} Filter ", Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled("· sidebar ", Style::new().bg(ink()).fg(muted())),
        ]));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let window_mark = |hours: u64| {
        if app.merged_window_hours == hours {
            Span::styled("● ", Style::new().bg(ink()).fg(amber()).bold())
        } else {
            Span::styled("○ ", Style::new().bg(ink()).fg(muted()))
        }
    };
    let toggle = |on: bool, label: &str, key: &str| {
        // Nerd Font Font Awesome fa-check-square / fa-square-o. Checked
        // stays paper, unchecked stays muted; the key hint is unchanged.
        let box_glyph = if on { "\u{f14a}" } else { "\u{f096}" };
        let style = if on {
            Style::new().bg(ink()).fg(paper())
        } else {
            Style::new().bg(ink()).fg(muted())
        };
        vec![
            Span::styled(format!("{box_glyph} {label} "), style),
            Span::styled(format!("({key})"), Style::new().bg(ink()).fg(muted())),
        ]
    };
    let mut lines = vec![];
    let query_text = if app.query_edit { &app.query_draft } else { &app.query_active };
    let mut qline = vec![
        Span::styled("query ", Style::new().bg(ink()).fg(muted())),
        Span::styled(
            if query_text.is_empty() { "server github search, e to edit …".to_string() } else { query_text.clone() },
            Style::new().bg(ink()).fg(if query_text.is_empty() { muted() } else { paper() }),
        ),
    ];
    if app.query_edit {
        qline.push(Span::styled("▌", Style::new().bg(ink()).fg(amber()).bold()));
    } else {
        qline.push(Span::styled(" (e)".to_string(), Style::new().bg(ink()).fg(muted())));
    }
    lines.push(Line::from(qline));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        // Nerd Font Font Awesome fa-clock-o. The query row keeps its plain
        // "query " label so the edit-cursor column math below stays exact.
        Span::styled("\u{f017} window ", Style::new().bg(ink()).fg(muted())),
        window_mark(1),
        Span::styled("1h (1)  ", Style::new().bg(ink()).fg(paper())),
        window_mark(24),
        Span::styled("24h (2)  ", Style::new().bg(ink()).fg(paper())),
        window_mark(168),
        Span::styled("7d (3)", Style::new().bg(ink()).fg(paper())),
    ]));
    lines.push(Line::from(""));
    let mut open_line = vec![Span::styled("show   ", Style::new().bg(ink()).fg(muted()))];
    open_line.extend(toggle(app.show_open_prs, "open PRs", "o"));
    open_line.push(Span::raw("   "));
    open_line.extend(toggle(app.show_merged_prs, "merged PRs", "m"));
    lines.push(Line::from(open_line));
    let mut closed_line = vec![Span::styled("       ", Style::new().bg(ink()))];
    closed_line.extend(toggle(app.show_open_issues, "open issues", "i"));
    closed_line.push(Span::raw("   "));
    closed_line.extend(toggle(app.show_closed_issues, "closed issues", "c"));
    lines.push(Line::from(closed_line));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "1/2/3 window · e query · o/m/i/c toggle · w age · f/esc close",
        Style::new().bg(ink()).fg(muted()),
    )));
    f.render_widget(Paragraph::new(lines).style(base()), inner);
    if app.query_edit {
        f.set_cursor_position((inner.x + 6 + app.query_draft.len() as u16, inner.y));
    }
}

/// Board filter form (`f` with the board open): the text row plus one
/// milestone pill per milestone on the board. The active widget carries
/// the `▌` marker; `space` toggles a pill, typing on the text row edits
/// it, Enter applies, Esc clears. Empty pills select every card.
fn render_board_filter(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, base, ink, muted, paper};
    let pills = app.board_pills();
    let popup = centered(area, 62, (34 + pills.len() as u16 * 4).min(86));
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Line::from(vec![
            Span::styled(" \u{f0b0} Filter ", Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled("· board cards ", Style::new().bg(ink()).fg(muted())),
        ]));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    // Nerd Font Font Awesome fa-check-square / fa-square-o, same as the
    // sidebar filter toggles. The marker echoes the board card selection.
    let marker = |active: bool| {
        Span::styled(
            if active { "▌ " } else { "  " },
            Style::new().bg(ink()).fg(if active { amber() } else { muted() }).bold(),
        )
    };
    let empty = app.query_draft.is_empty();
    let mut text_row = vec![
        marker(app.filter_focus == 0),
        Span::styled("filter ", Style::new().bg(ink()).fg(muted())),
        Span::styled(
            if empty { "label, title, #num …".to_string() } else { app.query_draft.clone() },
            Style::new().bg(ink()).fg(if empty { muted() } else { paper() }),
        ),
    ];
    if app.query_edit {
        text_row.push(Span::styled("▌", Style::new().bg(ink()).fg(amber()).bold()));
    } else {
        text_row.push(Span::styled(" (e)".to_string(), Style::new().bg(ink()).fg(muted())));
    }
    let mut lines = vec![
        Line::from(text_row),
        Line::from(""),
        Line::from(Span::styled("milestones", Style::new().bg(ink()).fg(muted()))),
    ];
    if pills.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no milestones on this board",
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    for (i, pill) in pills.iter().enumerate() {
        let on = app.filter_milestone_draft.contains(pill);
        let box_glyph = if on { "\u{f14a}" } else { "\u{f096}" };
        let name = pill.clone().unwrap_or_else(|| "(no milestone)".to_string());
        lines.push(Line::from(vec![
            marker(app.filter_focus == i + 1),
            Span::styled(
                format!("{box_glyph} {name}"),
                Style::new().bg(ink()).fg(if on { paper() } else { muted() }),
            ),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "j/k move · space toggle · type filters text · enter apply · esc clear",
        Style::new().bg(ink()).fg(muted()),
    )));
    f.render_widget(Paragraph::new(lines).style(base()), inner);
    if app.query_edit {
        f.set_cursor_position((inner.x + 9 + app.query_draft.chars().count() as u16, inner.y));
    }
}

/// Darken every rendered cell toward the background so the modal sits proud.
fn dim_background(f: &mut ratatui::Frame, amount: f32) {
    let Color::Rgb(ir, ig, ib) = crate::theme::ink() else {
        return;
    };
    for cell in f.buffer_mut().content.iter_mut() {
        match cell.fg {
            Color::Rgb(r, g, b) => {
                let mix = |c: u8, i: u8| (i as f32 + (c as f32 - i as f32) * amount) as u8;
                cell.set_fg(Color::Rgb(mix(r, ir), mix(g, ig), mix(b, ib)));
            }
            Color::Reset => {}
            _ => {
                cell.set_fg(Color::Rgb(70, 76, 88));
            }
        }
    }
}

/// Split `text` into plain spans plus highlighted spans for each query
/// hit, so `/` search visibly marks the matched characters.
fn highlighted_spans(text: String, query: &str, base: Style) -> Vec<Span<'static>> {
    use crate::theme::{amber, ink};
    let hit = Style::new().fg(ink()).bg(amber()).bold();
    let ranges = crate::core::match_ranges(&text, query);
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

/// Rows the detail block of an expanded row occupies at `inner_width`.
/// Shares `expanded_row_lines` so hit-testing and rendering agree.
pub(crate) fn expanded_row_height(d: &IssueData, inner_width: usize) -> usize {
    expanded_row_lines(d, inner_width).len()
}

/// Compact CI glyph for a PR row or header: the head-commit rollup in
/// its theme color, or `None` when the PR runs no checks (or the state
/// is unknown). Issues never carry a rollup, so they render nothing.
fn check_span(d: &IssueData) -> Option<Span<'static>> {
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

/// One pane (Issues or PRs): pinned, requested, then recent rows with a
/// thin divider row between non-empty groups. Everything is owned, so the
/// returned list outlives the `&App` borrow and the caller can persist its
/// `ListState` back into `App` for scroll continuity.
fn section_list(
    title: Line<'static>,
    footer: Line<'static>,
    rows: &[crate::model::SideRow],
    app: &App,
    inner_width: usize,
    mark: Option<u64>,
    focused: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> List<'static> {
    use crate::theme::{amber, code_bg, ink, muted, paper, state_color};
    use crate::model::SideRow;
    let query = app.search.clone();
    let mut items: Vec<ListItem<'static>> = vec![ListItem::new(Line::from(""))];
    for row in rows {
        let SideRow::Item(n) = row else {
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
        // The selection marker renders on the focused pane for the
        // active issue only: `current` is the single active item, so the
        // wheel hover never draws the active marker. Hover stays a
        // transient preview in state (j/k continue from it); the
        // ListState cursor underneath is untouched, so tabbing away and
        // back restores the exact row and scroll position.
        let sel = focused && (app.current == Some(*n) || mark == Some(*n));
        let marker = if sel { "█ " } else { "  " };
        let marker_style = if sel {
            Style::new().fg(amber()).bold()
        } else {
            Style::new().fg(muted())
        };
        let (title_text, labels, updated, status) = match app.data.get(n) {
            Some(d) => {
                let st = app.statuses.get(n).cloned().unwrap_or_default();
                let (_, labels, updated, status) = model::sidebar_lines(
                    *n,
                    &d.title,
                    &d.labels,
                    &st,
                    &d.updated_at,
                    now,
                    38,
                    app.pinned.contains(n),
                );
                (model::slug(&d.title, 30), labels, updated, status)
            }
            None => (String::new(), String::new(), String::new(), String::new()),
        };
        // Nerd Font Font Awesome fa-thumb-tack marks pinned rows. Same two
        // cells as the marker it replaces, so sidebar hit-testing is untouched.
        let pin = if app.pinned.contains(n) { "\u{f08d} " } else { "" };
        let data = app.data.get(n);
        let num_style = Style::new()
            .fg(data.map(|d| state_color(&d.state)).unwrap_or(muted()))
            .bold();
        let title_style = if sel {
            Style::new().fg(paper()).bold()
        } else {
            Style::new().fg(paper())
        };
        let mut first = vec![
            Span::styled(marker, marker_style),
            Span::styled(pin, Style::new().fg(amber())),
        ];
        first.extend(highlighted_spans(format!("#{n} "), &query, num_style));
        first.extend(highlighted_spans(title_text, &query, title_style));
        // PR head-commit CI rollup: one glyph, no text, so hit-testing
        // and row height are untouched.
        if let Some(d) = data {
            if let Some(check) = check_span(d) {
                first.push(check);
            }
        }
        // Second line: labels · relative updated · project status. Empty
        // segments collapse; the status keeps the board color and glyph.
        let status_color = crate::theme::column_color(&status);
        let mut second = vec![Span::raw("    ")];
        let mut first_segment = true;
        for (text, style) in [
            (labels, Style::new().fg(muted())),
            (updated, Style::new().fg(muted())),
            (status, Style::new().fg(status_color)),
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
        if app.expanded_rows.contains(n) {
            if let Some(d) = data {
                lines.extend(expanded_row_lines(d, inner_width));
            }
        }
        lines.push(Line::from(""));
        items.push(ListItem::new(lines));
    }
    let border = if focused { amber() } else { muted() };
    List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(if focused { BorderType::Double } else { BorderType::Plain })
                .border_style(Style::new().bg(ink()).fg(border))
                .title(title)
                .title_bottom(footer)
                .style(Style::new().bg(ink())),
        )
        .highlight_style(if focused {
            Style::new().bold().bg(code_bg())
        } else {
            Style::new()
        })
}

/// Styled header for the body pane: the color-coded meta lines. The
/// `#N title` line lives only in the masthead (`render_main`), so it is
/// not repeated here. Each segment carries its semantic role; colors stay
/// consistent by construction.
fn header_lines(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<Line<'static>> {
    use crate::theme::{amber, ink, muted, paper, teal};
    use model::MetaKind;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    for (kind, text) in model::meta_spans(d, statuses) {
        let style = match kind {
            MetaKind::Dim => Style::new().bg(ink()).fg(muted()),
            MetaKind::State => Style::new().bg(ink()).fg(crate::theme::state_color(&d.state)).bold(),
            MetaKind::Label => Style::new().bg(ink()).fg(amber()),
            MetaKind::Milestone => Style::new().bg(ink()).fg(amber()),
            MetaKind::Normal => Style::new().bg(ink()).fg(paper()),
            MetaKind::Project => Style::new()
                .bg(ink())
                .fg(crate::theme::column_color(&text))
                .bold(),
            MetaKind::Url => Style::new().bg(ink()).fg(teal()).underlined(),
        };
        let mut chunks = text.split('\n');
        if let Some(first) = chunks.next() {
            // Project status carries the board glyph in the board color.
            if kind == MetaKind::Project {
                let icon = crate::theme::column_icon(&text);
                if !icon.is_empty() && first != "—" && !first.is_empty() {
                    current.push(Span::styled(format!("{icon} "), style));
                }
            }
            if !first.is_empty() {
                current.push(Span::styled(first.to_string(), style));
            }
        }
        for chunk in chunks {
            lines.push(Line::from(std::mem::take(&mut current)));
            if !chunk.is_empty() {
                current.push(Span::styled(chunk.to_string(), style));
            }
        }
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

/// First 50 files render; the rest collapse to a one-line note so a
/// huge PR cannot flood the pane.
pub(crate) const MAX_DIFF_FILES: usize = 50;

/// Diff tab body: one ruled section per file, folded by default.
/// Issues get a short note, PRs without data yet get a loading line.
/// `width` is the pane content width; narrow panes fall back to unified
/// lines because two columns would fit nothing.
fn diff_text(app: &App, d: &IssueData, width: usize) -> ratatui::text::Text<'static> {
    use crate::theme::{amber, green, ink, muted, paper, red};
    use model::DiffKind;
    if !d.is_pr {
        return ratatui::text::Text::from(Line::from(Span::styled(
            "Diff is only available for pull requests.",
            Style::new().bg(ink()).fg(muted()),
        )));
    }
    let Some(files) = app.diffs.get(&d.number) else {
        let note = if app.diff_pending.contains(&d.number) {
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
    let split = app.diff_split && width >= 40;
    let cursor = app.diff_cursor.min(files.len().saturating_sub(1).max(0));
    let cursor_name = files.get(cursor).map(|f| f.filename.as_str()).unwrap_or("");
    // Nerd Font Font Awesome fa-file-text reuses the per-file vocabulary;
    // the status words keep their own icons and stay spelled out. The `❯`
    // separator drops its dots so the icon nets zero width and the summary
    // wraps exactly as before.
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "\u{f15c} {} · {} (s: {}) · ↑↓ file, space fold ❯ {}",
            model::diff_summary(files),
            if split { "split" } else { "unified" },
            if split { "unified" } else { "split" },
            cursor_name,
        ),
        Style::new().bg(ink()).fg(muted()),
    ))];
    let cursor_here = app.pane == crate::app::Pane::Main;
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
        let expanded = app.expanded_files.contains(&(d.number, f.filename.clone()));
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
        if at_cursor {
            for span in head.spans.iter_mut() {
                span.style = span.style.patch(Style::new().bg(amber()).fg(ink()));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(rule.clone()));
        lines.push(head);
        for thread in d.review_threads.iter().filter(|t| t.path == f.filename && model::thread_patch_row(t, f).is_none()) {
            append_review_thread(&mut lines, thread, false);
        }
        if !expanded {
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
    lines.push(Line::from(Span::styled(format!("{indent}\u{f075} {line} · {state}"), Style::new().bg(crate::theme::ink()).fg(color).bold())));
    for comment in &thread.comments {
        let author = comment.author.as_ref().map(|a| a.login.as_str()).unwrap_or("unknown");
        lines.push(Line::from(Span::styled(format!("{indent}  {author}"), Style::new().bg(crate::theme::ink()).fg(crate::theme::paper()).bold())));
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

/// Graph tab body: the dependency star around the current issue, center
/// first then one section per relation. The cursor row carries the amber
/// marker; nodes that block nothing render gray/dim.
fn graph_text(app: &App, d: &IssueData) -> ratatui::text::Text<'static> {
    use crate::theme::{ink, muted};
    let dim = || Style::new().bg(ink()).fg(muted());
    let Some(data) = app.graphs.get(&d.number) else {
        let note = if app.graph_pending.contains(&d.number) {
            "Fetching graph …"
        } else {
            "No graph loaded yet."
        };
        return ratatui::text::Text::from(Line::from(Span::styled(note, dim())));
    };
    let order = model::graph_order(data);
    let mut lines = vec![Line::from(Span::styled(format!("{} related", order.len().saturating_sub(1)), dim()))];
    for (position, index) in order.iter().take(8).enumerate() {
        let node = &data.nodes[*index];
        if position > 0 {
            if let Some((kind, _)) = model::graph_sections(data).iter().find(|(_, group)| group.contains(index)) {
                lines.push(Line::from(Span::styled(format!("  {}", model::graph_edge_label(*kind)), dim())));
            }
        }
        let label = format!("#{} {}", node.number, model::slug(&node.title, 40));
        let width = label.chars().count() + 2;
        let prefix = if position == 0 { "  " } else { "  └─" };
        let marker = if app.graph_cursor == position { "█ " } else { "  " };
        let color = crate::theme::state_color(&node.state);
        lines.push(Line::from(Span::styled(format!("{prefix}╭{}╮", "─".repeat(width)), dim())));
        lines.push(Line::from(vec![Span::styled(format!("{prefix}│"), dim()), Span::styled(format!("{marker}{label}"), Style::new().bg(ink()).fg(color).bold()), Span::styled("│", dim())]));
        lines.push(Line::from(Span::styled(format!("{prefix}╰{}╯", "─".repeat(width)), dim())));
    }
    if order.len() > 8 { lines.push(Line::from(Span::styled(format!("+{} more", order.len() - 8), dim()))); }
    if data.truncated { lines.push(Line::from(Span::styled("More nodes omitted by data cap", dim()))); }
    if data.from_references { lines.push(Line::from(Span::styled("refs-derived (blocking fields unavailable)", dim()))); }
    ratatui::text::Text::from(lines)
}

fn section_title<'a>(name: &str, detail: &str, icon: &str, active: bool) -> Line<'a> {
    use crate::theme::{amber, muted};
    // Inactive panes go quiet: the colored name drops to the detail grey
    // so the active pane is obvious at a glance.
    let (icon_fg, name_style) = if active {
        (muted(), Style::new().fg(amber()).bold())
    } else {
        (muted(), Style::new().fg(muted()))
    };
    let icon_span = Span::styled(format!(" {icon} "), Style::new().fg(icon_fg));
    let name_span = Span::styled(format!("{name} "), name_style);
    let detail_span = Span::styled(detail.to_string(), Style::new().fg(muted()));
    Line::from(vec![icon_span, name_span, detail_span])
}

/// Bottom border label for a list pane: per-group counts in the same
/// muted voice as the title subtitle. Overlong footers clip at the
/// border; callers order counts before context so counts survive.
fn footer_line(text: &str) -> Line<'static> {
    use crate::theme::muted;
    Line::from(Span::styled(text.to_string(), Style::new().fg(muted())))
}

/// Position of `current` in display-row order, +1 for the top spacer.
fn selected_index(rows: &[model::SideRow], current: Option<u64>) -> Option<usize> {
    current
        .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
        .map(|p| p + 1)
}

fn render_sidebar(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    let issue_rows = app.issue_rows();
    let pr_rows = app.pr_rows();
    app.side_issue_area = panes[0];
    app.side_pr_area = panes[1];
    app.side_issue_rows = issue_rows.clone();
    app.side_pr_rows = pr_rows.clone();
    let inner = |a: Rect| (a.width as usize).saturating_sub(2);
    let now = chrono::Utc::now();
    let issues_focused = app.pane == crate::app::Pane::Issues;
    let il = section_list(
        // Nerd Font Font Awesome fa-exclamation-circle.
        section_title("Issues", &app.side_subtitle(false), "\u{f06a}", issues_focused),
        footer_line(&app.side_footer(false)),
        &issue_rows,
        app,
        inner(panes[0]),
        None,
        issues_focused,
        now,
    );
    let mut istate = std::mem::take(&mut app.issue_state);
    istate.select(selected_index(&issue_rows, app.effective(&issue_rows)));
    f.render_stateful_widget(il, panes[0], &mut istate);
    app.issue_state = istate;
    let prs_focused = app.pane == crate::app::Pane::PRs;
    let pl = section_list(
        // Nerd Font Font Awesome fa-code-fork.
        section_title("Pull requests", &app.side_subtitle(true), "\u{f126}", prs_focused),
        footer_line(&app.side_footer(true)),
        &pr_rows,
        app,
        inner(panes[1]),
        None,
        prs_focused,
        now,
    );
    let mut pstate = std::mem::take(&mut app.pr_state);
    pstate.select(selected_index(&pr_rows, app.effective(&pr_rows)));
    f.render_stateful_widget(pl, panes[1], &mut pstate);
    app.pr_state = pstate;
}

/// Right pane: issues/PRs mentioned in the current body, with the same row
/// look as the left sidebar. Renders only when `refs_visible`; the caller
/// hides the column otherwise so the main pane keeps its width.
fn render_refs(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let rows = app.refs_rows();
    app.refs_area = area;
    app.ref_rows = rows.clone();
    let inner = (area.width as usize).saturating_sub(2);
    let refs_focused = app.pane == crate::app::Pane::Refs;
    let list = section_list(
        // Nerd Font Font Awesome fa-link.
        section_title("Refs", &app.refs_subtitle(), "\u{f0c1}", refs_focused),
        footer_line(&app.refs_footer()),
        &rows,
        app,
        inner,
        app.ref_highlight,
        refs_focused,
        chrono::Utc::now(),
    );
    let mut state = std::mem::take(&mut app.refs_state);
    let sel = app
        .ref_highlight
        .and_then(|n| rows.iter().position(|r| r.number() == Some(n)))
        .map(|p| p + 1);
    state.select(sel);
    f.render_stateful_widget(list, area, &mut state);
    app.refs_state = state;
}

/// Greedy word-wrap of styled lines to `width` cells. Words longer than
/// the width hard-split; single spaces rejoin the words. Blank lines pass
/// through so paragraph breaks survive. Lets the title/label block above
/// the tabs claim exactly the rows it needs at any window width.
fn wrap_lines<'a>(lines: Vec<Line<'a>>, width: usize) -> Vec<Line<'a>> {
    let width = width.max(1);
    let mut out: Vec<Line<'a>> = vec![];
    for line in lines {
        let mut words: Vec<(String, Style)> = vec![];
        for span in &line.spans {
            for chunk in span.content.split(' ') {
                if chunk.is_empty() {
                    continue;
                }
                if chunk.chars().count() <= width {
                    words.push((chunk.to_string(), span.style));
                    continue;
                }
                // Hard-split an overlong word so nothing clips.
                let mut rest = chunk;
                while !rest.is_empty() {
                    let take: String = rest.chars().take(width).collect();
                    words.push((take.clone(), span.style));
                    rest = &rest[take.len()..];
                }
            }
        }
        let mut cur: Vec<Span<'a>> = vec![];
        let mut cur_w = 0;
        for (word, style) in words {
            let w = word.chars().count();
            if cur_w == 0 {
                cur.push(Span::styled(word, style));
                cur_w = w;
            } else if cur_w + 1 + w <= width {
                cur.push(Span::styled(" ".to_string(), style));
                cur.push(Span::styled(word, style));
                cur_w += 1 + w;
            } else {
                out.push(Line::from(std::mem::take(&mut cur)));
                cur.push(Span::styled(word, style));
                cur_w = w;
            }
        }
        out.push(Line::from(cur));
    }
    out
}

/// Light up every line that mentions issue `n`, for refs-row selection.
fn highlight_mentions(lines: &mut [Line<'_>], n: u64, repo: Option<&str>) {
    use crate::theme::{amber, ink};
    for line in lines.iter_mut() {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let mentions = model::links_in_line(&plain, repo)
            .into_iter()
            .any(|(_, t)| t == model::LinkTarget::Issue(n));
        if mentions {
            for span in line.spans.iter_mut() {
                span.style = span.style.patch(Style::new().bg(amber()).fg(ink()));
            }
        }
    }
}

/// Display-only line under the Details header meta: the will-close
/// counterparts with their state chips. Empty pairs collapse the line.
fn paired_line(d: &IssueData) -> Option<Line<'static>> {
    use crate::theme::{ink, muted, state_color};
    if d.paired.is_empty() {
        return None;
    }
    let mut spans = vec![Span::styled("Paired  ", Style::new().bg(ink()).fg(muted()).bold())];
    for (i, pair) in d.paired.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
        }
        let color = state_color(&pair.state);
        spans.push(Span::styled(format!("#{}", pair.number), Style::new().bg(ink()).fg(color).bold()));
        if !pair.state.is_empty() {
            spans.push(Span::styled(format!(" {}", pair.state), Style::new().bg(ink()).fg(color)));
        }
    }
    Some(Line::from(spans))
}

fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let repo = app.repo_key().unwrap_or_default();
    // The meta/label block lives above the tabs so long titles wrap in
    // place instead of overflowing the masthead. Its height is measured
    // first (same inner width as the body) so the rows below just fit.
    let inner_w = (area.width as usize).saturating_sub(5);
    let mut header: Vec<Line> = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => {
            let st = app.statuses.get(&d.number).cloned().unwrap_or_default();
            header_lines(d, &st)
        }
        None => vec![],
    };
    if let (Some(n), crate::app::Tab::Details) = (app.ref_highlight, app.tab) {
        highlight_mentions(&mut header, n, app.repo_key().as_deref());
    }
    let header = wrap_lines(header, inner_w);
    // The paired line is display-only: it renders with the header block but
    // stays out of `header_rows`, so clicks never navigate from it.
    let paired = match (app.tab, app.current.and_then(|n| app.data.get(&n))) {
        (crate::app::Tab::Details, Some(d)) => paired_line(d)
            .map(|line| wrap_lines(vec![line], inner_w))
            .unwrap_or_default(),
        _ => vec![],
    };
    app.header_rows = header
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let header: Vec<Line> = header.into_iter().chain(paired).collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(header.len() as u16),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(area);

    // rows[0] masthead (brand line, then the issue line), rows[1]
    // meta/labels, rows[2] breathing room, rows[3] tabs, rows[4] pane.
    app.tabs_area = rows[3];
    // Nerd Font Font Awesome fa-git brands the wordmark; repo and number
    // stay plain text so identifiers scan without a patched font. The PR
    // head-commit CI glyph rides the issue line, the single title source
    // now that the header block starts at the meta lines.
    let mast: Vec<Line> = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => {
            let mut title = vec![
                Span::styled(format!("#{} ", d.number), Style::new().fg(muted())),
                Span::styled(model::slug(&d.title, 60), Style::new().fg(paper()).bold()),
            ];
            if let Some(check) = check_span(d) {
                title.push(check);
            }
            vec![
                Line::from(vec![
                    Span::styled("\u{f1d3} ghit ", Style::new().fg(amber()).bold()),
                    Span::styled(repo, Style::new().fg(muted())),
                ]),
                Line::from(title),
            ]
        }
        None => vec![Line::from(vec![
            Span::styled("\u{f1d3} ghit ", Style::new().fg(amber()).bold()),
            Span::styled(repo, Style::new().fg(muted())),
        ])],
    };
    f.render_widget(Paragraph::new(mast).style(base()), rows[0]);

    let main_focused = app.pane == crate::app::Pane::Main;
    // Second tab follows the current item: PRs show their diff, issues
    // the dependency graph. Nothing loaded yet reads as Graph.
    let visual_label = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) if d.is_pr => "Diff View",
        _ => "Graph",
    };
    // Nerd Font glyphs: file-text, comments, code. Labels stay so a
    // missing font degrades to a tofu box plus readable text.
    let tabs = Tabs::new(vec![
        format!("\u{f15c}  Details"),
        format!("\u{f121}  {visual_label}"),
    ])
        .select(match app.tab { Tab::Details => 0, Tab::Visual => 1 })
        .style(Style::new().bg(ink()).fg(muted()))
        .highlight_style(Style::new().bg(ink()).fg(amber()).bold().underlined())
        .divider("  ")
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(if main_focused { BorderType::Double } else { BorderType::Plain })
                .border_style(Style::new().bg(ink()).fg(if main_focused { amber() } else { muted() }))
                .style(Style::new().bg(ink())),
        );
    f.render_widget(tabs, rows[3]);

    // Meta/labels render above the tabs with the body's gutter, so the
    // header starts where the body text starts.
    let hgutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[1]);
    app.header_area = hgutter[1];
    f.render_widget(Paragraph::new(header).style(base()), hgutter[1]);

    let markdown;
    let mut text = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => match app.tab {
            Tab::Details => {
                markdown = format!("## Body\n\n{}\n\n## Comments\n\n{}", model::body_md(d), model::build_comments_md(d));
                crate::theme::render_markdown(&markdown)
            }
            Tab::Visual => match model::visual_content(d.is_pr) {
                model::VisualContent::Diff => diff_text(app, d, (rows[4].width as usize).saturating_sub(5)),
                model::VisualContent::Graph => graph_text(app, d),
            },
        },
        None => crate::theme::render_markdown(
            "No issue loaded.\n\nRecent issues and PRs are on the left.\nType a number and hit Enter (`:`), or instantly filter loaded rows (title, label, author) with `/`.",
        ),
    };
    // A refs-row selection lights up the mentioning line instead of opening
    // the issue. Body tab only: the mentions come from the body markdown
    // (and the header above the tabs).
    if let (Some(n), crate::app::Tab::Details) = (app.ref_highlight, app.tab) {
        highlight_mentions(&mut text.lines, n, app.repo_key().as_deref());
    }
    let pane = Paragraph::new(text)
        .style(base())
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    // Breathing room: text never touches the column edges.
    let gutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[4]);
    app.body_area = gutter[1];
    f.render_widget(pane, gutter[1]);

}

/// Status bar spinner frames, stepped by the event-loop tick. Braille
/// dots read as motion at a glance and stay one cell wide.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, base, ink, muted, paper};
    if app.focus == crate::app::Focus::Bar {
        let text = if app.searching { &app.search } else { &app.bar };
        let prompt = if app.searching { "/ " } else { "❯ " };
        let shown = if text.is_empty() {
            if app.searching {
                format!(
                    "instant filter loaded: title, #n, label, author · sort {} (a/A) …",
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
        f.set_cursor_position((area.x + (2 + text.len() as u16).min(area.width.saturating_sub(1)), area.y));
        return;
    }
    // Version-only brand: no commit-SHA wiring exists, and a build.rs
    // just for the SHA would over-engineer the bar.
    let hint = format!("? for shortcuts · ghit v{}", env!("CARGO_PKG_VERSION"));
    let hint_width = hint.chars().count() as u16;
    let left_width = area.width.saturating_sub(hint_width + 1);
    let mut segments: Vec<(String, bool)> = Vec::new();
    if let Some(repo) = app.repo_key().filter(|s| !s.is_empty()) {
        segments.push((format!("\u{f1d3} {repo}"), false));
    }
    if let Some(statuses) = app.current.and_then(|n| app.statuses.get(&n)) {
        for (title, status) in statuses.iter().filter(|(title, _)| !title.is_empty()) {
            let suffix = status.as_ref().filter(|s| !s.is_empty()).map(|s| format!(" ({s})")).unwrap_or_default();
            segments.push((format!("\u{f0db} {title}{suffix}"), false));
        }
    }
    // Loading spinner between project and message: muted, with
    // determinate `n/m` where known. Explicit messages win, so the
    // derivation already hides this under one; empty collapses per the
    // existing rule. The label truncates first so the `?` hint never
    // shrinks, keeping the 80-cell bar intact.
    let loads = crate::core::loading(&app.fetch_state());
    if !loads.is_empty() {
        let label = crate::core::loading_label(&loads);
        let used: usize = segments.iter().map(|(s, _)| s.chars().count()).sum::<usize>();
        let room = (left_width as usize).saturating_sub(used + segments.len() * 3 + 2);
        if room > 0 {
            let shown = crate::core::toast_snippet(&label, room.max(1));
            let frame = SPINNER[(app.tick as usize) % SPINNER.len()];
            segments.push((format!("{frame} {shown}"), false));
        }
    }
    if app.message_visible() && !app.message.is_empty() {
        segments.push((format!("\u{f071} {}", app.message), true));
    }
    let mut spans = Vec::new();
    for (i, (text, accent)) in segments.into_iter().enumerate() {
        if i > 0 { spans.push(Span::styled(" · ", Style::new().fg(muted()))); }
        spans.push(Span::styled(text, Style::new().fg(if accent { amber() } else { paper() })));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(base()), Rect::new(area.x, area.y, left_width, area.height));
    if area.width >= hint_width {
        f.render_widget(Paragraph::new(hint).style(Style::new().bg(ink()).fg(muted())), Rect::new(area.x + area.width - hint_width, area.y, hint_width, area.height));
    }
}

/// Collapsed card: title, labels, spacer. Expanded adds status, url, excerpt.
const BASE_CARD_H: u16 = 3;
const EXPANDED_CARD_H: u16 = 6;

fn card_height(expanded: bool) -> u16 {
    if expanded {
        EXPANDED_CARD_H
    } else {
        BASE_CARD_H
    }
}

/// Column areas with a 1-cell gutter so borders never touch.
fn column_layout(area: Rect, n_cols: usize, col_width: u16) -> Vec<Rect> {
    let mut constraints = Vec::with_capacity(n_cols * 2);
    for i in 0..n_cols {
        constraints.push(Constraint::Length(col_width));
        if i + 1 < n_cols {
            constraints.push(Constraint::Length(1));
        }
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    chunks.iter().step_by(2).copied().collect()
}

fn render_board(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, base};
    let popup = centered(area, 92, 92);
    f.render_widget(Clear, popup);
    // Nerd Font Font Awesome fa-columns; the board name keeps its label.
    // Column headers stay icon-free: their vocabularies are user-defined
    // and already color-coded, so glyphs would add noise, not signal.
    let title = match &app.board {
        Some(b) => Line::from(vec![
            Span::styled(" \u{f0db} ", Style::new().bg(ink()).fg(muted())),
            Span::styled(format!("{} ", b.title), Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled(
                format!("{} cards", b.items.len()),
                Style::new().bg(ink()).fg(muted()),
            ),
        ]),
        None => Line::from(vec![
            Span::styled(" \u{f0db} Fetching board … ", Style::new().bg(ink()).fg(amber()).bold()),
        ]),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(title);
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    render_board_links(f, popup, app);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (inner, hint_area) = (chunks[0], chunks[1]);
    let filtered = app.board_filtered();
    let mut hint = vec![];
    if !app.board_filter.trim().is_empty() {
        hint.push(Span::styled("filter ", Style::new().bg(ink()).fg(muted())));
        hint.push(Span::styled(
            format!("\"{}\"", app.board_filter),
            Style::new().bg(ink()).fg(amber()).bold(),
        ));
        hint.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
    }
    if !app.board_milestones.is_empty() {
        let names: Vec<String> = app
            .board_pills()
            .into_iter()
            .filter(|p| app.board_milestones.contains(p))
            .map(|p| p.unwrap_or_else(|| "(no milestone)".to_string()))
            .collect();
        hint.push(Span::styled("milestones ", Style::new().bg(ink()).fg(muted())));
        hint.push(Span::styled(
            names.join(", "),
            Style::new().bg(ink()).fg(amber()).bold(),
        ));
        hint.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
    }
    hint.push(Span::styled(
        if filtered {
            "h/l columns · j/k cards · space expand · enter open · f filter · esc clear"
        } else {
            "h/l columns · j/k cards · space expand · enter open · f filter · esc close"
        },
        Style::new().bg(ink()).fg(muted()),
    ));
    f.render_widget(Paragraph::new(Line::from(hint)), hint_area);

    let Some(board) = &app.board else {
        render_skeleton(f, inner);
        return;
    };
    let cols = crate::app::board_columns(board, &app.board_filter, &app.board_milestones);
    if cols.is_empty() {
        f.render_widget(Paragraph::new("empty board").style(base()), inner);
        return;
    }
    while app.board_states.len() < cols.len() {
        app.board_states.push(ListState::default());
    }
    let col_areas = column_layout(inner, cols.len(), 36);
    app.board_cells.clear();
    for (i, (name, cards)) in cols.iter().enumerate() {
        let Some(a) = col_areas.get(i) else { continue };
        let inner_w = a.width.saturating_sub(2);
        let items: Vec<ListItem> = cards
            .iter()
            .enumerate()
            .map(|(ri, c)| {
                let sel = app.board_col == i && app.board_row == ri;
                let detail = c.number.and_then(|n| app.board_details.get(&n));
                board_card(c, sel, app.board_expanded == Some((i, ri)), detail, inner_w)
            })
            .collect();
        let active = app.board_col == i;
        let color = crate::theme::column_color(name);
        let icon = crate::theme::column_icon(name);
        let mut head = vec![];
        if icon.is_empty() {
            head.push(Span::styled(
                format!(" {name} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
        } else {
            head.push(Span::styled(
                format!(" {icon} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
            head.push(Span::styled(
                format!("{name} "),
                Style::new().bg(ink()).fg(color).bold(),
            ));
        }
        head.push(Span::styled(
            format!("({})", cards.len()),
            Style::new().bg(ink()).fg(muted()),
        ));
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(if active { amber() } else { muted() }))
                .style(base())
                .title(Line::from(head)),
        );
        let state = &mut app.board_states[i];
        state.select(if active { Some(app.board_row) } else { None });
        f.render_stateful_widget(list, *a, state);
        let col_inner = Block::default().borders(Borders::ALL).inner(*a);
        if cards.is_empty() {
            // An empty lane keeps its width and says so, instead of
            // reading as a broken fetch. A filter that hides every card
            // reads as no match, not as an empty column.
            let label = if filtered { "  no matches" } else { "  no cards" };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    label,
                    Style::new().bg(ink()).fg(muted()),
                )))
                .style(base()),
                col_inner,
            );
        }
        // Map visible cards back to click areas using the widget's scroll
        // offset. Cards have variable height, so walk line offsets.
        let offset = app.board_states[i].offset();
        let mut y = col_inner.y;
        for (ri, _) in cards.iter().enumerate().skip(offset) {
            let h = card_height(app.board_expanded == Some((i, ri)));
            if y + h > col_inner.y + col_inner.height {
                break;
            }
            app.board_cells.push((Rect::new(col_inner.x, y, col_inner.width, h), i, ri));
            y += h;
        }
    }
}

/// Top-right header icons: a flag for the repo milestones page plus an
/// external-link for the project board itself. Nerd Font glyphs only,
/// in the same teal-underlined URL style as card links, so the header
/// stays one row tall. Hit areas land in `app.board_links`.
fn render_board_links(f: &mut ratatui::Frame, popup: Rect, app: &mut App) {
    use crate::theme::{base, ink, teal};
    app.board_links.clear();
    let mut links: Vec<(String, String)> = vec![];
    // Nerd Font Font Awesome fa-flag: the repo milestones page.
    if let Some(repo) = app.repo_flag.clone().or_else(|| app.repo_slug.clone()) {
        links.push((
            "\u{f024}".to_string(),
            format!("https://github.com/{repo}/milestones"),
        ));
    }
    // Nerd Font Font Awesome fa-external-link: the project board page,
    // from `gh project list` (old caches predate it and show no icon
    // until the board refetches).
    if let Some(url) = app.board.as_ref().map(|b| b.url.clone()) {
        if !url.is_empty() {
            links.push(("\u{f08e}".to_string(), url));
        }
    }
    if links.is_empty() || popup.width < 12 {
        return;
    }
    let width = links.len() as u16 * 3;
    let x = popup.x + popup.width.saturating_sub(width + 1);
    let spans: Vec<Span> = links
        .iter()
        .map(|(glyph, _)| {
            Span::styled(
                format!(" {glyph} "),
                Style::new().bg(ink()).fg(teal()).underlined(),
            )
        })
        .collect();
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(base()),
        Rect::new(x, popup.y, width, 1),
    );
    for (i, (_, url)) in links.iter().enumerate() {
        app.board_links
            .push((Rect::new(x + i as u16 * 3, popup.y, 3, 1), url.clone()));
    }
}

/// What kind of item this is: issue, PR, or draft.
fn card_kind(c: &model::BoardItem) -> String {
    match c.content.get("type").and_then(|t| t.as_str()) {
        Some("PullRequest") => "PR".to_string(),
        Some("DraftIssue") => "DRAFT".to_string(),
        Some("Issue") => "ISSUE".to_string(),
        Some(other) => other.to_string(),
        None => "ITEM".to_string(),
    }
}

pub(crate) fn card_url(c: &model::BoardItem) -> Option<String> {
    c.content.get("url").and_then(|u| u.as_str()).map(|s| s.to_string())
}

fn card_excerpt(c: &model::BoardItem) -> String {
    let body = c.content.get("body").and_then(|b| b.as_str()).unwrap_or("");
    match body.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some(first) => model::slug(first, 28),
        None => "–".to_string(),
    }
}

/// A board card: 3 lines collapsed (head, breathing room, footer with
/// labels left and dates right), 6 lines expanded (status, URL, excerpt,
/// dates plus comment count). Matches `BASE_CARD_H` / `EXPANDED_CARD_H`.
fn board_card(
    c: &model::BoardItem,
    sel: bool,
    expanded: bool,
    detail: Option<&model::BoardDetail>,
    inner_width: u16,
) -> ListItem<'static> {
    use crate::theme::{amber, column_color, ink, muted, paper, teal};
    let title = model::slug(&c.title, 26);
    let head = match c.number {
        Some(n) => format!("#{n} {title}"),
        None => format!("◇ {title}"),
    };
    let first = if sel {
        Line::from(vec![
            Span::styled("▌ ", Style::new().bg(ink()).fg(amber()).bold()),
            Span::styled(head, Style::new().bg(ink()).fg(paper()).bold()),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ", Style::new().bg(ink())),
            Span::styled(head, Style::new().bg(ink()).fg(paper())),
        ])
    };
    if !expanded {
        // Labels own the left; the swimlane carries the status, so the
        // collapsed card never repeats it.
        let room = (inner_width as usize).saturating_sub(4);
        let right = model::card_footer_date(detail);
        let max_left = room.saturating_sub(right.chars().count() + 2);
        let mut left = model::card_footer_labels(c);
        if left.chars().count() > max_left && max_left > 1 {
            left = format!("{}…", left.chars().take(max_left.saturating_sub(1)).collect::<String>());
        }
        let pad = room.saturating_sub(left.chars().count() + right.chars().count()).max(1);
        let footer = format!("    {left}{}{right}", " ".repeat(pad));
        return ListItem::new(vec![
            first,
            Line::from(Span::styled("", Style::new().bg(ink()))),
            Line::from(Span::styled(footer, Style::new().bg(ink()).fg(muted()))),
        ]);
    }
    let status = c.status.clone().unwrap_or_else(|| "No status".to_string());
    let icon = crate::theme::column_icon(&status);
    let status_label = if icon.is_empty() {
        status.clone()
    } else {
        format!("{icon} {status}")
    };
    let mut meta = format!("{} · {}", status_label, card_kind(c));
    if let Some(login) = card_assignee(c) {
        meta.push_str(&format!(" · @{login}"));
    }
    // No icon here: expanded date lines already fill the 34-cell column
    // inner width, so any glyph would push the comment count off-screen.
    let dates = match detail {
        Some(d) => format!("{} → {} · {}", d.created, d.updated, comment_word(d.comments)),
        None => "…".to_string(),
    };
    ListItem::new(vec![
        first,
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(meta, Style::new().bg(ink()).fg(column_color(&status))),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(
                card_url(c).unwrap_or_else(|| "–".to_string()),
                Style::new().bg(ink()).fg(teal()).underlined(),
            ),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(card_excerpt(c), Style::new().bg(ink()).fg(paper())),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::new().bg(ink())),
            Span::styled(dates, Style::new().bg(ink()).fg(muted())),
        ]),
        Line::from(Span::styled("", Style::new().bg(ink()))),
    ])
}

fn comment_word(n: usize) -> String {
    if n == 0 {
        "no comments".to_string()
    } else {
        format!("{} comment{}", n, if n == 1 { "" } else { "s" })
    }
}

/// First assignee login off the card payload, when the board ships one.
fn card_assignee(c: &model::BoardItem) -> Option<String> {
    let arr = c.content.get("assignees").and_then(|v| v.as_array())?;
    arr.iter().filter_map(|a| {
        a.get("login").and_then(|l| l.as_str()).or_else(|| a.as_str())
    }).map(str::to_string).next()
}

/// Comment compose modal (`c`): textarea over a rendered markdown
/// preview through the shared `GhitSheet` stylesheet. The draft shows a
/// `▌` cursor marker; while the background submit runs, the title reads
/// Sending and editing locks. Footer names the keys; resolving threads
/// is out of scope and never offered here.
fn render_compose(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, ink, muted, base, paper};
    let Some(c) = app.compose.as_ref() else { return };
    let popup = centered(area, 80, 76);
    f.render_widget(Clear, popup);
    let head = if c.sending { " Sending… " } else { " Compose " };
    let title = Line::from(vec![
        Span::styled(head, Style::new().bg(ink()).fg(amber()).bold()),
        Span::styled(format!("{} ", c.target.title(c.number)), Style::new().bg(ink()).fg(paper())),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(title);
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    // Draft with a visible cursor marker at the char index.
    let cursor = c.cursor.min(c.body.chars().count());
    let mut draft = String::with_capacity(c.body.len() + 3);
    for (i, ch) in c.body.chars().enumerate() {
        if i == cursor {
            draft.push('▌');
        }
        draft.push(ch);
    }
    if cursor == c.body.chars().count() {
        draft.push('▌');
    }
    let draft_text = if c.body.trim().is_empty() {
        Paragraph::new(Span::styled("write a comment…", Style::new().bg(ink()).fg(muted()).italic()))
    } else {
        Paragraph::new(draft).style(Style::new().bg(ink()).fg(paper()))
    };
    f.render_widget(
        draft_text.block(Block::default().borders(Borders::BOTTOM).border_style(Style::new().bg(ink()).fg(muted())).title(Span::styled(" draft ", Style::new().bg(ink()).fg(muted())))),
        chunks[0],
    );
    let preview = if c.body.trim().is_empty() {
        ratatui::text::Text::from(Line::from(Span::styled("(nothing to preview)", Style::new().bg(ink()).fg(muted()).italic())))
    } else {
        crate::theme::render_markdown(&c.body)
            .lines
            .into_iter()
            .map(|line| Line::from(line.spans.into_iter().map(|s| Span::styled(s.content.into_owned(), s.style)).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
            .into()
    };
    f.render_widget(
        Paragraph::new(preview).style(base()).block(Block::default().title(Span::styled(" preview ", Style::new().bg(ink()).fg(muted())))),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if c.sending { "sending… esc closes, the send still lands" } else { "type to write · ctrl-s send · esc discard" },
            Style::new().bg(ink()).fg(muted()),
        ))),
        chunks[2],
    );
}

/// Placeholder columns shown while the network fetch is in flight.
fn render_skeleton(f: &mut ratatui::Frame, area: Rect) {
    use crate::theme::{ink, muted, base};
    let col_areas = column_layout(area, 3, 36);
    for a in col_areas.iter() {
        let ghost: Vec<ListItem> = (0..4)
            .map(|_| {
                ListItem::new(vec![
                    Line::from(Span::styled("  ▓▓▓▓▓▓▓▓▓▓▓▓", Style::new().bg(ink()).fg(muted()))),
                    Line::from(Span::styled("    ▓▓▓▓▓▓▓▓", Style::new().bg(ink()).fg(muted()))),
                    Line::from(""),
                ])
            })
            .collect();
        let list = List::new(ghost).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .style(base())
                .title(Span::styled(" ━━━ ", Style::new().bg(ink()).fg(muted()))),
        );
        f.render_widget(list, *a);
    }
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v)[1]
}

#[cfg(test)]
mod render_smoke {
    use super::render;
    use crate::app::{App, Tab, board_move, poll_sections};
    use crate::input::{handle_key, handle_mouse};
    use crate::model::{
        Author, BoardData, BoardDetail, BoardItem, GraphData, GraphEdgeKind, GraphLinkedPr,
        GraphNode, IssueData, Label, PairedRef,
    };
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn board_app(n_cards: usize) -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let items: Vec<BoardItem> = (0..n_cards)
            .map(|i| BoardItem {
                title: format!("Card {i}"),
                number: Some(100 + i as u64),
                status: Some("Todo".to_string()),
                labels: vec![Label { name: "x".to_string() }],
                repo: Some("o/r".to_string()),
                milestone: None,
                content: serde_json::json!({
                    "type": "Issue",
                    "url": format!("https://github.com/o/r/issues/{}", 100 + i),
                    "body": "## Why\n\nSome details here",
                }),
            })
            .collect();
        app.board = Some(BoardData {
            title: "Roadmap".to_string(),
            order: vec!["Todo".to_string()],
            items,
            url: "https://github.com/orgs/o/projects/1".to_string(),
        });
        app
    }

    #[test]
    fn board_skeleton_shows_while_loading() {
        let mut app = App::new(Some("o/r".to_string()));
        app.board_loading = true;
        let text = frame_text(&mut app);
        assert!(text.contains("Fetching"), "skeleton title");
    }

    /// 80-cell bar text, so tests can assert the hint survives truncation.
    fn frame_text_narrow(app: &mut App, width: u16) -> String {
        let backend = TestBackend::new(width, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn bar_shows_spinner_while_loading_then_collapses() {
        let mut app = loaded_app();
        app.tick = 0;
        app.issue_pending.insert(7);
        let text = frame_text(&mut app);
        assert!(text.contains("⠋ issue"), "spinner plus label: {text}");
        // Determinate progress rides the label where known.
        app.issue_progress.insert(7, (2, 7));
        let text = frame_text(&mut app);
        assert!(text.contains("issue 2/7"), "determinate progress: {text}");
        // The spinner animates off the cheap frame tick.
        app.tick = 1;
        let text = frame_text(&mut app);
        assert!(text.contains("⠙ issue 2/7"), "next frame: {text}");
        // Done means collapsed, not blank space.
        app.issue_pending.clear();
        app.issue_progress.clear();
        let text = frame_text(&mut app);
        assert!(!text.contains("⠋") && !text.contains("⠙"), "spinner gone: {text}");
    }

    #[test]
    fn bar_message_wins_over_spinner() {
        let mut app = loaded_app();
        app.issue_pending.insert(7);
        app.issue_progress.insert(7, (2, 7));
        app.notify("refreshed #7".to_string());
        let text = frame_text(&mut app);
        assert!(text.contains("refreshed #7"), "message shows: {text}");
        assert!(!text.contains("2/7"), "spinner yields: {text}");
    }

    #[test]
    fn click_flash_appears_then_expires() {
        use ratatui::style::Modifier;
        use std::time::{Duration, Instant};
        let mut app = loaded_app();
        // First frame populates hit areas and the per-frame row cache.
        let _ = frame_text(&mut app);
        assert!(app.last_click.is_none(), "no flash before input");
        let (x, y) = (app.side_issue_area.x + 1, app.side_issue_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        let Some((rect, _)) = app.last_click else {
            panic!("press records a flash");
        };
        let body_before = app.body_rows.clone();
        // The next frame inverts the clicked rect, style-only.
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        assert_eq!(app.body_rows, body_before, "flash paint keeps hit areas");
        let buf = terminal.backend().buffer();
        let flashed = (rect.y..rect.y + rect.height).any(|yy| {
            (rect.x..rect.x + rect.width)
                .any(|xx| buf[(xx, yy)].style().add_modifier.contains(Modifier::REVERSED))
        });
        assert!(flashed, "clicked row inverts while the flash lives");
        // Past the dwell the flash clears instead of painting.
        app.last_click = Some((rect, Instant::now() - Duration::from_millis(1000)));
        let _ = frame_text(&mut app);
        assert!(app.last_click.is_none(), "expired flash clears");
    }

    #[test]
    fn drag_hovers_without_visiting_and_up_is_noop() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let visited_before = app.visited.clone();
        let (x, y) = (app.side_pr_area.x + 1, app.side_pr_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::Drag(MouseButton::Left), x, y));
        assert_eq!(app.hover, Some(9), "drag previews the row");
        assert_eq!(app.current, Some(7), "drag never visits");
        assert_eq!(app.visited, visited_before, "drag files nothing");
        assert!(app.last_click.is_some(), "drag flashes");
        let current = app.current;
        let hover = app.hover;
        let visited = app.visited.clone();
        handle_mouse(&mut app, click(MouseEventKind::Up(MouseButton::Left), x, y));
        assert_eq!(app.current, current, "release changes nothing");
        assert_eq!(app.hover, hover, "release keeps hover");
        assert_eq!(app.visited, visited, "release files nothing");
    }

    #[test]
    fn bar_loading_fits_the_80_cell_bar() {
        let mut app = loaded_app();
        app.issue_pending.insert(7);
        app.diff_pending.insert(9);
        app.graph_pending.insert(7);
        app.detail_pending.insert(100);
        app.board_loading = true;
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        let _ = tx.send(crate::github::SectionEvent::Progress { done: 120, total: 500 });
        crate::app::poll_sections(&mut app);
        // Narrow render must not panic, and the `?` hint always survives:
        // the loading label truncates first, never the hint.
        let text = frame_text_narrow(&mut app, 80);
        assert!(text.contains("? for shortcuts"), "hint survives: {text}");
        assert!(text.contains("⠋"), "spinner shows: {text}");
    }

    #[test]
    fn board_cards_show_labels_not_status() {
        let mut app = board_app(3);
        let text = frame_text(&mut app);
        assert!(text.contains("Card 0"));
        assert!(text.contains("● x"));
        // The swimlane carries the status; collapsed cards don't repeat it.
        // (The column header "Todo (n)" is not indented like a card line.)
        assert!(!text.contains("    Todo"), "status leaks into collapsed card");
    }

    #[test]
    fn board_header_links_milestones_and_project() {
        let mut app = board_app(1);
        let _ = frame_text(&mut app);
        let urls: Vec<&str> = app.board_links.iter().map(|(_, u)| u.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://github.com/o/r/milestones",
                "https://github.com/orgs/o/projects/1"
            ],
            "flag then project link: {urls:?}"
        );
        assert!(app.board_links.iter().all(|(r, _)| r.width == 3 && r.height == 1));
    }

    #[test]
    fn board_headers_show_card_counts() {
        let mut app = board_app(2);
        let board = app.board.as_mut().expect("board");
        board.order.push("Done".to_string());
        board.items.push(BoardItem {
            title: "Card 2".to_string(),
            number: Some(102),
            status: Some("Done".to_string()),
            labels: vec![Label { name: "x".to_string() }],
            repo: Some("o/r".to_string()),
            milestone: None,
            content: serde_json::json!({ "type": "Issue" }),
        });
        let text = frame_text(&mut app);
        assert!(text.contains("Todo (2)"), "todo count missing: {text}");
        assert!(text.contains("Done (1)"), "done count missing: {text}");
    }

    #[test]
    fn board_empty_column_shows_empty_state() {
        let mut app = board_app(1);
        app.board.as_mut().expect("board").order.push("Done".to_string());
        let text = frame_text(&mut app);
        assert!(text.contains("Done (0)"), "zero count missing: {text}");
        assert!(text.contains("no cards"), "empty label missing: {text}");
    }

    #[test]
    fn board_filter_narrows_columns_then_esc_restores() {
        use crossterm::event::KeyEvent;
        let mut app = board_app(2);
        let board = app.board.as_mut().expect("board");
        board.order.push("Done".to_string());
        board.items.push(BoardItem {
            title: "Other card".to_string(),
            number: Some(200),
            status: Some("Done".to_string()),
            labels: vec![Label { name: "chore".to_string() }],
            repo: Some("o/r".to_string()),
            milestone: None,
            content: serde_json::json!({ "type": "Issue" }),
        });
        board.items[0].milestone = Some("v0.3".to_string());
        let all = frame_text(&mut app);
        assert!(all.contains("Todo (2)") && all.contains("Done (1)"), "unfiltered counts: {all}");

        let key = |app: &mut App, code| handle_key(app, KeyEvent::new(code, KeyModifiers::empty()));
        key(&mut app, KeyCode::Char('f'));
        let dialog = frame_text(&mut app);
        assert!(dialog.contains("board cards"), "subtitle names the target: {dialog}");
        assert!(dialog.contains("v0.3"), "milestone pill lists the card milestone: {dialog}");
        // Down moves to the milestone pill, space toggles it, Enter applies.
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char(' '));
        key(&mut app, KeyCode::Enter);

        let narrowed = frame_text(&mut app);
        assert!(narrowed.contains("Todo (1)"), "filtered count: {narrowed}");
        assert!(narrowed.contains("Done (0)"), "empty lane count: {narrowed}");
        assert!(narrowed.contains("no matches"), "empty lane label: {narrowed}");
        assert!(narrowed.contains("Card 0") && !narrowed.contains("Card 1"), "hit stays, miss hides: {narrowed}");
        assert!(narrowed.contains("milestones v0.3"), "footer names the filter: {narrowed}");
        // Cursor math reads the narrowed grid too: the hit is selected and
        // the single row cannot scroll past itself.
        assert_eq!(crate::app::board_selected_number(&app), Some(100));
        board_move(&mut app, 0, 1);
        assert_eq!((app.board_col, app.board_row), (0, 0), "cursor clamps to the filtered column");

        key(&mut app, KeyCode::Esc);
        let restored = frame_text(&mut app);
        assert!(restored.contains("Todo (2)") && restored.contains("Done (1)"), "clear restores: {restored}");
        assert!(!restored.contains("no matches"), "empty label clears: {restored}");
        assert!(!restored.contains("milestones"), "footer clears: {restored}");
    }

    #[test]
    fn board_footer_shows_loading_then_dates() {
        let mut app = board_app(1);
        let before = frame_text(&mut app);
        assert!(before.contains("● x"), "{before}");
        assert!(before.contains("…"), "date cell loads in");
        app.board_details.insert(100, BoardDetail {
            created: "2026-11-01".to_string(),
            updated: "2026-11-05".to_string(),
            comments: 2,
        });
        let after = frame_text(&mut app);
        assert!(after.contains("2 comments · 2026-11-05"), "{after}");
    }

    #[test]
    fn board_expanded_card_shows_detail() {
        let mut app = board_app(3);
        app.board_expanded = Some((0, 0));
        let text = frame_text(&mut app);
        assert!(text.contains("Todo · ISSUE"));
        assert!(text.contains("https://github.com/o/r/issues"));
        assert!(text.contains("Why"));
    }

    #[test]
    fn space_toggles_expansion_and_move_clears_it() {
        use crossterm::event::KeyEvent;
        let mut app = board_app(3);
        let space = || KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty());
        handle_key(&mut app, space());
        assert_eq!(app.board_expanded, Some((0, 0)));
        handle_key(&mut app, space());
        assert_eq!(app.board_expanded, None);
        handle_key(&mut app, space());
        board_move(&mut app, 0, 1);
        assert_eq!(app.board_expanded, None);
        assert_eq!((app.board_col, app.board_row), (0, 1));
    }

    #[test]
    fn board_scrolls_selected_into_view() {
        let mut app = board_app(30);
        app.board_row = 29;
        let text = frame_text(&mut app);
        assert!(text.contains("Card 29"), "selected card visible");
        assert!(app.board_states[0].offset() > 0, "list scrolled");
    }

    fn sample(number: u64) -> IssueData {
        IssueData {
            number,
            title: "Fix the thing".to_string(),
            state: "OPEN".to_string(),
            author: Author { login: "bo".to_string() },
            labels: vec![],
            assignees: vec![],
            created_at: "2026-01-02T00:00:00Z".to_string(),
            updated_at: "2026-01-05T00:00:00Z".to_string(),
            url: format!("https://github.com/o/r/issues/{number}"),
            body: "# hello\n\n- a\n- b\n".to_string(),
            comments: vec![],
            review_threads: vec![],
            paired: vec![],
            checks: None,
            milestone: None,
            is_pr: false,
        }
    }

    fn loaded_app() -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut pr = sample(9);
        pr.is_pr = true;
        for d in [sample(7), pr] {
            let n = d.number;
            app.data.insert(n, d);
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.show_issue(7);
        app
    }

    fn key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, KeyModifiers::empty())
    }

    fn click(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn cached_click_still_joins_its_section() {
        // Data can arrive before the sidebar knows the number (warm cache,
        // board prefetch): an explicit open files it under requested.
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut pr = sample(9);
        pr.is_pr = true;
        pr.url = "https://github.com/o/r/pull/9".to_string();
        app.data.insert(9, pr);
        assert!(app.visited.is_empty());
        app.ensure_visited(9);
        app.load_issue(9, false);
        assert_eq!(app.visited, vec![9]);
        assert_eq!(app.current, Some(9));
        // A second click files nothing twice.
        app.ensure_visited(9);
        app.load_issue(9, false);
        assert_eq!(app.visited, vec![9]);
    }

    #[test]
    fn viewing_a_recent_row_does_not_relocate_it() {
        // Regression: cursoring onto a recent row completes its fetch;
        // the row must stay under recent, not move to requested.
        use crate::app::poll_issues;
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let mut d = sample(42);
        d.is_pr = true;
        app.data.insert(42, d.clone());
        app.statuses.insert(42, vec![]);
        app.recent.push(42);
        app.show_issue(42);
        assert_eq!(app.visited, Vec::<u64>::new());
        // Background fetch lands.
        app.issue_tx.send((42, crate::github::IssueEvent::Done(Ok((d, vec![]))))).unwrap();
        poll_issues(&mut app);
        assert_eq!(app.visited, Vec::<u64>::new());
        let (_, _, recent) = app.pane_groups(true, false);
        assert_eq!(recent, vec![42]);
        assert_eq!(app.current, Some(42));
    }

    #[test]
    fn merged_window_label_reads_plain() {
        let mut app = App::new(Some("o/r".to_string()));
        assert_eq!(app.window_label(), "open + merged ≤24h");
        app.merged_window_hours = 1;
        assert_eq!(app.window_label(), "open + merged ≤1h");
        app.merged_window_hours = 168;
        assert_eq!(app.window_label(), "open + merged ≤7d");
    }

    #[test]
    fn jk_stays_in_the_focused_pane() {
        use crate::app::Pane;
        let mut app = loaded_app();
        // A second issue to walk between.
        let mut extra = sample(5);
        extra.body = String::new();
        app.data.insert(5, extra);
        app.statuses.insert(5, vec![]);
        app.visited.push(5);
        app.show_issue(7);
        assert_eq!(app.pane, Pane::Issues);
        // j never crosses into PRs: it walks issues only.
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(5));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(5));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.current, Some(7));
        // The PRs pane walks PRs only, starting from the first PR even
        // when the selection sits on an issue.
        app.pane = Pane::PRs;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.current, Some(9));
    }

    #[test]
    fn tab_cycles_issues_prs_refs() {
        use crate::app::Pane;
        let mut app = loaded_app();
        assert_eq!(app.pane, Pane::Issues);
        // No refs yet: Tab alternates the two sidebar lists.
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::PRs);
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::Issues);
        // With mentions, Refs joins the cycle after PRs; the body keeps
        // its dedicated keys so it never takes Tab focus.
        app.data.get_mut(&7).unwrap().body = "blocked by #9".to_string();
        app.show_issue(7);
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::PRs);
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::Refs);
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::Issues);
        // From the body (mouse), Tab returns to the sidebar.
        app.pane = Pane::Main;
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::Issues);
    }

    /// Foreground color of the first cell spelling `needle` on buffer row
    /// `y`, if the row spells it.
    fn title_fg(buf: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<ratatui::style::Color> {
        let want: Vec<char> = needle.chars().collect();
        let w = buf.area.width;
        for x in 0..w {
            let mut ok = !want.is_empty();
            for (i, ch) in want.iter().enumerate() {
                let xi = x as usize + i;
                if xi >= w as usize || buf[(xi as u16, y)].symbol() != ch.to_string() {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some(buf[(x, y)].fg);
            }
        }
        None
    }

    #[test]
    fn pane_focus_drives_border_title_and_marker() {
        use crate::app::Pane;
        let mut app = loaded_app();
        assert_eq!(app.pane, Pane::Issues);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        // Focused pane: thick double border, amber title, row marker.
        assert_eq!(buf[(40, 0)].symbol(), "═", "active pane has the thick border");
        assert_eq!(title_fg(&buf, 0, "Issues"), Some(crate::theme::amber()));
        let pr_top = app.side_pr_area.y;
        assert_eq!(title_fg(&buf, pr_top, "Pull requests"), Some(crate::theme::muted()));
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("█"), "focused pane marks its row");
        let marker = buf.content().iter().find(|c| c.symbol() == "█").expect("marker cell");
        assert_eq!(marker.bg, crate::theme::code_bg(), "marked row carries the shade");
        // Tab away: marker and color go quiet, border thins, cursor stays.
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::PRs);
        assert_eq!(app.current, Some(7));
        assert_eq!(app.issue_state.selected(), Some(1), "cursor kept while blurred");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf[(40, 0)].symbol(), "─", "inactive pane thins its border");
        assert_eq!(title_fg(&buf, 0, "Issues"), Some(crate::theme::muted()));
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!text.contains("█"), "blurred panes drop the marker");
        // Tab back: everything returns on the same row.
        handle_key(&mut app, key(KeyCode::Tab));
        assert_eq!(app.pane, Pane::Issues);
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        assert_eq!(buf[(40, 0)].symbol(), "═");
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("█"), "marker returns with focus");
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn jk_in_main_scrolls_instead_of_moving_selection() {
        use crate::app::Pane;
        let mut app = loaded_app();
        app.pane = Pane::Main;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.current, Some(7));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.scroll, 0);
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn jk_in_refs_moves_highlight_instead_of_opening() {
        use crate::app::Pane;
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "see #9 then #12".to_string();
        app.show_issue(7);
        app.pane = Pane::Refs;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.ref_highlight, Some(9));
        assert_eq!(app.current, Some(7));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.ref_highlight, Some(12));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.ref_highlight, Some(9));
    }

    #[test]
    fn panes_group_pins_requested_and_recent_with_dividers() {
        let mut app = loaded_app();
        app.pin_order = vec![9];
        app.pinned = [9].into_iter().collect();
        app.recent = vec![9, 7, 42];
        let (ip, iv, ir) = app.pane_groups(false, false);
        assert_eq!((ip, iv, ir), (vec![], vec![7], vec![42]));
        let (pp, pv, pr) = app.pane_groups(true, false);
        assert_eq!((pp, pv, pr), (vec![9], vec![], vec![]));
        use crate::model::SideRow::{Divider, Item};
        assert_eq!(app.issue_rows(), vec![Item(7), Divider("recent"), Item(42)]);
        assert_eq!(app.pr_rows(), vec![Item(9)]);
        let text = frame_text(&mut app);
        assert!(text.contains("Issues"));
        assert!(text.contains("Pull requests"));
    }

    #[test]
    fn pane_titles_carry_count_and_sort_with_group_footers() {
        let mut app = loaded_app();
        // loaded_app files one issue and one PR under requested.
        assert_eq!(app.side_subtitle(false), "1 · default");
        assert_eq!(app.side_subtitle(true), "1 · default");
        assert_eq!(app.side_footer(false), "pinned (0) · requested (1) · recent (0)");
        assert_eq!(
            app.side_footer(true),
            "pinned (0) · requested (1) · recent (0) · open + merged ≤24h"
        );
        let text = frame_text(&mut app);
        assert!(text.contains("1 · default"), "count plus sort in subtitle: {text}");
        assert!(
            text.contains("pinned (0) · requested (1) · recent (0)"),
            "group counts in footer: {text}"
        );
        // Sort state rides the subtitle; reverse flips the arrow.
        app.sort = crate::core::SideSort::Updated;
        app.sort_desc = true;
        let text = frame_text(&mut app);
        assert!(text.contains("1 · updated ↓"), "sort indicator: {text}");
        app.toggle_sort_dir();
        let text = frame_text(&mut app);
        assert!(text.contains("1 · updated ↑"), "reverse indicator: {text}");
        // An active search query moves to the PR footer, not the title.
        app.query_active = "is:open".to_string();
        assert!(app.side_footer(true).contains("\"is:open\""), "query in footer");
        assert!(!app.side_subtitle(true).contains("is:open"), "query out of title");
    }

    #[test]
    fn refs_pane_shares_title_and_footer_shape() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "see #9".to_string();
        app.show_issue(7);
        assert!(app.refs_visible());
        assert_eq!(app.refs_subtitle(), "1 mentioned");
        assert_eq!(app.refs_footer(), "mentioned (1) · paired (0)");
        let text = frame_text(&mut app);
        assert!(text.contains("1 mentioned"), "refs subtitle: {text}");
        assert!(text.contains("mentioned (1) · paired (0)"), "refs footer: {text}");
    }

    #[test]
    fn board_title_carries_card_count() {
        let mut app = board_app(2);
        assert!(frame_text(&mut app).contains("2 cards"), "board subtitle");
    }

    #[test]
    fn details_header_shows_display_only_paired_line() {
        let mut app = loaded_app();
        let text = frame_text(&mut app);
        assert!(!text.contains("Paired"), "no line without pairs: {text}");
        app.data.get_mut(&7).unwrap().paired =
            vec![PairedRef { number: 9, state: "MERGED".to_string() }];
        app.show_issue(7);
        let text = frame_text(&mut app);
        assert!(text.contains("Paired"), "{text}");
        assert!(text.contains("#9 MERGED"), "{text}");
        // Display-only: the line renders with the header but never takes
        // part in header click resolution.
        assert!(!app.header_rows.iter().any(|l| l.contains("Paired")), "{:?}", app.header_rows);
    }

    #[test]
    fn refs_pane_lists_paired_rows_and_enter_opens() {
        use crate::app::Pane;
        use crate::model::SideRow::{Divider, Item};
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.body = String::new();
        d.paired = vec![PairedRef { number: 9, state: "MERGED".to_string() }];
        app.show_issue(7);
        assert!(app.refs_visible());
        assert_eq!(app.refs_rows(), vec![Divider("Paired"), Item(9)]);
        let text = frame_text(&mut app);
        assert!(text.contains("Paired"), "{text}");
        app.pane = Pane::Refs;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.ref_highlight, Some(9));
        assert_eq!(app.current, Some(7), "j only moves the highlight");
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.current, Some(9), "Enter opens the paired counterpart");
    }

    #[test]
    fn paired_refs_row_click_opens_instead_of_highlighting() {
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.body = String::new();
        d.paired = vec![PairedRef { number: 9, state: "MERGED".to_string() }];
        app.show_issue(7);
        let _ = frame_text(&mut app); // records hit areas
        // Divider sits on the first list row; the paired item below it.
        let (x, y) = (app.refs_area.x + 2, app.refs_area.y + 3);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(9));
        assert_eq!(app.ref_highlight, None);
    }

    #[test]
    fn refs_pane_lists_mentioned_rows_and_toggles() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "blocked by #9, see details".to_string();
        app.show_issue(7);
        assert_eq!(app.ref_numbers, vec![9]);
        assert!(app.refs_visible());
        let text = frame_text(&mut app);
        assert!(text.contains("Refs"), "{text}");
        handle_key(&mut app, key(KeyCode::Char('e')));
        assert!(!app.show_refs);
        assert!(!app.refs_visible());
        let hidden = frame_text(&mut app);
        assert!(!hidden.contains("Refs"), "{hidden}");
        // Recompute on arrival: a fresh body fetch repopulates the pane.
        handle_key(&mut app, key(KeyCode::Char('e')));
        let mut d = sample(7);
        d.body = "see #9".to_string();
        app.issue_tx.send((7, crate::github::IssueEvent::Done(Ok((d, vec![]))))).unwrap();
        crate::app::poll_issues(&mut app);
        assert_eq!(app.ref_numbers, vec![9]);
    }

    #[test]
    fn mouse_click_on_refs_row_highlights_source_instead_of_opening() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "intro\n\nblocked by #9, see details".to_string();
        app.show_issue(7);
        let _ = frame_text(&mut app); // records hit areas
        // +2 skips the border and the breathing-room spacer row.
        let (x, y) = (app.refs_area.x + 2, app.refs_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7), "stays on the issue");
        assert_eq!(app.ref_highlight, Some(9));
        // Scrolled so the mentioning row sits at the top.
        let row = app.body_rows.iter().position(|l| l.contains("#9")).unwrap();
        assert_eq!(app.scroll, row as u16);
        // Clicking the same row again clears the highlight.
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.ref_highlight, None);
        // Switching issues clears it too.
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.ref_highlight, Some(9));
        // The mentioning line renders highlighted at the top of the pane.
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let cell = &buf[(app.body_area.x + 1, app.body_area.y)];
        assert_eq!(cell.bg, crate::theme::amber(), "source line highlighted");
        app.show_issue(9);
        assert_eq!(app.ref_highlight, None);
    }

    #[test]
    fn overflowing_pane_keeps_selected_row_visible() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for n in 1..=30 {
            let d = sample(n);
            app.data.insert(n, d);
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.show_issue(30);
        let _ = frame_text(&mut app);
        assert!(app.issue_state.offset() > 0, "pane scrolled");
    }

    #[test]
    fn wheel_over_sidebar_hovers_without_visiting() {
        let mut app = loaded_app();
        // A second issue row so the wheel has somewhere to go.
        let d = sample(8);
        app.data.insert(8, d);
        app.statuses.insert(8, vec![]);
        app.visited.push(8);
        let _ = frame_text(&mut app); // records hit areas
        let (x, y) = (app.side_issue_area.x + 2, app.side_issue_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::ScrollDown, x, y));
        assert_eq!(app.hover, Some(8));
        assert_eq!(app.current, Some(7), "wheel never visits");
        let _ = frame_text(&mut app); // selection stays on the active issue
        assert_eq!(app.issue_state.selected(), Some(1), "hover never outranks current");
        assert_eq!(app.current, Some(7));
        assert_eq!(app.scroll, 0, "body scroll untouched");
    }

    #[test]
    fn slash_search_filters_and_enters_first_hit() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut app = loaded_app();
        app.data.get_mut(&9).unwrap().title = "Zebra migration".to_string();
        handle_key(&mut app, key(KeyCode::Char('/')));
        assert!(app.searching);
        for c in "zebra".chars() {
            handle_key(&mut app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty()));
        }
        assert!(app.issue_rows().is_empty());
        assert_eq!(app.pr_rows(), vec![crate::model::SideRow::Item(9)]);
        let text = frame_text(&mut app);
        assert!(text.contains("zebra"));
        handle_key(&mut app, key(KeyCode::Enter));
        assert!(!app.searching);
        assert_eq!(app.current, Some(9));
        // Highlight persists after commit.
        assert_eq!(app.search, "zebra");
    }

    #[test]
    fn search_n_jumps_between_hits() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().title = "Fix login".to_string();
        app.data.get_mut(&9).unwrap().title = "Fix logout".to_string();
        app.search = "fix".to_string();
        assert_eq!(app.search_hits(), vec![7, 9]);
        handle_key(&mut app, key(KeyCode::Char('n')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('N')));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn filter_dialog_toggles_hide_recent_rows() {
        let mut app = loaded_app();
        let mut merged = sample(20);
        merged.is_pr = true;
        merged.state = "MERGED".to_string();
        app.data.insert(20, merged);
        app.statuses.insert(20, vec![]);
        app.recent.push(20);
        use crate::model::SideRow::{Divider, Item};
        assert_eq!(app.pr_rows(), vec![Item(9), Divider("recent"), Item(20)]);
        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(app.filter_open);
        let text = frame_text(&mut app);
        assert!(text.contains("Filter"));
        assert!(text.contains("query"));
        handle_key(&mut app, key(KeyCode::Char('m')));
        assert!(!app.show_merged_prs);
        assert_eq!(app.pr_rows(), vec![Item(9)]);
        // Requested rows ignore the toggle.
        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(!app.filter_open);
    }

    #[test]
    fn filter_query_draft_commits_and_clears() {
        use crate::app::QueryAction;
        let mut app = loaded_app();
        app.filter_open = true;
        app.start_query_edit();
        assert_eq!(app.query_draft, "");
        app.query_draft = "label:bug".to_string();
        assert_eq!(app.commit_query_draft(), QueryAction::Run("label:bug".to_string()));
        assert_eq!(app.query_active, "label:bug");
        assert!(!app.filter_open);
        app.filter_open = true;
        app.start_query_edit();
        assert_eq!(app.query_draft, "label:bug");
        // Esc in edit mode reverts the draft but keeps the dialog.
        app.query_draft = "other".to_string();
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(app.filter_open);
        assert!(!app.query_edit);
        assert_eq!(app.query_draft, "label:bug");
        assert_eq!(app.query_active, "label:bug");
        // Empty commit clears back to the window listing.
        app.start_query_edit();
        app.query_draft = String::new();
        assert_eq!(app.commit_query_draft(), QueryAction::Refetch);
        assert!(app.query_active.is_empty());
    }

    #[test]
    fn arrows_scroll_instead() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.scroll, 3);
        assert_eq!(app.current, Some(7));
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn mouse_click_selects_sidebar_row() {
        let mut app = loaded_app();
        app.show_issue(9);
        let _ = frame_text(&mut app); // records hit areas
        // +2 skips the border and the breathing-room spacer row.
        let (x, y) = (app.side_issue_area.x + 2, app.side_issue_area.y + 2);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn space_expands_sidebar_row_detail_and_collapse_restores_it() {
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.title = "Fix the thing that keeps breaking tailmarker".to_string();
        d.labels = ["bug", "ui", "docs", "chore", "ci-pipeline"]
            .iter()
            .map(|name| Label { name: name.to_string() })
            .collect();
        app.show_issue(7);
        let collapsed = sidebar_text(&mut app);
        assert!(collapsed.contains("#7 Fix the thing"), "{collapsed}");
        assert!(!collapsed.contains("tailmarker"), "collapsed title is a slug");
        assert!(!collapsed.contains("ci-pipeline"), "summary label chips overflow");

        handle_key(&mut app, key(KeyCode::Char(' ')));
        let expanded = sidebar_text(&mut app);
        assert!(expanded.contains("tailmarker"), "full title wraps in: {expanded}");
        assert!(expanded.contains("● bug"), "every label: {expanded}");
        assert!(expanded.contains("ci-pipeline"), "every label: {expanded}");
        assert!(expanded.contains("by bo · OPEN"), "author and state: {expanded}");
        assert!(expanded.contains("created 2026-01-02"), "created date: {expanded}");
        assert!(expanded.contains("updated 2026-01-05"), "updated date: {expanded}");
        // Only the selected row's number expands.
        assert_eq!(expanded.matches("created 2026-01-02").count(), 1);

        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert_eq!(sidebar_text(&mut app), collapsed, "collapse restores the row");
    }

    #[test]
    fn sidebar_rows_show_labels_updated_and_status_and_sort_hint() {
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.labels = vec![Label { name: "bug".to_string() }];
        d.updated_at = "2020-01-05T00:00:00Z".to_string();
        app.statuses.insert(7, vec![("Roadmap".to_string(), Some("Todo".to_string()))]);
        let mut newer = sample(5);
        newer.updated_at = "2020-02-05T00:00:00Z".to_string();
        app.data.insert(5, newer);
        app.statuses.insert(5, vec![]);
        app.visited.push(5);
        app.show_issue(7);

        let text = sidebar_text(&mut app);
        assert!(text.contains("● bug · 01/05/20"), "labels and date: {text}");
        assert!(text.contains("01/05/20 · \u{f10c} Todo"), "date and status: {text}");

        // One key cycles the sort; the `/` hint names key and direction.
        handle_key(&mut app, key(KeyCode::Char('a')));
        assert_eq!(app.sort, crate::core::SideSort::Updated);
        let sorted = sidebar_text(&mut app);
        assert!(
            sorted.find("#5").unwrap_or(usize::MAX) < sorted.find("#7").unwrap_or(usize::MAX),
            "newest row first: {sorted}"
        );
        handle_key(&mut app, key(KeyCode::Char('/')));
        let text = frame_text(&mut app);
        assert!(text.contains("sort updated ↓"), "sort hint: {text}");
    }

    /// Text of the PRs sidebar only, mirroring `sidebar_text`.
    fn pr_sidebar_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        let area = app.side_pr_area;
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    fn check_pr(number: u64, checks: Option<crate::model::CheckState>) -> IssueData {
        let mut d = sample(number);
        d.is_pr = true;
        d.checks = checks;
        d
    }

    fn check_app(number: u64, checks: Option<crate::model::CheckState>) -> App {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        app.data.insert(number, check_pr(number, checks));
        app.statuses.insert(number, vec![]);
        app.visited.push(number);
        app.show_issue(number);
        app
    }

    #[test]
    fn pr_rows_show_check_rollup_glyph_per_state() {
        use crate::model::CheckState;
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        for (n, checks) in [
            (21, Some(CheckState::Pass)),
            (22, Some(CheckState::Fail)),
            (23, Some(CheckState::Pending)),
        ] {
            app.data.insert(n, check_pr(n, checks));
            app.statuses.insert(n, vec![]);
            app.visited.push(n);
        }
        app.show_issue(21);
        let text = pr_sidebar_text(&mut app);
        assert!(text.contains("\u{f00c}"), "pass glyph: {text}");
        assert!(text.contains("\u{f00d}"), "fail glyph: {text}");
        assert!(text.contains("\u{f017}"), "pending glyph, never pass: {text}");
    }

    #[test]
    fn pr_row_without_checks_shows_no_glyph() {
        use crate::model::CheckState;
        // No rollup, or an unknown future state, renders nothing.
        for checks in [None, Some(CheckState::Unknown)] {
            let mut app = check_app(21, checks);
            let text = pr_sidebar_text(&mut app);
            assert!(
                !text.contains("\u{f00c}") && !text.contains("\u{f00d}") && !text.contains("\u{f017}"),
                "{checks:?} renders no glyph: {text}"
            );
        }
    }

    /// Masthead rows only (brand line plus the issue line), so masthead
    /// assertions cannot pass off sidebar rows as the title source.
    fn masthead_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        let buf = terminal.backend().buffer();
        (0..2)
            .map(|y| (44..120).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn pr_masthead_shows_check_rollup_glyph_per_state() {
        use crate::model::CheckState;
        for (checks, glyph) in [
            (Some(CheckState::Pass), "\u{f00c}"),
            (Some(CheckState::Fail), "\u{f00d}"),
            (Some(CheckState::Pending), "\u{f017}"),
        ] {
            // The masthead title carries the glyph now that the header
            // block starts at the meta lines …
            let mut app = check_app(9, checks);
            let mast = masthead_text(&mut app);
            assert!(mast.contains(glyph), "{checks:?} glyph in masthead: {mast}");
            assert!(mast.contains("#9"), "masthead keeps the number: {mast}");
            // … and the header block itself stays clean.
            let d = check_pr(9, checks);
            let header: String = super::header_lines(&d, &[])
                .iter()
                .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
                .collect::<Vec<_>>()
                .join("");
            assert!(
                !header.contains("\u{f00c}") && !header.contains("\u{f00d}") && !header.contains("\u{f017}"),
                "{checks:?} header stays clean: {header}"
            );
            // … and the full frame renders it.
            assert!(frame_text(&mut app).contains(glyph), "{checks:?} glyph in frame");
        }
        // No rollup, an unknown state, or a non-PR renders no masthead glyph.
        for checks in [None, Some(CheckState::Unknown)] {
            let mut app = check_app(9, checks);
            let mast = masthead_text(&mut app);
            assert!(
                !mast.contains("\u{f00c}") && !mast.contains("\u{f00d}") && !mast.contains("\u{f017}"),
                "{checks:?} masthead stays clean: {mast}"
            );
        }
        let mut issue = sample(7);
        issue.checks = Some(CheckState::Pass);
        let header: String = super::header_lines(&issue, &[])
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join("");
        assert!(
            !header.contains("\u{f00c}"),
            "issues never render the rollup: {header}"
        );
    }

    #[test]
    fn mouse_click_switches_tabs() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 12, y),
        );
        assert_eq!(app.tab, Tab::Visual);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 1, y),
        );
        assert_eq!(app.tab, Tab::Details);
    }

    #[test]
    fn mouse_click_on_ref_opens_issue() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "blocked by #9, see details".to_string();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("#9"))
            .expect("ref visible");
        let (x, y) = (app.body_area.x + 2, app.body_area.y + row as u16);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(9));
    }

    #[test]
    fn mouse_click_opens_nearest_ref_on_row() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "fix #7 then #9 ok".to_string();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("#7 then #9"))
            .expect("refs visible");
        let line = app.body_rows[row].clone();
        let (bax, bay) = (app.body_area.x, app.body_area.y);
        let x9 = bax + line.find("#9").unwrap() as u16;
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x9, bay + row as u16));
        assert_eq!(app.current, Some(9));
        let x7 = bax + line.find("#7").unwrap() as u16;
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x7, bay + row as u16));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn mouse_click_on_url_opens_browser_not_issue() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "see https://example.com/a ok".to_string();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("example.com"))
            .expect("url visible");
        let (x, y) = (app.body_area.x + 2, app.body_area.y + row as u16);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7));
        assert!(app.message.contains("opened https://example.com/a"));
    }

    #[test]
    fn mouse_click_on_plain_text_keeps_issue() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let row = app
            .body_rows
            .iter()
            .position(|l| l.contains("hello"))
            .expect("body visible");
        let (x, y) = (app.body_area.x + 2, app.body_area.y + row as u16);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), x, y));
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn mouse_wheel_scrolls() {
        let mut app = loaded_app();
        handle_mouse(&mut app, click(MouseEventKind::ScrollDown, 60, 20));
        assert_eq!(app.scroll, 3);
        handle_mouse(&mut app, click(MouseEventKind::ScrollUp, 60, 20));
        assert_eq!(app.scroll, 0);
    }

    fn frame_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// Text of the Issues sidebar only, so main-pane text cannot satisfy
    /// or break row-render assertions.
    fn sidebar_text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, app)).expect("draw");
        let area = app.side_issue_area;
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn desk_renders_issue() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let d = sample(7);
        app.data.insert(7, d);
        app.statuses.insert(7, vec![]);
        app.visited.push(7);
        app.show_issue(7);
        let text = frame_text(&mut app);
        assert!(text.contains("ghit"), "masthead wordmark");
        assert!(text.contains("#7"), "issue number");
        assert!(text.contains("Body"), "tab label");
        assert!(text.contains("hello"), "markdown body");
    }

    #[test]
    fn masthead_splits_brand_and_issue_lines() {
        let mut app = loaded_app();
        let mast = masthead_text(&mut app);
        let mut lines = mast.lines();
        let brand = lines.next().unwrap_or_default();
        let issue = lines.next().unwrap_or_default();
        assert!(brand.contains("ghit"), "brand line keeps the wordmark: {brand}");
        assert!(!brand.contains("#7"), "number leaves the brand line: {brand}");
        assert!(issue.contains("#7"), "issue line keeps the number: {issue}");
    }

    #[test]
    fn main_pane_keeps_a_single_title_line_with_footer_brand() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().title = "Quasar title nine".to_string();
        let text = frame_text(&mut app);
        // Masthead plus the one sidebar row; the header block starts at
        // the meta lines and no longer repeats the title.
        assert_eq!(text.matches("Quasar title nine").count(), 2, "single main-pane title: {text}");
        assert!(
            !app.header_rows.iter().any(|l| l.contains("Quasar title nine")),
            "header starts at meta: {:?}",
            app.header_rows
        );
        let brand = format!("ghit v{}", env!("CARGO_PKG_VERSION"));
        assert!(text.contains(&brand), "footer brand: {text}");
        assert!(text.contains("? for shortcuts"), "hint survives: {text}");
    }

    #[test]
    fn background_section_rows_render_without_changing_selection() {
        let mut app = loaded_app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        let mut issue = sample(42);
        issue.is_pr = true;
        issue.title = "background pull request".to_string();
        tx.send(crate::github::SectionEvent::Done(Ok(vec![crate::api::Row { issue, statuses: vec![], detail: crate::model::BoardDetail::default(), complete: false, window_at: None }]))).unwrap();
        poll_sections(&mut app);
        assert_eq!(app.current, Some(7));
        assert!(app.partial.contains(&42));
        assert!(frame_text(&mut app).contains("background pull request"));
    }

    #[test]
    fn desk_renders_empty() {
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        let text = frame_text(&mut app);
        assert!(text.contains("ghit"));
        assert!(text.contains("No issue loaded"));
    }

    fn diff_pr(number: u64) -> IssueData {
        let mut d = sample(number);
        d.is_pr = true;
        d
    }

    #[test]
    fn compose_modal_shows_thread_target_and_preview() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('c')));
        assert!(app.compose_open());
        for ch in "# hi".chars() {
            handle_key(&mut app, key(KeyCode::Char(ch)));
        }
        let text = frame_text(&mut app);
        assert!(text.contains("Compose"), "{text}");
        assert!(text.contains("thread #7"), "{text}");
        assert!(text.contains("hi"), "draft plus preview: {text}");
        assert!(text.contains("preview"), "{text}");
        assert!(text.contains("ctrl-s send · esc discard"), "{text}");
    }

    #[test]
    fn auth_modal_names_scopes_and_command() {
        let mut app = loaded_app();
        app.auth_prompt = Some(crate::app::AuthPrompt {
            missing: vec!["project".to_string()],
            cause: "token lacks scopes ghit needs".to_string(),
            scopes_known: true,
        });
        let text = frame_text(&mut app);
        assert!(text.contains("GitHub auth"), "{text}");
        assert!(text.contains("missing scopes:"), "{text}");
        assert!(text.contains("project"), "{text}");
        assert!(text.contains("gh auth refresh -s repo -s read:org -s project"), "{text}");
        assert!(text.contains("y re-auth now"), "{text}");
    }

    // No second frame test for the dead-token state: same modal,
    // same layout, and the live drive covers it.

    #[test]
    fn compose_on_diff_line_targets_file_and_line() {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.expanded_files.insert((9, "a.rs".into()));
        app.set_tab(Tab::Visual);
        let _ = frame_text(&mut app);
        // Header, hunk, deletion, then the addition the comment lands on.
        for _ in 0..3 {
            handle_key(&mut app, key(KeyCode::Down));
        }
        handle_key(&mut app, key(KeyCode::Char('c')));
        assert_eq!(
            app.compose.as_ref().map(|c| c.target.clone()),
            Some(crate::app::ComposeTarget::Line { path: "a.rs".to_string(), line: 1 }),
        );
        let text = frame_text(&mut app);
        assert!(text.contains("a.rs:1"), "{text}");
    }

    #[test]
    fn compose_on_thread_block_targets_the_reply() {
        use crate::model::{DiffFile, ReviewThread, ReviewComment};
        let mut app = loaded_app();
        let mut pr = diff_pr(9);
        pr.review_threads = vec![
            ReviewThread { id: "t1".into(), path: "a.rs".into(), line: Some(1), comments: vec![ReviewComment { id: "c".into(), body: "inline note".into(), ..Default::default() }], ..Default::default() },
        ];
        app.data.insert(9, pr);
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.expanded_files.insert((9, "a.rs".into()));
        app.set_tab(Tab::Visual);
        let _ = frame_text(&mut app);
        // Header, hunk, deletion, addition, then the thread header the reply lands in.
        for _ in 0..4 {
            handle_key(&mut app, key(KeyCode::Down));
        }
        handle_key(&mut app, key(KeyCode::Char('c')));
        assert_eq!(
            app.compose.as_ref().map(|c| c.target.clone()),
            Some(crate::app::ComposeTarget::Reply { thread_id: "t1".into() }),
        );
        let text = frame_text(&mut app);
        assert!(text.contains("Reply · #9"), "{text}");
    }

    #[test]
    fn compose_modal_shows_reply_target_and_discards() {
        let mut app = loaded_app();
        app.compose = Some(crate::app::ComposeState {
            number: 7,
            target: crate::app::ComposeTarget::Reply { thread_id: "t".into() },
            body: "reply".into(),
            cursor: 5,
            sending: false,
        });
        let text = frame_text(&mut app);
        assert!(text.contains("Reply"), "{text}");
        assert!(text.contains("reply"), "{text}");
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(!app.compose_open());
    }

    #[test]
    fn details_contains_body_before_comments() {
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.body = "body evidence".into();
        d.comments.push(crate::model::Comment { body: "comment evidence".into(), ..Default::default() });
        let text = frame_text(&mut app);
        assert!(text.find("Body").unwrap() < text.find("body evidence").unwrap());
        assert!(text.find("body evidence").unwrap() < text.find("Comments").unwrap());
        assert!(text.find("Comments").unwrap() < text.find("comment evidence").unwrap());
    }

    #[test]
    fn review_threads_render_at_matching_lines_in_both_modes() {
        use crate::model::{DiffFile, ReviewThread, ReviewComment};
        let mut app = loaded_app();
        let mut pr = diff_pr(9);
        pr.review_threads = vec![
            ReviewThread { id: "a".into(), path: "a.rs".into(), line: Some(1), comments: vec![ReviewComment { id: "c".into(), body: "inline note".into(), ..Default::default() }], ..Default::default() },
            ReviewThread { id: "b".into(), path: "a.rs".into(), line: Some(8), ..Default::default() },
        ];
        app.data.insert(9, pr); app.show_issue(9);
        app.diffs.insert(9, vec![DiffFile { filename: "a.rs".into(), patch: "@@ -1 +1 @@\n-old\n+new".into(), ..Default::default() }]);
        app.expanded_files.insert((9, "a.rs".into())); app.set_tab(Tab::Visual);
        for split in [false, true] {
            app.diff_split = split;
            let text = frame_text(&mut app);
            assert!(text.find("line 8").unwrap() < text.find("+new").unwrap(), "{text}");
            assert!(text.find("+new").unwrap() < text.find("line 1").unwrap(), "{text}");
            assert!(text.contains('\u{f075}') && text.contains("inline note"), "{text}");
        }
    }

    #[test]
    fn diff_groups_review_threads_by_file() {
        use crate::model::{DiffFile, parse_review_threads};
        let mut app = loaded_app();
        let mut pr = diff_pr(9);
        pr.review_threads = parse_review_threads(&serde_json::json!({"reviewThreads":{"nodes":[
            {"id":"t1","path":"src/a.rs","line":3,"isResolved":true,"comments":{"nodes":[{"id":"c1","author":{"login":"ada"},"body":"resolved body"}]}},
            {"id":"t2","path":"src/a.rs","line":7,"isResolved":false,"comments":{"nodes":[{"id":"c2","author":{"login":"bob"},"body":"**open body**"}]}},
            {"id":"t3","path":"src/b.rs","line":null,"isResolved":false,"comments":{"nodes":[{"id":"c3","author":{"login":"cam"},"body":"other body"}]}}
        ]}}));
        app.data.insert(9, pr);
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), ..Default::default() },
            DiffFile { filename: "src/b.rs".into(), ..Default::default() },
        ]);
        app.set_tab(Tab::Visual);
        let text = frame_text(&mut app);
        let a = text.find("▸ ").unwrap();
        let open = text.find("line 7 · unresolved").unwrap();
        let resolved = text.find("line 3 · resolved").unwrap();
        let b = text.find("src/b.rs").unwrap();
        assert!(a < open && open < resolved && resolved < b, "{text}");
        assert!(text.contains("bob") && text.contains("open body") && text.contains("ada") && text.contains("resolved body"), "{text}");
        assert!(text.contains("line unavailable · unresolved") && text.contains("other body"), "{text}");
    }

    #[test]
    fn diff_tab_folds_files_by_default() {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
            DiffFile { filename: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: String::new() },
        ]);
        app.set_tab(Tab::Visual);
        let text = frame_text(&mut app);
        assert!(text.contains("Diff View"), "{text}");
        assert!(!text.contains("Graph"), "{text}");
        assert!(text.contains("2 files"), "{text}");
        assert!(text.contains("src/a.rs"), "{text}");
        assert!(text.contains("▸"), "folded by default");
        assert!(text.contains("2 changed"), "{text}");
        assert!(text.contains("──"), "ruled sections");
        assert!(text.contains("❯ src/a.rs"), "summary names the cursor file");
        assert!(text.contains('\u{f15c}'), "file icon");
        assert!(text.contains('\u{f040}'), "modified icon");
        assert!(!text.contains("-old"), "hunks hidden while folded");
        assert!(!text.contains("binary or large file"), "note hidden while folded");
        // Space unfolds the file under the active line.
        handle_key(&mut app, key(KeyCode::Char(' ')));
        assert!(app.expanded_files.contains(&(9, "src/a.rs".to_string())));
        let open = frame_text(&mut app);
        assert!(open.contains("▾"), "{open}");
        assert!(open.contains("@@ -1"), "{open}");
        assert!(open.contains("1 │ -old"), "old-side gutter on deletions: {open}");
        assert!(open.contains("1 │ +new"), "new-side gutter on additions: {open}");
        assert!(open.contains("│ @@ -1 +1 @@"), "blank gutter on hunk headers: {open}");
        // Down walks line by line: header, hunk, deletion, addition, then
        // the second file's blank, rule, and header rows.
        for _ in 0..6 {
            handle_key(&mut app, key(KeyCode::Down));
        }
        assert_eq!(app.diff_cursor, 0, "still resolving file 0 until render");
        let _ = frame_text(&mut app);
        assert_eq!(app.diff_cursor, 1, "landed on the second header");
        assert!(app.expanded_files.contains(&(9, "img.png".to_string())), "landed-on collapsed file opens");
        // Unfolding the second file shows its binary note.
        let both = frame_text(&mut app);
        assert!(both.contains("binary or large file"), "{both}");
        // Up walks back to the first header; clicking it folds it back.
        for _ in 0..6 {
            handle_key(&mut app, key(KeyCode::Up));
        }
        let row = app.body_rows.iter().position(|l| l.starts_with("▾ ")).unwrap();
        let (bx, by) = (app.body_area.x, app.body_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), bx + 1, by + row as u16));
        assert!(!app.expanded_files.contains(&(9, "src/a.rs".to_string())));
        assert_eq!(app.diff_cursor, 0);
    }

    #[test]
    fn arrows_walk_diff_lines() {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
            DiffFile { filename: "src/b.rs".into(), status: "modified".into(), additions: 2, deletions: 0, patch: "@@ -1 +1 @@\n+new".into() },
        ]);
        app.set_tab(Tab::Visual);
        let _ = frame_text(&mut app);
        // The cursor anchors to the first header without expanding it.
        let h0 = app.diff_headers[0];
        assert_eq!(app.diff_line, Some(h0));
        assert!(app.expanded_files.is_empty());
        // Down steps one row at a time and never touches the selection.
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.diff_line, Some(h0 + 1));
        assert_eq!(app.current, Some(9), "selection untouched");
        // Walking onto the second header and rendering resolves the file
        // and unfolds it: two blank/rule rows then the header.
        handle_key(&mut app, key(KeyCode::Down));
        handle_key(&mut app, key(KeyCode::Down));
        let _ = frame_text(&mut app);
        assert_eq!(app.diff_cursor, 1);
        assert!(app.expanded_files.contains(&(9, "src/b.rs".to_string())));
        // Up steps back a single row.
        let back = app.diff_line.unwrap();
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.diff_line, Some(back - 1));
        // j/k stays with the side panes: in Issues it walks issues only.
        app.show_issue(7);
        app.pane = crate::app::Pane::Issues;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.current, Some(7));
    }

    #[test]
    fn diff_active_line_highlights_and_yanks() {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.set_tab(Tab::Visual);
        let _ = frame_text(&mut app);
        // Unfold, then step onto the deletion line.
        handle_key(&mut app, key(KeyCode::Char(' ')));
        let _ = frame_text(&mut app);
        let h0 = app.diff_headers[0];
        handle_key(&mut app, key(KeyCode::Down));
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.diff_line, Some(h0 + 2));
        let _ = frame_text(&mut app);
        // The active line yanks with its file and PR identity.
        assert_eq!(
            app.yank_text().unwrap(),
            "#9 Fix the thing — src/a.rs: \"-old\" — https://github.com/o/r/issues/9"
        );
        // The active row paints amber across the body width.
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let vis = h0 + 2;
        let cell = &buf[(app.body_area.x + 1, app.body_area.y + vis as u16)];
        assert_eq!(cell.bg, crate::theme::amber(), "active line highlighted");
    }

    #[test]
    fn mouse_click_highlights_the_row_it_expands() {
        use crate::app::Pane;
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.set_tab(Tab::Visual);
        let _ = frame_text(&mut app);
        let row = app.body_rows.iter().position(|l| l.starts_with("▸ ")).unwrap();
        let (bx, by) = (app.body_area.x, app.body_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), bx + 1, by + row as u16));
        assert_eq!(app.pane, Pane::Main, "click takes body focus");
        assert_eq!(app.diff_cursor, 0);
        // The click parks the line; render resolves the file and expands it.
        let _ = frame_text(&mut app);
        assert!(app.expanded_files.contains(&(9, "src/a.rs".to_string())));
        // The expanded header renders with the accent background.
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let buf = terminal.backend().buffer().clone();
        let hrow = app.diff_headers[0];
        let cell = &buf[(app.body_area.x + 1, app.body_area.y + hrow as u16)];
        assert_eq!(cell.bg, crate::theme::amber(), "expanded row highlighted");
    }

    #[test]
    fn content_tabs_show_nerd_icons() {
        let mut app = loaded_app();
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f15c}'), "{text}");
        assert!(text.contains('\u{f121}'), "{text}");
        assert!(text.contains("Graph"), "issue mode names the Graph tab: {text}");
        assert!(!text.contains("Diff View"), "{text}");
    }

    #[test]
    fn content_tab_names_diff_view_for_prs() {
        // loaded_app files issue 7 and PR 9; parking on the PR renames it.
        let mut app = loaded_app();
        app.show_issue(9);
        let text = frame_text(&mut app);
        assert!(text.contains("Diff View"), "PR mode names the Diff View tab: {text}");
        assert!(!text.contains("Graph"), "{text}");
    }

    #[test]
    fn tab_click_zones_match_wider_labels() {
        // Glyph plus two spaces plus label: Body spans rel 0..7, Comments
        // rel 9..20, Diff rel 22..29, Graph rel 31..39. Dividers map to
        // the tab on their left. Clicks on the boundaries prove it.
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 8, y));
        assert_eq!(app.tab, Tab::Details);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 12, y));
        assert_eq!(app.tab, Tab::Visual);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 22, y));
        assert_eq!(app.tab, Tab::Visual);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 30, y));
        assert_eq!(app.tab, Tab::Visual);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 31, y));
        assert_eq!(app.tab, Tab::Visual);
    }

    #[test]
    fn section_headers_show_nerd_icons() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "blocked by #9".to_string();
        app.show_issue(7);
        let text = frame_text(&mut app);
        // Font Awesome: fa-exclamation-circle, fa-code-fork, fa-link.
        assert!(text.contains('\u{f06a}'), "issues: {text}");
        assert!(text.contains('\u{f126}'), "pull requests: {text}");
        assert!(text.contains('\u{f0c1}'), "refs: {text}");
        assert!(text.contains("Issues"));
        assert!(text.contains("Pull requests"));
        assert!(text.contains("Refs"));
    }

    #[test]
    fn masthead_wordmark_shows_git_icon() {
        let mut app = loaded_app();
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f1d3}'), "{text}");
        assert!(text.contains("ghit"));
        assert!(text.contains("#7"));
    }

    #[test]
    fn issue_header_renders_above_tabs_not_in_body() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        assert!(!app.header_rows.is_empty());
        // The header block starts at the meta lines: state and author
        // render there, while the `#N title` lives only in the masthead.
        assert!(app.header_rows.iter().any(|l| l.contains("OPEN")));
        assert!(!app.header_rows.iter().any(|l| l.contains("#7")));
        assert!(app.header_area.y + app.header_area.height <= app.tabs_area.y);
        // The meta/label block left the scrollable body: meta text like
        // the state and assignees no longer appear in body rows.
        assert!(!app.body_rows.iter().any(|l| l.contains("assignees:")), "{:?}", app.body_rows);
        assert!(!app.body_rows.iter().any(|l| l.contains("OPEN")), "{:?}", app.body_rows);
    }

    #[test]
    fn header_url_click_opens() {
        // The title left the header block, so header clicks resolve
        // through the meta lines now: the issue URL line opens its link.
        // (The test `open_url` stub notifies instead of opening a browser.)
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().url = "https://example.com/note".to_string();
        let _ = frame_text(&mut app);
        let idx = app.header_rows.iter().position(|l| l.contains("example.com")).expect("url row");
        let col = app.header_area.x + app.header_rows[idx].find("example.com").expect("url col") as u16;
        let row_y = app.header_area.y + idx as u16;
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), col, row_y));
        assert!(app.message.contains("opened https://example.com/note"), "{}", app.message);
    }

    #[test]
    fn narrow_header_wraps_without_clipping_labels() {
        use crate::model::Label;
        let mut app = loaded_app();
        let d = app.data.get_mut(&7).unwrap();
        d.labels = vec![Label { name: "modernization".to_string() }, Label { name: "new-modernization".to_string() }];
        let backend = TestBackend::new(70, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
        let text: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("modernization"), "{text}");
        assert!(text.contains("new-modernization"), "{text}");
        // The long URL hard-splits across wrapped rows; both halves render.
        assert!(text.contains("https://github.com/o/"), "{text}");
        assert!(text.contains("r/issues/7"), "{text}");
        // One logical meta line became several wrapped rows.
        assert!(app.header_rows.len() > 3, "{:?}", app.header_rows);
    }

    #[test]
    fn filter_dialog_shows_nerd_icons() {
        let mut app = loaded_app();
        app.filter_open = true;
        let text = frame_text(&mut app);
        // Font Awesome: fa-filter title, fa-clock-o window row. All four
        // toggles default on, so the checked box renders.
        assert!(text.contains('\u{f0b0}'), "title: {text}");
        assert!(text.contains('\u{f017}'), "window: {text}");
        assert!(text.contains('\u{f14a}'), "checked: {text}");
        app.show_closed_issues = false;
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f096}'), "unchecked: {text}");
    }

    #[test]
    fn toast_marker_uses_warning_icon() {
        let mut app = loaded_app();
        app.notify("hello toast".to_string());
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f071}'), "{text}");
        assert!(text.contains("hello toast"));
    }

    #[test]
    fn board_title_shows_columns_icon() {
        let mut app = board_app(1);
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f0db}'), "{text}");
        assert!(text.contains("Roadmap"));
    }

    #[test]
    fn pinned_row_uses_tack_icon() {
        let mut app = loaded_app();
        app.pin_order = vec![7];
        app.pinned = [7].into_iter().collect();
        app.recent = vec![7, 9];
        assert!(app.issue_rows().iter().any(|r| r.number() == Some(7)));
        let text = frame_text(&mut app);
        assert!(text.contains('\u{f08d}'), "{text}");
    }

    #[test]
    fn diff_summary_shows_file_icon() {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
            DiffFile { filename: "img.png".into(), status: "added".into(), additions: 0, deletions: 0, patch: String::new() },
        ]);
        app.set_tab(Tab::Visual);
        let text = frame_text(&mut app);
        assert!(text.contains("\u{f15c} 2 files"), "{text}");
    }

    #[test]
    fn status_bar_shows_repo_project_and_pinned_hint() {
        let mut app = loaded_app();
        app.statuses.insert(7, vec![("ghit demo".into(), Some("Todo".into()))]);
        let text = frame_text(&mut app);
        let line: String = text.chars().skip(120 * 39).take(120).collect();
        assert!(line.contains("o/r"), "{line}");
        assert!(line.contains("ghit demo (Todo)"), "{line}");
        assert!(line.contains("? for shortcuts"), "{line}");
        let brand = format!("ghit v{}", env!("CARGO_PKG_VERSION"));
        assert!(line.ends_with(brand.as_str()), "{line}");
    }

    #[test]
    fn focused_input_replaces_status_and_positions_cursor() {
        let mut app = loaded_app();
        app.focus = crate::app::Focus::Bar;
        app.bar = "123".into();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let line: String = (0..120).map(|x| terminal.backend().buffer()[(x, 39)].symbol()).collect();
        assert!(line.starts_with("❯ 123"), "{line}");
        assert!(!line.contains("? for shortcuts"), "{line}");
        assert_eq!(terminal.get_cursor_position().unwrap(), (5, 39).into());
    }

    #[test]
    fn visible_message_appears_only_in_status_bar() {
        let mut app = loaded_app();
        app.notify("hello status".into());
        let text = frame_text(&mut app);
        assert_eq!(text.matches("hello status").count(), 1);
        let line: String = text.chars().skip(120 * 39).take(120).collect();
        assert!(line.contains("hello status"), "{line}");
        assert!(line.contains("? for shortcuts"), "{line}");
        let brand = format!("ghit v{}", env!("CARGO_PKG_VERSION"));
        assert!(line.ends_with(brand.as_str()), "{line}");
    }

    #[test]
    fn shortcuts_modal_opens_and_each_close_key_dismisses_it() {
        for close in [KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')] {
            let mut app = loaded_app();
            handle_key(&mut app, key(KeyCode::Char('?')));
            assert!(frame_text(&mut app).contains("Navigation"));
            assert!(frame_text(&mut app).contains("ctrl-o back"));
            handle_key(&mut app, key(close));
            assert!(!app.shortcuts_open);
            assert!(!app.quit);
            assert!(!frame_text(&mut app).contains("Navigation"));
        }
    }

    #[test]
    fn shortcuts_modal_shows_credit_footer() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('?')));
        assert!(app.shortcuts_open);
        let text = frame_text(&mut app);
        // Existing sections still render above the footer.
        assert!(text.contains("Navigation"), "sections survive: {text}");
        assert!(text.contains("Esc / ? / q close"), "closer survives: {text}");
        // GHIT ASCII banner (4 lines), author, and repo URL.
        assert!(text.contains("___ _   _ ___"), "banner renders: {text}");
        assert!(text.contains("created by bolu-atx"), "author renders: {text}");
        assert!(
            text.contains("https://github.com/bolu-atx/gh-ghit"),
            "url renders: {text}"
        );
    }

    #[test]
    fn yank_text_covers_issue_diff_and_refs() {
        use crate::app::Pane;
        use crate::model::DiffFile;
        // Issue identity.
        let app = loaded_app();
        assert_eq!(
            app.yank_text().unwrap(),
            "#7 Fix the thing — https://github.com/o/r/issues/7"
        );
        // Diff file context follows the file cursor, folded by default.
        let mut app = loaded_app();
        app.show_issue(9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.set_tab(crate::app::Tab::Visual);
        assert_eq!(
            app.yank_text().unwrap(),
            "#9 Fix the thing — src/a.rs (modified +1 -1) — https://github.com/o/r/issues/9"
        );
        // Expanded files add the hunk header for agents.
        app.expanded_files.insert((9, "src/a.rs".to_string()));
        assert!(app.yank_text().unwrap().contains("@@ -1 +1 @@"), "{}", app.yank_text().unwrap());
        // Refs context names the raw body line and the ticket.
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().body = "intro\nsee #9 here\n".to_string();
        app.ref_numbers = vec![9];
        app.pane = Pane::Refs;
        app.ref_highlight = Some(9);
        let text = app.yank_text().unwrap();
        assert!(text.contains("#7 refs #9 (body line 2)"), "{text}");
        assert!(text.contains("https://github.com/o/r/issues/9"), "{text}");
        // No selection to yank.
        let mut app = loaded_app();
        app.pane = Pane::Refs;
        app.ref_highlight = None;
        assert!(app.yank_text().is_none());
    }

    #[test]
    fn yank_key_and_empty_state_toast() {
        // `y` reaches `yank`: an empty viewer explains instead of copying.
        let mut app = App::new(Some("o/r".to_string()));
        app.repo_slug = Some("o/r".to_string());
        handle_key(&mut app, key(KeyCode::Char('y')));
        assert_eq!(app.message, "view an issue first");
        // A long yank toast still renders with the footer hints intact.
        let mut app = loaded_app();
        app.notify(format!(
            "yanked {}",
            crate::core::toast_snippet("#7 refs #9 (body line 2): \"see #9 here\" — https://github.com/o/r/issues/9", 80)
        ));
        let text = frame_text(&mut app);
        assert!(text.contains("yanked #7"), "{text}");
        assert!(text.contains("? for shortcuts"), "status keeps the hint");
    }

    #[test]
    fn diff_tab_notes_issues_have_no_diff() {
        let mut app = loaded_app();
        app.set_tab(Tab::Visual);
        let text = frame_text(&mut app);
        assert!(text.contains("graph"), "{text}");
        assert!(text.contains("Graph"), "issue mode tab label: {text}");
    }

    #[test]
    fn diff_tab_shows_loading_while_pending() {
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diffs.remove(&9);
        app.diff_pending.insert(9);
        app.set_tab(Tab::Visual);
        assert!(frame_text(&mut app).contains("Fetching diff"), "loading state");
    }

    #[test]
    fn diff_fetch_lands_without_touching_selection() {
        use crate::app::poll_diffs;
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        let files = vec![DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 0, patch: "@@ -1 +1 @@\n+new".into() }];
        app.diff_tx.send((9, Ok(files))).unwrap();
        poll_diffs(&mut app);
        assert!(!app.diff_pending.contains(&9));
        assert_eq!(app.diffs.get(&9).unwrap().len(), 1);
        assert_eq!(app.current, Some(9));
    }

    #[test]
    fn number_keys_switch_content_tabs() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('3')));
        assert_eq!(app.tab, Tab::Details);
        handle_key(&mut app, key(KeyCode::Char('4')));
        assert_eq!(app.tab, Tab::Details);
        handle_key(&mut app, key(KeyCode::Char('1')));
        assert_eq!(app.tab, Tab::Details);
        handle_key(&mut app, key(KeyCode::Char('2')));
        assert_eq!(app.tab, Tab::Visual);
    }

    #[test]
    fn mouse_click_reaches_diff_tab() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 24, y));
        assert_eq!(app.tab, Tab::Visual);
    }

    fn diff_loaded_app() -> App {
        use crate::model::DiffFile;
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diff_pending.remove(&9);
        app.diffs.insert(9, vec![
            DiffFile { filename: "src/a.rs".into(), status: "modified".into(), additions: 1, deletions: 1, patch: "@@ -1 +1 @@\n-old\n+new".into() },
        ]);
        app.expanded_files.insert((9, "src/a.rs".to_string()));
        app.set_tab(Tab::Visual);
        app
    }

    #[test]
    fn split_mode_pairs_old_and_new_side_by_side() {
        let mut app = diff_loaded_app();
        app.diff_split = true;
        let text = frame_text(&mut app);
        assert!(text.contains("split (s: unified)"), "{text}");
        assert!(text.contains("│"), "{text}");
        assert!(text.contains("-old"), "{text}");
        assert!(text.contains("+new"), "{text}");
        assert!(text.contains("1 -old"), "old-side gutter in split left cell: {text}");
        assert!(text.contains("1 +new"), "new-side gutter in split right cell: {text}");
    }

    #[test]
    fn unified_mode_hints_at_split() {
        let mut app = diff_loaded_app();
        let text = frame_text(&mut app);
        assert!(text.contains("unified (s: split)"), "{text}");
    }

    #[test]
    fn s_toggles_split_only_on_diff_tab() {
        let mut app = diff_loaded_app();
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert!(app.diff_split);
        assert!(app.message.contains("split"));
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert!(!app.diff_split);
        assert!(app.message.contains("unified"));
        app.set_tab(Tab::Details);
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert!(!app.diff_split, "s outside Diff does nothing");
    }

    fn graph_node(number: u64, title: &str, state: &str) -> GraphNode {
        GraphNode {
            number,
            title: title.to_string(),
            state: state.to_string(),
            is_pr: false,
            labels: vec![Label { name: "bug".to_string() }],
            project_status: None,
            linked_prs: vec![],
        }
    }

    fn graph_app() -> App {
        let mut app = loaded_app();
        let mut blocker = graph_node(11, "Upstream blocker", "OPEN");
        blocker.project_status = Some("Todo".to_string());
        blocker.linked_prs = vec![GraphLinkedPr { number: 44, state: "MERGED".to_string() }];
        let center = GraphNode {
            number: 7,
            title: "Fix the thing".to_string(),
            state: "OPEN".to_string(),
            is_pr: false,
            labels: vec![],
            project_status: None,
            linked_prs: vec![],
        };
        let data: GraphData = crate::model::build_graph(
            center,
            vec![
                (graph_node(9, "Blocked work", "OPEN"), GraphEdgeKind::Blocks),
                (blocker, GraphEdgeKind::BlockedBy),
                (graph_node(10, "Plain ref", "CLOSED"), GraphEdgeKind::ReferencedBy),
            ],
        );
        app.graphs.insert(7, data);
        app
    }

    #[test]
    fn graph_tab_shows_skeleton_while_pending() {
        let mut app = loaded_app();
        app.tab = Tab::Visual;
        app.graph_pending.insert(7);
        let text = frame_text(&mut app);
        assert!(text.contains("Graph"), "tab label: {text}");
        assert!(text.contains("Fetching graph"), "skeleton: {text}");
    }

    #[test]
    fn graph_tab_renders_sections_labels_and_prs() {
        let mut app = graph_app();
        app.set_tab(Tab::Visual);
        let text = frame_text(&mut app);
        assert!(text.contains("3 related"), "{text}");
        assert!(text.contains("#9"), "{text}");
        assert!(text.contains("#11"), "{text}");
        assert!(text.contains("#10"), "{text}");
        assert!(text.contains("╭") && text.contains("└─") && text.contains("█"), "node boxes and cursor: {text}");
    }

    #[test]
    fn graph_draws_at_most_eight_nodes() {
        let mut app = graph_app();
        let center = graph_node(7, "Center", "OPEN");
        let neighbors = (20..30).map(|n| (graph_node(n, "Neighbor", "OPEN"), GraphEdgeKind::References)).collect();
        app.graphs.insert(7, crate::model::build_graph(center, neighbors));
        app.set_tab(Tab::Visual);
        let text = super::graph_text(&app, app.data.get(&7).unwrap()).lines.iter().map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert_eq!(text.matches('╭').count(), 8, "{text}");
        assert!(text.contains("+3 more"), "{text}");
        assert_eq!(app.graph_order_numbers().len(), 8);
    }

    #[test]
    fn four_opens_graph_and_keys_stay_tab_scoped() {
        let mut app = graph_app();
        app.pane = crate::app::Pane::Main;
        // j on Body scrolls; the graph cursor never moves.
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.graph_cursor, 0);
        assert!(app.scroll > 0);
        // `4` opens the tab; arrows/hjkl move between nodes.
        handle_key(&mut app, key(KeyCode::Char('2')));
        assert_eq!(app.tab, Tab::Visual);
        let scroll = app.scroll;
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.graph_cursor, 1);
        assert_eq!(app.scroll, scroll, "graph takes the arrows, not scroll");
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.graph_cursor, 2);
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.graph_cursor, 1);
        handle_key(&mut app, key(KeyCode::Char('h')));
        assert_eq!(app.graph_cursor, 0);
        handle_key(&mut app, key(KeyCode::Char('l')));
        assert_eq!(app.graph_cursor, 1);
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.graph_cursor, 0);
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.graph_cursor, 0, "clamped at the top");
    }

    #[test]
    fn graph_enter_opens_selected_issue_per_issue() {
        use crate::model::DiffFile;
        let mut app = graph_app();
        app.pane = crate::app::Pane::Main;
        // Switching issues on the Graph tab refetches per issue: cache #9's
        // graph and diff so the open stays synchronous with no fetch.
        app.graphs.insert(9, crate::model::build_graph(graph_node(9, "Blocked work", "OPEN"), vec![]));
        app.diffs.insert(9, Vec::<DiffFile>::new());
        handle_key(&mut app, key(KeyCode::Char('2')));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.graph_cursor, 1);
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.current, Some(9));
        assert_eq!(app.graph_cursor, 0, "issue switch resets the cursor");
        assert!(app.graphs.contains_key(&9), "per-issue graph");
        // Tab switch away and back keeps the graph and the cursor.
        handle_key(&mut app, key(KeyCode::Char('1')));
        assert_eq!(app.tab, Tab::Details);
        handle_key(&mut app, key(KeyCode::Char('2')));
        assert_eq!(app.tab, Tab::Visual);
        assert!(app.graphs.contains_key(&9));
    }

    #[test]
    fn graph_fetch_lands_without_touching_selection() {
        use crate::app::poll_graphs;
        let mut app = loaded_app();
        app.graph_tx.send((7, Ok(crate::model::build_graph(graph_node(7, "seven", "OPEN"), vec![])))).unwrap();
        poll_graphs(&mut app);
        assert!(!app.graph_pending.contains(&7));
        assert!(app.graphs.contains_key(&7));
        assert_eq!(app.current, Some(7));
    }
}
