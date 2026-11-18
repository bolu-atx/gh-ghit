//! Focus model: which layer owns input, and which pane is active.
//!
//! Input and render read ONE derivation, `layer_stack`, instead of
//! re-checking scattered flags. The stack runs bottom to top: base panes,
//! then dialogs, then text inputs. The top layer owns every key.

/// The panes that can take focus, in screen order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Issues,
    PRs,
    Main,
    Refs,
    Labels,
    History,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialog {
    Shortcuts,
    Confirm,
    Compose,
    /// Sidebar Format dialog.
    Format,
    Board,
    /// Format form drawn over the board.
    BoardFilter,
    /// Right-click or repo-segment actions menu.
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextInput {
    /// The `:` / `/` bar.
    Bar,
    /// Query row of the Format dialog.
    Query,
    /// Saved-query name row of the Format dialog.
    Name,
    /// Text row of the board filter form.
    BoardQuery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    Base,
    Dialog(Dialog),
    Input(TextInput),
}

/// The stored flags the stack derives from.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayerFlags {
    pub(crate) bar: bool,
    pub(crate) board: bool,
    /// Format dialog open (over the board, it is the board filter).
    pub(crate) format: bool,
    pub(crate) query_edit: bool,
    pub(crate) name_edit: bool,
    pub(crate) compose: bool,
    pub(crate) menu: bool,
    pub(crate) confirm: bool,
    pub(crate) shortcuts: bool,
}

/// Open layers, bottom (`Base`) to top. Order mirrors key priority: the
/// bar sits lowest, any dialog beats it, the menu opens over the board
/// and format dialogs, and confirm/shortcuts beat all.
pub(crate) fn layer_stack(f: &LayerFlags) -> Vec<Layer> {
    let mut stack = vec![Layer::Base];
    if f.bar {
        stack.push(Layer::Input(TextInput::Bar));
    }
    if f.board {
        stack.push(Layer::Dialog(Dialog::Board));
    }
    if f.format {
        stack.push(Layer::Dialog(if f.board { Dialog::BoardFilter } else { Dialog::Format }));
        if f.board && f.query_edit {
            stack.push(Layer::Input(TextInput::BoardQuery));
        } else if !f.board && f.name_edit {
            stack.push(Layer::Input(TextInput::Name));
        } else if !f.board && f.query_edit {
            stack.push(Layer::Input(TextInput::Query));
        }
    }
    if f.menu {
        stack.push(Layer::Dialog(Dialog::Menu));
    }
    if f.compose {
        stack.push(Layer::Dialog(Dialog::Compose));
    }
    if f.confirm {
        stack.push(Layer::Dialog(Dialog::Confirm));
    }
    if f.shortcuts {
        stack.push(Layer::Dialog(Dialog::Shortcuts));
    }
    stack
}

/// Which panes are on screen.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaneVisibility {
    pub(crate) sidebar: bool,
    pub(crate) refs: bool,
    pub(crate) labels: bool,
    pub(crate) history: bool,
}

/// Visible panes in screen order. Main is always visible.
pub(crate) fn visible_panes(v: &PaneVisibility) -> Vec<Pane> {
    let mut order = vec![];
    if v.sidebar {
        order.extend([Pane::Issues, Pane::PRs]);
    }
    order.push(Pane::Main);
    for (shown, pane) in [(v.refs, Pane::Refs), (v.labels, Pane::Labels), (v.history, Pane::History)] {
        if shown {
            order.push(pane);
        }
    }
    order
}

/// Fixed widths of the side columns.
pub(crate) const SIDEBAR_WIDTH: u16 = 44;
pub(crate) const RIGHT_WIDTH: u16 = 36;
/// Narrowest main pane. The issue title and body stay readable at this
/// width; below it the body turns into a vertical strip.
pub(crate) const MIN_MAIN_WIDTH: u16 = 40;

/// Widths of the sidebar and right column for a terminal `total` columns
/// wide. A column that is not wanted gets 0. When main would fall under
/// `MIN_MAIN_WIDTH`, the right column drops first, then the sidebar.
pub(crate) fn columns(total: u16, sidebar_wanted: bool, right_wanted: bool) -> (u16, u16) {
    let mut sidebar = if sidebar_wanted { SIDEBAR_WIDTH } else { 0 };
    let mut right = if right_wanted { RIGHT_WIDTH } else { 0 };
    if total < sidebar + right + MIN_MAIN_WIDTH {
        right = 0;
    }
    if total < sidebar + right + MIN_MAIN_WIDTH {
        sidebar = 0;
    }
    (sidebar, right)
}

