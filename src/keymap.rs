//! Keymap registry: one table of key bindings per focus context.
//!
//! Input dispatch, the `?` help screen and (later) the status-bar hints
//! all read these tables. The module is pure data plus lookup: it never
//! changes `App`, and `input` runs the `Action` a binding names.
//!
//! Resolution follows the focus layer stack. A dialog or text input
//! owns every key, so only its own table is searched. Under them the
//! base layer searches the Graph sub-mode, the focused pane, the Diff
//! sub-mode, then Global. In each table the first match wins.
//!
//! A binding may name a `prefix`: it then fires on its key only right
//! after the prefix key, as in `gg`. A key that starts a sequence and
//! has no binding of its own waits for the next key; `input` keeps the
//! pending prefix on `App`.

mod tables;

#[cfg(test)]
pub(crate) use tables::ALLOWED_NO_MOUSE;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Focus};
use crate::core::{Dialog, Layer, Pane, TextInput};

/// Declares `Action` and `Action::ALL` from one list of names.
macro_rules! actions {
    ($($name:ident),* $(,)?) => {
        /// What a binding does. `input::actions::perform` runs it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum Action { $($name),* }

        impl Action {
            #[cfg(test)]
            pub(crate) const ALL: &'static [Action] = &[$(Action::$name),*];
        }
    };
}

actions! {
    // Global.
    HistoryBack, HistoryForward, OpenMenu, ShowHelp, Quit, Undo, TabDetails, TabVisual, TabMap, CycleTheme,
    OpenBoard, OpenFinder, Comment, CloseItem, OpenFormat, QuerySearch, SearchGithub, MoveDown, MoveUp, StepPrev, StepNext,
    JumpTop, JumpBottom, HalfPageDown, HalfPageUp, PageDown, PageUp, CommandBar, FindBar, NextHit, PrevHit,
    ContentFind, ContentNext, ContentPrev,
    OpenBrowser, Refresh, Reauth, Pin, Yank, ToggleRefs, ToggleSidebar, SubmitReview,
    ToggleRow, MergedWindow, Back, NextPane, PrevPane,
    // Panes.
    TreeExpand, TreeCollapse, TreeToggle, TreeFoldAll, CycleSort, ReverseSort, Evict,
    OpenRef, ToggleLabel, ClearLabels, OpenHistoryRow, OpenRow,
    // Main sub-modes.
    DiffNextFile, DiffPrevFile, DiffLineDown, DiffLineUp, DiffSplit, DiffToggleFile, DiffExpandAll, DiffContext,
    DiffFold, DiffUnfold, GraphNext, GraphPrev, GraphOpen, GraphPanLeft, GraphPanRight, GraphPanUp,
    GraphPanDown, GraphMoveLeft, GraphMoveRight, GraphMoveUp, GraphMoveDown, GraphZoomIn, GraphZoomOut,
    GraphFit, GraphUnpin, GraphScope, GraphCard, GraphNextEdge, GraphPrevEdge, GraphCloseCard,
    GraphPhysics, GraphStrip, GraphStripNext, GraphStripPrev,
    // Map physics dialog.
    ClosePhysics, PhysicsDown, PhysicsUp, PhysicsLower, PhysicsRaise, PhysicsReset,
    // Map filter dialog.
    CloseMapFilter, MapFilterDown, MapFilterUp, MapFilterToggle, MapFilterScope, MapFilterClosed,
    MapFilterPrs, MapFilterHubs, MapFilterReset,
    // Shortcuts, confirm, compose.
    CloseHelp, HelpDown, HelpUp, HelpFilter, HelpFilterDone, HelpFilterBackspace, ConfirmFirst, CloseMenu, MenuDown, MenuUp, MenuRun,
    ComposeSend, ComposeTogglePreview, ComposeEditor, ComposeLineStart, ComposeLineEnd, ComposeLeft, ComposeRight,
    ComposeUp, ComposeDown, ComposeBackspace, ComposeDelete, ComposeKillEnd, ComposeKillStart,
    ComposeDeleteWordBack, ComposeYank, ComposeTranspose, ComposeNewline, ComposeTab,
    ComposeWordBack, ComposeWordForward, ComposeDeleteWordForward,
    // Format dialog.
    CloseFormat, FormatEditQuery, FormatSaveName, FormatDeleteQuery, Window1h, Window1d,
    Window1w, WindowAll, WindowReset, FormatCycleWindow, ToggleOpenPrs, ToggleMergedPrs,
    ToggleOpenIssues, ToggleClosedIssues, CycleGroup, FormatColumn, FormatDown, FormatUp,
    FormatToggle,
    // Board and board filter.
    CloseBoard, BoardFilter, BoardDown, BoardUp, BoardLeft,
    BoardRight, BoardExpand, BoardOpen, BoardApply, BoardToggle, BoardFilterNext,
    BoardFilterPrev, BoardClearFilter,
    // Finder.
    FinderDown, FinderUp, FinderRun, FinderBackspace,
    // Text inputs.
    InputSubmit, InputBackspace, InputUp, InputDown,
}

