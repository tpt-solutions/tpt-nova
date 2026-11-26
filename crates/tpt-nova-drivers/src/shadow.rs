//! Shadow execution for A/B testing a candidate driver (Phase 8, Layer 3).
//!
//! Before a candidate version is promoted, it runs for one tick against a
//! *fork* of the live world alongside the currently active (baseline) version,
//! also on a fork. The two results are compared per sampled entity through a
//! caller-supplied `observe` function that projects the component state of
//! interest to JSON. The live world is never mutated.
//!
//! `World` is not `Clone`, so the caller supplies the fork function. A scene
//! round-trip (`tpt_nova_scene::dump_world` then `load_world`) is one way to build
//! one; the drivers crate deliberately does not depend on the scene crate.
//!
//! The sample is an explicit list of entities, so a caller can shadow a subset
//! (for example a deterministic every-Nth selection) rather than the whole world.
//! Only the sampled entities are compared; the drivers themselves still run over
//! the full cloned world, because a driver's update is defined over the world.
//!
//! [`promote_if_clean`] activates the candidate only when the report is clean,
//! so a divergent candidate cannot reach the active slot through this path.

use tpt_nova_ecs::{ComponentTypeId, Entity, World};

use crate::driver::{Driver, DriverContext, DriverError, DriverVersion};
use crate::registry::{DriverRegistry, RegistryError};

/// Outcome of one shadow comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowReport {
    pub baseline: DriverVersion,
    pub candidate: DriverVersion,
    /// How many sampled entities were compared.
    pub sampled: usize,
    /// Sampled entities whose observed state differed between the two versions,
    /// in the order they were supplied.
    pub divergent: Vec<Entity>,
}

impl ShadowReport {
    /// True when the candidate produced the same observed state as the baseline
    /// for every sampled entity.
    pub fn is_clean(&self) -> bool {
        self.divergent.is_empty()
    }
}

/// Errors from a shadow run or its promotion.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ShadowError {
    #[error("shadow sample is empty; nothing to compare")]
    EmptySample,
    #[error("sampled entity {0:?} does not exist in the world")]
    UnknownEntity(Entity),
    #[error("driver {version} failed during shadow execution: {source}")]
    Driver {
        version: DriverVersion,
        source: DriverError,
    },
    #[error("candidate {candidate} diverged on {divergent} sampled entities; not promoted")]
    Divergent {
        candidate: DriverVersion,
        divergent: usize,
    },
    #[error("registry refused the promotion: {0}")]
    Registry(#[from] RegistryError),
}

/// Run `baseline` and `candidate` for one tick on forks of `world` and
/// compare `observe(world, entity)` for each sampled entity.
///
/// `observe` should return the state the candidate is allowed to change, as JSON.
/// Anything it omits is invisible to the comparison.
pub fn shadow_compare<Fork, Observe>(
    baseline: &dyn Driver,
    candidate: &dyn Driver,
    world: &World,
    ctx: &DriverContext,
    sample: &[Entity],
    fork: Fork,
    observe: Observe,
) -> Result<ShadowReport, ShadowError>
where
    Fork: Fn(&World) -> World,
    Observe: Fn(&World, Entity) -> serde_json::Value,
{
    if sample.is_empty() {
        return Err(ShadowError::EmptySample);
    }
    for &entity in sample {
        if !world.entities().contains(&entity) {
            return Err(ShadowError::UnknownEntity(entity));
        }
    }

    let mut base_world = fork(world);
    baseline
        .update(ctx, &mut base_world)
        .map_err(|source| ShadowError::Driver {
            version: baseline.version(),
            source,
        })?;

    let mut cand_world = fork(world);
    candidate
        .update(ctx, &mut cand_world)
        .map_err(|source| ShadowError::Driver {
            version: candidate.version(),
            source,
        })?;

    let divergent = sample
        .iter()
        .copied()
        .filter(|&e| observe(&base_world, e) != observe(&cand_world, e))
        .collect();

    Ok(ShadowReport {
        baseline: baseline.version(),
        candidate: candidate.version(),
        sampled: sample.len(),
        divergent,
    })
}

