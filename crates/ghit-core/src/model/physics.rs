//! Force-directed layout engine for the repo-wide issue graph. Pure: nodes
//! are indices, edges carry a style, and `step()` advances the simulation.
//! World space is isotropic; the renderer corrects for tall terminal cells.

use serde::{Deserialize, Serialize};
use crate::model::NodeId;

/// Kinetic energy per node under which the layout counts as settled.
const SETTLED_ENERGY_PER_NODE: f64 = 1e-3;
/// Integration time step.
const DT: f64 = 1.0;
/// Squared distance under which two nodes count as coincident.
const COINCIDENT: f64 = 1e-12;

/// How an edge pulls its two ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeStyle {
    /// Plain spring.
    Link,
    /// Node to its milestone hub; the spring is stronger.
    Hub,
    /// `a` blocks `b`: a spring plus the growth force pushing `b` ahead of `a`.
    Directed,
}

/// Axis along which blockers sit before what they block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Growth {
    Off,
    Left,
    Up,
    #[default]
    Right,
    Down,
}

impl Growth {
    /// Unit vector of the growth axis in world space (y grows downward).
    fn unit(self) -> (f64, f64) {
        match self {
            Growth::Off => (0.0, 0.0),
            Growth::Left => (-1.0, 0.0),
            Growth::Up => (0.0, -1.0),
            Growth::Right => (1.0, 0.0),
            Growth::Down => (0.0, 1.0),
        }
    }
}

/// Every tunable of the simulation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Physics {
    /// Pair repulsion strength: force is `repulsion * r / (r^2 + soft^2)^1.5`.
    pub repulsion: f64,
    /// Softening distance that keeps close pairs from exploding.
    pub softening: f64,
    /// Spring rest length.
    pub spring_length: f64,
    /// Spring constant for `Link` and `Directed` edges.
    pub spring_strength: f64,
    /// Multiplier on the spring constant for `Hub` edges.
    pub hub_multiplier: f64,
    /// Pull toward the origin, per unit of distance.
    pub gravity: f64,
    /// Axis that directed edges grow along.
    pub growth: Growth,
    /// Constant force pushing the blocked node ahead of its blocker.
    pub growth_strength: f64,
    /// Velocity kept after each step (1.0 = no damping).
    pub damping: f64,
    /// Speed clamp per step.
    pub max_speed: f64,
}

impl Default for Physics {
    fn default() -> Self {
        Physics {
            repulsion: 60.0,
            softening: 1.0,
            spring_length: 6.0,
            spring_strength: 0.08,
            hub_multiplier: 3.0,
            gravity: 0.03,
            growth: Growth::Right,
            growth_strength: 0.3,
            damping: 0.6,
            max_speed: 3.0,
        }
    }
}

/// A parameter the physics dialog tunes live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    Repulsion,
    SpringLength,
    SpringStrength,
    Gravity,
    Growth,
    GrowthStrength,
    Damping,
}

impl Param {
    /// Dialog row order.
    pub const ALL: [Param; 7] = [
        Param::Repulsion,
        Param::SpringLength,
        Param::SpringStrength,
        Param::Gravity,
        Param::Growth,
        Param::GrowthStrength,
        Param::Damping,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Param::Repulsion => "repulsion",
            Param::SpringLength => "spring length",
            Param::SpringStrength => "spring strength",
            Param::Gravity => "gravity",
            Param::Growth => "growth direction",
            Param::GrowthStrength => "growth strength",
            Param::Damping => "damping",
        }
    }

    /// Step size and inclusive range of a numeric parameter. Growth
    /// direction is a cycle and has none.
    fn scale(self) -> Option<(f64, f64, f64)> {
        match self {
            Param::Repulsion => Some((10.0, 0.0, 300.0)),
            Param::SpringLength => Some((1.0, 1.0, 30.0)),
            Param::SpringStrength => Some((0.01, 0.0, 0.5)),
            Param::Gravity => Some((0.01, 0.0, 0.2)),
            Param::Growth => None,
            Param::GrowthStrength => Some((0.05, 0.0, 2.0)),
            Param::Damping => Some((0.05, 0.1, 0.95)),
        }
    }
}

impl Growth {
    const CYCLE: [Growth; 5] = [Growth::Off, Growth::Left, Growth::Up, Growth::Right, Growth::Down];