/// Next pane in `order` after `current`, wrapping. `delta` is 1 for Tab
/// and -1 for Shift-Tab. A pane that is not in `order` falls to `Main`.
pub(crate) fn step_pane(order: &[Pane], current: Pane, delta: isize) -> Pane {
    let Some(at) = order.iter().position(|p| *p == current) else {
        return Pane::Main;
    };
    let len = order.len() as isize;
    order[(at as isize + delta).rem_euclid(len) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: PaneVisibility =
        PaneVisibility { sidebar: true, refs: true, labels: true, history: true };

    #[test]
    fn stack_orders_base_then_dialog_then_input() {
        let flags = LayerFlags { format: true, query_edit: true, ..Default::default() };
        assert_eq!(
            layer_stack(&flags),
            vec![Layer::Base, Layer::Dialog(Dialog::Format), Layer::Input(TextInput::Query)]
        );
        assert_eq!(layer_stack(&LayerFlags::default()), vec![Layer::Base]);
    }

    #[test]
    fn format_over_board_is_the_board_filter() {
        let flags = LayerFlags { board: true, format: true, name_edit: true, ..Default::default() };
        assert_eq!(
            layer_stack(&flags),
            vec![Layer::Base, Layer::Dialog(Dialog::Board), Layer::Dialog(Dialog::BoardFilter)],
            "name edit never reaches over the board"
        );
    }

    #[test]
    fn dialogs_beat_the_bar_and_confirm_beats_dialogs() {
        let flags = LayerFlags { bar: true, board: true, confirm: true, ..Default::default() };
        assert_eq!(layer_stack(&flags).last(), Some(&Layer::Dialog(Dialog::Confirm)));
        let flags = LayerFlags { bar: true, compose: true, ..Default::default() };
        assert_eq!(layer_stack(&flags).last(), Some(&Layer::Dialog(Dialog::Compose)));
        let flags = LayerFlags { bar: true, ..Default::default() };
        assert_eq!(layer_stack(&flags).last(), Some(&Layer::Input(TextInput::Bar)));
    }

    #[test]
    fn menu_opens_over_the_board_and_under_confirm() {
        let flags = LayerFlags { board: true, menu: true, ..Default::default() };
        assert_eq!(layer_stack(&flags).last(), Some(&Layer::Dialog(Dialog::Menu)));
        let flags = LayerFlags { menu: true, confirm: true, ..Default::default() };
        assert_eq!(layer_stack(&flags).last(), Some(&Layer::Dialog(Dialog::Confirm)));
    }

    #[test]
    fn tab_walks_screen_order_and_wraps() {
        let order = visible_panes(&ALL);
        assert_eq!(
            order,
            vec![Pane::Issues, Pane::PRs, Pane::Main, Pane::Refs, Pane::Labels, Pane::History]
        );
        assert_eq!(step_pane(&order, Pane::PRs, 1), Pane::Main);
        assert_eq!(step_pane(&order, Pane::History, 1), Pane::Issues);
        assert_eq!(step_pane(&order, Pane::Issues, -1), Pane::History);
    }

    #[test]
    fn hidden_panes_leave_the_cycle() {
        let order = visible_panes(&PaneVisibility { sidebar: false, refs: false, labels: true, history: false });
        assert_eq!(order, vec![Pane::Main, Pane::Labels]);
        assert_eq!(step_pane(&order, Pane::Refs, 1), Pane::Main, "an unseen pane falls to Main");
        let solo = visible_panes(&PaneVisibility { sidebar: false, refs: false, labels: false, history: false });
        assert_eq!(step_pane(&solo, Pane::Main, 1), Pane::Main);
    }

    #[test]
    fn narrow_terminal_drops_right_then_sidebar() {
        let full = SIDEBAR_WIDTH + RIGHT_WIDTH + MIN_MAIN_WIDTH;
        assert_eq!(columns(full, true, true), (SIDEBAR_WIDTH, RIGHT_WIDTH));
        assert_eq!(columns(full - 1, true, true), (SIDEBAR_WIDTH, 0), "right drops first");
        let both = SIDEBAR_WIDTH + MIN_MAIN_WIDTH;
        assert_eq!(columns(both, true, true), (SIDEBAR_WIDTH, 0));
        assert_eq!(columns(both - 1, true, true), (0, 0), "then the sidebar");
        assert_eq!(columns(80, true, false), (0, 0));
        assert_eq!(columns(200, false, true), (0, RIGHT_WIDTH), "unwanted stays 0");
        assert_eq!(columns(MIN_MAIN_WIDTH + RIGHT_WIDTH - 1, false, true), (0, 0));
    }
}
