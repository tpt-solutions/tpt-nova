//! Anomaly observation (Phase 8, Layer 4): invariant monitors over world state.
//!
//! An [`Invariant`] names a property every sampled entity must satisfy, such as
//! "velocity bounded" or "position finite". A [`Monitor`] checks a world against
//! its invariants and returns one [`Violation`] per failing entity. The repair
//! loop in `repair.rs` consumes these violations.
//!
//! This is a local, deterministic detector. `tpt-anima` (the other Layer 4
//! dependency) exposes swarm, INP, ZK and compute-budget telemetry, not an
//! anomaly detector, so there is no upstream API to call.

use tpt_nova_ecs::{Entity, World};

use crate::verify::Bounds;

/// One failed check: which invariant, which entity, and the offending value.
/// A non-finite value (NaN or infinity) is recorded with `observed` as that value.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub invariant: String,
    pub entity: Entity,
    pub observed: f64,
}

/// A property of world state that must hold for every entity it applies to.
pub trait Invariant: Send + Sync {
    fn name(&self) -> &str;

    /// The integer interval the observed value must stay in. The verifier uses
    /// the same bounds as premise and conclusion for a one-tick model.
    fn bounds(&self) -> Bounds;

    fn check(&self, world: &World) -> Vec<Violation>;
}

/// An invariant that reads one scalar per entity and requires it to be finite and
/// within `bounds`. Entities for which `read` returns `None` are skipped.
pub struct ValueBound<R>
where
    R: Fn(&World, Entity) -> Option<f64> + Send + Sync,
{
    name: String,
    bounds: Bounds,
    read: R,
}

impl<R> ValueBound<R>
where
    R: Fn(&World, Entity) -> Option<f64> + Send + Sync,
{
    pub fn new(name: impl Into<String>, bounds: Bounds, read: R) -> Self {
        ValueBound {
            name: name.into(),
            bounds,
            read,
        }
    }
}

impl<R> Invariant for ValueBound<R>
where
    R: Fn(&World, Entity) -> Option<f64> + Send + Sync,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn bounds(&self) -> Bounds {
        self.bounds
    }

    fn check(&self, world: &World) -> Vec<Violation> {
        let lo = self.bounds.lo as f64;
        let hi = self.bounds.hi as f64;
        world
            .entities()
            .into_iter()
            .filter_map(|entity| {
                let observed = (self.read)(world, entity)?;
                let ok = observed.is_finite() && observed >= lo && observed <= hi;
                (!ok).then(|| Violation {
                    invariant: self.name.clone(),
                    entity,
                    observed,
                })
            })
            .collect()
    }
}

/// A set of invariants checked together.
#[derive(Default)]
pub struct Monitor {
    invariants: Vec<Box<dyn Invariant>>,
}

impl Monitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, invariant: impl Invariant + 'static) {
        self.invariants.push(Box::new(invariant));
    }

    /// Look up an invariant by name.
    pub fn invariant(&self, name: &str) -> Option<&dyn Invariant> {
        self.invariants
            .iter()
            .find(|i| i.name() == name)
            .map(|b| b.as_ref())
    }

    /// Check every invariant against `world`. Violations come out in invariant
    /// insertion order, then entity order.
    pub fn check(&self, world: &World) -> Vec<Violation> {
        self.invariants
            .iter()
            .flat_map(|i| i.check(world))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_nova_ecs::Component;

    #[derive(Debug, Default)]
    struct Velocity {
        v: f32,
    }
    impl Component for Velocity {}

    fn speed_bound() -> ValueBound<impl Fn(&World, Entity) -> Option<f64> + Send + Sync> {
        ValueBound::new(
            "velocity-bounded",
            Bounds::new(-10, 10).unwrap(),
            |w: &World, e| w.get_component::<Velocity>(e).map(|v| v.v as f64),
        )
    }

    #[test]
    fn in_bound_values_produce_no_violations() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Velocity { v: 9.5 });
        assert!(speed_bound().check(&world).is_empty());
    }

    #[test]
    fn out_of_bound_and_non_finite_values_are_violations() {
        let mut world = World::new();
        let hot = world.spawn();
        world.add_component(hot, Velocity { v: 12.0 });
        let nan = world.spawn();
        world.add_component(nan, Velocity { v: f32::NAN });
        let violations = speed_bound().check(&world);
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].entity, hot);
        assert_eq!(violations[0].observed, 12.0);
        assert!(violations[1].observed.is_nan());
    }

    #[test]
    fn entities_without_the_component_are_skipped() {
        let mut world = World::new();
        world.spawn();
        assert!(speed_bound().check(&world).is_empty());
    }

    #[test]
    fn monitor_collects_across_invariants_and_finds_by_name() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Velocity { v: 50.0 });
        let mut monitor = Monitor::new();
        monitor.add(speed_bound());
        monitor.add(ValueBound::new(
            "velocity-finite",
            Bounds::new(i64::MIN / 2, i64::MAX / 2).unwrap(),
            |w: &World, e| w.get_component::<Velocity>(e).map(|v| v.v as f64),
        ));
        let v = monitor.check(&world);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].invariant, "velocity-bounded");
        assert!(monitor.invariant("velocity-bounded").is_some());
        assert!(monitor.invariant("missing").is_none());
    }
}
