//! The binding tables. One `const` array per context; `tables` maps a
//! context to the arrays searched for it. Order is match order.

use crossterm::event::KeyCode;

use super::{
    Action, Binding, Context, MainMode, Gesture, board_row_focused, board_text_focused, help_browsing, help_typing, content_search_live, main_pane, query_live, side_pane,
    sub_issue_tree, undo_live, map_card_open, map_strip_open, graph_focused,
};
use crate::core::{Dialog, Pane, TextInput};

const fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

const fn key(code: KeyCode, action: Action, label: &'static str, group: &'static str, mouse: Gesture) -> Binding {
    Binding::new(code, action, label, group, mouse)
}

/// Every reason a binding may give for having no mouse gesture. A test
/// pins `ALLOWED_NO_MOUSE`, so a new gap needs a deliberate new reason.
pub(super) const ALIAS_REASON: &str = "alias of a sibling key";
pub(super) const EDIT_REASON: &str = "text editing key";
pub(super) const SCROLL_REASON: &str = "scroll key: the wheel covers it";
pub(super) const DISMISS_REASON: &str = "dismiss key: clicking elsewhere does the same";
pub(super) const PICK_REASON: &str = "covered by clicking the options";
pub(super) const LIST_REASON: &str = "search jump: click or wheel selects the visible row";
#[cfg(test)]
pub(crate) const ALLOWED_NO_MOUSE: [&str; 6] =
    [ALIAS_REASON, EDIT_REASON, SCROLL_REASON, DISMISS_REASON, PICK_REASON, LIST_REASON];

const ALIAS: Gesture = Gesture::NoMouse(ALIAS_REASON);
const EDIT: Gesture = Gesture::NoMouse(EDIT_REASON);
const SCROLL: Gesture = Gesture::NoMouse(SCROLL_REASON);
const DISMISS: Gesture = Gesture::NoMouse(DISMISS_REASON);
const PICK: Gesture = Gesture::NoMouse(PICK_REASON);
const LIST: Gesture = Gesture::NoMouse(LIST_REASON);
const MENU: Gesture = Gesture::Menu;
const TITLE: Gesture = Gesture::Title;
const HINT: Gesture = Gesture::Hint;
const OUTSIDE: Gesture = Gesture::Outside;
const DRAG: Gesture = Gesture::Drag;