/// Sub-modes of the Main pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainMode {
    Details,
    Diff,
    Graph,
}

/// Where a table applies. A context is one keymap table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
    Global,
    Pane(Pane),
    Main(MainMode),
    Dialog(Dialog),
    Input(TextInput),
}

/// Modifier rule of a binding.
///
/// `Plain` ignores Shift but fails while Ctrl or Alt is held. `Ctrl` and
/// `Alt` need exactly their modifier. A strict context (see
/// `Context::strict`) also folds letter case in chords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mods {
    Plain,
    Ctrl,
    Alt,
    /// Any modifiers.
    Any,
}

/// The mouse gesture that does the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gesture {
    Click,
    /// The first press selects, the second press commits.
    SecondClick,
    Wheel,
    /// Right-click opens the actions menu.
    RightClick,
    /// The action is a row of a right-click or repo-segment menu.
    Menu,
    /// A muted control in a pane title, footer or the Main toolbar.
    Title,
    /// A clickable status-bar or dialog-footer hint.
    Hint,
    /// Press, move and release the left button.
    Drag,
    /// A click outside the layer dismisses it.
    Outside,
    /// No gesture fits. The reason is one of `ALLOWED_NO_MOUSE`.
    NoMouse(&'static str),
}

#[derive(Clone, Copy)]
pub(crate) struct Binding {
    pub(crate) key: KeyCode,
    /// Key that must come right before this one, as the first `g` of `gg`.
    pub(crate) prefix: Option<char>,
    pub(crate) mods: Mods,
    pub(crate) action: Action,
    /// Short help and hint text, imperative, sentence case.
    pub(crate) label: &'static str,
    /// Help sub-heading inside the context.
    pub(crate) group: &'static str,
    /// Status-bar hint rank; lower shows first. `None` never shows.
    pub(crate) hint: Option<u8>,
    /// Extra condition for showing the hint; the key works either way.
    pub(crate) hint_when: Option<fn(&App) -> bool>,
    pub(crate) mouse: Gesture,
    /// State beyond context. The binding is skipped while it is false.
    pub(crate) when: Option<fn(&App) -> bool>,
}

impl Binding {
    pub(crate) const fn new(
        key: KeyCode,
        action: Action,
        label: &'static str,
        group: &'static str,
        mouse: Gesture,
    ) -> Self {
        Self { key, prefix: None, mods: Mods::Plain, action, label, group, hint: None, hint_when: None, mouse, when: None }
    }

    pub(crate) const fn ctrl(mut self) -> Self {
        self.mods = Mods::Ctrl;
        self
    }

    pub(crate) const fn alt(mut self) -> Self {
        self.mods = Mods::Alt;
        self
    }

    pub(crate) const fn any(mut self) -> Self {
        self.mods = Mods::Any;
        self
    }

    pub(crate) const fn after(mut self, prefix: char) -> Self {
        self.prefix = Some(prefix);
        self
    }

    pub(crate) const fn hint(mut self, rank: u8) -> Self {
        self.hint = Some(rank);
        self
    }

    /// Like `hint`, but the hint shows only while `predicate` holds.
    pub(crate) const fn hint_if(mut self, rank: u8, predicate: fn(&App) -> bool) -> Self {
        self.hint = Some(rank);
        self.hint_when = Some(predicate);
        self
    }

    pub(crate) const fn when(mut self, predicate: fn(&App) -> bool) -> Self {
        self.when = Some(predicate);
        self
    }

