//! The binding tables. One `const` array per context; `tables` maps a
//! context to the arrays searched for it. Order is match order.

use crossterm::event::KeyCode;

use super::{
    Action, Binding, Context, MainMode, Gesture, board_row_focused, board_text_focused, sub_issue_tree,
    undo_live,
};
use crate::core::{Dialog, Pane, TextInput};

const fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

const fn key(code: KeyCode, action: Action, label: &'static str, group: &'static str, mouse: Gesture) -> Binding {
    Binding::new(code, action, label, group, mouse)
}

const NONE: Gesture = Gesture::NoMouse("no pointer control yet");
const ALIAS: Gesture = Gesture::NoMouse("alias of a sibling key");
const MODAL: Gesture = Gesture::NoMouse("modal editor takes no pointer input");
const CLOSE: Gesture = Gesture::NoMouse("no close control; only the Format dialog closes on an outside click");

const GLOBAL: &[Binding] = &[
    key(ch('o'), Action::HistoryBack, "Back in history", "Move", NONE).ctrl(),
    key(ch('i'), Action::HistoryForward, "Forward in history", "Move", NONE).ctrl(),
    key(KeyCode::Tab, Action::HistoryForward, "Forward in history", "Move", ALIAS).ctrl(),
    key(ch('c'), Action::Quit, "Quit", "Session", NONE).ctrl(),
    key(ch('?'), Action::ShowHelp, "Show shortcuts", "Session", NONE).hint(1),
    key(ch('q'), Action::Quit, "Quit", "Session", NONE).hint(2),
    key(ch('u'), Action::Undo, "Undo", "Session", Gesture::Click).when(undo_live).hint(3),
    key(ch('1'), Action::TabDetails, "Details tab", "View", Gesture::Click),
    key(ch('2'), Action::TabVisual, "Visual tab", "View", Gesture::Click),
    key(ch('t'), Action::CycleTheme, "Cycle theme", "View", NONE),
    key(ch('b'), Action::OpenBoard, "Open board", "Open", NONE).hint(6),
    key(ch('c'), Action::Comment, "Comment", "Open", NONE),
    key(ch('f'), Action::OpenFormat, "Format and filter", "Open", NONE),
    key(ch('g'), Action::QuerySearch, "Search GitHub", "Open", NONE),
    key(ch('j'), Action::MoveDown, "Select next", "Move", Gesture::Wheel).hint(4),
    key(ch('k'), Action::MoveUp, "Select previous", "Move", Gesture::Wheel),
    key(ch('h'), Action::StepPrev, "Previous issue number", "Move", NONE),
    key(ch('l'), Action::StepNext, "Next issue number", "Move", NONE),
    key(KeyCode::Down, Action::ScrollDown, "Scroll down", "Move", Gesture::Wheel),
    key(KeyCode::Up, Action::ScrollUp, "Scroll up", "Move", Gesture::Wheel),
    key(KeyCode::PageDown, Action::PageDown, "Scroll a page down", "Move", NONE),
    key(KeyCode::PageUp, Action::PageUp, "Scroll a page up", "Move", NONE),
    key(KeyCode::Tab, Action::NextPane, "Next pane", "Move", Gesture::Click).hint(5),
    key(KeyCode::BackTab, Action::PrevPane, "Previous pane", "Move", ALIAS),
    key(ch(':'), Action::CommandBar, "Command", "Search", Gesture::Click).hint(7),
    key(ch('/'), Action::FindBar, "Find", "Search", NONE),
    key(ch('n'), Action::NextHit, "Next match", "Search", NONE),
    key(ch('N'), Action::PrevHit, "Previous match", "Search", NONE),
    key(ch('o'), Action::OpenBrowser, "Open in browser", "Item", NONE),
    key(ch('r'), Action::Refresh, "Refresh", "Item", NONE),
    key(ch('R'), Action::Reauth, "Sign in again", "Item", NONE),
    key(ch('p'), Action::Pin, "Pin", "Item", NONE),
    key(ch('y'), Action::Yank, "Copy", "Item", NONE),
    key(ch('S'), Action::SubmitReview, "Submit pending review", "Item", NONE),
    key(ch('e'), Action::ToggleRefs, "Toggle references pane", "View", NONE),
    key(ch('E'), Action::ToggleSidebar, "Toggle sidebar", "View", NONE),
    key(ch(' '), Action::ToggleRow, "Expand row", "View", NONE),
    key(ch('w'), Action::MergedWindow, "Cycle merged window", "View", NONE),
    // Esc pops one layer wherever nothing nearer claims it.
    key(KeyCode::Esc, Action::Back, "Back", "Session", NONE),
];