const GLOBAL: &[Binding] = &[
    key(ch('o'), Action::HistoryBack, "Back in history", "Move", MENU).ctrl(),
    key(ch('i'), Action::HistoryForward, "Forward in history", "Move", MENU).ctrl(),
    key(KeyCode::Tab, Action::HistoryForward, "Forward in history", "Move", ALIAS).ctrl(),
    key(ch('c'), Action::Quit, "Quit", "Session", ALIAS).ctrl(),
    key(ch('m'), Action::OpenMenu, "Actions menu", "Session", Gesture::RightClick),
    key(ch('?'), Action::ShowHelp, "Shortcuts", "Session", HINT).hint(1),
    key(ch('q'), Action::Quit, "Quit", "Session", HINT).hint(2),
    key(ch('u'), Action::Undo, "Undo", "Session", Gesture::Click).when(undo_live).hint(1),
    key(ch('1'), Action::TabDetails, "Details tab", "View", Gesture::Click).hint_if(7, main_pane),
    key(ch('2'), Action::TabVisual, "Diff tab", "View", Gesture::Click).hint_if(8, main_pane),
    key(ch('3'), Action::TabMap, "Map tab", "View", Gesture::Click).hint(8),
    key(ch('t'), Action::CycleTheme, "Cycle theme", "View", MENU),
    key(ch('b'), Action::OpenBoard, "Open board", "Open", HINT).hint(10),
    key(ch('p'), Action::OpenFinder, "Go to issue or PR", "Open", MENU).ctrl(),
    key(ch('c'), Action::Comment, "Comment", "Open", HINT).hint(6),
    key(ch('C'), Action::CloseItem, "Close or reopen", "Open", TITLE).hint_if(12, main_pane),
    // On the focused map, `f` opens the map filter instead (`input::actions`).
    key(ch('f'), Action::OpenFormat, "Format and filter", "Open", TITLE).hint_if(4, graph_focused),
    key(ch('F'), Action::QuerySearch, "Edit query", "Open", MENU),
    key(ch('j'), Action::MoveDown, "Select next", "Move", Gesture::Wheel).hint_if(2, side_pane),
    key(ch('k'), Action::MoveUp, "Select previous", "Move", Gesture::Wheel).hint_if(3, side_pane),
    key(KeyCode::Down, Action::MoveDown, "Select next", "Move", ALIAS),
    key(KeyCode::Up, Action::MoveUp, "Select previous", "Move", ALIAS),
    key(ch('['), Action::StepPrev, "Previous issue number", "Move", MENU),
    key(ch(']'), Action::StepNext, "Next issue number", "Move", MENU),
    key(ch('g'), Action::JumpTop, "Jump to top", "Move", SCROLL).after('g'),
    key(ch('G'), Action::JumpBottom, "Jump to bottom", "Move", SCROLL),
    key(ch('d'), Action::HalfPageDown, "Half page down", "Move", SCROLL).ctrl(),
    key(ch('u'), Action::HalfPageUp, "Half page up", "Move", SCROLL).ctrl(),
    key(KeyCode::PageDown, Action::PageDown, "Page down", "Move", SCROLL),
    key(KeyCode::PageUp, Action::PageUp, "Page up", "Move", SCROLL),
    key(KeyCode::Tab, Action::NextPane, "Next pane", "Move", Gesture::Click).hint(9),
    key(KeyCode::BackTab, Action::PrevPane, "Previous pane", "Move", ALIAS),
    key(ch(':'), Action::CommandBar, "Command", "Search", Gesture::Click).hint(11),
    key(ch('/'), Action::FindBar, "Find", "Search", MENU),
    key(ch('H'), Action::SearchGithub, "Search GitHub", "Search", HINT).hint_if(1, query_live),
    key(ch('n'), Action::NextHit, "Next match", "Search", LIST),
    key(ch('N'), Action::PrevHit, "Previous match", "Search", LIST),
    key(ch('o'), Action::OpenBrowser, "Open in browser", "Item", MENU),
    key(ch('r'), Action::Refresh, "Refresh", "Item", MENU),
    key(ch('R'), Action::Reauth, "Sign in again", "Item", MENU),
    key(ch('p'), Action::Pin, "Pin", "Item", MENU).hint_if(5, side_pane),
    key(ch('y'), Action::Yank, "Copy", "Item", MENU),
    key(ch('s'), Action::SubmitReview, "Submit pending review", "Item", MENU).ctrl(),
    key(ch('e'), Action::ToggleRefs, "Toggle references pane", "View", MENU),
    key(ch('E'), Action::ToggleSidebar, "Toggle sidebar", "View", MENU),
    key(ch(' '), Action::ToggleRow, "Expand row", "View", MENU),
    key(ch('w'), Action::MergedWindow, "Cycle merged window", "View", MENU),
    // Esc pops one layer wherever nothing nearer claims it.
    key(KeyCode::Esc, Action::Back, "Back", "Session", DISMISS),
];

/// With the Main pane focused, search keys act on its content. The
/// sidebar panes keep the row search from `GLOBAL`.
const MAIN: &[Binding] = &[
    key(ch('/'), Action::ContentFind, "Find in content", "Search", MENU),
    key(ch('n'), Action::ContentNext, "Next match", "Search", TITLE).hint_if(13, content_search_live),
    key(ch('N'), Action::ContentPrev, "Previous match", "Search", TITLE).hint_if(14, content_search_live),
];

const SIDEBAR: &[Binding] = &[
    key(ch('s'), Action::CycleSort, "Cycle sort", "Rows", TITLE).hint(3),
    key(ch('S'), Action::ReverseSort, "Reverse sort", "Rows", TITLE),
    key(ch('x'), Action::Evict, "Evict row", "Rows", MENU).hint(2),
    key(KeyCode::Enter, Action::OpenRow, "Open row", "Rows", Gesture::SecondClick).hint(1),
];

