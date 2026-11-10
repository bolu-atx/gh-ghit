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
    use crate::theme::{amber, ink, muted};
    f.render_widget(Block::default().style(Style::new().bg(ink())), f.area());
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(44), Constraint::Min(1)])
        .split(root[0]);

    render_sidebar(f, cols[0], app);
    render_main(f, cols[1], app);
    app.capture_body_rows(f.buffer_mut());

    let footer_line = if app.message_visible() {
        Line::from(vec![
            Span::styled("! ", Style::new().fg(amber()).bold()),
            Span::styled(app.message.clone(), Style::new().fg(amber())),
        ])
    } else {
        Line::from(Span::styled(
            "j/k move · ↑↓ scroll · 1/2 tab · b board · : cmd · / find · n/N next · f filter · o open · r refresh · p pin · t theme · w age · q quit",
            Style::new().fg(muted()),
        ))
    };
    f.render_widget(Paragraph::new(footer_line), root[1]);

    if crate::app::board_open(app) {
        dim_background(f, 0.35);
        render_board(f, f.area(), app);
    }

    if app.filter_open {
        dim_background(f, 0.35);
        render_filter(f, f.area(), app);
    }
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
        .title(Span::styled(" Filter ", Style::new().bg(ink()).fg(amber()).bold()));
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
        let box_glyph = if on { "[x]" } else { "[ ]" };
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
            if query_text.is_empty() { "github search, e to edit …".to_string() } else { query_text.clone() },
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
        Span::styled("window ", Style::new().bg(ink()).fg(muted())),
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

/// One pane (Issues or PRs): pinned, requested, then recent rows with a
/// thin divider row between non-empty groups. Everything is owned, so the
/// returned list outlives the `&App` borrow and the caller can persist its
/// `ListState` back into `App` for scroll continuity.
fn section_list(
    title: Line<'static>,
    rows: &[Option<u64>],
    app: &App,
    inner_width: usize,
) -> List<'static> {
    use crate::theme::{amber, ink, muted, paper, state_color};
    let query = app.search.clone();
    let mut items: Vec<ListItem<'static>> = vec![ListItem::new(Line::from(""))];
    for row in rows {
        let Some(n) = row else {
            let rule = "─".repeat(inner_width.max(8));
            items.push(ListItem::new(Line::from(Span::styled(
                rule,
                Style::new().bg(ink()).fg(muted()),
            ))));
            continue;
        };
        let sel = app.current == Some(*n);
        let marker = if sel { "▌ " } else { "  " };
        let marker_style = if sel {
            Style::new().fg(amber()).bold()
        } else {
            Style::new().fg(muted())
        };
        let (title_text, second) = match app.data.get(n) {
            Some(d) => {
                let st = app.statuses.get(n).cloned().unwrap_or_default();
                (
                    model::slug(&d.title, 30),
                    model::sidebar_lines(*n, &d.title, &d.labels, &st, 38, app.pinned.contains(n)).1,
                )
            }
            None => (String::new(), String::new()),
        };
        let pin = if app.pinned.contains(n) { "◆ " } else { "" };
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
        items.push(ListItem::new(vec![
            Line::from(first),
            Line::from(vec![
                Span::raw("    "),
                Span::styled(second, Style::new().fg(muted())),
            ]),
            Line::from(""),
        ]));
    }
    List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .title(title)
                .style(Style::new().bg(ink())),
        )
        .highlight_style(Style::new().bold())
}

