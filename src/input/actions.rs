//! Action runner: maps a keymap `Action` to the `App` call that does it.
//!
//! The keymap decides which action a key means; this file decides what
//! the action does. Actions that several contexts share (`Back`,
//! `InputSubmit`, `InputBackspace`) read the top focus layer.

use crate::app::{App, FilterRow, Focus, QueryAction, Slot, Tab, board_move, board_selected_number};
use crate::core::{self, Dialog, Layer, Pane, TextInput};
use crate::keymap::Action;

use super::menu;
use super::reach::{Reach, reach};
use super::{jump_search_hit, move_diff_cursor, pane_down};

pub(super) fn perform(app: &mut App, action: Action) {
    match action {
        Action::HistoryBack => app.go_back(),
        Action::HistoryForward => app.go_forward(),
        Action::OpenMenu => menu::open_for_focus(app),
        Action::CloseMenu => { app.close(Slot::Menu); }
        Action::MenuDown => menu::step(app, 1),
        Action::MenuUp => menu::step(app, -1),
        Action::MenuRun => menu::run(app),
        Action::ShowHelp => {
            app.open(crate::app::Entry::Shortcuts);
            app.help_scroll = 0;
            app.help_filter.clear();
            app.help_typing = false;
        }
        Action::Quit => app.request_quit(),
        Action::Undo => app.undo_last(),
        Action::TabDetails => app.set_tab(Tab::Details),
        Action::TabVisual => app.set_tab(Tab::Visual),
        Action::TabMap => app.set_tab(Tab::Map),
        Action::CycleTheme => app.notify(format!("theme: {}", crate::theme::cycle())),
        Action::OpenBoard => app.open_board(),
        Action::OpenFinder => app.open_finder(),
        Action::FinderDown => app.finder_step(1),
        Action::FinderUp => app.finder_step(-1),
        Action::FinderRun => app.finder_open(),
        Action::FinderBackspace => app.finder_backspace(),
        Action::Comment => app.open_compose(),
        Action::CloseItem => app.request_set_state(),
        // `f` is global, so a map binding would shadow it: on the focused
        // map the same action opens the map filter instead.
        Action::OpenFormat if crate::keymap::graph_focused(app) => app.open_map_filter(),
        Action::OpenFormat => app.open_sidebar_format(),
        Action::QuerySearch => {
            app.open_sidebar_format();
            app.start_query_edit();
        }
        Action::SearchGithub => app.search_github(),
        Action::MoveDown => pane_down(app, 1),
        Action::MoveUp => pane_down(app, -1),
        Action::StepPrev => app.step_issue(-1),
        Action::StepNext => app.step_issue(1),
        Action::JumpTop => reach(app, Reach::Top),
        Action::JumpBottom => reach(app, Reach::Bottom),
        Action::HalfPageDown => reach(app, Reach::Half(1)),
        Action::HalfPageUp => reach(app, Reach::Half(-1)),
        Action::PageDown => reach(app, Reach::Page(1)),
        Action::PageUp => reach(app, Reach::Page(-1)),
        Action::CommandBar => {
            app.sidebar.searching = false;
            app.nav.focus = Focus::Bar;
        }
        Action::FindBar => {
            app.sidebar.searching = true;
            app.sidebar.search.clear();
            app.nav.focus = Focus::Bar;
        }
        Action::ContentFind => app.start_content_search(),
        Action::ContentNext => app.step_content_match(1),
        Action::ContentPrev => app.step_content_match(-1),
        Action::NextHit => jump_search_hit(app, 1),
        Action::PrevHit => jump_search_hit(app, -1),
        Action::OpenBrowser => open_focused(app),
        Action::Refresh => app.refresh_current(),
        Action::Reauth => app.reopen_reauth(),
        Action::Pin => app.toggle_pin(),
        Action::Yank => app.yank(),
        Action::ToggleRefs => app.toggle_refs(),
        Action::ToggleSidebar => app.toggle_sidebar(),
        Action::SubmitReview => app.submit_pending_review(),
        Action::ToggleRow => app.toggle_row_expansion(),
        Action::MergedWindow => {
            app.sections.merged_window_hours = core::cycle_window(app.sections.merged_window_hours);
            app.reload_recent();
            app.persist_prefs();
            app.notify(format!("merged window: {}", app.window_label()));
        }
        Action::Back => app.pop_layer(),
        Action::NextPane => app.cycle_pane(1),
        Action::PrevPane => app.cycle_pane(-1),

        Action::TreeFoldAll => fold_all(app),
        Action::TreeExpand | Action::TreeCollapse | Action::TreeToggle => tree_step(app, action),
        Action::CycleSort => app.cycle_sort(),
        Action::ReverseSort => app.toggle_sort_dir(),
        Action::Evict => app.evict_current(),
        Action::OpenRef => app.open_selected_ref(),
        Action::ToggleLabel => app.toggle_filter_item(app.labels.cursor),
        Action::ClearLabels => app.clear_filter_items(),
        Action::ToggleDoneMilestones => app.toggle_done_milestones(),
        Action::OpenHistoryRow => app.open_history_row(),
        Action::OpenRow => open_row(app),

        Action::DiffNextFile => app.jump_diff_file(1),
        Action::DiffPrevFile => app.jump_diff_file(-1),
        Action::DiffLineDown => move_diff_cursor(app, 1),
        Action::DiffLineUp => move_diff_cursor(app, -1),
        Action::DiffSplit => app.toggle_diff_split(),
        Action::DiffToggleFile => app.toggle_diff_file(),
        Action::DiffExpandAll => app.diff_expand_all(),
        Action::DiffContext => app.cycle_diff_context(),
        Action::DiffFold => app.set_diff_file(Some(false)),
        Action::DiffUnfold => app.set_diff_file(Some(true)),
        Action::GraphNext => app.map_step(1),
        Action::GraphPrev => app.map_step(-1),
        Action::GraphOpen => app.map_open(),
        Action::GraphPanLeft => app.map_pan(-1.0, 0.0),
        Action::GraphPanRight => app.map_pan(1.0, 0.0),
        Action::GraphPanUp => app.map_pan(0.0, -1.0),
        Action::GraphPanDown => app.map_pan(0.0, 1.0),
        Action::GraphMoveLeft => app.map_nudge(-1, 0),
        Action::GraphMoveRight => app.map_nudge(1, 0),
        Action::GraphMoveUp => app.map_nudge(0, -1),
        Action::GraphMoveDown => app.map_nudge(0, 1),
        Action::GraphZoomIn => app.map_zoom(1),
        Action::GraphZoomOut => app.map_zoom(-1),
        Action::GraphFit => app.map_fit(),
        Action::GraphUnpin => app.map_unpin(),
        Action::GraphScope => app.map_toggle_scope(),
        Action::GraphCard => app.map_toggle_card(),
        Action::GraphNextEdge => app.map_step_edge(1),
        Action::GraphPrevEdge => app.map_step_edge(-1),
        Action::GraphCloseCard => app.map_close_card(),
        Action::GraphPhysics => app.open_physics(),
        Action::GraphStrip => app.map_toggle_strip(),
        Action::GraphStripNext => app.map_strip_step(1),
        Action::GraphStripPrev => app.map_strip_step(-1),
        Action::ClosePhysics => { app.close(Slot::Physics); }
        Action::CloseMapFilter => { app.close(Slot::MapFilter); }
        Action::MapFilterDown => app.map_filter_step(1),
        Action::MapFilterUp => app.map_filter_step(-1),
        Action::MapFilterToggle => app.map_filter_toggle(),
        Action::MapFilterScope => app.map_filter_flip(FilterRow::Scope),
        Action::MapFilterClosed => app.map_filter_flip(FilterRow::Closed),
        Action::MapFilterPrs => app.map_filter_flip(FilterRow::Prs),
        Action::MapFilterHubs => app.map_filter_flip(FilterRow::Hubs),
        Action::MapFilterReset => app.map_filter_reset(),
        Action::PhysicsDown => app.physics_step(1),
        Action::PhysicsUp => app.physics_step(-1),
        Action::PhysicsLower => app.physics_nudge(-1),
        Action::PhysicsRaise => app.physics_nudge(1),
        Action::PhysicsReset => app.physics_reset(),

        Action::CloseHelp => { app.close(Slot::Shortcuts); }
        Action::HelpDown => app.help_scroll = app.help_scroll.saturating_add(1),
        Action::HelpUp => app.help_scroll = app.help_scroll.saturating_sub(1),
        Action::HelpFilter => app.help_typing = true,
        Action::HelpFilterDone => app.help_typing = false,
        Action::HelpFilterBackspace => {
            app.help_filter.pop();
            app.help_scroll = 0;
        }
        Action::ConfirmFirst => app.confirm_choose(0),

        Action::ComposeSend => app.submit_compose(),
        Action::ComposeTogglePreview => app.compose_toggle_preview(),
        Action::ComposeEditor => app.request_compose_editor(),
        Action::ComposeLineStart => app.compose_home(),
        Action::ComposeLineEnd => app.compose_end(),
        Action::ComposeLeft => app.compose_step(-1),
        Action::ComposeRight => app.compose_step(1),
        Action::ComposeUp => app.compose_line(-1),
        Action::ComposeDown => app.compose_line(1),
        Action::ComposeBackspace => app.compose_backspace(),
        Action::ComposeDelete => app.compose_delete_forward(),
        Action::ComposeKillEnd => app.compose_kill_to_end(),
        Action::ComposeKillStart => app.compose_kill_to_start(),
        Action::ComposeDeleteWordBack => app.compose_delete_word_back(),
        Action::ComposeYank => app.compose_yank(),
        Action::ComposeTranspose => app.compose_transpose(),
        Action::ComposeNewline => app.compose_insert('\n'),
        Action::ComposeTab => app.compose_insert('\t'),
        Action::ComposeWordBack => app.compose_word_back(),
        Action::ComposeWordForward => app.compose_word_forward(),
        Action::ComposeDeleteWordForward => app.compose_delete_word_forward(),

        Action::CloseFormat => { app.close(Slot::Format); }
        Action::FormatEditQuery => app.start_query_edit(),
        Action::FormatSaveName => app.start_name_edit(),
        Action::FormatDeleteQuery => app.delete_named_query(),
        Action::Window1h => app.set_window(1),
        Action::Window1d => app.set_window(24),
        Action::Window1w => app.set_window(168),
        // All time: the listing window stops limiting merged/closed rows
        // and the search path adds no `updated:` token.
        Action::WindowAll => app.set_window(0),
        Action::WindowReset => app.reset_recent_default(),
        Action::FormatCycleWindow => {
            app.set_window(core::cycle_window(app.sections.merged_window_hours));
        }
        // List toggles refetch: a hidden list is left out of the sections
        // query. They never edit the query text.
        Action::ToggleOpenPrs => app.toggle_list(0),
        Action::ToggleMergedPrs => app.toggle_list(1),
        Action::ToggleOpenIssues => app.toggle_list(2),
        Action::ToggleClosedIssues => app.toggle_list(3),
        Action::CycleGroup => {
            app.sidebar.group_by = app.sidebar.group_by.next();
            app.persist_prefs();
        }
        Action::FormatColumn => app.format_switch_column(),
        Action::FormatDown => app.format_step(1),
        Action::FormatUp => app.format_step(-1),
        Action::FormatToggle => app.toggle_format_widget(),

        Action::CloseBoard => app.close_board(),
        Action::BoardFilter => app.open_board_filter(),
        Action::BoardClearFilter => app.clear_board_filter(),
        Action::BoardDown => board_move(app, 0, 1),
        Action::BoardUp => board_move(app, 0, -1),
        Action::BoardLeft => board_move(app, -1, 0),
        Action::BoardRight => board_move(app, 1, 0),
        Action::BoardExpand => {
            let pos = (app.board.col, app.board.row);
            app.board.expanded = if app.board.expanded == Some(pos) { None } else { Some(pos) };
        }
        Action::BoardOpen => {
            if let Some(n) = board_selected_number(app) {
                app.close_board();
                app.open_issue(n);
            }
        }
        Action::BoardApply => {
            app.close(Slot::Format);
            app.board.filter_focus = 0;
            app.apply_board_filter();
        }
        Action::BoardToggle => super::toggle_board_focused(app),
        Action::BoardFilterNext => app.board_filter_step(1),
        Action::BoardFilterPrev => app.board_filter_step(-1),

        Action::InputSubmit => submit_input(app),
        Action::InputBackspace => backspace_input(app),
        Action::InputUp => {
            app.set_format_edit(crate::app::FormatEdit::None);
            app.board_filter_step(-1);
        }
        Action::InputDown => {
            app.set_format_edit(crate::app::FormatEdit::None);
            app.board_filter_step(1);
        }
    }
}