const TREE: &[Binding] = &[
    key(KeyCode::Right, Action::TreeExpand, "Expand children", "Tree", Gesture::Click).when(sub_issue_tree),
    key(KeyCode::Left, Action::TreeCollapse, "Collapse or go to parent", "Tree", Gesture::Click).when(sub_issue_tree),
    key(ch('l'), Action::TreeExpand, "Expand children", "Tree", ALIAS).when(sub_issue_tree),
    key(ch('h'), Action::TreeCollapse, "Collapse or go to parent", "Tree", ALIAS).when(sub_issue_tree),
    key(ch('z'), Action::TreeToggle, "Fold or unfold", "Tree", Gesture::Click).when(sub_issue_tree),
    key(ch('Z'), Action::TreeFoldAll, "Fold or unfold all", "Tree", MENU).when(sub_issue_tree),
];

const REFS: &[Binding] = &[
    key(KeyCode::Enter, Action::OpenRef, "Open reference", "Refs", Gesture::SecondClick).hint(1),
];

const LABELS: &[Binding] = &[
    key(KeyCode::Enter, Action::ToggleLabel, "Toggle filter", "Filters", Gesture::Click),
    key(ch(' '), Action::ToggleLabel, "Toggle filter", "Filters", ALIAS).hint(1),
    key(ch('X'), Action::ClearLabels, "Clear all filters", "Filters", Gesture::Click).hint(2),
    key(ch('d'), Action::ToggleDoneMilestones, "Hide or show done milestones", "Filters", Gesture::Click).hint(3),
];

const HISTORY: &[Binding] = &[
    key(KeyCode::Enter, Action::OpenHistoryRow, "Go to row", "History", Gesture::Click).hint(1),
];

const DIFF: &[Binding] = &[
    key(ch('J'), Action::DiffNextFile, "Next file", "Diff", TITLE).hint(1),
    key(ch('K'), Action::DiffPrevFile, "Previous file", "Diff", TITLE).hint(2),
    key(ch('j'), Action::DiffLineDown, "Next line", "Diff", Gesture::Click),
    key(KeyCode::Down, Action::DiffLineDown, "Next line", "Diff", ALIAS),
    key(ch('k'), Action::DiffLineUp, "Previous line", "Diff", Gesture::Click),
    key(KeyCode::Up, Action::DiffLineUp, "Previous line", "Diff", ALIAS),
    key(ch('h'), Action::DiffFold, "Fold file", "Diff", Gesture::Click),
    key(ch('l'), Action::DiffUnfold, "Unfold file", "Diff", Gesture::Click),
    key(ch('s'), Action::DiffSplit, "Split view", "Diff", TITLE).hint(4),
    key(ch(' '), Action::DiffToggleFile, "Expand or fold file", "Diff", Gesture::Click).hint(3),
    key(ch('a'), Action::DiffExpandAll, "Expand or fold all files", "Diff", TITLE).hint(5),
    key(ch('x'), Action::DiffContext, "Cycle context lines", "Diff", TITLE).hint(6),
];

