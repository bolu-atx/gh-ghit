//! Main pane: header, issue body, the Map tab's map and wrapping helpers.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Paragraph, Tabs, Wrap,
};

use crate::app::{App, Tab};
use crate::keymap::{Action, Context};
use ghit_core::model::{self, IssueData};
use super::{controls, hints};

use super::{spinning, loading_body};
use super::sidebar::check_span;
use super::diff::diff_text_aux;
use super::search::SearchAux;
use ghit_core::model::ItemRef;

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
pub(super) fn style_links(
    lines: &mut [Line<'_>],
    repo: Option<&str>,
    is_pr: impl Fn(ItemRef) -> Option<bool>,
) {
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
fn highlight_mentions(lines: &mut [Line<'_>], n: ItemRef, repo: Option<&str>) {
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
        spans.push(Span::styled(
            pair.number.display(""),
            Style::new().bg(ink()).fg(color).bold(),
        ));
        if !pair.state.is_empty() {
            spans.push(Span::styled(format!(" {}", pair.state), Style::new().bg(ink()).fg(color)));
        }
    }
    Some(Line::from(spans))
}

pub(super) fn render_main(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, paper, base};
    let repo = app.session.repo_key().unwrap_or_default();
    // The meta/label block lives above the tabs so long titles wrap in
    // place instead of overflowing the masthead. Its height is measured
    // first (same inner width as the body) so the rows below just fit.
    let inner_w = (area.width as usize).saturating_sub(5);
    let mut header: Vec<Line> = match app.nav.current.and_then(|n| app.session.issues.data.get(&n)) {
        Some(d) => {
            let st = app.session.issues.statuses.get(&d.number).cloned().unwrap_or_default();
            header_lines(d, &st)
        }
        None => vec![],
    };
    if let (Some(n), crate::app::Tab::Details) = (app.refs.highlight, app.nav.tab) {
        highlight_mentions(&mut header, n, app.session.repo_key().as_deref());
    }
    let header = wrap_lines(header, inner_w);
    // The paired line renders with the header block and joins
    // `header_rows`, so clicking `#N` navigates to the counterpart.
    let paired = match (app.nav.tab, app.nav.current.and_then(|n| app.session.issues.data.get(&n))) {
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
    let mast: Vec<Line> = match app.nav.current.and_then(|n| app.session.issues.data.get(&n)) {
        Some(d) => {
            let mut title = vec![
                Span::styled(
                    format!("{} ", d.number.display("")),
                    Style::new().fg(muted()),
                ),
                Span::styled(model::slug(&d.title, 60), Style::new().fg(paper()).bold()),
            ];
            if let Some(check) = check_span(d) {
                title.push(check);
            }
            vec![
                brand(repo),
                Line::from(title),
            ]
        }
        None => vec![brand(repo)],
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

    let main_focused = app.pane_focused(ghit_core::core::Pane::Main);
    // Nerd Font glyphs: file-text, code, map. Labels stay so a missing
    // font degrades to a tofu box plus readable text. The `1`/`2`/`3`
    // prefixes mirror the tab keys; the click router splits on the
    // same titles via `tab_at`.
    let tabs = Tabs::new(crate::app::tabs().iter().map(|(_, title)| *title).collect::<Vec<_>>())
        .select(crate::app::tabs().iter().position(|(tab, _)| *tab == app.nav.tab))
        .style(Style::new().bg(ink()).fg(muted()))
        .highlight_style(Style::new().bg(ink()).fg(amber()).bold().underlined())
        .padding("", "")
        .divider(" ")
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(if main_focused { BorderType::Double } else { BorderType::Plain })
                .border_style(Style::new().bg(ink()).fg(if main_focused { amber() } else { muted() }))
                .style(Style::new().bg(ink())),
        );
    f.render_widget(tabs, rows[3]);
    controls::toolbar(f, rows[3], app);

    // Meta/labels render above the tabs with the body's gutter, so the
    // header starts where the body text starts.
    let hgutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[1]);
    app.hits.header_area = hgutter[1];
    f.render_widget(Paragraph::new(header).style(base()), hgutter[1]);

    // Breathing room: text never touches the column edges.
    let gutter = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(3), Constraint::Min(1), Constraint::Length(2)])
        .split(rows[4]);
    app.hits.body_area = gutter[1];
    if app.map_shown() {
        app.hits.body_total = 0;
        super::map::render_map(f, gutter[1], app);
        return;
    }

    let markdown;
    let mut aux = SearchAux::default();
    let mut text = match app.nav.current.and_then(|n| app.session.issues.data.get(&n)) {
        Some(d) => match app.nav.tab {
            Tab::Details => {
                let comment_key = hints::key_in(Context::Global, Action::Comment);
                markdown = format!("## Body\n\n{}\n\n## Comments\n\n{}", model::body_md(d), model::build_comments_md(d))
                    .replace("_No comments yet._", &format!("_No comments yet. Press `{comment_key}` to write one._"));
                crate::theme::render_markdown(&markdown)
            }
            Tab::Visual if d.is_pr => diff_text_aux(app, d, (rows[4].width as usize).saturating_sub(5), &mut aux),
            Tab::Visual => {
                markdown = format!(
                    "Issues have no diff.\n\nThe dependency map is on the Map tab (`{}`).",
                    hints::key_in(Context::Global, Action::TabMap),
                );
                crate::theme::render_markdown(&markdown)
            }
            // The map draws itself above and returns early.
            Tab::Map => ratatui::text::Text::default(),
        },
        None if app.pane_loading().main => {
            let label = app.nav.requested.map_or("Loading…".to_string(), |n| {
                format!("Loading {n}…", n = n.display(""))
            });
            ratatui::text::Text::from(loading_body(&label, app.tick, usize::from(rows[4].height)))
        }
        None => {
            markdown = format!(
                "No issue loaded.\n\nRecent issues and PRs are on the left.\nType a number and hit Enter (`{}`), or instantly filter loaded rows (title, label, author) with `{}`.",
                hints::key_in(Context::Global, Action::CommandBar),
                hints::key_in(Context::Global, Action::FindBar),
            );
            crate::theme::render_markdown(&markdown)
        }
    };
    // Clickable refs and URLs read as links in the body.
    if app.nav.tab == Tab::Details {
        let is_pr = |n: ItemRef| app.session.issues.data.get(&n).map(|d| d.is_pr);
        style_links(&mut text.lines, app.session.repo_key().as_deref(), is_pr);
    }
    // A refs-row selection lights up the mentioning line instead of opening
    // the issue. Body tab only: the mentions come from the body markdown
    // (and the header above the tabs).
    if let (Some(n), crate::app::Tab::Details) = (app.refs.highlight, app.nav.tab) {
        highlight_mentions(&mut text.lines, n, app.session.repo_key().as_deref());
    }
    let wrap_width = usize::from(rows[4].width.saturating_sub(5)).max(1);
    super::search::apply(app, &mut text, wrap_width, usize::from(rows[4].height), &aux);
    app.hits.body_total = text_rows(&text, usize::from(rows[4].width.saturating_sub(5)).max(1));
    let pane = Paragraph::new(text)
        .style(base())
        .wrap(Wrap { trim: false })
        .scroll((app.nav.scroll, 0));
    f.render_widget(pane, gutter[1]);
}

/// Rows `text` takes at `width` cells, counting each line's wraps.
fn text_rows(text: &ratatui::text::Text, width: usize) -> usize {
    text.lines.iter().map(|line| line.width().max(1).div_ceil(width)).sum()
}

/// The masthead brand: `ghit`, its version in gray, then the repo.
fn brand(repo: String) -> Line<'static> {
    use crate::theme::{amber, muted};
    Line::from(vec![
        Span::styled(format!("{}ghit ", crate::theme::nf("\u{f1d3} ", "")), Style::new().fg(amber()).bold()),
        Span::styled(concat!("v", env!("CARGO_PKG_VERSION"), "  "), Style::new().fg(muted())),
        Span::styled(repo, Style::new().fg(muted())),
    ])
}
