//! Dependency map geometry: the camera that maps world positions to
//! terminal cells, label detail per zoom, and line rasterizing. Pure, so
//! render and mouse input share one mapping.

/// Cells are about twice as tall as wide, so one world unit spans this
/// many columns per row.
pub const ASPECT: f64 = 2.0;
pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 4.0;
/// Zoom step of one `+`/`-` press or one wheel notch.
pub const ZOOM_STEP: f64 = 1.25;
/// Most a fit widens the view past true proportions, so a round layout
/// still fills a wide pane.
const MAX_STRETCH: f64 = 2.0;

/// The view onto the world: the world point at the view center, rows per
/// world unit, and an extra horizontal factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub stretch: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { x: 0.0, y: 0.0, zoom: 1.0, stretch: 1.0 }
    }
}

/// How much of a node label fits at a zoom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelDetail {
    /// `#n title…`, the title cut at this many characters.
    Title(usize),
    /// `#n` only.
    Number,
    /// `•` only.
    Dot,
}

impl LabelDetail {
    /// This title detail, or `other` when that already shows a longer title.
    pub fn max_title(self, other: LabelDetail) -> LabelDetail {
        match (self, other) {
            (LabelDetail::Title(a), LabelDetail::Title(b)) => LabelDetail::Title(a.max(b)),
            _ => self,
        }
    }
}

impl Camera {
    /// Cell of world point `(x, y)` in a `width` x `height` view, relative
    /// to the view's top-left. It may fall outside the view.
    pub fn cell(&self, x: f64, y: f64, width: u16, height: u16) -> (i32, i32) {
        let col = (x - self.x) * self.columns() + f64::from(width) / 2.0;
        let row = (y - self.y) * self.zoom + f64::from(height) / 2.0;
        (col.floor() as i32, row.floor() as i32)
    }

    /// World point at the center of cell `(col, row)`.
    pub fn world(&self, col: i32, row: i32, width: u16, height: u16) -> (f64, f64) {
        let x = (f64::from(col) + 0.5 - f64::from(width) / 2.0) / self.columns() + self.x;
        let y = (f64::from(row) + 0.5 - f64::from(height) / 2.0) / self.zoom + self.y;
        (x, y)
    }

    /// Move the view by whole cells; positive moves it right and down.
    pub fn pan(&mut self, cols: f64, rows: f64) {
        self.x += cols / self.columns();
        self.y += rows / self.zoom;
    }

    /// Columns per world unit.
    fn columns(&self) -> f64 {
        self.zoom * ASPECT * self.stretch
    }

