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
    if app.tab == crate::app::Tab::Diff {
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
}

fn render_shortcuts(f: &mut ratatui::Frame, area: Rect) {
    use crate::theme::{base, ink, muted, paper};
    let width = area.width.min(76);
    let height = area.height.min(18);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base()).title(" Shortcuts ");
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = [
        "Navigation", "  tab panes · j/k select · ↑/↓ scroll · page up/down",
        "  1/2/3 body/comments/diff · e refs · n/N search matches", "",
        "Actions", "  : command · / find · f filter · b board · w window",
        "  o open · r refresh · p pin · y yank · t theme · q quit", "",
        "Board", "  h/l columns · j/k cards · space expand · enter open", "",
        "Diff", "  ↑/↓ lines · space expand · s split", "",
        "Esc / ? / q close",
    ];
    f.render_widget(Paragraph::new(lines.join("\n")).style(Style::new().bg(ink()).fg(paper())), inner);
}

/// Filter dialog (`f`): recent window, open/closed toggles, and a GitHub
/// search query box. Window and query commits refetch; toggles only hide
/// recent rows on this screen.
fn render_filter(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let popup = centered(area, 62, 52);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        // Nerd Font Font Awesome fa-filter.
        .title(Span::styled(" \u{f0b0} Filter ", Style::new().bg(ink()).fg(amber()).bold()));
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

/// One pane (Issues or PRs): pinned, requested, then recent rows with a
/// thin divider row between non-empty groups. Everything is owned, so the
/// returned list outlives the `&App` borrow and the caller can persist its
/// `ListState` back into `App` for scroll continuity.
fn section_list(
    title: Line<'static>,
    rows: &[crate::model::SideRow],
    app: &App,
    inner_width: usize,
    mark: Option<u64>,
    focused: bool,
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
        // The selection marker only renders on the focused pane. The
        // ListState cursor underneath is untouched, so tabbing away and
        // back restores the exact row and scroll position.
        let sel = focused && (app.current == Some(*n) || mark == Some(*n));
        let marker = if sel { "█ " } else { "  " };
        let marker_style = if sel {
            Style::new().fg(amber()).bold()
        } else {
            Style::new().fg(muted())
        };
        let (title_text, body, seg) = match app.data.get(n) {
            Some(d) => {
                let st = app.statuses.get(n).cloned().unwrap_or_default();
                let (_, b, s) =
                    model::sidebar_lines(*n, &d.title, &d.labels, &st, 38, app.pinned.contains(n));
                (model::slug(&d.title, 30), b, s)
            }
            None => (String::new(), String::new(), String::new()),
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
        // Project status in the board color with the board glyph.
        let seg_color = crate::theme::column_color(&seg);
        let mut lines = vec![
            Line::from(first),
            Line::from(vec![
                Span::raw("    "),
                Span::styled(body, Style::new().fg(muted())),
                Span::styled(seg, Style::new().fg(seg_color)),
            ]),
        ];
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
                .style(Style::new().bg(ink())),
        )
        .highlight_style(if focused {
            Style::new().bold().bg(code_bg())
        } else {
            Style::new()
        })
}

/// Styled header for the body pane: title plus the color-coded meta line.
/// Each segment carries its semantic role; colors stay consistent by construction.
fn header_lines(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<Line<'static>> {
    use crate::theme::{amber, ink, muted, paper, teal};
    use model::MetaKind;
    let title = Line::from(vec![
        Span::styled("■ ", Style::new().bg(ink()).fg(muted())),
        Span::styled(format!("#{} ", d.number), Style::new().bg(ink()).fg(muted())),
        Span::styled(model::slug(&d.title, 60), Style::new().bg(ink()).fg(paper()).bold()),
    ]);
    let mut lines = vec![title];
    let mut current: Vec<Span<'static>> = Vec::new();
    for (kind, text) in model::meta_spans(d, statuses) {
        let style = match kind {
            MetaKind::Dim => Style::new().bg(ink()).fg(muted()),
            MetaKind::State => Style::new().bg(ink()).fg(crate::theme::state_color(&d.state)).bold(),
            MetaKind::Label => Style::new().bg(ink()).fg(amber()),
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
    if files.is_empty() {
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
            lines.extend(split_file_lines(&f.patch, width.saturating_sub(2)).into_iter().map(|mut l| {
                l.spans.insert(0, dent());
                l
            }));
        } else {
            for (kind, text) in model::diff_lines(&f.patch) {
                let style = match kind {
                    DiffKind::Hunk => Style::new().bg(ink()).fg(muted()).bold(),
                    DiffKind::Add => Style::new().bg(ink()).fg(green()),
                    DiffKind::Del => Style::new().bg(ink()).fg(red()),
                    DiffKind::Context => Style::new().bg(ink()).fg(paper()),
                };
                lines.push(Line::from(vec![dent(), Span::styled(text, style)]));
            }
        }
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

/// Fit `s` into exactly `w` cells: truncate, then pad with spaces.
fn fit_cell(s: &str, w: usize) -> String {
    let mut out: String = s.chars().take(w).collect();
    while out.chars().count() < w {
        out.push(' ');
    }
    out
}

/// Side-by-side rows for one file patch at `width` cells wide.
fn split_file_lines(patch: &str, width: usize) -> Vec<Line<'static>> {
    use crate::theme::{green, ink, muted, paper, red};
    use model::{DiffKind, SplitRow};
    let sep = " │ ";
    let lw = width.saturating_sub(sep.len()) / 2;
    let rw = width.saturating_sub(sep.len() + lw);
    let mut lines = vec![Line::from(vec![
        Span::styled(fit_cell("— old", lw), Style::new().bg(ink()).fg(muted())),
        Span::styled(sep, Style::new().bg(ink()).fg(muted())),
        Span::styled(fit_cell("+ new", rw), Style::new().bg(ink()).fg(muted())),
    ])];
    for row in model::split_rows(patch) {
        match row {
            SplitRow::Hunk(text) => lines.push(Line::from(Span::styled(
                fit_cell(&text, width),
                Style::new().bg(ink()).fg(muted()).bold(),
            ))),
            SplitRow::Pair { left, right } => {
                let change = left.starts_with('-') && right.starts_with('+');
                lines.push(Line::from(vec![
                    Span::styled(
                        fit_cell(&left, lw),
                        Style::new().bg(ink()).fg(if change { red() } else { paper() }),
                    ),
                    Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                    Span::styled(
                        fit_cell(&right, rw),
                        Style::new().bg(ink()).fg(if change { green() } else { paper() }),
                    ),
                ]));
            }
            SplitRow::LeftOnly { text, kind } => lines.push(Line::from(vec![
                Span::styled(
                    fit_cell(&text, lw),
                    Style::new().bg(ink()).fg(if kind == DiffKind::Del { red() } else { paper() }),
                ),
                Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                Span::styled(fit_cell("", rw), Style::new().bg(ink())),
            ])),
            SplitRow::RightOnly { text, kind } => lines.push(Line::from(vec![
                Span::styled(fit_cell("", lw), Style::new().bg(ink())),
                Span::styled(sep, Style::new().bg(ink()).fg(muted())),
                Span::styled(
                    fit_cell(&text, rw),
                    Style::new().bg(ink()).fg(if kind == DiffKind::Add { green() } else { paper() }),
                ),
            ])),
        }
    }
    lines
}

/// Pane header: one decorative Nerd Font glyph (muted) plus the text label
/// (amber), so a terminal without the font degrades to tofu plus readable
/// text. Glyph sources are Font Awesome codepoints from the Nerd Fonts
/// cheatsheet, all long-stable single-cell icons.
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
    let key = app.repo_key().unwrap_or_default();
    app.side_issue_area = panes[0];
    app.side_pr_area = panes[1];
    app.side_issue_rows = issue_rows.clone();
    app.side_pr_rows = pr_rows.clone();
    let inner = |a: Rect| (a.width as usize).saturating_sub(2);
    let issues_focused = app.pane == crate::app::Pane::Issues;
    let il = section_list(
        // Nerd Font Font Awesome fa-exclamation-circle.
        section_title("Issues", &key, "\u{f06a}", issues_focused),
        &issue_rows,
        app,
        inner(panes[0]),
        None,
        issues_focused,
    );
    let mut istate = std::mem::take(&mut app.issue_state);
    istate.select(selected_index(&issue_rows, app.current));
    f.render_stateful_widget(il, panes[0], &mut istate);
    app.issue_state = istate;
    let prs_focused = app.pane == crate::app::Pane::PRs;
    let pl = section_list(
        // Nerd Font Font Awesome fa-code-fork.
        section_title("Pull requests", &app.recent_detail(), "\u{f126}", prs_focused),
        &pr_rows,
        app,
        inner(panes[1]),
        None,
        prs_focused,
    );
    let mut pstate = std::mem::take(&mut app.pr_state);
    pstate.select(selected_index(&pr_rows, app.current));
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
        section_title("Refs", &format!("{} mentioned", rows.len()), "\u{f0c1}", refs_focused),
        &rows,
        app,
        inner,
        app.ref_highlight,
        refs_focused,
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

fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let repo = app.repo_key().unwrap_or_default();
    // The title/label block lives above the tabs so long titles wrap in
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
    if let (Some(n), crate::app::Tab::Body) = (app.ref_highlight, app.tab) {
        highlight_mentions(&mut header, n, app.repo_key().as_deref());
    }
    let header = wrap_lines(header, inner_w);
    app.header_rows = header
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(header.len() as u16),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(area);

    // rows[0] masthead, rows[1] title/labels, rows[2] breathing room,
    // rows[3] tabs, rows[4] pane.
    app.tabs_area = rows[3];
    // Nerd Font Font Awesome fa-git brands the wordmark; repo and number
    // stay plain text so identifiers scan without a patched font.
    let mast = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => Line::from(vec![
            Span::styled("\u{f1d3} ghit ", Style::new().fg(amber()).bold()),
            Span::styled(format!("{repo}  "), Style::new().fg(muted())),
            Span::styled(format!("#{} ", d.number), Style::new().fg(muted())),
            Span::styled(model::slug(&d.title, 60), Style::new().fg(paper()).bold()),
        ]),
        None => Line::from(vec![
            Span::styled("\u{f1d3} ghit ", Style::new().fg(amber()).bold()),
            Span::styled(repo, Style::new().fg(muted())),
        ]),
    };
    f.render_widget(Paragraph::new(mast).style(base()), rows[0]);

    let main_focused = app.pane == crate::app::Pane::Main;
    // Nerd Font glyphs: file-text, comments, code. Labels stay so a
    // missing font degrades to a tofu box plus readable text.
    let tabs = Tabs::new(vec!["\u{f15c}  Body", "\u{f086}  Comments", "\u{f121}  Diff"])
        .select(match app.tab {
            Tab::Body => 0,
            Tab::Comments => 1,
            Tab::Diff => 2,
        })
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

    // Title/labels render above the tabs with the body's gutter, so the
    // title starts where the body text starts.
    let hgutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[1]);
    app.header_area = hgutter[1];
    f.render_widget(Paragraph::new(header).style(base()), hgutter[1]);

    let owned_body: String;
    let owned_comments: String;
    let mut text = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => match app.tab {
            Tab::Body => {
                owned_body = model::body_md(d);
                crate::theme::render_markdown(&owned_body)
            }
            Tab::Comments => {
                owned_comments = model::build_comments_md(d);
                crate::theme::render_markdown(&owned_comments)
            }
            Tab::Diff => {
                let width = (rows[4].width as usize).saturating_sub(5);
                diff_text(app, d, width)
            }
        },
        None => crate::theme::render_markdown(
            "No issue loaded.\n\nRecent issues and PRs are on the left.\nType a number and hit Enter (`:`), or instantly filter loaded rows (title, label, author) with `/`.",
        ),
    };
    // A refs-row selection lights up the mentioning line instead of opening
    // the issue. Body tab only: the mentions come from the body markdown
    // (and the header above the tabs).
    if let (Some(n), crate::app::Tab::Body) = (app.ref_highlight, app.tab) {
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

fn render_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use crate::theme::{amber, base, ink, muted, paper};
    if app.focus == crate::app::Focus::Bar {
        let text = if app.searching { &app.search } else { &app.bar };
        let prompt = if app.searching { "/ " } else { "❯ " };
        let shown = if text.is_empty() {
            if app.searching { "instant filter loaded: title, #n, label, author …" } else { "type #123, o, q …" }
        } else { text };
        f.render_widget(Paragraph::new(Line::from(vec![
            Span::styled(prompt, Style::new().fg(amber()).bold()),
            Span::styled(shown, Style::new().fg(if text.is_empty() { muted() } else { paper() })),
        ])).style(base()), area);
        f.set_cursor_position((area.x + (2 + text.len() as u16).min(area.width.saturating_sub(1)), area.y));
        return;
    }
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
    if app.message_visible() && !app.message.is_empty() {
        segments.push((format!("\u{f071} {}", app.message), true));
    }
    let mut spans = Vec::new();
    for (i, (text, accent)) in segments.into_iter().enumerate() {
        if i > 0 { spans.push(Span::styled(" · ", Style::new().fg(muted()))); }
        spans.push(Span::styled(text, Style::new().fg(if accent { amber() } else { paper() })));
    }
    let hint = "? for shortcuts";
    let hint_width = hint.len() as u16;
    let left_width = area.width.saturating_sub(hint_width + 1);
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (inner, hint_area) = (chunks[0], chunks[1]);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "h/l columns · j/k cards · space expand · enter open · esc close",
            Style::new().bg(ink()).fg(muted()),
        ))),
        hint_area,
    );

    let Some(board) = &app.board else {
        render_skeleton(f, inner);
        return;
    };
    let cols = model::board_columns(&board.items, &board.order);
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
            // reading as a broken fetch.
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  no cards",
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
    use crate::model::{Author, BoardData, BoardDetail, BoardItem, IssueData, Label};
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
        app.issue_tx.send((42, Ok((d, vec![])))).unwrap();
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
        app.issue_tx.send((7, Ok((d, vec![])))).unwrap();
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
    fn mouse_click_switches_tabs() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 12, y),
        );
        assert_eq!(app.tab, Tab::Comments);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 1, y),
        );
        assert_eq!(app.tab, Tab::Body);
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
    fn background_section_rows_render_without_changing_selection() {
        let mut app = loaded_app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.section_rx = Some(rx);
        let mut issue = sample(42);
        issue.is_pr = true;
        issue.title = "background pull request".to_string();
        tx.send(Ok(vec![crate::api::Row { issue, statuses: vec![], detail: crate::model::BoardDetail::default(), complete: false }])).unwrap();
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
        app.set_tab(Tab::Diff);
        let text = frame_text(&mut app);
        assert!(text.contains("Diff"), "{text}");
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
        assert!(open.contains("  -old"), "hunks indent under their file");
        assert!(open.contains("+new"), "{open}");
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
        app.set_tab(Tab::Diff);
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
        app.set_tab(Tab::Diff);
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
        app.set_tab(Tab::Diff);
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
        assert!(text.contains('\u{f086}'), "{text}");
        assert!(text.contains('\u{f121}'), "{text}");
    }

    #[test]
    fn tab_click_zones_match_wider_labels() {
        // Glyph plus two spaces plus label: Body spans rel 0..7, Comments
        // rel 9..20, the rest is Diff. Clicks on the boundaries prove it.
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 8, y));
        assert_eq!(app.tab, Tab::Body);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 9, y));
        assert_eq!(app.tab, Tab::Comments);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 22, y));
        assert_eq!(app.tab, Tab::Diff);
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
        assert!(app.header_rows.iter().any(|l| l.contains("#7")));
        assert!(app.header_area.y + app.header_area.height <= app.tabs_area.y);
        // The title/label block left the scrollable body: meta text like
        // the state and assignees no longer appear in body rows.
        assert!(!app.body_rows.iter().any(|l| l.contains("assignees:")), "{:?}", app.body_rows);
        assert!(!app.body_rows.iter().any(|l| l.contains("OPEN")), "{:?}", app.body_rows);
    }

    #[test]
    fn header_ref_click_navigates() {
        let mut app = loaded_app();
        app.data.get_mut(&7).unwrap().title = "Fix #9 regression".to_string();
        let _ = frame_text(&mut app);
        let row = app.header_rows.iter().find(|l| l.contains("#9")).expect("title row").clone();
        let (hx, hy) = (app.header_area.x, app.header_area.y);
        let col = hx + row.find("#9").expect("ref col") as u16;
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), col, hy));
        assert_eq!(app.current, Some(9));
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
        app.set_tab(Tab::Diff);
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
        assert!(line.ends_with("? for shortcuts"), "{line}");
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
        assert!(line.ends_with("? for shortcuts"), "{line}");
    }

    #[test]
    fn shortcuts_modal_opens_and_each_close_key_dismisses_it() {
        for close in [KeyCode::Esc, KeyCode::Char('?'), KeyCode::Char('q')] {
            let mut app = loaded_app();
            handle_key(&mut app, key(KeyCode::Char('?')));
            assert!(frame_text(&mut app).contains("Navigation"));
            handle_key(&mut app, key(close));
            assert!(!app.shortcuts_open);
            assert!(!app.quit);
            assert!(!frame_text(&mut app).contains("Navigation"));
        }
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
        app.set_tab(crate::app::Tab::Diff);
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
        app.set_tab(Tab::Diff);
        let text = frame_text(&mut app);
        assert!(text.contains("only available for pull requests"), "{text}");
    }

    #[test]
    fn diff_tab_shows_loading_while_pending() {
        let mut app = loaded_app();
        app.data.insert(9, diff_pr(9));
        app.show_issue(9);
        app.diffs.remove(&9);
        app.diff_pending.insert(9);
        app.set_tab(Tab::Diff);
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
        assert_eq!(app.tab, Tab::Diff);
        handle_key(&mut app, key(KeyCode::Char('1')));
        assert_eq!(app.tab, Tab::Body);
        handle_key(&mut app, key(KeyCode::Char('2')));
        assert_eq!(app.tab, Tab::Comments);
    }

    #[test]
    fn mouse_click_reaches_diff_tab() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(&mut app, click(MouseEventKind::Down(MouseButton::Left), tx + 24, y));
        assert_eq!(app.tab, Tab::Diff);
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
        app.set_tab(Tab::Diff);
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
        app.set_tab(Tab::Body);
        handle_key(&mut app, key(KeyCode::Char('s')));
        assert!(!app.diff_split, "s outside Diff does nothing");
    }
}
