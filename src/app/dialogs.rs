//! The dialog stack: the one stored answer to "which dialogs and text
//! rows are open, and in what order". Each entry carries the state that
//! exists only while it is open. Long-lived data (the board cache, the
//! compose channels) stays with its own slot on `App`.

use crate::app::{App, ConfirmDialog, MenuState};
use crate::app::compose::ComposeState;
use crate::app::finder::FinderState;
use crate::core::{Dialog, Layer, TextInput};

/// Which row of the Format dialog takes typed text, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormatEdit {
    None,
    Query,
    Name,
}

/// One open dialog. Over an open board, `Format` is the board filter.
pub(crate) enum Entry {
    Board,
    Format(FormatEdit),
    Menu(MenuState),
    Finder(FinderState),
    Compose(ComposeState),
    Confirm(ConfirmDialog),
    Shortcuts,
    /// The map physics dialog, holding the focused row.
    Physics(usize),
    /// The map filter dialog, holding the focused row.
    MapFilter(usize),
    /// The settings dialog, holding the focused row.
    Settings(usize),
}

/// The kind of an entry. Declaration order is stacking order: a kind
/// opens above every kind declared before it, so a late confirm or a
/// menu never lands under the help screen or the board. The finder sits
/// above the menu and the board, but under the editor, the confirm and
/// the help: a late confirm must stay on top of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
    Board,
    Format,
    Physics,
    MapFilter,
    Settings,
    Menu,
    Finder,
    Compose,
    Confirm,
    Shortcuts,
}

impl Entry {
    fn slot(&self) -> Slot {
        match self {
            Entry::Board => Slot::Board,
            Entry::Format(_) => Slot::Format,
            Entry::Physics(_) => Slot::Physics,
            Entry::MapFilter(_) => Slot::MapFilter,
            Entry::Settings(_) => Slot::Settings,
            Entry::Menu(_) => Slot::Menu,
            Entry::Finder(_) => Slot::Finder,
            Entry::Compose(_) => Slot::Compose,
            Entry::Confirm(_) => Slot::Confirm,
            Entry::Shortcuts => Slot::Shortcuts,
        }
    }
}

impl App {
    /// Open `entry`. A kind that is already open is replaced.
    pub(crate) fn open(&mut self, entry: Entry) {
        let slot = entry.slot();
        self.dialogs.retain(|e| e.slot() != slot);
        let at = self.dialogs.partition_point(|e| e.slot() < slot);
        self.dialogs.insert(at, entry);
    }

    /// Close the entry of `slot` and hand it back.
    pub(crate) fn close(&mut self, slot: Slot) -> Option<Entry> {
        let at = self.dialogs.iter().position(|e| e.slot() == slot)?;
        Some(self.dialogs.remove(at))
    }

    pub(crate) fn is_open(&self, slot: Slot) -> bool {
        self.dialogs.iter().any(|e| e.slot() == slot)
    }

    pub(crate) fn menu(&self) -> Option<&MenuState> {
        self.dialogs.iter().find_map(|e| if let Entry::Menu(m) = e { Some(m) } else { None })
    }

    pub(crate) fn menu_mut(&mut self) -> Option<&mut MenuState> {
        self.dialogs.iter_mut().find_map(|e| if let Entry::Menu(m) = e { Some(m) } else { None })
    }

    pub(crate) fn take_menu(&mut self) -> Option<MenuState> {
        match self.close(Slot::Menu)? {
            Entry::Menu(m) => Some(m),
            _ => None,
        }
    }

    pub(crate) fn confirm(&self) -> Option<&ConfirmDialog> {
        self.dialogs.iter().find_map(|e| if let Entry::Confirm(c) = e { Some(c) } else { None })
    }

    pub(crate) fn open_confirm(&mut self, dialog: ConfirmDialog) {
        self.open(Entry::Confirm(dialog));
    }