const SIDEBAR: &[Binding] = &[
    key(ch('a'), Action::CycleSort, "Cycle sort", "Rows", NONE),
    key(ch('A'), Action::ReverseSort, "Reverse sort", "Rows", NONE),
    key(ch('x'), Action::Evict, "Evict row", "Rows", NONE),
];

const TREE: &[Binding] = &[
    key(KeyCode::Right, Action::TreeExpand, "Expand children", "Tree", Gesture::Click).when(sub_issue_tree),
    key(KeyCode::Left, Action::TreeCollapse, "Collapse or go to parent", "Tree", NONE).when(sub_issue_tree),
    key(ch('z'), Action::TreeToggle, "Fold or unfold", "Tree", Gesture::Click).when(sub_issue_tree),
    key(ch('Z'), Action::TreeFoldAll, "Fold or unfold all", "Tree", NONE).when(sub_issue_tree),
];

const REFS: &[Binding] = &[
    key(KeyCode::Enter, Action::OpenRef, "Open reference", "Refs", Gesture::SecondClick),
];

const LABELS: &[Binding] = &[
    key(KeyCode::Enter, Action::ToggleLabel, "Toggle filter", "Filters", Gesture::Click),
    key(ch(' '), Action::ToggleLabel, "Toggle filter", "Filters", ALIAS),
    key(ch('X'), Action::ClearLabels, "Clear all filters", "Filters", NONE),
];

const HISTORY: &[Binding] = &[
    key(KeyCode::Enter, Action::OpenHistoryRow, "Go to row", "History", Gesture::Click),
];

const DIFF: &[Binding] = &[
    key(ch('J'), Action::DiffNextFile, "Next file", "Diff", NONE),
    key(ch('K'), Action::DiffPrevFile, "Previous file", "Diff", NONE),
    key(KeyCode::Down, Action::DiffLineDown, "Next line", "Diff", Gesture::Click),
    key(KeyCode::Up, Action::DiffLineUp, "Previous line", "Diff", ALIAS),
    key(ch('s'), Action::DiffSplit, "Split view", "Diff", NONE),
    key(ch(' '), Action::DiffToggleFile, "Expand or fold file", "Diff", Gesture::Click),
];

const GRAPH: &[Binding] = &[
    key(ch('j'), Action::GraphNext, "Next node", "Graph", NONE),
    key(ch('l'), Action::GraphNext, "Next node", "Graph", ALIAS),
    key(KeyCode::Down, Action::GraphNext, "Next node", "Graph", ALIAS),
    key(ch('k'), Action::GraphPrev, "Previous node", "Graph", NONE),
    key(ch('h'), Action::GraphPrev, "Previous node", "Graph", ALIAS),
    key(KeyCode::Up, Action::GraphPrev, "Previous node", "Graph", ALIAS),
    key(KeyCode::Enter, Action::GraphOpen, "Open node", "Graph", NONE),
];

