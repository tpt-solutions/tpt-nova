//! Internal anomaly observation (Phase 8, Layer 4): an early-warning detector.
//!
//! This replaces the planned `tpt-anima` integration, which has no anomaly API.
//! Where [`crate::monitor::Monitor`] checks a fixed bound, an [`AnomalyTracker`]
//! learns each entity's normal range and flags sudden departures from it. The
//! flag can come before the bound is crossed.
//!
//! For each entity it keeps an exponentially weighted mean and variance (West's
//! incremental update) and scores each new sample by its z-score against the
//! prediction. Everything is deterministic: state is kept in a `BTreeMap`, and
//! observations are visited in entity order.

use std::collections::BTreeMap;

use tpt_nova_ecs::{Entity, World};

/// One flagged sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anomaly {
    pub entity: Entity,
    pub value: f64,
    /// Standard scores against the prediction made before `value` was seen.
    /// Non-finite values carry `f64::INFINITY`.
    pub z: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Track {
    mean: f64,
    var: f64,
    seen: u32,
}

/// Per-entity EWMA tracker with a z-score threshold.
#[derive(Debug, Clone)]
pub struct AnomalyTracker {
    /// Smoothing factor in `(0, 1]`. Larger values adapt faster.
    alpha: f64,
    /// Flag when `|z| > threshold`.
    threshold: f64,
    /// Samples to learn from before any flag is raised.
    warmup: u32,
    tracks: BTreeMap<Entity, Track>,
}

impl AnomalyTracker {
    /// Build a tracker. `alpha` is clamped to `(0, 1]` and `threshold` to be positive.
    pub fn new(alpha: f64, threshold: f64, warmup: u32) -> Self {
        AnomalyTracker {
            alpha: alpha.clamp(1e-6, 1.0),
            threshold: threshold.max(1e-6),
            warmup,
            tracks: BTreeMap::new(),
        }
    }

    /// Learn from one sample for `entity`. Returns an [`Anomaly`] when the sample
    /// departs from the learned range.
    pub fn observe(&mut self, entity: Entity, value: f64) -> Option<Anomaly> {
        if !value.is_finite() {
            return Some(Anomaly {
                entity,
                value,
                z: f64::INFINITY,
            });
        }
        let alpha = self.alpha;
        let track = self.tracks.entry(entity).or_default();
        if track.seen == 0 {
            track.mean = value;
            track.var = 0.0;
            track.seen = 1;
            return None;
        }
        let prev_mean = track.mean;
        let sd = track.var.sqrt();
        // Floor the scale so a perfectly steady signal does not make every change infinite.
        let z = (value - prev_mean) / sd.max(1e-9);

        // West's EWMA update of mean and variance.
        let diff = value - prev_mean;
        let incr = alpha * diff;
        track.mean = prev_mean + incr;
        track.var = (1.0 - alpha) * (track.var + diff * incr);
        track.seen = track.seen.saturating_add(1);

        let flagged = track.seen > self.warmup && z.abs() > self.threshold;
        flagged.then_some(Anomaly { entity, value, z })
    }

    /// Observe every entity in `world` that `read` returns a value for, in entity order.
    pub fn observe_world<R>(&mut self, world: &World, read: R) -> Vec<Anomaly>
    where
        R: Fn(&World, Entity) -> Option<f64>,
    {
        let mut entities = world.entities();
        entities.sort();
        entities
            .into_iter()
            .filter_map(|e| read(world, e).and_then(|v| self.observe(e, v)))
            .collect()
    }

    /// The learned mean for `entity`, if it has been observed.
    pub fn mean(&self, entity: Entity) -> Option<f64> {
        self.tracks.get(&entity).map(|t| t.mean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_nova_ecs::Component;

    #[derive(Debug, Default)]
    struct Speed {
        v: f32,
    }
    impl Component for Speed {}

    fn entity(i: u32) -> Entity {
        // Entities are opaque handles; spawn to get real ones.
        let mut w = World::new();
        let mut last = w.spawn();
        for _ in 0..i {
            last = w.spawn();
        }
        last
    }

    #[test]
    fn a_steady_signal_is_never_flagged() {
        let mut t = AnomalyTracker::new(0.1, 4.0, 5);
        let e = entity(0);
        for _ in 0..200 {
            assert!(t.observe(e, 5.0).is_none());
        }
    }

    #[test]
    fn a_sudden_jump_is_flagged_after_warmup() {
        let mut t = AnomalyTracker::new(0.1, 4.0, 5);
        let e = entity(0);
        // Noisy but stable baseline around 5.
        for i in 0..50 {
            let noise = if i % 2 == 0 { 0.1 } else { -0.1 };
            assert!(t.observe(e, 5.0 + noise).is_none());
        }
        let hit = t.observe(e, 50.0).expect("a 45-unit jump is anomalous");
        assert!(hit.z > 4.0);
        assert_eq!(hit.entity, e);
    }

    #[test]
    fn warmup_suppresses_early_flags() {
        let mut t = AnomalyTracker::new(0.5, 1.0, 10);
        let e = entity(0);
        t.observe(e, 1.0);
        assert!(t.observe(e, 100.0).is_none(), "still warming up");
    }

    #[test]
    fn non_finite_samples_are_always_anomalous() {
        let mut t = AnomalyTracker::new(0.1, 4.0, 100);
        let a = t.observe(entity(0), f64::NAN).expect("NaN is flagged");
        assert!(a.z.is_infinite());
    }

    #[test]
    fn world_observation_is_deterministic_in_entity_order() {
        let build = || {
            let mut w = World::new();
            for i in 0..4 {
                let e = w.spawn();
                w.add_component(e, Speed { v: i as f32 });
            }
            w
        };
        let read = |w: &World, e: Entity| w.get_component::<Speed>(e).map(|s| s.v as f64);
        let run = || {
            let w = build();
            let mut t = AnomalyTracker::new(0.2, 2.0, 0);
            t.observe_world(&w, read);
            t.observe_world(&w, read)
        };
        assert_eq!(run(), run());
    }
}
