//! Force-directed layout engine for the repo-wide issue graph. Pure: nodes
//! are indices, edges carry a style, and `step()` advances the simulation.
//! World space is isotropic; the renderer corrects for tall terminal cells.

use serde::{Deserialize, Serialize};

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

/// One simulated node.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// Caller-supplied stable id; survives a rebuild.
    pub id: u64,
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
    pub fn rebuild(&mut self, ids: &[u64], edges: &[(usize, usize, EdgeStyle)]) {
        let radius = self.physics.spring_length * (1.0 + (ids.len() as f64).sqrt());
        let old: std::collections::HashMap<u64, Body> =
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
fn seeded_point(seed: u64, id: u64, radius: f64) -> (f64, f64) {
    let h1 = splitmix(seed ^ splitmix(id));
    let h2 = splitmix(h1);
    let unit = |h: u64| (h >> 11) as f64 / (1u64 << 53) as f64;
    let (angle, r) = (unit(h1) * std::f64::consts::TAU, radius * unit(h2).sqrt());
    (r * angle.cos(), r * angle.sin())
}
