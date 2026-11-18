//! A minimal system scheduler.
//!
//! A [`System`] is any `FnMut(&mut World)`. Systems run in insertion order
//! inside [`Scheduler::run`]. For Phase 1 this linear execution model is
//! enough; later phases can add parallel stages and ordering constraints.

use crate::world::World;

/// A unit of game logic that mutates the world.
pub type System = Box<dyn FnMut(&mut World) + Send + 'static>;

/// Runs a fixed list of systems against the world each tick.
#[derive(Default)]
pub struct Scheduler {
    systems: Vec<System>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            systems: Vec::new(),
        }
    }

    /// Append a system to the schedule.
    pub fn add_system<F>(&mut self, system: F)
    where
        F: FnMut(&mut World) + Send + 'static,
    {
        self.systems.push(Box::new(system));
    }

    /// Run every system once against the world.
    pub fn run(&mut self, world: &mut World) {
        for system in &mut self.systems {
            system(world);
        }
    }
}

/// A named, ordered collection of stages. Each stage runs its systems
/// sequentially; stages run in the order they were added.
#[derive(Default)]
pub struct Schedule {
    stages: Vec<(String, Vec<System>)>,
}

impl Schedule {
    pub fn new() -> Self {
        Schedule { stages: Vec::new() }
    }

    /// Add a new named stage. Systems added afterwards join this stage.
    pub fn add_stage(&mut self, name: impl Into<String>) {
        self.stages.push((name.into(), Vec::new()));
    }

    pub fn add_system_to(&mut self, stage: &str, system: impl FnMut(&mut World) + Send + 'static) {
        let stage = stage.to_string();
        if !self.stages.iter().any(|(n, _)| *n == stage) {
            self.stages.push((stage.clone(), Vec::new()));
        }
        for (name, systems) in &mut self.stages {
            if *name == stage {
                systems.push(Box::new(system));
                return;
            }
        }
    }

    /// Run all stages in order.
    pub fn run(&mut self, world: &mut World) {
        for (_, systems) in &mut self.stages {
            for system in systems {
                system(world);
            }
        }
    }
}
