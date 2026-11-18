//! Mouse controls beyond rows: the actions menu and the hit areas of
//! pane-title controls and dialog-footer hints.
//!
//! Render records the areas in `ControlHits`; input reads them. Menu
//! entries come from the keymap registry, so a label and its key never
//! drift from the real binding.

use ratatui::layout::Rect;

use crate::app::App;
use crate::core::Pane;
use crate::keymap::{self, Action};

/// What a menu is about. It picks the entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuKind {
    Issue,
    Ref,
    History,
    Labels,
    Body,
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

/// Hit areas of the frame last drawn. Render rebuilds them each frame.
#[derive(Default)]
pub(crate) struct ControlHits {
    /// Title and footer controls: rect, action, pane the click focuses.
    pub(crate) controls: Vec<(Rect, Action, Pane)>,
    /// Dialog footer hints: rect plus the action a click runs.
    pub(crate) legend: Vec<(Rect, Action)>,
    /// Rect of the top dialog; a click outside it dismisses the dialog.
    pub(crate) popup: Rect,
    /// Menu rows as drawn: rect plus entry index.
    pub(crate) menu_rows: Vec<(Rect, usize)>,
    /// The repo segment of the status bar.
    pub(crate) repo: Rect,
}

fn kind_actions(kind: MenuKind, app: &App) -> Vec<Action> {
    use Action::*;
    match kind {
        MenuKind::Issue => vec![OpenRow, Pin, Yank, OpenBrowser, Comment, ToggleRow, TreeFoldAll, Evict],
        MenuKind::Ref => vec![OpenRef, Pin, Yank, OpenBrowser, Comment],
        MenuKind::History => vec![OpenHistoryRow, Pin, Yank, OpenBrowser],
        MenuKind::Labels => vec![ToggleLabel, ClearLabels],
        MenuKind::Body => {
            let mut actions = vec![Comment, Yank, Pin, OpenBrowser, Refresh, StepPrev, StepNext];
            if app.nav.current.and_then(|n| app.issues.data.get(&n)).is_some_and(|d| d.is_pr) {
                actions.push(SubmitReview);
            }
            if app.visual_graph() {
                actions.extend([GraphPrev, GraphNext, GraphOpen]);
            }
            if app.visual_diff() {
                actions.extend([DiffPrevFile, DiffNextFile, DiffSplit, DiffToggleFile]);
            }
            actions
        }
        MenuKind::Board => vec![BoardOpen, BoardExpand, BoardFilter, BoardClearFilter, CloseBoard],
        MenuKind::Global => vec![
            Refresh, CycleTheme, OpenBoard, OpenFormat, QuerySearch, FindBar, ToggleSidebar, ToggleRefs,
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

    pub(crate) fn step(&mut self, delta: isize) {
        let last = self.entries.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
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
            MenuKind::Body, MenuKind::Board, MenuKind::Global,
        ] {
            for action in kind_actions(kind, &app) {
                assert!(keymap::describe(action).is_some(), "{kind:?} lists {action:?} with no key");
            }
        }
    }
}
