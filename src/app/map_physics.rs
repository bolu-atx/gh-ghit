//! The map physics dialog's actions: move the focused row, step its
//! value, reset. A change reaches the running layout at once, wakes it
//! without touching node positions, and is saved for the next session.

use crate::app::{App, Entry};
use crate::github;
use crate::model::physics::{Param, Physics};

impl App {
    pub(crate) fn open_physics(&mut self) {
        self.open(Entry::Physics(0));
    }

    /// Move the focus `delta` rows, wrapping at the ends.
    pub(crate) fn physics_step(&mut self, delta: isize) {
        let Some(focus) = self.physics_focus() else { return };
        let rows = Param::ALL.len() as isize;
        self.set_physics_focus((focus as isize + delta).rem_euclid(rows) as usize);
    }

    /// Step the focused row's value `steps` times.
    pub(crate) fn physics_nudge(&mut self, steps: i32) {
        let Some(param) = self.physics_focus().and_then(|row| Param::ALL.get(row).copied()) else { return };
        let mut physics = self.map.layout.physics.clone();
        if physics.nudge(param, steps) {
            self.apply_physics(physics);
        }
    }

    pub(crate) fn physics_reset(&mut self) {
        self.apply_physics(Physics::default());
        self.notify("map physics reset".to_string());
    }

    /// Install `physics` in the running layout, wake it and save it.
    fn apply_physics(&mut self, physics: Physics) {
        self.map.layout.physics = physics;
        self.map.wake();
        if self.prefs_loaded {
            github::save_physics(&self.map.layout.physics);
        }
    }
}
