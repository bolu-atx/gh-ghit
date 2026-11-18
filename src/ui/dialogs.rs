//! Modal dialogs: confirm, Format, compose.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Wrap,
};

use crate::app::App;
use crate::core::Dialog;
use crate::keymap::{Action, Context};
use crate::model::self;
use crate::theme;
use super::hints;

use super::{sized_popup, scroll_to, centered};
use super::board::render_board_filter;

/// Generic confirm modal: title, message lines, and one hint segment
/// per choice (`s Submit them · d Discard`), plus `esc Cancel`. The
/// renderer never interprets the kind — choices carry their own keys
/// and labels, so new specializations reuse it untouched.
pub(super) fn render_confirm(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, base, ink, muted, paper};
    let Some(d) = app.confirm().cloned() else { return };
    let popup = centered(area, 64, 30);
    app.hits.popup = popup;
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(ink()).fg(amber()))
        .style(base())
        .title(format!(" {} ", d.title));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let mut lines: Vec<Line> = d.body.iter().map(|l| {
        let style = if l.emphasis {
            Style::new().bg(ink()).fg(amber()).bold()
        } else {
            Style::new().bg(ink()).fg(paper())
        };
        Line::from(Span::styled(l.text.clone(), style))
    }).collect();
    lines.push(Line::from(""));
    // Click rects for the choice labels. The hint row sits below the
    // body lines (wrapped ones counted by width) and one blank row.
    let width = inner.width.max(1) as usize;
    let body_rows: usize = d.body.iter().map(|l| l.text.chars().count().div_ceil(width).max(1)).sum();
    let hint_y = inner.y + body_rows as u16 + 1;
    app.hits.confirm_choices.clear();
    let mut x = inner.x;
    let mut hints = vec![];
    for (i, choice) in d.choices.iter().enumerate() {
        if i > 0 {
            hints.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
            x += 3;
        }
        let w = (1 + 1 + choice.label.chars().count()) as u16;
        if hint_y < inner.y + inner.height && x + w <= inner.x + inner.width {
            app.hits.confirm_choices.push((Rect::new(x, hint_y, w, 1), i));
        }
        x += w;
        hints.push(Span::styled(choice.key.to_string(), Style::new().bg(ink()).fg(amber())));
        hints.push(Span::styled(format!(" {}", choice.label), Style::new().bg(ink()).fg(muted())));
    }
    let legend = hints::collect(&[Context::Dialog(Dialog::Confirm)], app);
    for (i, hint) in legend.iter().enumerate() {
        if i == 0 {
            hints.push(Span::styled(" · ", Style::new().bg(ink()).fg(muted())));
            hints.extend(hint.spans());
        } else {
            lines.push(Line::from(hints.split_off(0)));
            hints.extend(hint.spans());
        }
    }
    lines.push(Line::from(hints));
    f.render_widget(Paragraph::new(lines).style(base()).wrap(Wrap { trim: false }), inner);
}

/// One column of the sidebar Format dialog. Every focusable row records
/// its line so `h`/`l` can land on the matching row in the other column.
struct FormatColumn {
    lines: Vec<Line<'static>>,
    width: usize,
    focus: usize,
    focus_line: Option<usize>,
    rows: Vec<(usize, usize)>,
    /// Click cells: line, start cell, width, widget index, option index.
    cells: Vec<FormCell>,
    gutter: usize,
}

type FormCell = (usize, usize, usize, usize, Option<usize>);

impl FormatColumn {
    fn new(width: usize, focus: usize) -> Self {
        Self { lines: Vec::new(), width, focus, focus_line: None, rows: Vec::new(), cells: Vec::new(), gutter: 0 }
    }

    /// Label gutter shared by every row in the column: the widest label
    /// plus two cells of air. Chips start on the same cell on every row.
    fn gutter(&mut self, labels: &[&str]) {
        let widest = labels.iter().map(|label| Line::raw(*label).width()).max().unwrap_or(0);
        self.gutter = (widest + 2).min(self.width.saturating_sub(1));
    }

    /// Blank line after a row. Rows breathe; wrapped chip lines stay tight.
    fn gap(&mut self) {
        self.lines.push(Line::raw(""));
    }