/// Enter in Issues or PRs: the selected row becomes the current item and
/// is filed, as the second click on a row does.
fn open_row(app: &mut App) {
    let order = if app.nav.pane == crate::core::Pane::PRs { app.pr_numbers() } else { app.issue_numbers() };
    let Some(n) = app.nav.hover.filter(|n| order.contains(n)).or(app.nav.current.filter(|n| order.contains(n)))
    else {
        return;
    };
    app.enter_item(n, app.nav.pane);
}

/// `Z`: fold every parent, or unfold all when any is folded.
fn fold_all(app: &mut App) {
    let (_, _, recent) = app.pane_groups(false, true);
    let all = core::tree_rows(&recent, &app.issues.data, &Default::default(), &app.subs.state);
    let parents: Vec<u64> = all.iter().filter_map(|row| match row {
        crate::model::SideRow::Tree { number, has_children: true, .. } => Some(*number),
        _ => None,
    }).collect();
    if parents.iter().any(|n| app.sidebar.folded_issues.contains(n)) {
        app.sidebar.folded_issues.clear();
    } else {
        app.sidebar.folded_issues.extend(parents);
    }
}

/// `→`, `←` and `z` act on the highlighted tree row.
fn tree_step(app: &mut App, action: Action) {
    let rows = app.issue_rows();
    let Some(n) = app.effective(&rows) else { return };
    let Some(crate::model::SideRow::Tree { has_children, folded, depth, .. }) =
        rows.iter().find(|row| row.number() == Some(n))
    else {
        return;
    };
    let (has_children, folded, depth) = (*has_children, *folded, *depth);
    match action {
        Action::TreeExpand if has_children => {
            app.sidebar.folded_issues.remove(&n);
            app.load_sub_issues(n);
        }
        // Act on the fold the row shows: an unloaded parent
        // reads folded without being in `folded_issues`.
        Action::TreeToggle if has_children => {
            if folded {
                app.sidebar.folded_issues.remove(&n);
                app.load_sub_issues(n);
            } else {
                app.sidebar.folded_issues.insert(n);
            }
        }
        Action::TreeCollapse if has_children && !folded => {
            app.sidebar.folded_issues.insert(n);
        }
        Action::TreeCollapse if depth > 0 => {
            if let Some(parent) = app.issues.data.get(&n).and_then(|d| d.parent) {
                app.request_issue(parent);
            }
        }
        _ => {}
    }
}