    fn matches(&self, key: KeyEvent, strict: bool) -> bool {
        if !same_code(self.key, key.code, strict && self.mods != Mods::Plain) {
            return false;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match (self.mods, strict) {
            (Mods::Any, _) => true,
            (Mods::Plain, _) => !ctrl && !alt,
            (Mods::Ctrl, _) => ctrl && !alt,
            (Mods::Alt, _) => alt && !ctrl,
        }
    }

    /// Human key name for help: `ctrl-o`, `space`, `↑`.
    pub(crate) fn key_name(&self) -> String {
        let base = match self.key {
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Tab => "tab".to_string(),
            KeyCode::BackTab => "shift-tab".to_string(),
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Delete => "delete".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::PageUp => "page up".to_string(),
            KeyCode::PageDown => "page down".to_string(),
            other => format!("{other:?}").to_lowercase(),
        };
        let base = match self.prefix {
            Some(prefix) => format!("{prefix}{base}"),
            None => base,
        };
        match self.mods {
            Mods::Ctrl => format!("ctrl-{base}"),
            Mods::Alt => format!("alt-{base}"),
            Mods::Plain | Mods::Any => base,
        }
    }
}

fn same_code(want: KeyCode, got: KeyCode, fold_case: bool) -> bool {
    match (want, got) {
        (KeyCode::Char(a), KeyCode::Char(b)) if fold_case => a.eq_ignore_ascii_case(&b),
        _ => want == got,
    }
}

impl Context {
    /// Strict contexts need exact modifiers and fold letter case in
    /// chords. The comment editor is the one: every Ctrl or Alt chord
    /// there is an editing key, never a plain key in disguise.
    pub(crate) fn strict(self) -> bool {
        self == Context::Dialog(Dialog::Compose)
    }

    /// Whether printable keys type into this context unless a binding
    /// claims them first.
    #[cfg(test)]
    pub(crate) fn types_text(self) -> bool {
        matches!(
            self,
            Context::Input(_) | Context::Dialog(Dialog::Compose | Dialog::Confirm | Dialog::BoardFilter | Dialog::Finder)
        )
    }

    /// Help section title.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Context::Global => "Everywhere",
            Context::Pane(Pane::Issues) => "Issues pane",
            Context::Pane(Pane::PRs) => "Pull requests pane",
            Context::Pane(Pane::Main) => "Main pane",
            Context::Pane(Pane::Refs) => "References pane",
            Context::Pane(Pane::Labels) => "Filters pane",
            Context::Pane(Pane::History) => "History pane",
            Context::Main(MainMode::Details) => "Details tab",
            Context::Main(MainMode::Diff) => "Diff tab",
            Context::Main(MainMode::Graph) => "Map tab",
            Context::Dialog(Dialog::Shortcuts) => "Shortcuts",
            Context::Dialog(Dialog::Confirm) => "Confirm dialog",
            Context::Dialog(Dialog::Compose) => "Comment editor",
            Context::Dialog(Dialog::Menu) => "Actions menu",
            Context::Dialog(Dialog::Finder) => "Finder",
            Context::Dialog(Dialog::Format) => "Format dialog",
            Context::Dialog(Dialog::Physics) => "Map physics",
            Context::Dialog(Dialog::MapFilter) => "Map filter",
            Context::Dialog(Dialog::Board) => "Board",
            Context::Dialog(Dialog::BoardFilter) => "Board filter",
            Context::Input(TextInput::Bar) => "Command and search bar",
            Context::Input(TextInput::Query) => "Query field",
            Context::Input(TextInput::Name) => "Query name field",
            Context::Input(TextInput::BoardQuery) => "Board query field",
        }
    }
}