    /// Numbered pipeline section: amber number, paper name, a blank line
    /// under the name, and two blank lines of margin above it.
    fn section(&mut self, number: usize, title: &str) {
        if !self.lines.is_empty() {
            self.lines.push(Line::raw(""));
            self.lines.push(Line::raw(""));
        }
        self.lines.push(Line::from(vec![
            Span::styled(number.to_string(),
                Style::new().bg(theme::ink()).fg(theme::amber()).bold()),
            Span::styled(format!(" {title}"),
                Style::new().bg(theme::ink()).fg(theme::paper()).bold()),
        ]));
        self.gap();
    }

    /// Section outside the pipeline: set apart by a blank line and a thin
    /// muted rule, then a muted name.
    fn quiet_section(&mut self, title: &str) {
        if !self.lines.is_empty() {
            self.lines.push(Line::raw(""));
            self.lines.push(Line::raw(""));
        }
        self.lines.push(Line::styled("\u{2500}".repeat(self.width),
            Style::new().bg(theme::ink()).fg(theme::muted())));
        self.lines.push(Line::styled(title.to_string(),
            Style::new().bg(theme::ink()).fg(theme::muted())));
        self.gap();
    }

    /// Muted row label padded to the gutter. Unlabeled rows get a blank
    /// gutter, so every control in the column starts on the same cell.
    fn label_spans(&self, label: Option<&str>) -> Vec<Span<'static>> {
        vec![Span::styled(
            format!("{:<width$}", label.unwrap_or(""), width = self.gutter),
            Style::new().bg(theme::ink()).fg(theme::muted()))]
    }

    /// Left cell where a row's controls start.
    fn indent(&self, _label: Option<&str>) -> usize {
        self.gutter
    }

    /// Focusable text row: label, amber marker cell, then the caller's
    /// spans, starting on the same cell as chip and option names.
    fn field(&mut self, index: usize, label: Option<&str>, spans: Vec<Span<'static>>) {
        let active = self.focus == index;
        self.rows.push((index, self.lines.len()));
        self.cells.push((self.lines.len(), 0, self.width, index, None));
        if active {
            self.focus_line = Some(self.lines.len());
        }
        let mut line = self.label_spans(label);
        line.push(Span::styled(
            if active { "▌  " } else { "   " },
            Style::new().bg(theme::ink()).fg(theme::amber()),
        ));
        line.extend(spans);
        self.lines.push(Line::from(line));
        self.gap();
    }

    /// Focusable row of mutually exclusive options. The marker sits before
    /// the selected option, which reads amber while the row is focused.
    fn choice(&mut self, index: usize, label: Option<&str>, options: &[(bool, &str)]) {
        let active = self.focus == index;
        self.rows.push((index, self.lines.len()));
        if active {
            self.focus_line = Some(self.lines.len());
        }
        let indent = self.indent(label);
        let mut line = self.label_spans(label);
        let mut width = indent;
        // Same cells as a chip: marker, radio glyph, space, name, air.
        // The glyph keeps the selection readable without color.
        for (option, (selected, name)) in options.iter().enumerate() {
            let marked = active && *selected;
            let text = format!("{}{} {name}  ", if marked { "▌" } else { " " },
                if *selected { "●" } else { "○" });
            let chip_width = Line::raw(text.clone()).width();
            if width > indent && width + chip_width > self.width {
                self.lines.push(Line::from(std::mem::take(&mut line)));
                line = vec![self.blank_span(indent)];
                width = indent;
            }
            self.cells.push((self.lines.len(), width, chip_width, index, Some(option)));
            line.push(Span::styled(text, Style::new().bg(theme::ink()).fg(
                if marked { theme::amber() } else if *selected { theme::paper() } else { theme::muted() })));
            width += chip_width;
        }
        self.lines.push(Line::from(line));
        self.gap();
    }

    fn blank_span(&self, width: usize) -> Span<'static> {
        Span::styled(" ".repeat(width), Style::new().bg(theme::ink()))
    }

    /// Focusable checkbox chips, wrapped to the column width. The focused
    /// chip carries the amber marker; selected chips read paper.
    fn chips(&mut self, label: Option<&str>, chips: Vec<(usize, bool, String)>) {
        let indent = self.indent(label);
        let mut line = self.label_spans(label);
        let mut width = indent;
        for (index, on, name) in chips {
            let active = self.focus == index;
            let text = format!("{}{} {name}  ", if active { "▌" } else { " " },
                if on { "\u{f14a}" } else { "\u{f096}" });
            let chip_width = Line::raw(text.clone()).width();
            if width > indent && width + chip_width > self.width {
                self.lines.push(Line::from(std::mem::take(&mut line)));
                line = vec![self.blank_span(indent)];
                width = indent;
            }
            self.rows.push((index, self.lines.len()));
            self.cells.push((self.lines.len(), width, chip_width, index, None));
            if active {
                self.focus_line = Some(self.lines.len());
            }
            line.push(Span::styled(text, Style::new().bg(theme::ink()).fg(
                if active { theme::amber() } else if on { theme::paper() } else { theme::muted() })));
            width += chip_width;
        }
        if !line.is_empty() {
            self.lines.push(Line::from(line));
        }
        self.gap();
    }

    /// Muted empty-state text in the row's chip column.
    fn empty(&mut self, label: &str, text: &str) {
        let mut line = self.label_spans(Some(label));
        // Three cells of air: the marker, glyph and space a chip carries.
        line.push(Span::styled(format!("   {text}"),
            Style::new().bg(theme::ink()).fg(theme::muted())));
        self.lines.push(Line::from(line));
        self.gap();
    }

    /// Muted prose under a section, wrapped to the column width.
    fn hint(&mut self, text: &str) {
        let mut line = String::new();
        for word in text.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > self.width {
                self.lines.push(Line::styled(std::mem::take(&mut line),
                    Style::new().bg(theme::ink()).fg(theme::muted())));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            self.lines.push(Line::styled(line,
                Style::new().bg(theme::ink()).fg(theme::muted())));
        }
    }
}

