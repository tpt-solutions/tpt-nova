//! Internal fix compiler and patch search (Phase 8, Layer 4).
//!
//! These replace the planned `tpt-crucible` (candidate compilation) and `tpt-eve`
//! (patch generation) integrations.
//!
//! - [`FloatFix`] compiles a declarative fix, `value' = value * num/den + offset`
//!   on one `f32` field, into a [`Candidate`]. The driver and its affine model are
//!   built from the same spec, so the declared model cannot drift from the code.
//!   The verifier and shadow gates still run, but the spec removes one way to
//!   lie.
//! - [`GainSearch`] proposes fixes: it walks a ladder of contraction gains, and
//!   each is compiled and sent to the gates. The ordering is deterministic. The
//!   verifier decides what passes.
//!
//! Limit: the model is over the reals, while the driver computes in `f32`. A
//! result can exceed a bound by one unit in the last place when it lands exactly
//! on the edge. Keep bounds with a margin, and rely on the shadow gate for the
//! rest.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tpt_nova_ecs::{ComponentTypeId, Entity, World};

use crate::driver::{CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion};
use crate::monitor::{Invariant, Violation};
use crate::repair::{Candidate, Proposer};
use crate::verify::{AffineModel, VerifyError};

/// Access to one `f32` value per entity. Implemented by the caller for the
/// component field a fix targets.
pub trait FloatField: Send + Sync + 'static {
    fn read(&self, world: &World, entity: Entity) -> Option<f32>;
    fn write(&self, world: &mut World, entity: Entity, value: f32);
}

/// The declarative description of a fix. This is the single source for both the
/// driver's code and its model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixSpec {
    pub id: DriverId,
    pub version: DriverVersion,
    pub component: ComponentTypeId,
    /// Slope numerator; the slope is `num / den`.
    pub num: i64,
    /// Slope denominator; must be positive.
    pub den: i64,
    pub offset: i64,
}

/// Compile a [`FixSpec`] over `field` into a candidate.
pub fn compile<F: FloatField>(spec: &FixSpec, field: Arc<F>) -> Result<Candidate, VerifyError> {
    let model = AffineModel::new(spec.num, spec.den, spec.offset)?;
    let driver = FixDriver {
        spec: spec.clone(),
        field,
    };
    Ok(Candidate {
        driver: Arc::new(driver),
        model,
    })
}

struct FixDriver<F: FloatField> {
    spec: FixSpec,
    field: Arc<F>,
}

impl<F: FloatField> Driver for FixDriver<F> {
    fn id(&self) -> DriverId {
        self.spec.id.clone()
    }
    fn version(&self) -> DriverVersion {
        self.spec.version
    }
    fn component(&self) -> ComponentTypeId {
        self.spec.component
    }
    fn requested_capabilities(&self) -> CapabilitySet {
        CapabilitySet::empty()
    }
    fn update(&self, _ctx: &DriverContext, world: &mut World) -> Result<(), DriverError> {
        // Same arithmetic the model describes: slope = num/den, plus offset/den.
        let slope = self.spec.num as f64 / self.spec.den as f64;
        let intercept = self.spec.offset as f64 / self.spec.den as f64;
        for entity in world.entities() {
            if let Some(v) = self.field.read(world, entity) {
                let next = (v as f64 * slope + intercept) as f32;
                self.field.write(world, entity, next);
            }
        }
        Ok(())
    }
}

/// Proposes contraction gains in a fixed order. Each call takes fresh versions, so
/// a rejected version is never reused.
pub struct GainSearch<F: FloatField> {
    id: DriverId,
    component: ComponentTypeId,
    field: Arc<F>,
    base: DriverVersion,
    /// `(num, den)` pairs, tried in order. Gentler gains come first.
    ladder: Vec<(i64, i64)>,
    next_patch: AtomicU32,
}

/// The default ladder: damping gains from gentle to strong.
pub const DEFAULT_LADDER: &[(i64, i64)] = &[(9, 10), (3, 4), (1, 2)];

impl<F: FloatField> GainSearch<F> {
    pub fn new(
        id: DriverId,
        component: ComponentTypeId,
        field: Arc<F>,
        base: DriverVersion,
        ladder: &[(i64, i64)],
    ) -> Self {
        GainSearch {
            id,
            component,
            field,
            base,
            ladder: ladder.to_vec(),
            next_patch: AtomicU32::new(1),
        }
    }
}