/// Styled header for the body pane: title plus the color-coded meta line.
/// Each segment carries its semantic role; colors stay consistent by construction.
fn header_lines<'a>(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<Line<'a>> {
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
            MetaKind::Project => Style::new().bg(ink()).fg(teal()).bold(),
            MetaKind::Url => Style::new().bg(ink()).fg(teal()).underlined(),
        };
        let mut chunks = text.split('\n');
        if let Some(first) = chunks.next() {
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

fn section_title<'a>(name: &str, detail: &str) -> Line<'a> {
    use crate::theme::{amber, muted};
    let name_span = Span::styled(format!(" {name} "), Style::new().fg(amber()).bold());
    let detail_span = Span::styled(detail.to_string(), Style::new().fg(muted()));
    Line::from(vec![name_span, detail_span])
}

/// Position of `current` in display-row order, +1 for the top spacer.
fn selected_index(rows: &[Option<u64>], current: Option<u64>) -> Option<usize> {
    current
        .and_then(|n| rows.iter().position(|r| *r == Some(n)))
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
    let il = section_list(
        section_title("Issues", &key),
        &issue_rows,
        app,
        inner(panes[0]),
    );
    let mut istate = std::mem::take(&mut app.issue_state);
    istate.select(selected_index(&issue_rows, app.current));
    f.render_stateful_widget(il, panes[0], &mut istate);
    app.issue_state = istate;
    let pl = section_list(
        section_title("Pull requests", &app.recent_detail()),
        &pr_rows,
        app,
        inner(panes[1]),
    );
    let mut pstate = std::mem::take(&mut app.pr_state);
    pstate.select(selected_index(&pr_rows, app.current));
    f.render_stateful_widget(pl, panes[1], &mut pstate);
    app.pr_state = pstate;
}

fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(area);

    let repo = app.repo_key().unwrap_or_default();
    // rows[0] masthead, rows[1] breathing room, rows[2] tabs, rows[3] pane, rows[4] bar.
    app.tabs_area = rows[2];
    app.bar_area = rows[4];
    let mast = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => Line::from(vec![
            Span::styled("ghit ", Style::new().fg(amber()).bold()),
            Span::styled(format!("{repo}  "), Style::new().fg(muted())),
            Span::styled(format!("#{} ", d.number), Style::new().fg(muted())),
            Span::styled(model::slug(&d.title, 60), Style::new().fg(paper()).bold()),
        ]),
        None => Line::from(vec![
            Span::styled("ghit ", Style::new().fg(amber()).bold()),
            Span::styled(repo, Style::new().fg(muted())),
        ]),
    };
    f.render_widget(Paragraph::new(mast).style(base()), rows[0]);

    let tabs = Tabs::new(vec!["Body", "Comments"])
        .select(match app.tab {
            Tab::Body => 0,
            Tab::Comments => 1,
        })
        .style(Style::new().bg(ink()).fg(muted()))
        .highlight_style(Style::new().bg(ink()).fg(amber()).bold().underlined())
        .divider("  ")
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::new().bg(ink()).fg(muted()))
                .style(Style::new().bg(ink())),
        );
    f.render_widget(tabs, rows[2]);

    let owned_body: String;
    let owned_comments: String;
    let text = match app.current.and_then(|n| app.data.get(&n)) {
        Some(d) => {
            let st = app.statuses.get(&d.number).cloned().unwrap_or_default();
            match app.tab {
                Tab::Body => {
                    let mut lines = header_lines(d, &st);
                    lines.push(Line::from(""));
                    owned_body = model::body_md(d);
                    lines.extend(crate::theme::render_markdown(&owned_body));
                    ratatui::text::Text::from(lines)
                }
                Tab::Comments => {
                    owned_comments = model::build_comments_md(d);
                    crate::theme::render_markdown(&owned_comments)
                }
            }
        }
        None => crate::theme::render_markdown(
            "No issue loaded.\n\nRecent issues and PRs are on the left.\nType a number and hit Enter (`:`), or search titles with `/`.",
        ),
    };
    let pane = Paragraph::new(text)
        .style(base())
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    // Breathing room: text never touches the column edges.
    let gutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[3]);
    app.body_area = gutter[1];
    f.render_widget(pane, gutter[1]);

    let bar_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(base())
        .border_style(
            Style::new().bg(ink()).fg(if app.focus == crate::app::Focus::Main { muted() } else { amber() }),
        );
    render_bar(f, rows[4], bar_block, app);
}

