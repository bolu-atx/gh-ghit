//! Key hints read from the keymap registry: the status-bar hint line and
//! the key legend in dialog footers. Key in amber, label muted, one
//! muted ` · ` between hints.

use std::collections::HashSet;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::app::App;
use crate::keymap::{self, Action, Context};
use crate::theme::{amber, ink, muted};

const SEP: &str = " · ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hint {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) action: Action,
}

impl Hint {
    fn width(&self) -> usize {
        self.key.chars().count() + 1 + self.label.chars().count()
    }

    pub(super) fn spans(&self) -> [Span<'static>; 2] {
        [
            Span::styled(self.key.clone(), Style::new().bg(ink()).fg(amber())),
            Span::styled(format!(" {}", self.label), Style::new().bg(ink()).fg(muted())),
        ]
    }
}

/// Hint wording: the label with a lowercase first letter, unless the
/// second letter is capital (`PRs`) or the word is a variable (`$EDITOR`).
fn lower_first(label: &str) -> String {
    let mut chars = label.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if second.is_uppercase() => format!("{first}{second}{}", chars.as_str()),
        (Some(first), Some(_)) => format!("{}{}", first.to_lowercase(), &label[first.len_utf8()..]),
        _ => label.to_string(),
    }
}

/// Folds the last `run.len()` hints into one `first-last` hint when the
/// run is three or more digit keys of one group.
fn flush(run: &mut Vec<(char, &str)>, hints: &mut Vec<Hint>, group: &str) {
    if run.len() >= 3 {
        let (first, last) = (run[0].0, run[run.len() - 1].0);
        let start = hints.len() - run.len();
        let action = hints[start].action;
        hints.truncate(start);
        hints.push(Hint { key: format!("{first}-{last}"), label: lower_first(group), action });
    }
    run.clear();
}

/// Hints of `contexts`, top context first and by rank inside each one,
/// one per action. Runs of digit keys in one group show as `1-4`.
pub(crate) fn collect(contexts: &[Context], app: &App) -> Vec<Hint> {
    let mut seen = HashSet::new();
    let mut hints: Vec<Hint> = vec![];
    for context in contexts {
        let mut ranked: Vec<_> = keymap::bindings(*context)
            .filter(|b| b.when.is_none_or(|test| test(app)) && b.hint_when.is_none_or(|test| test(app)))
            .filter_map(|b| b.hint.map(|rank| (rank, b)))
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        let mut run: Vec<(char, &str)> = vec![];
        let mut group = "";
        for (_, binding) in ranked {
            if !seen.insert(binding.action) {
                continue;
            }
            let digit = match binding.key {
                crossterm::event::KeyCode::Char(c) if c.is_ascii_digit() => Some(c),
                _ => None,
            };
            let continues = digit.is_some_and(|d| {
                run.last().is_some_and(|(last, _)| (*last as u8 + 1) as char == d) && binding.group == group
            });
            if !continues {
                flush(&mut run, &mut hints, group);
            }
            if let Some(d) = digit {
                run.push((d, binding.group));
                group = binding.group;
            }
            hints.push(Hint { key: binding.key_name(), label: lower_first(binding.label), action: binding.action });
        }
        flush(&mut run, &mut hints, group);
    }
    hints
}

/// Status-bar hints for the base layer, as (context hints, essentials).
/// Dialogs and text inputs own the keys, so they get none here.
pub(crate) fn bar_hints(app: &App) -> (Vec<Hint>, Vec<Hint>) {
    if app.focus().layer != ghit_core::core::Layer::Base {
        return (vec![], vec![]);
    }
    let (essentials, rest): (Vec<_>, Vec<_>) =
        collect(&keymap::active_contexts(app), app).into_iter().partition(|h| matches!(h.action, Action::ShowHelp | Action::Quit));
    (rest, essentials)
}

pub(super) fn line_width(hints: &[Hint]) -> usize {
    hints.iter().map(Hint::width).sum::<usize>() + hints.len().saturating_sub(1) * SEP.chars().count()
}

/// Width of the essentials alone: the reserve content must leave free.
pub(crate) fn essentials_width(app: &App) -> u16 {
    line_width(&bar_hints(app).1) as u16
}

/// The hints that fit in `avail` cells, in display order. The lowest
/// priority goes first: context hints from the end, then `q`.
pub(crate) fn fit(rest: &[Hint], essentials: &[Hint], avail: usize) -> Vec<Hint> {
    for kept_essentials in (0..=essentials.len()).rev() {
        for kept in (0..=rest.len()).rev() {
            let shown: Vec<Hint> = rest[..kept].iter().chain(&essentials[..kept_essentials]).cloned().collect();
            if !shown.is_empty() && line_width(&shown) <= avail {
                return shown;
            }
        }
    }
    vec![]
}

/// Spans of a hint line plus each hint's rect when drawn from `x` on `row`.
pub(crate) fn line(hints: &[Hint], x: u16, row: u16) -> (Line<'static>, Vec<(Rect, Action)>) {
    let mut spans = vec![];
    let mut rects = vec![];
    let mut at = x;
    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(SEP, Style::new().bg(ink()).fg(muted())));
            at += SEP.chars().count() as u16;
        }
        spans.extend(hint.spans());
        let w = hint.width() as u16;
        rects.push((Rect::new(at, row, w, 1), hint.action));
        at += w;
    }
    (Line::from(spans), rects)
}

/// Hints of a dialog packed into at most `max_lines` lines of `width`
/// cells. Lower ranks win when space is short.
fn pack(context: Context, app: &App, width: u16, max_lines: usize) -> Vec<Vec<Hint>> {
    let hints = collect(&[context], app);
    let mut lines: Vec<Vec<Hint>> = vec![vec![]];
    for hint in hints {
        let current = lines.last_mut().expect("one line");
        let mut next = current.clone();
        next.push(hint.clone());
        if current.is_empty() || line_width(&next) <= width as usize {
            *current = next;
        } else if lines.len() < max_lines {
            lines.push(vec![hint]);
        } else {
            break;
        }
    }
    lines
}

/// Legend of a dialog: its own hints packed into at most `max_lines`
/// lines of `width` cells.
pub(crate) fn legend(context: Context, app: &App, width: u16, max_lines: usize) -> Vec<Line<'static>> {
    pack(context, app, width, max_lines).iter().map(|hints| line(hints, 0, 0).0).collect()
}

/// Click rects of the legend when its first line is drawn at (`x`, `y`).
pub(crate) fn legend_hits(
    context: Context,
    app: &App,
    width: u16,
    max_lines: usize,
    (x, y): (u16, u16),
) -> Vec<(Rect, Action)> {
    pack(context, app, width, max_lines).iter().enumerate()
        .flat_map(|(i, hints)| line(hints, x, y + i as u16).1)
        .collect()
}

/// Name of the first key `context` binds to `action`. Empty-state text
/// uses it so the named key is the one that works.
pub(crate) fn key_in(context: Context, action: Action) -> String {
    keymap::bindings(context).find(|b| b.action == action).map(|b| b.key_name()).unwrap_or_default()
}

/// Muted empty-state line with the fixing key in amber between
/// `before` and `after`.
pub(crate) fn tip(before: &str, context: Context, action: Action, after: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(before.to_string(), Style::new().fg(muted())),
        Span::styled(key_in(context, action), Style::new().fg(amber())),
        Span::styled(after.to_string(), Style::new().fg(muted())),
    ])
}