    fn label(self) -> &'static str {
        match self {
            Growth::Off => "off",
            Growth::Left => "left",
            Growth::Up => "up",
            Growth::Right => "right",
            Growth::Down => "down",
        }
    }
}

impl Physics {
    fn slot(&mut self, param: Param) -> Option<&mut f64> {
        match param {
            Param::Repulsion => Some(&mut self.repulsion),
            Param::SpringLength => Some(&mut self.spring_length),
            Param::SpringStrength => Some(&mut self.spring_strength),
            Param::Gravity => Some(&mut self.gravity),
            Param::Growth => None,
            Param::GrowthStrength => Some(&mut self.growth_strength),
            Param::Damping => Some(&mut self.damping),
        }
    }

    /// Move `param` by `steps` of its step size, clamped to its range
    /// (growth direction wraps around instead). True when the value changed.
    pub fn nudge(&mut self, param: Param, steps: i32) -> bool {
        let before = self.clone();
        match param.scale() {
            Some((step, min, max)) => {
                if let Some(value) = self.slot(param) {
                    let moved = ((*value / step).round() + f64::from(steps)) * step;
                    *value = moved.clamp(min, max);
                }
            }
            None => {
                let at = Growth::CYCLE.iter().position(|g| *g == self.growth).unwrap_or(0) as i32;
                self.growth = Growth::CYCLE[(at + steps).rem_euclid(Growth::CYCLE.len() as i32) as usize];
            }
        }
        *self != before
    }

    /// Pull every parameter into its range, as a hand-edited file may leave it.
    pub fn sanitize(&mut self) {
        for param in Param::ALL {
            if let Some((_, min, max)) = param.scale() {
                if let Some(value) = self.slot(param) {
                    *value = if value.is_finite() { value.clamp(min, max) } else { min };
                }
            }
        }
    }

    /// The value of `param` as the dialog shows it.
    pub fn show(&self, param: Param) -> String {
        match param {
            Param::Repulsion => format!("{:.0}", self.repulsion),
            Param::SpringLength => format!("{:.0}", self.spring_length),
            Param::SpringStrength => format!("{:.2}", self.spring_strength),
            Param::Gravity => format!("{:.2}", self.gravity),
            Param::Growth => self.growth.label().to_string(),
            Param::GrowthStrength => format!("{:.2}", self.growth_strength),
            Param::Damping => format!("{:.2}", self.damping),
        }
    }
}

/// One simulated node.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// Caller-supplied stable id; survives a rebuild.
    pub id: NodeId,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub pinned: bool,
}

/// The simulation: bodies, edges and the tunables.
#[derive(Clone, Debug)]
pub struct Layout {
    pub physics: Physics,
    seed: u64,
    bodies: Vec<Body>,
    edges: Vec<(usize, usize, EdgeStyle)>,
}

