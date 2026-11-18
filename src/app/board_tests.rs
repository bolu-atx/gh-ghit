//! Tests for the board filter.

use std::collections::HashSet;

use super::*;
use crate::model::BoardData;

#[test]
fn board_columns_gate_on_milestone_pills_and_text() {
    use crate::model::BoardItem;
    let card = |number: u64, milestone: Option<&str>| BoardItem {
        title: format!("card {number}"),
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
        items: vec![card(1, Some("v1")), card(2, None)],
        url: String::new(),
    };
    let numbers = |cols: Vec<(String, Vec<&BoardItem>)>| {
        cols.into_iter().flat_map(|(_, cards)| cards.into_iter().map(|c| c.number)).collect::<Vec<_>>()
    };
    assert_eq!(numbers(board_columns(&board, "", &HashSet::new())), vec![Some(1), Some(2)]);
    assert_eq!(
        numbers(board_columns(&board, "", &HashSet::from([Some("v1".to_string())]))),
        vec![Some(1)]
    );
    assert_eq!(
        numbers(board_columns(&board, "", &HashSet::from([None]))),
        vec![Some(2)]
    );
    // Text still narrows within the pill selection; milestones never
    // match the text.
    assert_eq!(
        numbers(board_columns(&board, "card 2", &HashSet::from([Some("v1".to_string())]))),
        Vec::<Option<u64>>::new()
    );
    assert_eq!(
        numbers(board_columns(&board, "v1", &HashSet::new())),
        Vec::<Option<u64>>::new()
    );
}
