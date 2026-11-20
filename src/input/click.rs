//! Left-click routing and sidebar hit-testing.

use ratatui::layout::Rect;

use super::hit::{hit, in_rect};
use super::menu;

use crate::app::{App, Focus};
use crate::core::Pane;

/// Sidebar rows are 3 lines each (`Item`) with 1-line labeled dividers,
/// below a 1-line border plus a 1-line spacer. Dividers never hit.
/// Takes the render-cached pane rows and their drawn heights by borrow
/// (no clone, no recompute): the caller holds what `render` recorded for
/// this frame.
pub(super) fn sidebar_hit(
    area: Rect,
    rows: &[crate::model::SideRow],
    heights: &[usize],
    x: u16,
    y: u16,
) -> Option<u64> {
    if !in_rect(area, x, y) || y < area.y + 2 {
        return None;
    }
    let mut rel = (y - area.y - 2) as usize;
    for (row, &h) in rows.iter().zip(heights) {
        if rel < h {
            return row.number();
        }
        rel -= h;
    }
    None
}

/// Screen rect of the sidebar row naming `target`, for the click flash.
/// Walks the same geometry as `sidebar_hit` so the flash covers exactly
/// the row the press landed on.
fn sidebar_row_rect(
    area: Rect,
    rows: &[crate::model::SideRow],
    heights: &[usize],
    target: u64,
) -> Option<Rect> {
    if area.width == 0 {
        return None;
    }
    let mut rel: u16 = 0;
    for (row, &h) in rows.iter().zip(heights) {
        let h = h as u16;
        if row.number() == Some(target) {
            return Some(Rect::new(area.x, area.y + 2 + rel, area.width, h));
        }
        rel += h;
    }
    None
}

