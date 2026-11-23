use super::*;
use crate::app::compose::ComposeState;
use crate::app::confirm::{ConfirmDialog, ConfirmKind};
use crate::core::{Dialog, Layer, TextInput};
use crate::model::ItemRef;

fn app() -> App {
    App::new(Some("o/r".to_string()))
}

fn top(app: &App) -> Layer {
    *app.layer_stack().last().unwrap()
}

fn draft() -> ComposeState {
    ComposeState {
        number: ItemRef::github(1), target: crate::app::ComposeTarget::Thread, body: String::new(),
        cursor: 0, sending: false, kill: String::new(), view: Default::default(),
    }
}

fn confirm() -> ConfirmDialog {
    ConfirmDialog {
        title: "t".to_string(), body: vec![], choices: vec![], kind: ConfirmKind::Quit,
    }
}

#[test]
fn stack_orders_base_then_dialog_then_input() {
    let mut app = app();
    assert_eq!(app.layer_stack(), vec![Layer::Base]);
    app.open(Entry::Format(FormatEdit::None));
    app.set_format_edit(FormatEdit::Query);
    assert_eq!(
        app.layer_stack(),
        vec![Layer::Base, Layer::Dialog(Dialog::Format), Layer::Input(TextInput::Query)]
    );
}

#[test]
fn format_over_board_is_the_board_filter() {
    let mut app = app();
    app.open(Entry::Board);
    app.open(Entry::Format(FormatEdit::Name));
    assert_eq!(
        app.layer_stack(),
        vec![Layer::Base, Layer::Dialog(Dialog::Board), Layer::Dialog(Dialog::BoardFilter)],
        "name edit never reaches over the board"
    );
    app.set_format_edit(FormatEdit::Query);
    assert_eq!(top(&app), Layer::Input(TextInput::BoardQuery));
}

#[test]
fn dialogs_beat_the_bar_and_confirm_beats_dialogs() {
    let mut app = app();
    app.nav.focus = Focus::Bar;
    assert_eq!(top(&app), Layer::Input(TextInput::Bar));
    app.open(Entry::Compose(draft()));
    assert_eq!(top(&app), Layer::Dialog(Dialog::Compose));
    app.open(Entry::Board);
    app.open_confirm(confirm());
    assert_eq!(top(&app), Layer::Dialog(Dialog::Confirm));
}

#[test]
fn late_dialogs_keep_the_old_priority_and_a_kind_opens_once() {
    let mut app = app();
    app.open(Entry::Shortcuts);
    app.open_confirm(confirm());
    assert_eq!(top(&app), Layer::Dialog(Dialog::Shortcuts), "help stays on top");
    app.open(Entry::Board);
    assert_eq!(app.layer_stack()[1], Layer::Dialog(Dialog::Board), "board opens under the rest");
    app.open_confirm(confirm());
    assert_eq!(app.layer_stack().len(), 4, "a second confirm replaces the first");
    assert!(app.take_confirm().is_some() && app.confirm().is_none());
}