/// Every context, in help order.
pub(crate) const ALL_CONTEXTS: [Context; 24] = [
    Context::Global,
    Context::Pane(Pane::Issues),
    Context::Pane(Pane::PRs),
    Context::Pane(Pane::Main),
    Context::Pane(Pane::Refs),
    Context::Pane(Pane::Labels),
    Context::Pane(Pane::History),
    Context::Main(MainMode::Details),
    Context::Main(MainMode::Diff),
    Context::Main(MainMode::Graph),
    Context::Dialog(Dialog::Shortcuts),
    Context::Dialog(Dialog::Confirm),
    Context::Dialog(Dialog::Compose),
    Context::Dialog(Dialog::Menu),
    Context::Dialog(Dialog::Finder),
    Context::Dialog(Dialog::Format),
    Context::Dialog(Dialog::Physics),
    Context::Dialog(Dialog::MapFilter),
    Context::Dialog(Dialog::Board),
    Context::Dialog(Dialog::BoardFilter),
    Context::Input(TextInput::Bar),
    Context::Input(TextInput::Query),
    Context::Input(TextInput::Name),
    Context::Input(TextInput::BoardQuery),
];

/// The tables of one context, in match order.
pub(crate) fn tables(context: Context) -> &'static [&'static [Binding]] {
    tables::tables(context)
}

/// Every binding of a context, in match order.
pub(crate) fn bindings(context: Context) -> impl Iterator<Item = &'static Binding> {
    tables(context).iter().flat_map(|table| table.iter())
}

/// The binding that names `action` in help and menus: the first plain
/// key, else the first binding of any kind.
pub(crate) fn describe(action: Action) -> Option<&'static Binding> {
    let all = || ALL_CONTEXTS.into_iter()
        .filter(|context| *context != Context::Dialog(Dialog::Menu))
        .flat_map(bindings)
        .filter(move |b| b.action == action);
    all().find(|b| b.mods == Mods::Plain && b.prefix.is_none()).or_else(|| all().next())
}

/// Graph keys act only while the Map tab has main focus.
pub(crate) fn graph_focused(app: &App) -> bool {
    app.map_shown() && app.nav.pane == Pane::Main && app.nav.focus == Focus::Main
}

/// Contexts that own keys right now, top first. A dialog or text input
/// is modal, so it stands alone. The base layer stacks sub-modes and
/// the focused pane over Global.
pub(crate) fn active_contexts(app: &App) -> Vec<Context> {
    match app.focus().layer {
        Layer::Dialog(dialog) => vec![Context::Dialog(dialog)],
        Layer::Input(input) => vec![Context::Input(input)],
        Layer::Base => base_contexts(app),
    }
}

/// Contexts under the dialogs, top first. The help screen reads this to
/// find the context it was opened from.
pub(crate) fn base_contexts(app: &App) -> Vec<Context> {
    let mut chain = vec![];
    if graph_focused(app) {
        chain.push(Context::Main(MainMode::Graph));
    }
    chain.push(Context::Pane(app.nav.pane));
    if app.visual_diff() && app.nav.pane == Pane::Main {
        chain.push(Context::Main(MainMode::Diff));
    } else if app.nav.pane == Pane::Main && !graph_focused(app) {
        chain.push(Context::Main(MainMode::Details));
    }
    chain.push(Context::Global);
    chain
}

/// First binding that takes `key` after `prefix`, searching `chain` top down.
fn find(chain: &[Context], key: KeyEvent, prefix: Option<char>, app: &App) -> Option<&'static Binding> {
    chain.iter().find_map(|context| {
        bindings(*context).find(|binding| {
            binding.prefix == prefix
                && binding.matches(key, context.strict())
                && binding.when.is_none_or(|test| test(app))
        })
    })
}

/// First binding that takes `key` on its own, searching `chain` top down.
pub(crate) fn lookup(chain: &[Context], key: KeyEvent, app: &App) -> Option<&'static Binding> {
    find(chain, key, None, app)
}

/// The binding that `key` completes after `prefix` was pressed.
pub(crate) fn lookup_after(
    chain: &[Context],
    prefix: char,
    key: KeyEvent,
    app: &App,
) -> Option<&'static Binding> {
    find(chain, key, Some(prefix), app)
}

/// The prefix `key` opens: a plain character that some live binding in
/// `chain` waits on. Call it only after `lookup` found nothing.
pub(crate) fn sequence_start(chain: &[Context], key: KeyEvent, app: &App) -> Option<char> {
    let KeyCode::Char(c) = key.code else { return None };
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    chain.iter()
        .flat_map(|context| bindings(*context))
        .any(|binding| binding.prefix == Some(c) && binding.when.is_none_or(|test| test(app)))
        .then_some(c)
}