/// Shared Left-press path. `drag` is a held-button move: side panes
/// preview the row under the cursor (transient hover, never a visit),
/// the refs pane previews the highlight without opening pairs, and
/// Main only takes focus — Scroll events keep owning the scroll, and
/// links/diff lines commit on a real press only. Both press and drag
/// record a click flash; release stays a no-op at the call site.
#[allow(clippy::too_many_lines)]
pub(super) fn handle_left(app: &mut App, x: u16, y: u16, drag: bool) {
    if in_rect(app.hits.bar_area, x, y) {
        if !drag {
            app.nav.focus = Focus::Bar;
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    if !drag {
        if let Some(pane) = menu::title_row_pane(app, x, y) {
            app.nav.focus = Focus::Main;
            app.nav.pane = pane;
            app.note_click(Rect::new(x, y, 1, 1));
            return;
        }
    }
    if !drag && in_rect(app.hits.history_area, x, y) {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::History;
        if let Some((_, steps)) = hit(&app.hits.history_rows, x, y) {
            app.nav.history_cursor = steps;
            app.open_history_row();
        }
        return;
    }
    if !drag && in_rect(app.hits.labels_area, x, y) {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Labels;
        if let Some((rect, index)) = hit(&app.hits.labels_rows, x, y) {
            app.toggle_filter_item(index);
            app.note_click(rect);
        }
        return;
    }
    // Hit-test against the render-cached pane rows by borrow: computed
    // once per frame in `render`, reused here with no clone.
    let issue_hit = sidebar_hit(
        app.hits.side_issue_area,
        &app.hits.side_issue_rows,
        &app.hits.side_issue_heights,
        x,
        y,
    );
    if let Some(n) = issue_hit {
        if !drag && app.sidebar.sub_issue_tree {
            if let Some(&crate::model::SideRow::Tree { depth, has_children: true, folded, .. }) =
                app.hits.side_issue_rows.iter().find(|row| row.number() == Some(n))
            {
                let glyph_x = app.hits.side_issue_area.x + 3 + 2 * (depth as u16);
                let rect = sidebar_row_rect(app.hits.side_issue_area, &app.hits.side_issue_rows,
                    &app.hits.side_issue_heights, n);
                if x == glyph_x && rect.is_some_and(|rect| y == rect.y) {
                    if folded {
                        app.sidebar.folded_issues.remove(&n);
                        app.load_sub_issues(n);
                    } else {
                        app.sidebar.folded_issues.insert(n);
                    }
                    app.nav.pane = Pane::Issues;
                    app.nav.focus = Focus::Main;
                    return;
                }
            }
        }
        let flash = sidebar_row_rect(
            app.hits.side_issue_area,
            &app.hits.side_issue_rows,
            &app.hits.side_issue_heights,
            n,
        );
        // A press on the already-visible row enters it like Enter (first
        // press previews, second focuses Main); a drag only hovers.
        // The focus/pane guard reads pre-click state, as before.
        if !drag
            && app.nav.focus == Focus::Main
            && app.nav.pane == Pane::Issues
            && app.nav.current == Some(n)
            && app.effective(&app.hits.side_issue_rows) == Some(n)
        {
            app.enter_item(n, Pane::Issues);
            if let Some(rect) = flash {
                app.note_click(rect);
            }
            return;
        }
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Issues;
        if drag {
            app.nav.hover = if Some(n) == app.nav.current { None } else { Some(n) };
        } else {
            app.request_issue(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    let pr_hit = sidebar_hit(
        app.hits.side_pr_area,
        &app.hits.side_pr_rows,
        &app.hits.side_pr_heights,
        x,
        y,
    );
    if let Some(n) = pr_hit {
        let flash = sidebar_row_rect(
            app.hits.side_pr_area,
            &app.hits.side_pr_rows,
            &app.hits.side_pr_heights,
            n,
        );
        if !drag
            && app.nav.focus == Focus::Main
            && app.nav.pane == Pane::PRs
            && app.nav.current == Some(n)
            && app.effective(&app.hits.side_pr_rows) == Some(n)
        {
            app.enter_item(n, Pane::PRs);
            if let Some(rect) = flash {
                app.note_click(rect);
            }
            return;
        }
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::PRs;
        if drag {
            app.nav.hover = if Some(n) == app.nav.current { None } else { Some(n) };
        } else {
            app.request_issue(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    // Mention rows only highlight their source line; paired rows
    // open their counterpart like sidebar rows.
    let ref_hit = sidebar_hit(
        app.hits.refs_area,
        &app.hits.ref_rows,
        &app.hits.ref_heights,
        x,
        y,
    );
    if let Some(n) = ref_hit {
        let flash = sidebar_row_rect(
            app.hits.refs_area,
            &app.hits.ref_rows,
            &app.hits.ref_heights,
            n,
        );
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Refs;
        if drag {
            app.refs.highlight = Some(n);
        } else if app.refs.paired.contains(&n) {
            app.open_issue(n);
            app.focus_content(Pane::Refs);
        } else {
            app.select_ref(n);
        }
        if let Some(rect) = flash {
            app.note_click(rect);
        }
        return;
    }
    if in_rect(app.hits.tabs_area, x, y) {
        if !drag {
            app.nav.pane = Pane::Main;
            let rel = x.saturating_sub(app.hits.tabs_area.x);
            app.set_tab(crate::app::tab_at(rel));
            app.note_click(Rect::new(x, y, 1, 1));
        }
        return;
    }
    // The meta/label block above the tabs resolves refs and URLs
    // exactly like body lines; only the body folds diff files.
    let in_body = in_rect(app.hits.body_area, x, y);
    let in_header = !in_body && in_rect(app.hits.header_area, x, y);
    if in_body || in_header {
        app.nav.focus = Focus::Main;
        app.nav.pane = Pane::Main;
        let area = if in_body { app.hits.body_area } else { app.hits.header_area };
        let row = (y - area.y) as usize;
        app.note_click(Rect::new(area.x, y, area.width, 1));
        if drag {
            return;
        }
        if in_body && app.visual_diff() {
            // Clicking parks the active line there; render resolves
            // the file and expands a collapsed header. Clicking an
            // expanded header folds it back instead. Other rows fall
            // through to link resolution below so refs still open.
            let abs = app.hits.body_base as usize + row;
            app.diff.line = Some(abs);
            app.diff.touched = true;
            let is_header = app.diff.headers.contains(&row);
            if is_header {
                if let Some(f) = app.diff.row_files.get(row).copied().flatten() {
                    app.diff.cursor = f;
                    let expanded = app.nav.current.and_then(|n| {
                        app.diff.files.get(&n).and_then(|fs| {
                            fs.get(f).map(|file| {
                                app.diff.expanded_files.contains(&(n, file.filename.clone()))
                            })
                        })
                    });
                    if expanded == Some(true) {
                        app.diff.touched = false;
                        app.toggle_diff_file();
                        return;
                    }
                }
                return;
            }
        }
        let line = {
            let rows = if in_body { &app.hits.body_rows } else { &app.hits.header_rows };
            rows.get(row).cloned()
        };
        if let Some(line) = line {
            let repo = app.repo_key();
            let rel_x = (x - area.x) as usize;
            let links = crate::model::links_in_line(&line, repo.as_deref());
            match crate::model::nearest_link(&line, &links, rel_x) {
                Some(crate::model::LinkTarget::Issue(n)) => {
                    app.open_issue(n);
                }
                Some(crate::model::LinkTarget::Url(url)) => open_url(&url, app),
                None => {}
            }
        }
        return;
    }
    app.nav.focus = Focus::Main;
}

/// Open a URL in the browser and say so in the bar.
pub(super) fn open_url(url: &str, app: &mut App) {
    if crate::app::open_in_browser(url).is_ok() {
        app.notify(format!("opened {url}"));
    } else {
        app.notify(format!("couldn't open {url}"));
    }
}