    pub fn zoom_by(&mut self, factor: f64) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// A camera that shows every point in a `width` x `height` view, with
    /// room left for labels at the sides.
    pub fn fit(points: &[(f64, f64)], width: u16, height: u16) -> Camera {
        if points.is_empty() {
            return Camera::default();
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let room_x = (f64::from(width) - 30.0).max(4.0);
        let room_y = (f64::from(height) - 4.0).max(2.0);
        let zoom_x = room_x / ((x1 - x0).max(1e-6) * ASPECT);
        let zoom_y = room_y / (y1 - y0).max(1e-6);
        let zoom = zoom_x.min(zoom_y).clamp(MIN_ZOOM, 1.5);
        let stretch = (zoom_x / zoom).clamp(1.0, MAX_STRETCH).min((1.5 / zoom).max(1.0));
        Camera { x: (x0 + x1) / 2.0, y: (y0 + y1) / 2.0, zoom, stretch }
    }

    /// Label detail at this zoom: titles while nodes sit far apart across,
    /// then numbers, then dots.
    pub fn detail(&self) -> LabelDetail {
        let across = self.zoom * self.stretch;
        if across >= 0.8 {
            LabelDetail::Title(((across * 18.0) as usize).clamp(8, 24))
        } else if across >= 0.3 {
            LabelDetail::Number
        } else {
            LabelDetail::Dot
        }
    }
}

/// Cells from `from` to `to`, both ends included (Bresenham).
pub fn line(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = ((to.0 - from.0).abs(), -(to.1 - from.1).abs());
    let (sx, sy) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    let (mut x, mut y, mut err) = (from.0, from.1, dx + dy);
    let mut cells = Vec::with_capacity((dx - dy) as usize + 1);
    loop {
        cells.push((x, y));
        if (x, y) == to {
            return cells;
        }
        let twice = 2 * err;
        if twice >= dy {
            err += dy;
            x += sx;
        }
        if twice <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Glyph of a line that runs `dx` columns and `dy` rows (rows grow down),
/// picked by its look on screen, where a row is `ASPECT` columns tall.
pub fn stroke(dx: i32, dy: i32) -> char {
    let (across, down) = (f64::from(dx.abs()), f64::from(dy.abs()) * ASPECT);
    if down <= across * 0.4 {
        '─'
    } else if across <= down * 0.4 {
        '│'
    } else if (dx > 0) == (dy > 0) {
        '╲'
    } else {
        '╱'
    }
}

/// Arrowhead for a line heading `dx` columns and `dy` rows.
pub fn arrow(dx: i32, dy: i32) -> char {
    if f64::from(dx.abs()) >= f64::from(dy.abs()) * ASPECT {
        if dx >= 0 { '▸' } else { '◂' }
    } else if dy > 0 {
        '▾'
    } else {
        '▴'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_and_world_points_round_trip() {
        let camera = Camera { x: 3.0, y: -2.0, zoom: 1.5, stretch: 1.0 };
        let (col, row) = camera.cell(3.0, -2.0, 80, 20);
        assert_eq!((col, row), (40, 10), "the camera point sits at the view center");
        let (x, y) = camera.world(col, row, 80, 20);
        assert_eq!(camera.cell(x, y, 80, 20), (col, row));
        let (right, _) = camera.cell(4.0, -2.0, 80, 20);
        let (_, down) = camera.cell(3.0, -1.0, 80, 20);
        assert_eq!((right - col, down - row), (3, 1), "one world unit is twice as wide as tall");
    }

    #[test]
    fn pan_and_zoom_move_the_view() {
        let mut camera = Camera::default();
        camera.pan(4.0, 2.0);
        assert_eq!((camera.x, camera.y), (2.0, 2.0));
        camera.zoom_by(100.0);
        assert_eq!(camera.zoom, MAX_ZOOM);
        camera.zoom_by(0.0);
        assert_eq!(camera.zoom, MIN_ZOOM);
    }

    #[test]
    fn fit_keeps_every_point_in_view() {
        let points = [(-30.0, -5.0), (40.0, 12.0), (0.0, 0.0)];
        let camera = Camera::fit(&points, 100, 30);
        for (x, y) in points {
            let (col, row) = camera.cell(x, y, 100, 30);
            assert!((0..100).contains(&col) && (0..30).contains(&row), "({x}, {y}) at ({col}, {row})");
        }
        assert_eq!(Camera::fit(&[], 100, 30), Camera::default());
        let round = Camera::fit(&[(-10.0, -10.0), (10.0, 10.0)], 120, 30);
        assert!(round.stretch > 1.0, "a round layout widens to fill a wide view");
    }

    #[test]
    fn labels_shrink_as_the_view_zooms_out() {
        let at = |zoom: f64| Camera { zoom, ..Camera::default() }.detail();
        assert!(matches!(at(1.0), LabelDetail::Title(_)));
        assert_eq!(at(0.5), LabelDetail::Number);
        assert_eq!(at(0.2), LabelDetail::Dot);
        let wide = Camera { zoom: 0.5, stretch: 2.0, ..Camera::default() };
        assert!(matches!(wide.detail(), LabelDetail::Title(_)), "stretch widens the room for titles");
    }

    #[test]
    fn lines_cover_both_ends_and_step_one_cell() {
        let cells = line((0, 0), (6, -3));
        assert_eq!((cells[0], *cells.last().unwrap()), ((0, 0), (6, -3)));
        for pair in cells.windows(2) {
            assert!((pair[1].0 - pair[0].0).abs() <= 1 && (pair[1].1 - pair[0].1).abs() <= 1);
        }
        assert_eq!(line((2, 2), (2, 2)), vec![(2, 2)]);
    }

    #[test]
    fn strokes_and_arrows_follow_the_slope() {
        assert_eq!(stroke(10, 1), '─');
        assert_eq!(stroke(1, 5), '│');
        assert_eq!(stroke(4, 2), '╲');
        assert_eq!(stroke(4, -2), '╱');
        assert_eq!((arrow(5, 1), arrow(-5, 1), arrow(1, 4), arrow(1, -4)), ('▸', '◂', '▾', '▴'));
    }
}
