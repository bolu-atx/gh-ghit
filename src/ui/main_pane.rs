//! Main pane: header, issue body, graph and wrapping helpers.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Tabs, Wrap,
};

use crate::app::{App, Tab};
use crate::keymap::{Action, Context};
use crate::model::{self, IssueData};
use super::{controls, hints};

use super::{spinning, skeleton_lines};
use super::sidebar::check_span;
use super::diff::diff_text;

/// Styled header for the body pane: the color-coded meta lines. The
/// `#N title` line lives only in the masthead (`render_main`), so it is
/// not repeated here. Each segment carries its semantic role; colors stay
/// consistent by construction.
pub(super) fn header_lines(d: &IssueData, statuses: &[(String, Option<String>)]) -> Vec<Line<'static>> {
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

/// Graph tab body: the dependency star around the current issue, center
/// first then one section per relation. The cursor row carries the amber
/// marker; nodes that block nothing render gray/dim.
pub(super) fn graph_text(app: &App, d: &IssueData) -> ratatui::text::Text<'static> {
    use crate::theme::{ink, muted};
    let dim = || Style::new().bg(ink()).fg(muted());
    let Some(data) = app.graph.data.get(&d.number) else {
        let note = if app.graph.pending.contains(&d.number) {
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
        let marker = if app.graph.cursor == position { "█ " } else { "  " };
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

/// Greedy word-wrap of styled lines to `width` cells. Words longer than
/// the width hard-split; single spaces rejoin the words. Blank lines pass
/// through so paragraph breaks survive. Lets the title/label block above
/// the tabs claim exactly the rows it needs at any window width.
pub(super) fn wrap_lines<'a>(lines: Vec<Line<'a>>, width: usize) -> Vec<Line<'a>> {
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

/// Paint every clickable ref and URL in link style (underlined, colored
/// by kind: issue, PR, or URL), using the same detection the click
/// handler resolves, so what looks like a link is exactly what opens.
/// `is_pr` answers for loaded numbers; unknown refs keep the URL color.
/// Spans split at range edges.
pub(super) fn style_links(lines: &mut [Line<'_>], repo: Option<&str>, is_pr: impl Fn(u64) -> Option<bool>) {
    use crate::theme::{issue_ref, pr_ref, teal};
    for line in lines.iter_mut() {
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let ranges: Vec<(usize, usize, Style)> = model::link_ranges(&plain, repo)
            .into_iter()
            .map(|(a, b, target)| {
                let fg = match target {
                    model::LinkTarget::Issue(n) => match is_pr(n) {
                        Some(true) => pr_ref(),
                        Some(false) => issue_ref(),
                        None => teal(),
                    },
                    model::LinkTarget::Url(_) => teal(),
                };
                (a, b, Style::new().fg(fg).underlined())
            })
            .collect();
        if ranges.is_empty() {
            continue;
        }
        let mut out: Vec<Span<'_>> = Vec::with_capacity(line.spans.len() + ranges.len() * 2);
        let mut offset = 0;
        for span in std::mem::take(&mut line.spans) {
            let (start, end) = (offset, offset + span.content.len());
            offset = end;
            // Cut points inside this span: every range edge that lands here.
            let mut cuts: Vec<usize> = ranges
                .iter()
                .flat_map(|&(a, b, _)| [a, b])
                .filter(|&c| c > start && c < end)
                .collect();
            cuts.sort_unstable();
            cuts.dedup();
            let mut from = start;
            for to in cuts.into_iter().chain(std::iter::once(end)) {
                let piece = &span.content[from - start..to - start];
                let style = match ranges.iter().find(|&&(a, b, _)| from >= a && to <= b) {
                    Some(&(_, _, link)) => span.style.patch(link),
                    None => span.style,
                };
                out.push(Span::styled(piece.to_string(), style));
                from = to;
            }
        }
        line.spans = out;
    }
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

pub(super) fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let repo = app.repo_key().unwrap_or_default();
    // The meta/label block lives above the tabs so long titles wrap in
    // place instead of overflowing the masthead. Its height is measured
    // first (same inner width as the body) so the rows below just fit.
    let inner_w = (area.width as usize).saturating_sub(5);
    let mut header: Vec<Line> = match app.nav.current.and_then(|n| app.issues.data.get(&n)) {
        Some(d) => {
            let st = app.issues.statuses.get(&d.number).cloned().unwrap_or_default();
            header_lines(d, &st)
        }
        None => vec![],
    };
    if let (Some(n), crate::app::Tab::Details) = (app.refs.highlight, app.nav.tab) {
        highlight_mentions(&mut header, n, app.repo_key().as_deref());
    }
    let header = wrap_lines(header, inner_w);
    // The paired line renders with the header block and joins
    // `header_rows`, so clicking `#N` navigates to the counterpart.
    let paired = match (app.nav.tab, app.nav.current.and_then(|n| app.issues.data.get(&n))) {
        (crate::app::Tab::Details, Some(d)) => paired_line(d)
            .map(|line| wrap_lines(vec![line], inner_w))
            .unwrap_or_default(),
        _ => vec![],
    };
    let header: Vec<Line> = header.into_iter().chain(paired).collect();
    app.hits.header_rows = header
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
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
    app.hits.tabs_area = rows[3];
    // Nerd Font Font Awesome fa-git brands the wordmark; repo and number
    // stay plain text so identifiers scan without a patched font. The PR
    // head-commit CI glyph rides the issue line, the single title source
    // now that the header block starts at the meta lines.
    let mast: Vec<Line> = match app.nav.current.and_then(|n| app.issues.data.get(&n)) {
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
    let mast = match mast.len() {
        0 => mast,
        n => {
            let mut mast = mast;
            mast[n - 1] = spinning(std::mem::take(&mut mast[n - 1]), app.pane_loading().main_spinner, app.tick);
            mast
        }
    };
    f.render_widget(Paragraph::new(mast).style(base()), rows[0]);

    let main_focused = app.pane_focused(crate::core::Pane::Main);
    // Second tab follows the current item: PRs show their diff, issues
    // the dependency graph. Nothing loaded yet reads as Graph.
    let visual_label = match app.nav.current.and_then(|n| app.issues.data.get(&n)) {
        Some(d) if d.is_pr => "Diff View",
        _ => "Graph",
    };
    // Nerd Font glyphs: file-text, comments, code. Labels stay so a
    // missing font degrades to a tofu box plus readable text. The
    // `1`/`2` prefixes mirror the Details/Visual keys; the click
    // router splits on the first title via `TAB_SPLIT_X`.
    let tabs = Tabs::new(crate::app::tab_titles(visual_label).to_vec())
        .select(match app.nav.tab { Tab::Details => 0, Tab::Visual => 1 })
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
    if app.visual_diff() {
        controls::diff_strip(f, rows[3], app);
    }

    // Meta/labels render above the tabs with the body's gutter, so the
    // header starts where the body text starts.
    let hgutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[1]);
    app.hits.header_area = hgutter[1];
    f.render_widget(Paragraph::new(header).style(base()), hgutter[1]);

    let markdown;
    let mut text = match app.nav.current.and_then(|n| app.issues.data.get(&n)) {
        Some(d) => match app.nav.tab {
            Tab::Details => {
                let comment_key = hints::key_in(Context::Global, Action::Comment);
                markdown = format!("## Body\n\n{}\n\n## Comments\n\n{}", model::body_md(d), model::build_comments_md(d))
                    .replace("_No comments yet._", &format!("_No comments yet. Press `{comment_key}` to write one._"));
                crate::theme::render_markdown(&markdown)
            }
            Tab::Visual => match model::visual_content(d.is_pr) {
                model::VisualContent::Diff => diff_text(app, d, (rows[4].width as usize).saturating_sub(5)),
                model::VisualContent::Graph => graph_text(app, d),
            },
        },
        None if app.pane_loading().main => ratatui::text::Text::from(skeleton_lines(6)),
        None => {
            markdown = format!(
                "No issue loaded.\n\nRecent issues and PRs are on the left.\nType a number and hit Enter (`{}`), or instantly filter loaded rows (title, label, author) with `{}`.",
                hints::key_in(Context::Global, Action::CommandBar),
                hints::key_in(Context::Global, Action::FindBar),
            );
            crate::theme::render_markdown(&markdown)
        }
    };
    // Clickable refs and URLs read as links in the body and the graph.
    if app.nav.tab == Tab::Details || app.visual_graph() {
        // Kind comes from loaded issue data, then the current graph's nodes.
        let graph = app.nav.current.and_then(|n| app.graph.data.get(&n));
        let is_pr = |n: u64| {
            app.issues.data.get(&n).map(|d| d.is_pr).or_else(|| graph?.nodes.iter().find(|g| g.number == n).map(|g| g.is_pr))
        };
        style_links(&mut text.lines, app.repo_key().as_deref(), is_pr);
    }
    // A refs-row selection lights up the mentioning line instead of opening
    // the issue. Body tab only: the mentions come from the body markdown
    // (and the header above the tabs).
    if let (Some(n), crate::app::Tab::Details) = (app.refs.highlight, app.nav.tab) {
        highlight_mentions(&mut text.lines, n, app.repo_key().as_deref());
    }
    app.hits.body_total = text_rows(&text, usize::from(rows[4].width.saturating_sub(5)).max(1));
    let pane = Paragraph::new(text)
        .style(base())
        .wrap(Wrap { trim: false })
        .scroll((app.nav.scroll, 0));
    // Breathing room: text never touches the column edges.
    let gutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[4]);
    app.hits.body_area = gutter[1];
    f.render_widget(pane, gutter[1]);

}

/// Rows `text` takes at `width` cells, counting each line's wraps.
fn text_rows(text: &ratatui::text::Text, width: usize) -> usize {
    text.lines.iter().map(|line| line.width().max(1).div_ceil(width)).sum()
}