pub(super) fn render_filter(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    app.hits.form_popup = Rect::default();
    app.hits.form_cells.clear();
    if crate::app::board_open(app) {
        render_board_filter(f, area, app);
        return;
    }
    // Border plus one cell of air on each side.
    let popup_width = sized_popup(area, 90, 1).width.saturating_sub(4) as usize;
    let two_columns = popup_width >= 76;
    app.sidebar.format_columns = two_columns;
    let column_width = if two_columns { (popup_width - 3) / 2 } else { popup_width };
    let focus = app.sidebar.format_focus;
    let mut left = FormatColumn::new(column_width, focus);
    left.gutter(&["search", "window", "states", "labels", "milestones"]);
    let mut right = FormatColumn::new(column_width, focus);
    right.gutter(&["key", "order", "issues", "PRs"]);
    left.section(1, "Query");
    let query = if app.query_edit() {
        &app.sidebar.query_draft
    } else {
        &app.sidebar.query_active
    };
    let mut search = Vec::new();
    if query.is_empty() && !app.query_edit() {
        search.push(Span::styled("recent activity (e to search)".to_string(),
            Style::new().bg(theme::ink()).fg(theme::muted())));
    } else {
        search.push(Span::styled(query.to_string(), Style::new().bg(theme::ink()).fg(
            if focus == 0 { theme::amber() } else { theme::paper() })));
        if app.query_edit() {
            search.push(Span::styled("▌", Style::new().bg(theme::ink()).fg(theme::amber())
                .bold()));
        }
    }
    left.field(0, Some("search"), search);
    let saved_count = app.sidebar.saved_queries.len();
    if app.name_edit() {
        left.field(1, Some("name"), vec![Span::styled(
            format!("{}▌", app.sidebar.name_draft), Style::new().fg(theme::amber()))]);
    } else if saved_count == 0 {
        left.empty("saved", &format!("none ({} saves the query)", hints::key_in(Context::Dialog(Dialog::Format), Action::FormatSaveName)));
    } else {
        left.chips(Some("saved"), app.sidebar.saved_queries.iter().enumerate().map(|(i, saved)|
            (1 + i, saved.query == app.sidebar.query_active.trim(), saved.name.clone())).collect());
    }
    let hours = app.sections.merged_window_hours;
    left.choice(1 + saved_count, Some("window"), &[
        (hours == 1, "1h"),
        (hours == 24, "24h"),
        (hours == 168, "7d"),
        (hours == 0, "all time"),
    ]);
    left.section(2, "Filter");
    left.chips(Some("states"), vec![
        (2 + saved_count, app.sidebar.show_open_prs, "open PRs".into()),
        (3 + saved_count, app.sidebar.show_merged_prs, "merged PRs".into()),
        (4 + saved_count, app.sidebar.show_open_issues, "open issues".into()),
        (5 + saved_count, app.sidebar.show_closed_issues, "closed issues".into()),
    ]);
    let label_chips: Vec<_> = app.sidebar.labels.iter().enumerate().map(|(i, name)|
        (6 + saved_count + i, app.sidebar.label_filter.contains(name), name.clone())).collect();
    if label_chips.is_empty() {
        left.empty("labels", if app.sidebar.meta_rx.is_some() {
            "loading labels …" } else { "No labels in this repo" });
    } else {
        left.chips(Some("labels"), label_chips);
    }
    let pills = app.milestone_pills();
    let milestone_chips: Vec<_> = pills.iter().enumerate().map(|(i, pill)|
        (6 + saved_count + app.sidebar.labels.len() + i, app.sidebar.milestone_filter.contains(pill),
         pill.clone().unwrap_or_else(|| "(no milestone)".into()))).collect();
    if milestone_chips.is_empty() {
        left.empty("milestones", if app.sidebar.meta_rx.is_some() {
            "loading milestones …" } else { "No milestones in this repo" });
    } else {
        left.chips(Some("milestones"), milestone_chips);
    }
    left.hint("Filters only narrow loaded rows. To fetch every match, search for it, e.g. label:bug");
    let base = app.format_left_widgets();
    right.section(3, "Group");
    right.choice(base, None, &[
        (app.sidebar.group_by == model::GroupBy::None, "none"),
        (app.sidebar.group_by == model::GroupBy::Milestone, "milestone"),
    ]);
    right.choice(base + 1, Some("nest sub-issues"), &[
        (!app.sidebar.sub_issue_tree, "off"), (app.sidebar.sub_issue_tree, "on"),
    ]);
    right.section(4, "Sort");
    use crate::core::SideSort;
    right.choice(base + 2, Some("key"), &[
        (app.sidebar.sort == SideSort::Default, "default"),
        (app.sidebar.sort == SideSort::Updated, "updated"),
        (app.sidebar.sort == SideSort::Author, "author"),
        (app.sidebar.sort == SideSort::State, "state"),
        (app.sidebar.sort == SideSort::Label, "label"),
    ]);
    right.choice(base + 3, Some("order"), &[
        (!app.sidebar.sort_desc, "ascending"),
        (app.sidebar.sort_desc, "descending"),
    ]);
    right.quiet_section("Display");
    right.chips(Some("issues"), model::RowFields::ISSUE_ORDER.iter().enumerate().map(|(i, name)|
        (base + 4 + i, app.sidebar.row_issue.shown(false, i).unwrap_or(false),
         name.to_string())).collect());
    right.chips(Some("PRs"), model::RowFields::PR_ORDER.iter().enumerate().map(|(i, name)|
        (base + 4 + model::RowFields::ISSUE_ORDER.len() + i,
         app.sidebar.row_pr.shown(true, i).unwrap_or(false), name.to_string())).collect());
    app.sidebar.format_rows = vec![0; app.format_widgets()];
    for &(index, row) in left.rows.iter().chain(&right.rows) {
        app.sidebar.format_rows[index] = row;
    }
    // The name prompt replaces the saved row (index 1) while it edits.
    let name_line = left.rows.iter().find(|&&(index, _)| index == 1).map_or(3, |&(_, row)| row);
    let focus_line = if app.name_edit() { name_line } else {
        left.focus_line.or_else(|| right.focus_line.map(|row| {
            if two_columns { row } else { row + left.lines.len() }
        })).unwrap_or(0)
    };
    let content_height = if two_columns {
        left.lines.len().max(right.lines.len())
    } else {
        left.lines.len() + right.lines.len()
    };
    // Content, a blank line, the legend, and one line of air top and bottom.
    let legend_width = sized_popup(area, 90, 1).width.saturating_sub(4);
    let footer_lines = hints::legend(Context::Dialog(Dialog::Format), app, legend_width, 3);
    let footer_height = footer_lines.len().max(1) as u16;
    let popup = sized_popup(area, 90, content_height + 3 + footer_height as usize);
    f.render_widget(Clear, popup);
    let block = Block::default().borders(Borders::ALL)
        .border_style(Style::new().bg(theme::ink()).fg(theme::muted())).style(theme::base())
        .title(Line::from(vec![
            Span::styled(" \u{f0b0} Format ", Style::new().bg(theme::ink())
                .fg(theme::amber()).bold()),
            Span::styled("sidebar ", Style::new().bg(theme::ink()).fg(theme::muted())),
        ]));
    let framed = block.inner(popup);
    f.render_widget(block, popup);
    let inner = framed.inner(ratatui::layout::Margin { horizontal: 1, vertical: 1 });
    let content = Rect { height: inner.height.saturating_sub(1 + footer_height), ..inner };
    let scroll = scroll_to(content, focus_line, app.query_edit() || app.name_edit());
    app.hits.form_popup = popup;
    let right_x = if two_columns { content.x + column_width as u16 + 3 } else { content.x };
    let right_skip = if two_columns { 0 } else { left.lines.len() };
    let cells = left.cells.iter().map(|&(l, x, w, i, o)| (l, content.x, x, w, i, o)).chain(
        right.cells.iter().map(|&(l, x, w, i, o)| (l + right_skip, right_x, x, w, i, o)));
    for (line, base, x, w, index, option) in cells {
        let Some(y) = line.checked_sub(scroll as usize).filter(|y| *y < content.height as usize)
        else { continue };
        let x = base + x as u16;
        let w = (w as u16).min(content.right().saturating_sub(x));
        app.hits.form_cells.push((Rect::new(x, content.y + y as u16, w, 1), index, option));
    }
    if two_columns {
        let left_area = Rect { width: column_width as u16, ..content };
        let divider = Rect { x: left_area.right(), width: 3, ..content };
        let right_area = Rect { x: divider.right(), width: content.right() - divider.right(),
            ..content };
        f.render_widget(Paragraph::new(left.lines).style(theme::base()).scroll((scroll, 0)),
            left_area);
        let rule = vec![Line::styled(" \u{2502} ", Style::new().fg(theme::muted()));
            content.height as usize];
        f.render_widget(Paragraph::new(rule).style(theme::base()), divider);
        f.render_widget(Paragraph::new(right.lines).style(theme::base()).scroll((scroll, 0)),
            right_area);
    } else {
        left.lines.extend(right.lines);
        f.render_widget(Paragraph::new(left.lines).style(theme::base()).scroll((scroll, 0)),
            content);
    }
    let footer = Rect { y: inner.y + inner.height.saturating_sub(footer_height),
        height: inner.height.min(footer_height), ..inner };
    f.render_widget(Paragraph::new(footer_lines).style(Style::new().bg(theme::ink()).fg(theme::muted())), footer);
    let legend_hits = hints::legend_hits(Context::Dialog(Dialog::Format), app, legend_width, 3, (footer.x, footer.y));
    app.hits.legend.extend(legend_hits);
    if app.name_edit() && content.height > 1 {
        let x = inner.x + left.gutter as u16 + 3
            + Line::raw(app.sidebar.name_draft.as_str()).width() as u16;
        let y = inner.y + (name_line as u16).saturating_sub(scroll);
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), y));
    } else if app.query_edit() && content.height > 1 {
        let x = inner.x + left.gutter as u16 + 3
            + Line::raw(app.sidebar.query_draft.as_str()).width() as u16;
        // The search row sits under the Query header and its blank line.
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y + 2));
    }
}

/// Comment compose modal (`c`): textarea over a rendered markdown
/// preview through the shared `GhitSheet` stylesheet. The draft shows a
/// `▌` cursor marker; while the background submit runs, the title reads
/// Sending and editing locks. The modal steals all keyboard and mouse
/// focus, edits with bash/readline chords, and offers `$EDITOR` on
/// `Ctrl+O`; resolving threads is out of scope and never offered here.
pub(super) fn render_compose(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    use crate::theme::{amber, ink, muted, base, paper};
    let Some(c) = app.draft() else { return };
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
    let footer = if c.sending {
        Line::styled("sending… esc closes, the send still lands", Style::new().bg(ink()).fg(muted()))
    } else {
        hints::legend(Context::Dialog(Dialog::Compose), app, chunks[2].width, 1).remove(0)
    };
    if !c.sending {
        let at = (chunks[2].x, chunks[2].y);
        let legend_hits = hints::legend_hits(Context::Dialog(Dialog::Compose), app, chunks[2].width, 1, at);
        app.hits.legend.extend(legend_hits);
    }
    f.render_widget(Paragraph::new(footer), chunks[2]);
}