impl Layout {
    pub fn new(seed: u64, physics: Physics) -> Self {
        Layout { physics, seed, bodies: Vec::new(), edges: Vec::new() }
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    /// Replace the node and edge set. Nodes whose id persists keep position,
    /// velocity and pin; new ones start at a seeded position derived from
    /// their id alone. Edges index into `ids`; out-of-range edges are dropped.
    pub fn rebuild(&mut self, ids: &[NodeId], edges: &[(usize, usize, EdgeStyle)]) {
        let radius = self.physics.spring_length * (1.0 + (ids.len() as f64).sqrt());
        let old: std::collections::HashMap<NodeId, Body> =
            self.bodies.drain(..).map(|b| (b.id, b)).collect();
        self.bodies = ids
            .iter()
            .map(|&id| {
                old.get(&id).cloned().unwrap_or_else(|| {
                    let (x, y) = seeded_point(self.seed, id, radius);
                    Body { id, x, y, vx: 0.0, vy: 0.0, pinned: false }
                })
            })
            .collect();
        let n = ids.len();
        self.edges = edges.iter().copied().filter(|&(a, b, _)| a < n && b < n && a != b).collect();
    }

    pub fn set_position(&mut self, i: usize, x: f64, y: f64) {
        if let Some(b) = self.bodies.get_mut(i) {
            b.x = x;
            b.y = y;
            b.vx = 0.0;
            b.vy = 0.0;
        }
    }

    pub fn pin(&mut self, i: usize) {
        if let Some(b) = self.bodies.get_mut(i) {
            b.pinned = true;
            b.vx = 0.0;
            b.vy = 0.0;
        }
    }

    pub fn unpin(&mut self, i: usize) {
        if let Some(b) = self.bodies.get_mut(i) {
            b.pinned = false;
        }
    }

    /// Total kinetic energy (unit mass).
    pub fn kinetic_energy(&self) -> f64 {
        self.bodies.iter().map(|b| 0.5 * (b.vx * b.vx + b.vy * b.vy)).sum()
    }

    /// True when motion is too small to be worth repainting.
    pub fn settled(&self) -> bool {
        self.kinetic_energy() < SETTLED_ENERGY_PER_NODE * self.bodies.len() as f64
    }

    /// Advance one step: accumulate forces, then semi-implicit Euler with
    /// damping and a speed clamp. Pinned nodes exert forces but never move.
    pub fn step(&mut self) {
        let p = &self.physics;
        let n = self.bodies.len();
        let mut f = vec![(0.0f64, 0.0f64); n];

        for i in 0..n {
            for j in (i + 1)..n {
                let (mut dx, mut dy) = (self.bodies[i].x - self.bodies[j].x, self.bodies[i].y - self.bodies[j].y);
                let mut r2 = dx * dx + dy * dy;
                if r2 < COINCIDENT {
                    // Coincident: push apart along a fixed pair-dependent direction.
                    let a = (i * 31 + j) as f64;
                    (dx, dy) = (a.cos() * 1e-3, a.sin() * 1e-3);
                    r2 = dx * dx + dy * dy;
                }
                let k = p.repulsion / (r2 + p.softening * p.softening).powf(1.5);
                f[i].0 += k * dx;
                f[i].1 += k * dy;
                f[j].0 -= k * dx;
                f[j].1 -= k * dy;
            }
        }

        let (ux, uy) = p.growth.unit();
        for &(a, b, style) in &self.edges {
            let (dx, dy) = (self.bodies[b].x - self.bodies[a].x, self.bodies[b].y - self.bodies[a].y);
            let r = (dx * dx + dy * dy).sqrt();
            if r > 1e-9 {
                let strength = match style {
                    EdgeStyle::Hub => p.spring_strength * p.hub_multiplier,
                    _ => p.spring_strength,
                };
                let k = strength * (r - p.spring_length) / r;
                f[a].0 += k * dx;
                f[a].1 += k * dy;
                f[b].0 -= k * dx;
                f[b].1 -= k * dy;
            }
            if style == EdgeStyle::Directed {
                f[b].0 += p.growth_strength * ux;
                f[b].1 += p.growth_strength * uy;
                f[a].0 -= p.growth_strength * ux;
                f[a].1 -= p.growth_strength * uy;
            }
        }

        for (b, (fx, fy)) in self.bodies.iter_mut().zip(f) {
            if b.pinned {
                b.vx = 0.0;
                b.vy = 0.0;
                continue;
            }
            let (fx, fy) = (fx - p.gravity * b.x, fy - p.gravity * b.y);
            b.vx = (b.vx + fx * DT) * p.damping;
            b.vy = (b.vy + fy * DT) * p.damping;
            let speed = (b.vx * b.vx + b.vy * b.vy).sqrt();
            if speed > p.max_speed {
                b.vx *= p.max_speed / speed;
                b.vy *= p.max_speed / speed;
            }
            b.x += b.vx * DT;
            b.y += b.vy * DT;
        }
    }
}

/// splitmix64: a tiny seeded generator, enough for start positions.
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Uniform point in a disc of `radius`, a pure function of seed and id.
fn seeded_point(seed: u64, id: NodeId, radius: f64) -> (f64, f64) {
    let id = match id {
        NodeId::Item(item) => item.number ^ (item.source as u64).wrapping_mul(0x4a49_5241),
        NodeId::Hub(hash) => hash,
    };
    let h1 = splitmix(seed ^ splitmix(id));
    let h2 = splitmix(h1);
    let unit = |h: u64| (h >> 11) as f64 / (1u64 << 53) as f64;
    let (angle, r) = (unit(h1) * std::f64::consts::TAU, radius * unit(h2).sqrt());
    (r * angle.cos(), r * angle.sin())
}