    pub(crate) fn take_confirm(&mut self) -> Option<ConfirmDialog> {
        match self.close(Slot::Confirm)? {
            Entry::Confirm(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn open_draft(&mut self, draft: ComposeState) {
        self.open(Entry::Compose(draft));
    }

    pub(crate) fn draft(&self) -> Option<&ComposeState> {
        self.dialogs.iter().find_map(|e| if let Entry::Compose(c) = e { Some(c) } else { None })
    }

    pub(crate) fn draft_mut(&mut self) -> Option<&mut ComposeState> {
        self.dialogs.iter_mut().find_map(|e| if let Entry::Compose(c) = e { Some(c) } else { None })
    }

    pub(crate) fn shortcuts_open(&self) -> bool {
        self.is_open(Slot::Shortcuts)
    }

    pub(crate) fn format_open(&self) -> bool {
        self.is_open(Slot::Format)
    }

    /// The focused row of the open physics dialog, `None` while closed.
    pub(crate) fn physics_focus(&self) -> Option<usize> {
        self.dialogs.iter().find_map(|e| if let Entry::Physics(row) = e { Some(*row) } else { None })
    }

    pub(crate) fn set_physics_focus(&mut self, row: usize) {
        for e in &mut self.dialogs {
            if let Entry::Physics(focus) = e {
                *focus = row;
            }
        }
    }

    /// The focused row of the open map filter dialog, `None` while closed.
    pub(crate) fn map_filter_focus(&self) -> Option<usize> {
        self.dialogs.iter()
            .find_map(|e| if let Entry::MapFilter(row) = e { Some(*row) } else { None })
    }

    pub(crate) fn set_map_filter_focus(&mut self, row: usize) {
        for e in &mut self.dialogs {
            if let Entry::MapFilter(focus) = e {
                *focus = row;
            }
        }
    }

    /// The focused row of the open settings dialog, `None` while closed.
    pub(crate) fn settings_focus(&self) -> Option<usize> {
        self.dialogs.iter().find_map(|e| if let Entry::Settings(row) = e { Some(*row) } else { None })
    }

    pub(crate) fn set_settings_focus(&mut self, row: usize) {
        for e in &mut self.dialogs {
            if let Entry::Settings(focus) = e {
                *focus = row;
            }
        }
    }

    fn format_edit(&self) -> FormatEdit {
        self.dialogs.iter()
            .find_map(|e| if let Entry::Format(edit) = e { Some(*edit) } else { None })
            .unwrap_or(FormatEdit::None)
    }

    pub(crate) fn query_edit(&self) -> bool {
        self.format_edit() == FormatEdit::Query
    }

    pub(crate) fn name_edit(&self) -> bool {
        self.format_edit() == FormatEdit::Name
    }

    /// Switch the open Format dialog to another text row (or none).
    /// Without an open dialog there is nothing to edit.
    pub(crate) fn set_format_edit(&mut self, to: FormatEdit) {
        for e in &mut self.dialogs {
            if let Entry::Format(edit) = e {
                *edit = to;
            }
        }
    }

    /// Open layers, bottom to top. The one derivation of "who owns
    /// input": the `:` bar, then the stack in order. A text row sits
    /// right above its dialog. Name edit never reaches over the board.
    pub(crate) fn layer_stack(&self) -> Vec<Layer> {
        let board = self.is_open(Slot::Board);
        let mut stack = vec![Layer::Base];
        if self.nav.focus == crate::app::Focus::Bar {
            stack.push(Layer::Input(TextInput::Bar));
        }
        for entry in &self.dialogs {
            match entry {
                Entry::Board => stack.push(Layer::Dialog(Dialog::Board)),
                Entry::Format(edit) => {
                    stack.push(Layer::Dialog(if board { Dialog::BoardFilter } else { Dialog::Format }));
                    match (edit, board) {
                        (FormatEdit::Query, true) => stack.push(Layer::Input(TextInput::BoardQuery)),
                        (FormatEdit::Query, false) => stack.push(Layer::Input(TextInput::Query)),
                        (FormatEdit::Name, false) => stack.push(Layer::Input(TextInput::Name)),
                        _ => {}
                    }
                }
                Entry::Physics(_) => stack.push(Layer::Dialog(Dialog::Physics)),
                Entry::MapFilter(_) => stack.push(Layer::Dialog(Dialog::MapFilter)),
                Entry::Settings(_) => stack.push(Layer::Dialog(Dialog::Settings)),
                Entry::Menu(_) => stack.push(Layer::Dialog(Dialog::Menu)),
                Entry::Finder(_) => stack.push(Layer::Dialog(Dialog::Finder)),
                Entry::Compose(_) => stack.push(Layer::Dialog(Dialog::Compose)),
                Entry::Confirm(_) => stack.push(Layer::Dialog(Dialog::Confirm)),
                Entry::Shortcuts => stack.push(Layer::Dialog(Dialog::Shortcuts)),
            }
        }
        stack
    }
}
