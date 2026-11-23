//! Tests for the board filter.

use std::collections::HashSet;

use super::*;
use ghit_core::model::BoardData;
use ghit_core::model::ItemRef;

#[test]
fn a_failed_board_load_closes_the_board() {
    let mut app = super::test_support::sidebar_app(&[ItemRef::github(1)]);
    let (tx, rx) = std::sync::mpsc::channel();
    app.open(Entry::Board);
    app.session.board.loading = true;
    app.session.board.rx = Some(rx);
    tx.send(Err("boom".to_string())).unwrap();
    assert!(poll_session(&mut app));
    assert!(!board_open(&app));
}

#[test]
fn board_columns_gate_on_milestone_pills_and_text() {
    use ghit_core::model::BoardItem;
    let card = |number: ItemRef, milestone: Option<&str>| BoardItem {
        title: format!("card {number}", number = number.number),
        number: Some(number),
        status: Some("Todo".to_string()),
        labels: vec![],
        repo: None,
        milestone: milestone.map(str::to_string),
        content: serde_json::Value::Null,
    };
    let board = BoardData {
        title: "b".to_string(),
        order: vec!["Todo".to_string()],
        items: vec![
            card(ItemRef::github(1), Some("v1")),
            card(ItemRef::github(2), None),
        ],
        url: String::new(),
    };
    let numbers = |cols: Vec<(String, Vec<&BoardItem>)>| {
        cols.into_iter().flat_map(|(_, cards)| cards.into_iter().map(|c| c.number)).collect::<Vec<_>>()
    };
    assert_eq!(
        numbers(board_columns(&board, "", &HashSet::from([Some("v1".to_string())]))),
        vec![Some(ItemRef::github(1))]
    );
    assert_eq!(
        numbers(board_columns(&board, "", &HashSet::from([None]))),
        vec![Some(ItemRef::github(2))]
    );
    // Text still narrows within the pill selection; milestones never
    // match the text.
    assert_eq!(
        numbers(board_columns(&board, "card 2", &HashSet::from([Some("v1".to_string())]))),
        Vec::<Option<ItemRef>>::new()
    );
    assert_eq!(
        numbers(board_columns(&board, "v1", &HashSet::new())),
        Vec::<Option<ItemRef>>::new()
    );
}