/// Command bar: prompt, input or hint, theme pill. Cursor math lives here
/// because `Frame::set_cursor_position` takes absolute cells. In search
/// mode the prompt is `/` and the text is the live title/# filter.
fn render_bar(f: &mut ratatui::Frame, area: Rect, bar_block: Block<'_>, app: &App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let inner = bar_block.inner(area);
    f.render_widget(bar_block, area);
    let mut spans = vec![Span::styled(
        if app.searching { "/ " } else { "❯ " },
        Style::new().bg(ink()).fg(amber()).bold(),
    )];
    let text = if app.searching { &app.search } else { &app.bar };
    if text.is_empty() {
        spans.push(Span::styled(
            if app.searching { "search title or #n …" } else { "type #123, o, q …" },
            Style::new().bg(ink()).fg(muted()).italic(),
        ));
    } else {
        spans.push(Span::styled(text.clone(), Style::new().bg(ink()).fg(paper())));
    }
    // Theme pill pinned right, like a status chip.
    let pill = format!(" {} ", crate::theme::name());
    let shown = if app.searching { &app.search } else { &app.bar };
    let used = 2 + shown.chars().count().max(19) + pill.chars().count() + 2;
    let fill = (inner.width as usize).saturating_sub(used);
    if fill >= 1 {
        spans.push(Span::raw(" ".repeat(fill)));
        spans.push(Span::styled(
            pill,
            Style::new().bg(amber()).fg(ink()).bold(),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(base()), inner);
    if app.focus == crate::app::Focus::Bar {
        let len = if app.searching { app.search.len() } else { app.bar.len() };
        let x = if len == 0 { inner.x + 2 } else { inner.x + 2 + len as u16 };
        f.set_cursor_position((x, inner.y));
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
    let title = match &app.board {
        Some(b) => format!(" {} ", b.title),
        None => " Fetching board … ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(muted()))
        .style(base())
        .title(Span::styled(title, Style::new().bg(ink()).fg(amber()).bold()));
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
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().bg(ink()).fg(if active { amber() } else { muted() }))
                .style(base())
                .title(Line::from(vec![
                    Span::styled(format!(" {} ", name), Style::new().bg(ink()).fg(crate::theme::column_color(name)).bold()),
                    Span::styled(format!("({})", cards.len()), Style::new().bg(ink()).fg(muted())),
                ])),
        );
        let state = &mut app.board_states[i];
        state.select(if active { Some(app.board_row) } else { None });
        f.render_stateful_widget(list, *a, state);
        // Map visible cards back to click areas using the widget's scroll
        // offset. Cards have variable height, so walk line offsets.
        let offset = app.board_states[i].offset();
        let col_inner = Block::default().borders(Borders::ALL).inner(*a);
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
    let mut meta = format!("{} · {}", status, card_kind(c));
    if let Some(login) = card_assignee(c) {
        meta.push_str(&format!(" · @{login}"));
    }
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
    fn jk_walks_issues_then_prs() {
        let mut app = loaded_app();
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.current, Some(9));
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.current, Some(7));
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
        assert_eq!(app.issue_rows(), vec![Some(7), None, Some(42)]);
        assert_eq!(app.pr_rows(), vec![Some(9)]);
        let text = frame_text(&mut app);
        assert!(text.contains("Issues"));
        assert!(text.contains("Pull requests"));
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
        assert_eq!(app.pr_rows(), vec![Some(9)]);
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
        assert_eq!(app.pr_rows(), vec![Some(9), None, Some(20)]);
        handle_key(&mut app, key(KeyCode::Char('f')));
        assert!(app.filter_open);
        let text = frame_text(&mut app);
        assert!(text.contains("Filter"));
        assert!(text.contains("query"));
        handle_key(&mut app, key(KeyCode::Char('m')));
        assert!(!app.show_merged_prs);
        assert_eq!(app.pr_rows(), vec![Some(9)]);
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
    fn mouse_click_switches_tabs() {
        let mut app = loaded_app();
        let _ = frame_text(&mut app);
        let (tx, y) = (app.tabs_area.x, app.tabs_area.y);
        handle_mouse(
            &mut app,
            click(MouseEventKind::Down(MouseButton::Left), tx + 8, y),
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
}