const GRAPH: &[Binding] = &[
    key(ch('.'), Action::GraphNext, "Next node", "Select", Gesture::Click).hint(1),
    key(ch(','), Action::GraphPrev, "Previous node", "Select", Gesture::Click),
    key(KeyCode::Enter, Action::GraphOpen, "Open node", "Select", Gesture::SecondClick).hint(2),
    key(ch('i'), Action::GraphCard, "Show or hide card", "Select", Gesture::Click).hint(6),
    key(ch('>'), Action::GraphNextEdge, "Next link", "Select", Gesture::Click).hint(7),
    key(ch('<'), Action::GraphPrevEdge, "Previous link", "Select", Gesture::Click),
    key(KeyCode::Esc, Action::GraphCloseCard, "Close card", "Select", Gesture::Click)
        .when(map_card_open),
    key(ch('h'), Action::GraphPanLeft, "Pan left", "View", DRAG),
    key(ch('j'), Action::GraphPanDown, "Pan down", "View", DRAG),
    key(ch('k'), Action::GraphPanUp, "Pan up", "View", DRAG),
    key(ch('l'), Action::GraphPanRight, "Pan right", "View", DRAG),
    key(KeyCode::Left, Action::GraphPanLeft, "Pan left", "View", ALIAS),
    key(KeyCode::Down, Action::GraphPanDown, "Pan down", "View", ALIAS),
    key(KeyCode::Up, Action::GraphPanUp, "Pan up", "View", ALIAS),
    key(KeyCode::Right, Action::GraphPanRight, "Pan right", "View", ALIAS),
    key(ch('+'), Action::GraphZoomIn, "Zoom in", "View", Gesture::Wheel),
    key(ch('='), Action::GraphZoomIn, "Zoom in", "View", ALIAS),
    key(ch('-'), Action::GraphZoomOut, "Zoom out", "View", Gesture::Wheel),
    key(ch('0'), Action::GraphFit, "Fit to view", "View", TITLE).hint(4),
    key(ch('s'), Action::GraphScope, "Linked items or whole repo", "View", TITLE).hint(3),
    key(ch('A'), Action::GraphMoveLeft, "Move node left", "Arrange", DRAG),
    key(ch('S'), Action::GraphMoveDown, "Move node down", "Arrange", DRAG),
    key(ch('W'), Action::GraphMoveUp, "Move node up", "Arrange", DRAG),
    key(ch('D'), Action::GraphMoveRight, "Move node right", "Arrange", DRAG),
    key(ch('x'), Action::GraphUnpin, "Unpin node", "Arrange", MENU).hint(5),
    key(ch('P'), Action::GraphPhysics, "Tune physics", "View", TITLE).hint(8),
    key(ch('B'), Action::GraphStrip, "Show or hide ready work and bottlenecks", "Rank", TITLE).hint(9),
    key(ch('J'), Action::GraphStripNext, "Next ranked item", "Rank", Gesture::Click)
        .when(map_strip_open).hint(10),
    key(ch('K'), Action::GraphStripPrev, "Previous ranked item", "Rank", Gesture::Click)
        .when(map_strip_open),
];

const PHYSICS: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Dialog", OUTSIDE).hint(1),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Dialog", ALIAS),
    key(ch('P'), Action::ClosePhysics, "Close", "Dialog", ALIAS),
    key(ch('q'), Action::ClosePhysics, "Close", "Dialog", ALIAS),
    key(ch('j'), Action::PhysicsDown, "Next row", "Move", Gesture::Click).hint(2),
    key(ch('k'), Action::PhysicsUp, "Previous row", "Move", Gesture::Click).hint(3),
    key(KeyCode::Down, Action::PhysicsDown, "Next row", "Move", ALIAS),
    key(KeyCode::Up, Action::PhysicsUp, "Previous row", "Move", ALIAS),
    key(ch('h'), Action::PhysicsLower, "Lower value", "Value", Gesture::Click).hint(4),
    key(ch('l'), Action::PhysicsRaise, "Raise value", "Value", Gesture::Click).hint(5),
    key(KeyCode::Left, Action::PhysicsLower, "Lower value", "Value", Gesture::Wheel),
    key(KeyCode::Right, Action::PhysicsRaise, "Raise value", "Value", Gesture::Wheel),
    key(ch('r'), Action::PhysicsReset, "Reset to defaults", "Value", HINT).hint(6),
];

const MAP_FILTER: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Dialog", OUTSIDE).hint(1),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Dialog", ALIAS),
    key(ch('f'), Action::CloseMapFilter, "Close", "Dialog", ALIAS),
    key(ch('q'), Action::CloseMapFilter, "Close", "Dialog", ALIAS),
    key(ch('j'), Action::MapFilterDown, "Next row", "Move", Gesture::Wheel).hint(2),
    key(ch('k'), Action::MapFilterUp, "Previous row", "Move", Gesture::Wheel).hint(3),
    key(KeyCode::Down, Action::MapFilterDown, "Next row", "Move", ALIAS),
    key(KeyCode::Up, Action::MapFilterUp, "Previous row", "Move", ALIAS),
    key(ch(' '), Action::MapFilterToggle, "Toggle row", "Show", Gesture::Click).hint(4),
    key(KeyCode::Enter, Action::MapFilterToggle, "Toggle row", "Show", ALIAS),
    key(ch('s'), Action::MapFilterScope, "Linked items or whole repo", "Show", Gesture::Click)
        .hint(5),
    key(ch('c'), Action::MapFilterClosed, "Toggle closed items", "Show", Gesture::Click).hint(6),
    key(ch('p'), Action::MapFilterPrs, "Toggle PRs", "Show", Gesture::Click).hint(7),
    key(ch('m'), Action::MapFilterHubs, "Toggle milestone hubs", "Show", Gesture::Click).hint(8),
    key(ch('r'), Action::MapFilterReset, "Reset filter", "Show", HINT).hint(9),
];