/// Activate `report.candidate` for `component`, but only if the report is clean.
///
/// The report must describe the version being promoted; a report for a different
/// candidate is rejected as a registry error so a stale report cannot promote
/// the wrong driver.
pub fn promote_if_clean(
    registry: &DriverRegistry,
    component: ComponentTypeId,
    report: &ShadowReport,
) -> Result<(), ShadowError> {
    if !report.is_clean() {
        return Err(ShadowError::Divergent {
            candidate: report.candidate,
            divergent: report.divergent.len(),
        });
    }
    registry.activate(component, report.candidate)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{CapabilitySet, DriverId};
    use crate::registry::DriverHandle;
    use std::sync::Arc;
    use tpt_nova_ecs::Component;

    #[derive(Debug, Default)]
    struct Speed {
        value: f32,
    }
    impl Component for Speed {}

    /// Adds `step` to every entity's Speed. Different steps give different results.
    struct Stepper {
        version: DriverVersion,
        step: f32,
        fail: bool,
    }

    impl Driver for Stepper {
        fn id(&self) -> DriverId {
            DriverId::new("speed-step").unwrap()
        }
        fn version(&self) -> DriverVersion {
            self.version
        }
        fn component(&self) -> ComponentTypeId {
            ComponentTypeId::of::<Speed>()
        }
        fn update(&self, _ctx: &DriverContext, world: &mut World) -> Result<(), DriverError> {
            if self.fail {
                return Err(DriverError::Failed("boom".into()));
            }
            let entities: Vec<Entity> = world.entities();
            for e in entities {
                if let Some(s) = world.get_component_mut::<Speed>(e) {
                    s.value += self.step;
                }
            }
            Ok(())
        }
    }

    fn stepper(major: u32, step: f32, fail: bool) -> Arc<Stepper> {
        Arc::new(Stepper {
            version: DriverVersion::new(major, 0, 0),
            step,
            fail,
        })
    }

    fn ctx() -> DriverContext {
        DriverContext::new(0, 1.0 / 60.0, CapabilitySet::empty())
    }

    fn observe(world: &World, e: Entity) -> serde_json::Value {
        serde_json::json!(world.get_component::<Speed>(e).map(|s| s.value))
    }

    fn world_with_speeds(n: usize) -> (World, Vec<Entity>) {
        let mut world = World::new();
        let entities = (0..n)
            .map(|_| {
                let e = world.spawn();
                world.add_component(e, Speed { value: 1.0 });
                e
            })
            .collect();
        (world, entities)
    }

    /// Test fork: rebuild the same world. Spawning is sequential, so the entity
    /// handles match the original's.
    fn fork_of(n: usize) -> impl Fn(&World) -> World {
        move |_| world_with_speeds(n).0
    }

    #[test]
    fn identical_candidate_is_clean() {
        let (world, sample) = world_with_speeds(3);
        let report = shadow_compare(
            stepper(1, 0.5, false).as_ref(),
            stepper(2, 0.5, false).as_ref(),
            &world,
            &ctx(),
            &sample,
            fork_of(3),
            observe,
        )
        .unwrap();
        assert!(report.is_clean());
        assert_eq!(report.sampled, 3);
        assert_eq!(report.candidate, DriverVersion::new(2, 0, 0));
    }

    #[test]
    fn divergent_candidate_is_reported_per_entity() {
        let (world, sample) = world_with_speeds(4);
        let report = shadow_compare(
            stepper(1, 0.5, false).as_ref(),
            stepper(2, 0.75, false).as_ref(),
            &world,
            &ctx(),
            &sample[..2],
            fork_of(4),
            observe,
        )
        .unwrap();
        assert!(!report.is_clean());
        assert_eq!(report.divergent, sample[..2].to_vec());
        assert_eq!(report.sampled, 2);
    }

    #[test]
    fn shadow_run_does_not_mutate_the_live_world() {
        let (world, sample) = world_with_speeds(2);
        shadow_compare(
            stepper(1, 0.5, false).as_ref(),
            stepper(2, 0.75, false).as_ref(),
            &world,
            &ctx(),
            &sample,
            fork_of(2),
            observe,
        )
        .unwrap();
        assert_eq!(world.get_component::<Speed>(sample[0]).unwrap().value, 1.0);
    }

    #[test]
    fn empty_sample_and_unknown_entities_are_rejected() {
        let (world, _) = world_with_speeds(1);
        let base = stepper(1, 0.5, false);
        let cand = stepper(2, 0.5, false);
        assert_eq!(
            shadow_compare(
                base.as_ref(),
                cand.as_ref(),
                &world,
                &ctx(),
                &[],
                fork_of(1),
                observe
            ),
            Err(ShadowError::EmptySample)
        );
        let mut other = World::new();
        other.spawn();
        let ghost = other.spawn();
        assert_eq!(
            shadow_compare(
                base.as_ref(),
                cand.as_ref(),
                &world,
                &ctx(),
                &[ghost],
                fork_of(1),
                observe
            ),
            Err(ShadowError::UnknownEntity(ghost))
        );
    }

    #[test]
    fn failing_candidate_surfaces_its_version() {
        let (world, sample) = world_with_speeds(1);
        let err = shadow_compare(
            stepper(1, 0.5, false).as_ref(),
            stepper(2, 0.5, true).as_ref(),
            &world,
            &ctx(),
            &sample,
            fork_of(1),
            observe,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ShadowError::Driver {
                version: DriverVersion::new(2, 0, 0),
                source: DriverError::Failed("boom".into()),
            }
        );
    }

    #[test]
    fn promotion_activates_only_clean_candidates() {
        let (world, sample) = world_with_speeds(2);
        let registry = DriverRegistry::new();
        let c = ComponentTypeId::of::<Speed>();
        let base: DriverHandle = stepper(1, 0.5, false);
        let good: DriverHandle = stepper(2, 0.5, false);
        let bad: DriverHandle = stepper(3, 0.9, false);
        registry.register(base.clone()).unwrap();
        registry.register(good.clone()).unwrap();
        registry.register(bad.clone()).unwrap();
        registry.activate(c, DriverVersion::new(1, 0, 0)).unwrap();

        let bad_report = shadow_compare(
            base.as_ref(),
            bad.as_ref(),
            &world,
            &ctx(),
            &sample,
            fork_of(2),
            observe,
        )
        .unwrap();
        assert!(matches!(
            promote_if_clean(&registry, c, &bad_report),
            Err(ShadowError::Divergent { divergent: 2, .. })
        ));
        assert_eq!(
            registry.snapshot().active_version(c),
            Some(DriverVersion::new(1, 0, 0))
        );

        let good_report = shadow_compare(
            base.as_ref(),
            good.as_ref(),
            &world,
            &ctx(),
            &sample,
            fork_of(2),
            observe,
        )
        .unwrap();
        promote_if_clean(&registry, c, &good_report).unwrap();
        assert_eq!(
            registry.snapshot().active_version(c),
            Some(DriverVersion::new(2, 0, 0))
        );
    }
}
