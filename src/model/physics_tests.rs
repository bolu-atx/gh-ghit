//! Tests for model::physics.

use super::physics::{EdgeStyle, Growth, Layout, Physics};

fn run(l: &mut Layout, steps: usize) {
    for _ in 0..steps {
        l.step();
    }
}

fn chain(growth: Growth) -> Layout {
    let physics = Physics { growth, ..Physics::default() };
    let mut l = Layout::new(7, physics);
    l.rebuild(&[1, 2, 3], &[(0, 1, EdgeStyle::Directed), (1, 2, EdgeStyle::Directed)]);
    run(&mut l, 1500);
    l
}

#[test]
fn small_graph_settles() {
    let mut l = Layout::new(1, Physics::default());
    let ids: Vec<u64> = (0..12).collect();
    let mut edges: Vec<_> = (1..12).map(|i| (i - 1, i, EdgeStyle::Link)).collect();
    edges.push((0, 11, EdgeStyle::Hub));
    l.rebuild(&ids, &edges);
    // A fresh layout is at rest; the first step sets it moving.
    l.step();
    assert!(!l.settled());
    let mut steps = 1;
    while !l.settled() && steps < 400 {
        l.step();
        steps += 1;
    }
    // The map animates about four steps a frame, so this is a few seconds at most.
    assert!(l.settled(), "not settled after {steps} steps");
}

#[test]
fn growth_orders_a_chain_along_each_axis() {
    for (growth, key) in [
        (Growth::Right, (|b: &super::physics::Body| b.x) as fn(&_) -> f64),
        (Growth::Left, |b| -b.x),
        (Growth::Down, |b| b.y),
        (Growth::Up, |b| -b.y),
    ] {
        let l = chain(growth);
        let k: Vec<f64> = l.bodies().iter().map(key).collect();
        assert!(k[0] < k[1] && k[1] < k[2], "{growth:?}: {k:?}");
    }
}

#[test]
fn growth_off_applies_no_bias() {
    let off = chain(Growth::Off);
    let right = chain(Growth::Right);
    let span = |l: &Layout| l.bodies()[2].x - l.bodies()[0].x;
    assert!(span(&right) > 4.0 && span(&right) > span(&off));
}

#[test]
fn pinned_nodes_do_not_move() {
    let mut l = Layout::new(3, Physics::default());
    l.rebuild(&[1, 2, 3], &[(0, 1, EdgeStyle::Link), (1, 2, EdgeStyle::Link)]);
    l.set_position(0, 5.0, -4.0);
    l.pin(0);
    run(&mut l, 200);
    let b = &l.bodies()[0];
    assert_eq!((b.x, b.y, b.vx, b.vy), (5.0, -4.0, 0.0, 0.0));
    l.unpin(0);
    run(&mut l, 50);
    assert_ne!((l.bodies()[0].x, l.bodies()[0].y), (5.0, -4.0));
}

#[test]
fn same_seed_same_layout() {
    let build = |seed| {
        let mut l = Layout::new(seed, Physics::default());
        l.rebuild(&[10, 20, 30, 40], &[(0, 1, EdgeStyle::Directed), (2, 3, EdgeStyle::Hub)]);
        run(&mut l, 100);
        l.bodies().to_vec()
    };
    assert_eq!(build(9), build(9));
    assert_ne!(build(9), build(10));
}

#[test]
fn rebuild_keeps_surviving_positions() {
    let mut l = Layout::new(5, Physics::default());
    l.rebuild(&[1, 2, 3], &[(0, 1, EdgeStyle::Link)]);
    run(&mut l, 50);
    let before: Vec<_> = l.bodies().to_vec();
    l.rebuild(&[3, 4, 1], &[(0, 2, EdgeStyle::Link)]);
    let after = l.bodies();
    assert_eq!(after[0], before[2]);
    assert_eq!(after[2], before[0]);
    assert_eq!(after[1].id, 4);
}

#[test]
fn coincident_nodes_separate_without_nan() {
    let mut l = Layout::new(1, Physics::default());
    l.rebuild(&[1, 2], &[]);
    l.set_position(0, 2.0, 2.0);
    l.set_position(1, 2.0, 2.0);
    run(&mut l, 20);
    let b = l.bodies();
    assert!(b.iter().all(|b| b.x.is_finite() && b.y.is_finite()));
    assert!((b[0].x - b[1].x).hypot(b[0].y - b[1].y) > 0.1);
}

#[test]
fn physics_round_trips_through_json() {
    let p = Physics { growth: Growth::Down, ..Physics::default() };
    let back: Physics = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(p, back);
}

#[test]
fn three_hundred_nodes_stay_finite() {
    let mut l = Layout::new(2, Physics::default());
    let ids: Vec<u64> = (0..300).collect();
    let edges: Vec<_> = (1..300).map(|i| (i / 2, i, EdgeStyle::Directed)).collect();
    l.rebuild(&ids, &edges);
    run(&mut l, 50);
    assert!(l.bodies().iter().all(|b| b.x.is_finite() && b.y.is_finite()));
}