const MENU_KEYS: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Menu", OUTSIDE).hint(1),
    key(ch('q'), Action::CloseMenu, "Close", "Menu", ALIAS),
    key(ch('j'), Action::MenuDown, "Next action", "Menu", Gesture::Wheel).hint(2),
    key(KeyCode::Down, Action::MenuDown, "Next action", "Menu", ALIAS),
    key(ch('k'), Action::MenuUp, "Previous action", "Menu", Gesture::Wheel).hint(3),
    key(KeyCode::Up, Action::MenuUp, "Previous action", "Menu", ALIAS),
    key(KeyCode::Enter, Action::MenuRun, "Run action", "Menu", Gesture::Click).hint(4),
];

const FINDER: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Finder", OUTSIDE).hint(4),
    key(KeyCode::Enter, Action::FinderRun, "Open", "Finder", Gesture::Click).hint(1),
    key(KeyCode::Up, Action::FinderUp, "Previous", "Finder", Gesture::Wheel).hint(2),
    key(KeyCode::Down, Action::FinderDown, "Next", "Finder", Gesture::Wheel).hint(3),
    key(ch('p'), Action::FinderUp, "Previous", "Finder", ALIAS).ctrl(),
    key(ch('n'), Action::FinderDown, "Next", "Finder", ALIAS).ctrl(),
    key(KeyCode::Backspace, Action::FinderBackspace, "Delete character", "Finder", EDIT).any(),
];

const SHORTCUTS: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Help", OUTSIDE).hint(1),
    // While the filter is typing, letters type; only these keys stay bound.
    key(ch('/'), Action::HelpFilter, "Filter shortcuts", "Help", HINT).hint(4).when(help_browsing),
    key(KeyCode::Enter, Action::HelpFilterDone, "Keep filter", "Help", EDIT).when(help_typing),
    key(KeyCode::Backspace, Action::HelpFilterBackspace, "Delete character", "Help", EDIT).when(help_typing),
    key(ch('?'), Action::CloseHelp, "Close", "Help", ALIAS).when(help_browsing),
    key(ch('q'), Action::CloseHelp, "Close", "Help", ALIAS).when(help_browsing),
    key(ch('j'), Action::HelpDown, "Scroll down", "Help", Gesture::Wheel).hint(2).when(help_browsing),
    key(KeyCode::Down, Action::HelpDown, "Scroll down", "Help", ALIAS),
    key(ch('k'), Action::HelpUp, "Scroll up", "Help", Gesture::Wheel).hint(3).when(help_browsing),
    key(KeyCode::Up, Action::HelpUp, "Scroll up", "Help", ALIAS),
    key(ch('g'), Action::JumpTop, "Jump to top", "Help", SCROLL).after('g').when(help_browsing),
    key(ch('G'), Action::JumpBottom, "Jump to bottom", "Help", SCROLL).when(help_browsing),
    key(ch('d'), Action::HalfPageDown, "Half page down", "Help", SCROLL).ctrl(),
    key(ch('u'), Action::HalfPageUp, "Half page up", "Help", SCROLL).ctrl(),
    key(KeyCode::PageDown, Action::PageDown, "Page down", "Help", SCROLL),
    key(KeyCode::PageUp, Action::PageUp, "Page up", "Help", SCROLL),
];

/// Choice letters are dynamic and typed through, so they are not rows.
const CONFIRM: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Cancel", "Choose", OUTSIDE).hint(1),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Choose", ALIAS),
    key(KeyCode::Enter, Action::ConfirmFirst, "Pick the first choice", "Choose", Gesture::Click).hint(2),
];

