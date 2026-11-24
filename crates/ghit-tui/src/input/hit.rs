//! Hit testing: the one place mouse cells meet recorded rects.

use ratatui::layout::{Position, Rect};

/// A hit-area entry that starts with its rect.
pub(super) trait Located {
    fn rect(&self) -> Rect;
}

impl<A> Located for (Rect, A) {
    fn rect(&self) -> Rect {
        self.0
    }
}

impl<A, B> Located for (Rect, A, B) {
    fn rect(&self) -> Rect {
        self.0
    }
}

pub(super) fn in_rect(area: Rect, x: u16, y: u16) -> bool {
    area.contains(Position::new(x, y))
}

/// The first entry whose rect holds the cell `(x, y)`.
pub(super) fn hit<T: Located + Clone>(items: &[T], x: u16, y: u16) -> Option<T> {
    items.iter().find(|item| in_rect(item.rect(), x, y)).cloned()
}