/// One line of the help screen: the keys that run one action.
#[derive(Clone)]
pub(crate) struct HelpRow {
    pub(crate) group: &'static str,
    pub(crate) keys: String,
    pub(crate) label: &'static str,
    /// The matching mouse gesture, when one exists today.
    pub(crate) mouse: Option<&'static str>,
}

impl Gesture {
    /// Help wording of the gesture; `None` when there is no gesture.
    pub(crate) fn gesture(self) -> Option<&'static str> {
        match self {
            Gesture::Click => Some("click"),
            Gesture::SecondClick => Some("click twice"),
            Gesture::Wheel => Some("wheel"),
            Gesture::RightClick => Some("right-click"),
            Gesture::Menu => Some("right-click menu"),
            Gesture::Title => Some("click title"),
            Gesture::Hint => Some("click hint"),
            Gesture::Drag => Some("drag"),
            Gesture::Outside => Some("click outside"),
            Gesture::NoMouse(_) => None,
        }
    }
}

/// Help rows of a context. Keys that run the same action share a row.
pub(crate) fn help_rows(context: Context) -> Vec<HelpRow> {
    let mut rows: Vec<(Action, HelpRow)> = vec![];
    for binding in bindings(context) {
        match rows.iter_mut().find(|(action, _)| *action == binding.action) {
            Some((_, row)) => {
                row.keys.push_str(" / ");
                row.keys.push_str(&binding.key_name());
                row.mouse = row.mouse.or(binding.mouse.gesture());
            }
            None => rows.push((binding.action, HelpRow {
                group: binding.group,
                keys: binding.key_name(),
                label: binding.label,
                mouse: binding.mouse.gesture(),
            })),
        }
    }
    // Rows of one group sit together, groups in order of first use.
    let mut groups: Vec<&'static str> = vec![];
    for (_, row) in &rows {
        if !groups.contains(&row.group) {
            groups.push(row.group);
        }
    }
    let mut ordered = Vec::with_capacity(rows.len());
    for group in groups {
        ordered.extend(rows.iter().filter(|(_, row)| row.group == group).map(|(_, row)| row));
    }
    ordered.into_iter().cloned().collect()
}

/// Contexts for the help screen: the ones the screen was opened from,
/// top first, then every other context. Contexts with no keys are left out.
pub(crate) fn help_sections(app: &App) -> Vec<(Context, bool)> {
    let below = app.layer_stack().into_iter()
        .rfind(|layer| *layer != Layer::Dialog(Dialog::Shortcuts));
    let current = match below {
        Some(Layer::Dialog(dialog)) => vec![Context::Dialog(dialog)],
        Some(Layer::Input(input)) => vec![Context::Input(input)],
        _ => base_contexts(app),
    };
    let rest = ALL_CONTEXTS.into_iter().filter(|context| !current.contains(context));
    current.iter().map(|context| (*context, true))
        .chain(rest.map(|context| (context, false)))
        .filter(|(context, _)| bindings(*context).next().is_some())
        .collect()
}

fn main_pane(app: &App) -> bool {
    app.nav.pane == Pane::Main
}

/// A content search has a query, so `n`/`N` have somewhere to go.
fn content_search_live(app: &App) -> bool {
    app.search.active()
}

/// A `/` query is active, so there is something to re-run on GitHub.
fn query_live(app: &App) -> bool {
    !app.sidebar.query_active.trim().is_empty()
}

fn side_pane(app: &App) -> bool {
    app.nav.pane != Pane::Main
}

/// A card shows on the map, so `Esc` closes it first.
fn map_card_open(app: &App) -> bool {
    app.map.card.is_some()
}

/// The ranking strip shows on the map, so its row keys act.
fn map_strip_open(app: &App) -> bool {
    app.map.strip
}

fn undo_live(app: &App) -> bool {
    app.undo_live()
}

fn sub_issue_tree(app: &App) -> bool {
    app.sidebar.sub_issue_tree
}

fn board_row_focused(app: &App) -> bool {
    app.board.filter_focus > 0
}

fn board_text_focused(app: &App) -> bool {
    app.board.filter_focus == 0
}

fn help_browsing(app: &App) -> bool {
    !app.help_typing
}

fn help_typing(app: &App) -> bool {
    app.help_typing
}

#[cfg(test)]
mod tests;