const COMPOSE: &[Binding] = &[
    key(KeyCode::Tab, Action::ComposeTab, "Insert tab", "Edit", EDIT).any(),
    key(ch('s'), Action::ComposeSend, "Send", "Send", HINT).ctrl().hint(1),
    key(ch('r'), Action::ComposeTogglePreview, "Preview", "Send", HINT).ctrl().hint(2),
    key(ch('o'), Action::ComposeEditor, "Edit in $EDITOR", "Send", HINT).ctrl().hint(3),
    key(ch('a'), Action::ComposeLineStart, "Line start", "Move", EDIT).ctrl(),
    key(ch('e'), Action::ComposeLineEnd, "Line end", "Move", EDIT).ctrl(),
    key(ch('b'), Action::ComposeLeft, "Back one character", "Move", EDIT).ctrl(),
    key(ch('f'), Action::ComposeRight, "Forward one character", "Move", EDIT).ctrl(),
    key(ch('p'), Action::ComposeUp, "Up one line", "Move", EDIT).ctrl(),
    key(ch('n'), Action::ComposeDown, "Down one line", "Move", EDIT).ctrl(),
    key(ch('h'), Action::ComposeBackspace, "Delete back", "Edit", EDIT).ctrl(),
    key(ch('d'), Action::ComposeDelete, "Delete forward", "Edit", EDIT).ctrl(),
    key(ch('k'), Action::ComposeKillEnd, "Cut to line end", "Edit", EDIT).ctrl(),
    key(ch('u'), Action::ComposeKillStart, "Cut to line start", "Edit", EDIT).ctrl(),
    key(ch('w'), Action::ComposeDeleteWordBack, "Delete word back", "Edit", EDIT).ctrl(),
    key(ch('y'), Action::ComposeYank, "Paste cut text", "Edit", EDIT).ctrl(),
    key(ch('t'), Action::ComposeTranspose, "Swap characters", "Edit", EDIT).ctrl(),
    key(ch('m'), Action::ComposeNewline, "New line", "Edit", EDIT).ctrl(),
    key(ch('j'), Action::ComposeNewline, "New line", "Edit", EDIT).ctrl(),
    key(KeyCode::Enter, Action::ComposeNewline, "New line", "Edit", EDIT).ctrl(),
    key(ch('i'), Action::ComposeTab, "Insert tab", "Edit", EDIT).ctrl(),
    key(ch('b'), Action::ComposeWordBack, "Word back", "Move", EDIT).alt(),
    key(ch('f'), Action::ComposeWordForward, "Word forward", "Move", EDIT).alt(),
    key(ch('d'), Action::ComposeDeleteWordForward, "Delete word forward", "Edit", EDIT).alt(),
    key(KeyCode::Backspace, Action::ComposeDeleteWordBack, "Delete word back", "Edit", EDIT).alt(),
    key(KeyCode::Esc, Action::Back, "Discard", "Send", HINT).hint(4),
    key(KeyCode::Enter, Action::ComposeNewline, "New line", "Edit", EDIT),
    key(KeyCode::Backspace, Action::ComposeBackspace, "Delete back", "Edit", EDIT),
    key(KeyCode::Delete, Action::ComposeDelete, "Delete forward", "Edit", EDIT),
    key(KeyCode::Home, Action::ComposeLineStart, "Line start", "Move", EDIT),
    key(KeyCode::End, Action::ComposeLineEnd, "Line end", "Move", EDIT),
    key(KeyCode::Left, Action::ComposeLeft, "Back one character", "Move", EDIT),
    key(KeyCode::Right, Action::ComposeRight, "Forward one character", "Move", EDIT),
    key(KeyCode::Up, Action::ComposeUp, "Up one line", "Move", EDIT),
    key(KeyCode::Down, Action::ComposeDown, "Down one line", "Move", EDIT),
];

