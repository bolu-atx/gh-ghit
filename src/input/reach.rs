//! Jump, half-page and page moves. One set of keys (`gg`, `G`, `ctrl-d`,
//! `ctrl-u`, `PgDn`, `PgUp`) works in every list and scroll area. The
//! layer and pane that own the keys decide what moves.

use ratatui::layout::Rect;

use crate::app::{App, board_move};
use crate::core::{Dialog, Layer, Pane};
use crate::keymap;

/// How far a move goes. The sign of `Half` and `Page` is the direction.
#[derive(Clone, Copy)]
pub(super) enum Reach {
    Top,
    Bottom,
    Half(isize),
    Page(isize),
}

impl Reach {
    /// Signed rows for a view `height` rows tall; `None` for the ends.
    fn rows(self, height: usize) -> Option<isize> {
        let height = height.max(1) as isize;
        match self {
            Reach::Top | Reach::Bottom => None,
            Reach::Half(dir) => Some(dir * (height / 2).max(1)),
            Reach::Page(dir) => Some(dir * height),
        }
    }

    fn toward_end(self) -> bool {
        matches!(self, Reach::Bottom | Reach::Half(1..) | Reach::Page(1..))
    }
}

/// Far enough to hit any end when a clamping move runs.
const FAR: isize = 1 << 20;

pub(super) fn reach(app: &mut App, to: Reach) {
    match app.focus().layer {
        Layer::Dialog(Dialog::Shortcuts) => help(app, to),
        Layer::Dialog(Dialog::Board) => board(app, to),
        Layer::Base if keymap::graph_focused(app) => graph(app, to),
        Layer::Base => pane(app, to),
        _ => {}
    }
}

fn help(app: &mut App, to: Reach) {
    app.help_scroll = match to.rows(10) {
        None if to.toward_end() => u16::MAX,
        None => 0,
        Some(rows) => app.help_scroll.saturating_add_signed(rows as i16),
    };
}

fn board(app: &mut App, to: Reach) {
    let visible = app.hits.board_cells.iter().filter(|(_, col, _)| *col == app.board.col).count();
    let dy = to.rows(visible).unwrap_or(if to.toward_end() { FAR } else { -FAR });
    board_move(app, 0, dy);
}

/// On the map the ends pick the first or last node; half and full pages
/// pan the view.
fn graph(app: &mut App, to: Reach) {
    match to.rows(app.hits.map_area.height as usize) {
        None => app.map_select_end(to.toward_end()),
        Some(rows) => app.map_scroll(0.0, rows as f64),
    }
}

fn pane(app: &mut App, to: Reach) {
    match app.nav.pane {
        Pane::Issues => list(app, app.issue_numbers(), app.hits.side_issue_area, to),
        Pane::PRs => list(app, app.pr_numbers(), app.hits.side_pr_area, to),
        Pane::Refs => {
            let order = app.ref_order();
            let rows = (app.hits.refs_area.height as usize / 3).max(1);
            if let Some(n) = target(&order, app.refs.highlight, rows, to) {
                app.select_ref(n);
            }
        }
        Pane::Labels => {
            let rows = app.hits.labels_area.height as usize;
            let last = app.filter_items().len().saturating_sub(1);
            app.labels.cursor = match to.rows(rows) {
                None if to.toward_end() => last,
                None => 0,
                Some(delta) => app.labels.cursor.saturating_add_signed(delta).min(last),
            };
        }
        Pane::History => {
            // History rows run from the oldest at the top to the current
            // issue at the bottom, and `history_step(1)` moves down.
            let rows = app.history_rows_back() + 1;
            let delta = to.rows(rows).unwrap_or(if to.toward_end() { FAR } else { -FAR });
            app.history_step(delta);
        }
        Pane::Main => body(app, to),
    }
}

/// Sidebar rows are three lines each.
fn list(app: &mut App, order: Vec<u64>, area: Rect, to: Reach) {
    let rows = (area.height as usize / 3).max(1);
    let base = app.nav.hover.filter(|n| order.contains(n)).or(app.nav.current);
    if let Some(n) = target(&order, base, rows, to) {
        app.request_issue(n);
    }
}

/// The row `to` lands on, clamped to the list. `None` for an empty list.
fn target(order: &[u64], base: Option<u64>, rows_per_view: usize, to: Reach) -> Option<u64> {
    let last = order.len().checked_sub(1)?;
    let at = base.and_then(|n| order.iter().position(|row| *row == n));
    let index = match (to.rows(rows_per_view), at) {
        (None, _) => if to.toward_end() { last } else { 0 },
        (Some(delta), Some(at)) => at.saturating_add_signed(delta).min(last),
        (Some(delta), None) => if delta > 0 { 0 } else { last },
    };
    order.get(index).copied()
}

/// The Main pane scrolls its body; the Diff tab also moves its line
/// cursor so the cursor stays on screen.
fn body(app: &mut App, to: Reach) {
    let height = app.hits.body_area.height as usize;
    let end = app.hits.body_total.saturating_sub(height).min(u16::MAX as usize) as u16;
    app.nav.scroll = match to.rows(height) {
        None if to.toward_end() => end,
        None => 0,
        Some(rows) => app.nav.scroll.saturating_add_signed(rows as i16).min(end.max(app.nav.scroll)),
    };
    if app.visual_diff() && height > 0 {
        let top = app.nav.scroll as usize;
        let edge = match to {
            Reach::Top => Some(top),
            Reach::Bottom => Some(top + height - 1),
            _ => None,
        };
        let line = edge.or(app.diff.line.map(|abs| abs.clamp(top, top + height - 1)));
        if line.is_some() {
            app.diff.line = line;
            app.diff.touched = true;
        }
    }
}