const SHORTCUTS: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Help", NONE),
    key(ch('?'), Action::CloseHelp, "Close", "Help", ALIAS),
    key(ch('q'), Action::CloseHelp, "Close", "Help", ALIAS),
    key(ch('j'), Action::HelpDown, "Scroll down", "Help", Gesture::Wheel),
    key(KeyCode::Down, Action::HelpDown, "Scroll down", "Help", ALIAS),
    key(ch('k'), Action::HelpUp, "Scroll up", "Help", Gesture::Wheel),
    key(KeyCode::Up, Action::HelpUp, "Scroll up", "Help", ALIAS),
    key(KeyCode::PageDown, Action::HelpPageDown, "Scroll a page down", "Help", NONE),
    key(KeyCode::PageUp, Action::HelpPageUp, "Scroll a page up", "Help", NONE),
];

/// Choice letters are dynamic and typed through, so they are not rows.
const CONFIRM: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Cancel", "Choose", NONE),
    key(KeyCode::Enter, Action::ConfirmFirst, "Pick the first choice", "Choose", Gesture::Click),
];

const COMPOSE: &[Binding] = &[
    key(KeyCode::Tab, Action::ComposeTab, "Insert tab", "Edit", MODAL).any(),
    key(ch('s'), Action::ComposeSend, "Send", "Send", MODAL).ctrl(),
    key(ch('o'), Action::ComposeEditor, "Edit in $EDITOR", "Send", MODAL).ctrl(),
    key(ch('a'), Action::ComposeLineStart, "Line start", "Move", MODAL).ctrl(),
    key(ch('e'), Action::ComposeLineEnd, "Line end", "Move", MODAL).ctrl(),
    key(ch('b'), Action::ComposeLeft, "Back one character", "Move", MODAL).ctrl(),
    key(ch('f'), Action::ComposeRight, "Forward one character", "Move", MODAL).ctrl(),
    key(ch('p'), Action::ComposeUp, "Up one line", "Move", MODAL).ctrl(),
    key(ch('n'), Action::ComposeDown, "Down one line", "Move", MODAL).ctrl(),
    key(ch('h'), Action::ComposeBackspace, "Delete back", "Edit", MODAL).ctrl(),
    key(ch('d'), Action::ComposeDelete, "Delete forward", "Edit", MODAL).ctrl(),
    key(ch('k'), Action::ComposeKillEnd, "Cut to line end", "Edit", MODAL).ctrl(),
    key(ch('u'), Action::ComposeKillStart, "Cut to line start", "Edit", MODAL).ctrl(),
    key(ch('w'), Action::ComposeDeleteWordBack, "Delete word back", "Edit", MODAL).ctrl(),
    key(ch('y'), Action::ComposeYank, "Paste cut text", "Edit", MODAL).ctrl(),
    key(ch('t'), Action::ComposeTranspose, "Swap characters", "Edit", MODAL).ctrl(),
    key(ch('m'), Action::ComposeNewline, "New line", "Edit", MODAL).ctrl(),
    key(ch('j'), Action::ComposeNewline, "New line", "Edit", MODAL).ctrl(),
    key(KeyCode::Enter, Action::ComposeNewline, "New line", "Edit", MODAL).ctrl(),
    key(ch('i'), Action::ComposeTab, "Insert tab", "Edit", MODAL).ctrl(),
    key(ch('b'), Action::ComposeWordBack, "Word back", "Move", MODAL).alt(),
    key(ch('f'), Action::ComposeWordForward, "Word forward", "Move", MODAL).alt(),
    key(ch('d'), Action::ComposeDeleteWordForward, "Delete word forward", "Edit", MODAL).alt(),
    key(KeyCode::Backspace, Action::ComposeDeleteWordBack, "Delete word back", "Edit", MODAL).alt(),
    key(KeyCode::Esc, Action::Back, "Discard", "Send", MODAL),
    key(KeyCode::Enter, Action::ComposeNewline, "New line", "Edit", MODAL),
    key(KeyCode::Backspace, Action::ComposeBackspace, "Delete back", "Edit", MODAL),
    key(KeyCode::Delete, Action::ComposeDelete, "Delete forward", "Edit", MODAL),
    key(KeyCode::Home, Action::ComposeLineStart, "Line start", "Move", MODAL),
    key(KeyCode::End, Action::ComposeLineEnd, "Line end", "Move", MODAL),
    key(KeyCode::Left, Action::ComposeLeft, "Back one character", "Move", MODAL),
    key(KeyCode::Right, Action::ComposeRight, "Forward one character", "Move", MODAL),
    key(KeyCode::Up, Action::ComposeUp, "Up one line", "Move", MODAL),
    key(KeyCode::Down, Action::ComposeDown, "Down one line", "Move", MODAL),
];

