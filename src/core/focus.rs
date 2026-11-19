//! Focus model: the layer and pane types. The stored dialog stack lives
//! in `app::dialogs`; the top layer owns every key.

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
    /// Ctrl+P fuzzy finder over issues and PRs.
    Finder,
    /// Dependency map physics steppers.
    Physics,
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
}