impl<F: FloatField> Proposer for GainSearch<F> {
    fn propose(&self, _violation: &Violation, _invariant: &dyn Invariant) -> Vec<Candidate> {
        self.ladder
            .iter()
            .filter_map(|&(num, den)| {
                let patch = self.next_patch.fetch_add(1, Ordering::Relaxed);
                let spec = FixSpec {
                    id: self.id.clone(),
                    version: DriverVersion::new(self.base.major, self.base.minor, patch),
                    component: self.component,
                    num,
                    den,
                    offset: 0,
                };
                compile(&spec, self.field.clone()).ok()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::ValueBound;
    use crate::registry::DriverRegistry;
    use crate::repair::{repair, FixMemory, RepairSite};
    use crate::verify::{verify_affine, Bounds};
    use tpt_nova_ecs::Component;

    #[derive(Debug, Default)]
    struct Velocity {
        v: f32,
        tag: u32,
    }
    impl Component for Velocity {}

    struct VelocityField;
    impl FloatField for VelocityField {
        fn read(&self, w: &World, e: Entity) -> Option<f32> {
            w.get_component::<Velocity>(e).map(|v| v.v)
        }
        fn write(&self, w: &mut World, e: Entity, value: f32) {
            if let Some(v) = w.get_component_mut::<Velocity>(e) {
                v.v = value;
            }
        }
    }

    fn component() -> ComponentTypeId {
        ComponentTypeId::of::<Velocity>()
    }

    fn spec(major: u32, num: i64, den: i64) -> FixSpec {
        FixSpec {
            id: DriverId::new("velocity-fix").unwrap(),
            version: DriverVersion::new(major, 0, 0),
            component: component(),
            num,
            den,
            offset: 0,
        }
    }

    #[test]
    fn a_compiled_fix_carries_the_model_its_spec_declares() {
        let c = compile(&spec(2, 9, 10), Arc::new(VelocityField)).unwrap();
        assert_eq!(c.model, AffineModel::new(9, 10, 0).unwrap());
        assert!(verify_affine(
            c.model,
            Bounds::new(-10, 10).unwrap(),
            Bounds::new(-10, 10).unwrap()
        )
        .is_ok());
    }

    #[test]
    fn a_compiled_driver_moves_the_field_as_its_model_says() {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Velocity { v: 8.0, tag: 0 });
        let c = compile(&spec(2, 9, 10), Arc::new(VelocityField)).unwrap();
        let ctx = DriverContext::new(0, 1.0 / 60.0, CapabilitySet::empty());
        c.driver.update(&ctx, &mut world).unwrap();
        let v = world.get_component::<Velocity>(e).unwrap().v;
        assert!((v - 7.2).abs() < 1e-5, "got {v}");
    }

    #[test]
    fn a_zero_denominator_spec_does_not_compile() {
        assert!(compile(&spec(2, 1, 0), Arc::new(VelocityField)).is_err());
    }

    fn tag(w: &World, e: Entity) -> serde_json::Value {
        serde_json::json!(w.get_component::<Velocity>(e).map(|v| v.tag))
    }

    /// End to end: the active driver is buggy (gain 3/2), and the search has to
    /// find a verified contraction. Both the search and the repair gates run.
    #[test]
    fn gain_search_repairs_a_growing_driver() {
        let field = Arc::new(VelocityField);
        let registry = DriverRegistry::with_policy(CapabilitySet::empty());

        let mut checkpoint = World::new();
        let sample: Vec<Entity> = (0..2)
            .map(|i| {
                let e = checkpoint.spawn();
                checkpoint.add_component(
                    e,
                    Velocity {
                        v: 8.0,
                        tag: 100 + i,
                    },
                );
                e
            })
            .collect();
        let fork = |_: &World| {
            let mut w = World::new();
            for i in 0..2 {
                let e = w.spawn();
                w.add_component(
                    e,
                    Velocity {
                        v: 8.0,
                        tag: 100 + i,
                    },
                );
            }
            w
        };

        // The buggy active driver: gain 3/2 (declared honestly as its model).
        let buggy = compile(&spec(1, 3, 2), field.clone()).unwrap();
        registry.register(buggy.driver.clone()).unwrap();
        registry
            .activate(component(), DriverVersion::new(1, 0, 0))
            .unwrap();

        let ctx = DriverContext::new(0, 1.0 / 60.0, CapabilitySet::empty());
        let mut after = fork(&checkpoint);
        buggy.driver.update(&ctx, &mut after).unwrap();

        let inv = ValueBound::new(
            "velocity-bounded",
            Bounds::new(-10, 10).unwrap(),
            |w: &World, e| w.get_component::<Velocity>(e).map(|v| v.v as f64),
        );
        let mut monitor = crate::monitor::Monitor::new();
        monitor.add(ValueBound::new(
            "velocity-bounded",
            Bounds::new(-10, 10).unwrap(),
            |w: &World, e| w.get_component::<Velocity>(e).map(|v| v.v as f64),
        ));
        let violation = monitor.check(&after).remove(0);

        let search = GainSearch::new(
            DriverId::new("velocity-fix").unwrap(),
            component(),
            field,
            DriverVersion::new(2, 0, 0),
            DEFAULT_LADDER,
        );
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &checkpoint,
            ctx: &ctx,
            sample: &sample,
        };
        let out = repair(&site, &inv, &violation, &search, &mut memory, fork, tag)
            .expect("a verified contraction exists");
        assert_eq!(out.verified.model(), AffineModel::new(9, 10, 0).unwrap());
    }
}