const FORMAT: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Dialog", OUTSIDE).hint(1),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Dialog", ALIAS),
    key(ch('f'), Action::CloseFormat, "Close", "Dialog", ALIAS),
    key(ch('q'), Action::CloseFormat, "Close", "Dialog", ALIAS),
    key(ch('e'), Action::FormatEditQuery, "Edit query", "Query", HINT).hint(6),
    key(ch('s'), Action::FormatSaveName, "Save query", "Query", HINT).hint(7),
    key(ch('x'), Action::FormatDeleteQuery, "Delete saved query", "Query", HINT).hint(8),
    key(ch('u'), Action::Undo, "Undo", "Query", ALIAS).when(undo_live),
    key(ch('1'), Action::Window1h, "Window: 1 hour", "Window", Gesture::Click).hint(9),
    key(ch('2'), Action::Window1d, "Window: 1 day", "Window", Gesture::Click).hint(9),
    key(ch('3'), Action::Window1w, "Window: 1 week", "Window", Gesture::Click).hint(9),
    key(ch('4'), Action::WindowAll, "Window: all time", "Window", Gesture::Click).hint(9),
    key(ch('0'), Action::WindowReset, "Reset window", "Window", HINT).hint(10),
    key(ch('w'), Action::FormatCycleWindow, "Cycle window", "Window", PICK),
    key(ch('o'), Action::ToggleOpenPrs, "Toggle open PRs", "Show", Gesture::Click).hint(1),
    key(ch('m'), Action::ToggleMergedPrs, "Toggle merged PRs", "Show", Gesture::Click).hint(2),
    key(ch('i'), Action::ToggleOpenIssues, "Toggle open issues", "Show", Gesture::Click).hint(3),
    key(ch('c'), Action::ToggleClosedIssues, "Toggle closed issues", "Show", Gesture::Click).hint(4),
    key(ch('g'), Action::CycleGroup, "Cycle grouping", "Show", Gesture::Click).hint(5),
    key(ch('h'), Action::FormatColumn, "Switch column", "Move", Gesture::Click),
    key(ch('l'), Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(KeyCode::Tab, Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(KeyCode::BackTab, Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(ch('j'), Action::FormatDown, "Next row", "Move", Gesture::Wheel),
    key(ch('k'), Action::FormatUp, "Previous row", "Move", Gesture::Wheel),
    key(ch(' '), Action::FormatToggle, "Toggle row", "Move", Gesture::Click).hint(11),
    key(KeyCode::Down, Action::FormatDown, "Next row", "Move", ALIAS),
    key(KeyCode::Up, Action::FormatUp, "Previous row", "Move", ALIAS),
    key(KeyCode::Enter, Action::FormatToggle, "Toggle row", "Move", ALIAS),
];

const BOARD: &[Binding] = &[
    key(ch('q'), Action::CloseBoard, "Close board", "Board", OUTSIDE),
    key(ch('m'), Action::OpenMenu, "Actions menu", "Board", Gesture::RightClick),
    key(KeyCode::Esc, Action::Back, "Close board", "Board", HINT).hint(6),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Board", ALIAS),
    key(ch('f'), Action::BoardFilter, "Filter", "Board", HINT).hint(5),
    key(ch('X'), Action::BoardClearFilter, "Clear filter", "Board", MENU),
    key(ch('j'), Action::BoardDown, "Next card", "Move", Gesture::Wheel),
    key(KeyCode::Down, Action::BoardDown, "Next card", "Move", ALIAS),
    key(ch('k'), Action::BoardUp, "Previous card", "Move", Gesture::Wheel),
    key(KeyCode::Up, Action::BoardUp, "Previous card", "Move", ALIAS),
    key(ch('g'), Action::JumpTop, "First card", "Move", SCROLL).after('g'),
    key(ch('G'), Action::JumpBottom, "Last card", "Move", SCROLL),
    key(ch('d'), Action::HalfPageDown, "Half page down", "Move", SCROLL).ctrl(),
    key(ch('u'), Action::HalfPageUp, "Half page up", "Move", SCROLL).ctrl(),
    key(KeyCode::PageDown, Action::PageDown, "Page down", "Move", SCROLL),
    key(KeyCode::PageUp, Action::PageUp, "Page up", "Move", SCROLL),
    key(ch('h'), Action::BoardLeft, "Previous column", "Move", Gesture::Click).hint(1),
    key(KeyCode::Left, Action::BoardLeft, "Previous column", "Move", ALIAS),
    key(ch('l'), Action::BoardRight, "Next column", "Move", Gesture::Click).hint(2),
    key(KeyCode::Right, Action::BoardRight, "Next column", "Move", ALIAS),
    key(ch('t'), Action::CycleTheme, "Cycle theme", "Board", ALIAS),
    key(ch(' '), Action::BoardExpand, "Expand card", "Board", HINT).hint(3),
    key(KeyCode::Enter, Action::BoardOpen, "Open card", "Board", Gesture::SecondClick).hint(4),
];

const BOARD_FILTER: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Filter", OUTSIDE).hint(4),
    key(ch('q'), Action::Back, "Close", "Filter", ALIAS).when(board_row_focused),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Filter", ALIAS).when(board_row_focused),
    key(KeyCode::Enter, Action::BoardApply, "Apply", "Filter", HINT).hint(3),
    key(ch(' '), Action::BoardToggle, "Toggle pill or field", "Filter", Gesture::Click).when(board_row_focused).hint(2),
    key(ch('j'), Action::BoardFilterNext, "Next widget", "Move", Gesture::Click).hint(1),
    key(KeyCode::Down, Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(ch('l'), Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(KeyCode::Right, Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(ch('k'), Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Up, Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(ch('h'), Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Left, Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Filter", EDIT).any().when(board_text_focused),
];

const BAR: &[Binding] = &[
    key(ch('c'), Action::Quit, "Quit", "Bar", ALIAS).ctrl(),
    key(ch('d'), Action::Quit, "Quit", "Bar", ALIAS).ctrl(),
    key(KeyCode::Esc, Action::Back, "Cancel", "Bar", EDIT),
    key(KeyCode::Enter, Action::InputSubmit, "Run", "Bar", EDIT).any(),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Bar", EDIT).any(),
];

const QUERY: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Revert", "Field", EDIT),
    key(KeyCode::Enter, Action::InputSubmit, "Apply", "Field", EDIT).any(),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", EDIT).any(),
];

const NAME: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Cancel", "Field", EDIT),
    key(KeyCode::Enter, Action::InputSubmit, "Save", "Field", EDIT).any(),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", EDIT).any(),
];

const BOARD_QUERY: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Revert", "Field", EDIT),
    key(KeyCode::Enter, Action::InputSubmit, "Apply", "Field", EDIT).any(),
    key(KeyCode::Up, Action::InputUp, "Previous widget", "Field", EDIT),
    key(KeyCode::Down, Action::InputDown, "Next widget", "Field", EDIT),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", EDIT).any(),
];

pub(super) fn tables(context: Context) -> &'static [&'static [Binding]] {
    match context {
        Context::Global => &[GLOBAL],
        Context::Pane(Pane::Issues) => &[TREE, SIDEBAR],
        Context::Pane(Pane::PRs) => &[SIDEBAR],
        Context::Pane(Pane::Main) => &[MAIN],
        Context::Pane(Pane::Refs) => &[REFS],
        Context::Pane(Pane::Labels) => &[LABELS],
        Context::Pane(Pane::History) => &[HISTORY],
        Context::Main(MainMode::Details) => &[],
        Context::Main(MainMode::Diff) => &[DIFF],
        Context::Main(MainMode::Graph) => &[GRAPH],
        Context::Dialog(Dialog::Shortcuts) => &[SHORTCUTS],
        Context::Dialog(Dialog::Confirm) => &[CONFIRM],
        Context::Dialog(Dialog::Compose) => &[COMPOSE],
        Context::Dialog(Dialog::Menu) => &[MENU_KEYS],
        Context::Dialog(Dialog::Finder) => &[FINDER],
        Context::Dialog(Dialog::Format) => &[FORMAT],
        Context::Dialog(Dialog::Physics) => &[PHYSICS],
        Context::Dialog(Dialog::MapFilter) => &[MAP_FILTER],
        Context::Dialog(Dialog::Board) => &[BOARD],
        Context::Dialog(Dialog::BoardFilter) => &[BOARD_FILTER],
        Context::Input(TextInput::Bar) => &[BAR],
        Context::Input(TextInput::Query) => &[QUERY],
        Context::Input(TextInput::Name) => &[NAME],
        Context::Input(TextInput::BoardQuery) => &[BOARD_QUERY],
    }
}