const FORMAT: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Close", "Dialog", CLOSE),
    key(ch('f'), Action::CloseFormat, "Close", "Dialog", ALIAS),
    key(ch('q'), Action::CloseFormat, "Close", "Dialog", ALIAS),
    key(ch('e'), Action::FormatEditQuery, "Edit query", "Query", NONE),
    key(ch('s'), Action::FormatSaveName, "Save query", "Query", NONE),
    key(ch('x'), Action::FormatDeleteQuery, "Delete saved query", "Query", NONE),
    key(ch('u'), Action::Undo, "Undo", "Query", NONE).when(undo_live),
    key(ch('1'), Action::Window1h, "Window: 1 hour", "Window", Gesture::Click),
    key(ch('2'), Action::Window1d, "Window: 1 day", "Window", Gesture::Click),
    key(ch('3'), Action::Window1w, "Window: 1 week", "Window", Gesture::Click),
    key(ch('4'), Action::WindowAll, "Window: all time", "Window", Gesture::Click),
    key(ch('0'), Action::WindowReset, "Reset window", "Window", NONE),
    key(ch('w'), Action::FormatCycleWindow, "Cycle window", "Window", NONE),
    key(ch('o'), Action::ToggleOpenPrs, "Toggle open PRs", "Show", Gesture::Click),
    key(ch('m'), Action::ToggleMergedPrs, "Toggle merged PRs", "Show", Gesture::Click),
    key(ch('i'), Action::ToggleOpenIssues, "Toggle open issues", "Show", Gesture::Click),
    key(ch('c'), Action::ToggleClosedIssues, "Toggle closed issues", "Show", Gesture::Click),
    key(ch('g'), Action::CycleGroup, "Cycle grouping", "Show", Gesture::Click),
    key(ch('h'), Action::FormatColumn, "Switch column", "Move", NONE),
    key(ch('l'), Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(KeyCode::Tab, Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(KeyCode::BackTab, Action::FormatColumn, "Switch column", "Move", ALIAS),
    key(ch('j'), Action::FormatDown, "Next row", "Move", NONE),
    key(ch('k'), Action::FormatUp, "Previous row", "Move", NONE),
    key(ch(' '), Action::FormatToggle, "Toggle row", "Move", Gesture::Click),
    key(KeyCode::Down, Action::FormatDown, "Next row", "Move", ALIAS),
    key(KeyCode::Up, Action::FormatUp, "Previous row", "Move", ALIAS),
    key(KeyCode::Enter, Action::FormatToggle, "Toggle row", "Move", ALIAS),
];

const BOARD: &[Binding] = &[
    key(ch('q'), Action::CloseBoard, "Close board", "Board", NONE),
    key(KeyCode::Esc, Action::Back, "Clear filter, then close", "Board", NONE),
    key(ch('f'), Action::BoardFilter, "Filter", "Board", NONE),
    key(ch('j'), Action::BoardDown, "Next card", "Move", Gesture::Wheel),
    key(KeyCode::Down, Action::BoardDown, "Next card", "Move", ALIAS),
    key(ch('k'), Action::BoardUp, "Previous card", "Move", Gesture::Wheel),
    key(KeyCode::Up, Action::BoardUp, "Previous card", "Move", ALIAS),
    key(KeyCode::PageDown, Action::BoardPageDown, "Five cards down", "Move", NONE),
    key(KeyCode::PageUp, Action::BoardPageUp, "Five cards up", "Move", NONE),
    key(ch('h'), Action::BoardLeft, "Previous column", "Move", Gesture::Click),
    key(KeyCode::Left, Action::BoardLeft, "Previous column", "Move", ALIAS),
    key(ch('l'), Action::BoardRight, "Next column", "Move", Gesture::Click),
    key(KeyCode::Right, Action::BoardRight, "Next column", "Move", ALIAS),
    key(ch('t'), Action::CycleTheme, "Cycle theme", "Board", NONE),
    key(ch(' '), Action::BoardExpand, "Expand card", "Board", NONE),
    key(KeyCode::Enter, Action::BoardOpen, "Open card", "Board", Gesture::SecondClick),
];

const BOARD_FILTER: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Clear filter and close", "Filter", CLOSE),
    key(KeyCode::Enter, Action::BoardApply, "Apply", "Filter", NONE),
    key(ch(' '), Action::BoardToggle, "Toggle pill or field", "Filter", Gesture::Click).when(board_row_focused),
    key(ch('j'), Action::BoardFilterNext, "Next widget", "Move", Gesture::Click),
    key(KeyCode::Down, Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(ch('l'), Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(KeyCode::Right, Action::BoardFilterNext, "Next widget", "Move", ALIAS),
    key(ch('k'), Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Up, Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(ch('h'), Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Left, Action::BoardFilterPrev, "Previous widget", "Move", ALIAS),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Filter", NONE).when(board_text_focused),
];

const BAR: &[Binding] = &[
    key(ch('c'), Action::Quit, "Quit", "Bar", NONE).ctrl(),
    key(ch('d'), Action::Quit, "Quit", "Bar", ALIAS).ctrl(),
    key(KeyCode::Esc, Action::Back, "Cancel", "Bar", NONE),
    key(KeyCode::Enter, Action::InputSubmit, "Run", "Bar", NONE),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Bar", NONE),
];

const QUERY: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Revert", "Field", NONE),
    key(KeyCode::Enter, Action::InputSubmit, "Apply", "Field", NONE),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", NONE),
];

const NAME: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Cancel", "Field", NONE),
    key(KeyCode::Enter, Action::InputSubmit, "Save", "Field", NONE),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", NONE),
];

const BOARD_QUERY: &[Binding] = &[
    key(KeyCode::Esc, Action::Back, "Revert", "Field", NONE),
    key(KeyCode::Enter, Action::InputSubmit, "Apply", "Field", NONE),
    key(KeyCode::Up, Action::InputUp, "Previous widget", "Field", NONE),
    key(KeyCode::Down, Action::InputDown, "Next widget", "Field", NONE),
    key(KeyCode::Backspace, Action::InputBackspace, "Delete character", "Field", NONE),
];

pub(super) fn tables(context: Context) -> &'static [&'static [Binding]] {
    match context {
        Context::Global => &[GLOBAL],
        Context::Pane(Pane::Issues) => &[TREE, SIDEBAR],
        Context::Pane(Pane::PRs) => &[SIDEBAR],
        Context::Pane(Pane::Main) => &[],
        Context::Pane(Pane::Refs) => &[REFS],
        Context::Pane(Pane::Labels) => &[LABELS],
        Context::Pane(Pane::History) => &[HISTORY],
        Context::Main(MainMode::Details) => &[],
        Context::Main(MainMode::Diff) => &[DIFF],
        Context::Main(MainMode::Graph) => &[GRAPH],
        Context::Dialog(Dialog::Shortcuts) => &[SHORTCUTS],
        Context::Dialog(Dialog::Confirm) => &[CONFIRM],
        Context::Dialog(Dialog::Compose) => &[COMPOSE],
        Context::Dialog(Dialog::Format) => &[FORMAT],
        Context::Dialog(Dialog::Board) => &[BOARD],
        Context::Dialog(Dialog::BoardFilter) => &[BOARD_FILTER],
        Context::Input(TextInput::Bar) => &[BAR],
        Context::Input(TextInput::Query) => &[QUERY],
        Context::Input(TextInput::Name) => &[NAME],
        Context::Input(TextInput::BoardQuery) => &[BOARD_QUERY],
    }
}
