//! The actions menu state. It lives with `App` because `ui` draws it and
//! `input` drives it. Entries come from the keymap registry, so a label
//! and its key never drift from the real binding.

use crate::app::App;
use crate::keymap::{self, Action};

/// What a menu is about. It picks the entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Issue,
    Ref,
    History,
    Labels,
    Body,
    /// The dependency map: its node under the pointer, or the view.
    Map,
    Board,
    Global,
}

pub(crate) struct MenuEntry {
    pub(crate) action: Action,
    pub(crate) label: &'static str,
    pub(crate) key: String,
}

pub(crate) struct MenuState {
    pub(crate) entries: Vec<MenuEntry>,
    pub(crate) cursor: usize,
    /// Top-left cell the popup wants; render clamps it into the frame.
    pub(crate) anchor: (u16, u16),
}

fn kind_actions(kind: MenuKind, app: &App) -> Vec<Action> {
    use Action::*;
    match kind {
        MenuKind::Issue => vec![OpenRow, Pin, Yank, OpenBrowser, Comment, ToggleRow, TreeFoldAll, Evict],
        MenuKind::Ref => vec![OpenRef, Pin, Yank, OpenBrowser, Comment],
        MenuKind::History => vec![OpenHistoryRow, Pin, Yank, OpenBrowser],
        MenuKind::Labels => vec![ToggleLabel, ClearLabels],
        MenuKind::Body => {
            let mut actions = vec![ContentFind, Comment, CloseItem, Yank, Pin, OpenBrowser, Refresh, StepPrev, StepNext];
            if app.nav.current.and_then(|n| app.issues.data.get(&n)).is_some_and(|d| d.is_pr) {
                actions.push(SubmitReview);
            }
            if app.visual_diff() {
                actions.extend([DiffPrevFile, DiffNextFile, DiffSplit, DiffToggleFile, DiffExpandAll, DiffContext]);
            }
            actions
        }
        MenuKind::Map => vec![
            GraphOpen, GraphNext, GraphPrev, GraphUnpin, GraphScope, GraphFit, GraphZoomIn, GraphZoomOut,
            Comment, Refresh, OpenBrowser,
        ],
        MenuKind::Board => vec![BoardOpen, BoardExpand, BoardFilter, BoardClearFilter, CloseBoard],
        MenuKind::Global => vec![
            Refresh, CycleTheme, OpenBoard, OpenFinder, OpenFormat, QuerySearch, FindBar, ToggleSidebar, ToggleRefs,
            MergedWindow, HistoryBack, HistoryForward, ShowHelp, Reauth, Quit,
        ],
    }
}

impl MenuState {
    /// A menu of `kind` at `anchor`. Entries whose binding is not live
    /// right now (its `when` fails) are left out.
    pub(crate) fn new(kind: MenuKind, app: &App, anchor: (u16, u16)) -> Self {
        let entries = kind_actions(kind, app).into_iter()
            .filter_map(|action| {
                let binding = keymap::describe(action)?;
                binding.when.is_none_or(|test| test(app)).then(|| MenuEntry {
                    action,
                    label: binding.label,
                    key: binding.key_name(),
                })
            })
            .collect();
        Self { entries, cursor: 0, anchor }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_action_has_a_binding_and_a_menu_gesture() {
        let app = App::new(Some("o/r".to_string()));
        for kind in [
            MenuKind::Issue, MenuKind::Ref, MenuKind::History, MenuKind::Labels,
            MenuKind::Body, MenuKind::Map, MenuKind::Board, MenuKind::Global,
        ] {
            for action in kind_actions(kind, &app) {
                assert!(keymap::describe(action).is_some(), "{kind:?} lists {action:?} with no key");
            }
        }
    }
}