/// Enter in a text input or on the board filter row.
fn submit_input(app: &mut App) {
    match app.focus().layer {
        Layer::Input(TextInput::Bar) => {
            if app.search.typing {
                app.commit_content_search();
                return;
            }
            if app.sidebar.searching {
                app.commit_search();
                return;
            }
            let quit = matches!(app.bar.trim(), "q" | "quit" | "exit");
            app.submit_bar();
            if quit {
                app.request_quit();
            }
        }
        Layer::Input(TextInput::Query) => {
            match app.commit_query_draft() {
                QueryAction::Run(q) => app.run_gh_search(q),
                QueryAction::Refetch => {
                    app.reload_recent();
                    app.notify("back to recent".to_string());
                }
            }
            app.persist_prefs();
        }
        Layer::Input(TextInput::Name) => app.save_named_query(),
        Layer::Input(TextInput::BoardQuery) => {
            app.close(Slot::Format);
            app.board.filter_focus = 0;
            app.apply_board_filter();
        }
        _ => {}
    }
}

fn backspace_input(app: &mut App) {
    match app.focus().layer {
        Layer::Input(TextInput::Bar) if app.search.typing => app.content_search_pop(),
        Layer::Input(TextInput::Bar) if app.sidebar.searching => {
            app.sidebar.search.pop();
        }
        Layer::Input(TextInput::Bar) => {
            app.bar.pop();
        }
        Layer::Input(TextInput::Name) => {
            app.sidebar.name_draft.pop();
        }
        Layer::Input(TextInput::Query | TextInput::BoardQuery)
        | Layer::Dialog(Dialog::BoardFilter) => {
            app.sidebar.query_draft.pop();
        }
        _ => {}
    }
}

/// `o`: open in the browser what the focus points at: the board card,
/// the highlighted filter, reference or History row, the selected map
/// node, else the current item.
fn open_focused(app: &mut App) {
    let target = if app.focus().layer == Layer::Dialog(Dialog::Board) {
        board_selected_number(app)
    } else {
        match app.nav.pane {
            Pane::Labels => return app.open_filter_item(),
            Pane::Refs => app.refs.highlight.or(app.nav.current),
            Pane::History => app.history_row_number(),
            Pane::Main if app.map_shown() => app.map.selected_index()
                .and_then(|i| app.map.graph.nodes[i].number)
                .or(app.nav.current),
            _ => app.nav.current,
        }
    };
    match target {
        Some(n) => app.open_number(n),
        None => app.notify("view an issue first".to_string()),
    }
}
