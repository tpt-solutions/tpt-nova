//! The self-debugging repair loop (Phase 8, Layer 4).
//!
//! The loop runs when a [`Violation`] is observed:
//!
//! ```text
//! observe  (Monitor::check)                  -> Violation
//! propose  (Proposer)                        -> candidate drivers with affine models
//! verify   (verify::verify_affine)           -> Verified token, or reject
//! shadow   (run candidate on checkpoint)     -> invariant must hold on the sample
//! preserve (shadow_compare on unrelated)     -> fields the fix should not touch agree
//! deploy   (register + activate)             -> next snapshot uses the fix
//! learn    (FixMemory)                       -> the fix is tried first next time
//! ```
//!
//! Every candidate passes all gates in order. A candidate that fails one is
//! recorded as rejected, never registered, and the next candidate is tried.
//! The only activation path in this module requires a [`Verified`] token, so
//! the invariant "no driver is activated without symbolic verification" holds
//! for loop-driven activations. Direct `register`/`activate` calls on the
//! registry bypass this module and are the caller's responsibility.
//!
//! The checkpoint is the world before the offending tick. The shadow gate
//! runs each candidate from there, and the invariant must hold for the sampled
//! entities afterwards. That is an empirical check of the declared model.
//!
//! Latency is measured from the start of [`repair`] to activation, and returned
//! so callers can check it against a frame budget.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tpt_nova_ecs::{ComponentTypeId, Entity, World};

use crate::driver::{CapabilitySet, Driver, DriverContext, DriverError, DriverId, DriverVersion};
use crate::monitor::{Invariant, Violation};
use crate::registry::{DriverHandle, DriverRegistry, RegistryError};
use crate::shadow::{shadow_compare, ShadowError};
use crate::verify::{verify_affine, AffineModel, Verified, VerifyError};

/// A proposed fix: a driver plus the one-tick model it claims to satisfy.
#[derive(Clone)]
pub struct Candidate {
    pub driver: DriverHandle,
    pub model: AffineModel,
}

/// Source of candidate fixes for a violation. Patch generation plugs in here.
/// A closure `Fn(&Violation, &dyn Invariant) -> Vec<Candidate>` also implements
/// this trait.
pub trait Proposer: Send + Sync {
    fn propose(&self, violation: &Violation, invariant: &dyn Invariant) -> Vec<Candidate>;
}

impl<F> Proposer for F
where
    F: Fn(&Violation, &dyn Invariant) -> Vec<Candidate> + Send + Sync,
{
    fn propose(&self, violation: &Violation, invariant: &dyn Invariant) -> Vec<Candidate> {
        self(violation, invariant)
    }
}

/// Remembers the last fix that deployed for each invariant. It is retried
/// first, but it still passes every gate again.
#[derive(Default)]
pub struct FixMemory {
    learned: HashMap<String, Candidate>,
}

impl FixMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn learned(&self, invariant: &str) -> Option<&Candidate> {
        self.learned.get(invariant)
    }
}

/// Where and against what a repair is checked.
pub struct RepairSite<'a> {
    pub registry: &'a DriverRegistry,
    pub component: ComponentTypeId,
    /// World before the offending tick. Candidates are shadow-run from here.
    pub checkpoint: &'a World,
    pub ctx: &'a DriverContext,
    /// Entities the shadow and preserve gates examine.
    pub sample: &'a [Entity],
}

/// Why one candidate was not deployed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    Verify(VerifyError),
    /// The candidate failed to run on the checkpoint.
    Driver(DriverError),
    /// The invariant still fails on this many sampled entities after the fix.
    StillViolates(usize),
    /// Fields outside the fix changed on this many sampled entities.
    Diverged(usize),
    /// The shadow comparison could not run.
    Shadow(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedFix {
    pub version: DriverVersion,
    pub reason: RejectReason,
}

/// A successful repair.
#[derive(Debug, Clone)]
pub struct RepairOutcome {
    pub version: DriverVersion,
    pub verified: Verified,
    /// Time from the start of `repair` to activation.
    pub latency: Duration,
    /// Candidates that were tried and rejected before this one passed.
    pub rejected: Vec<RejectedFix>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepairError {
    #[error("no driver is active for this component")]
    NoActiveDriver,
    #[error("no candidate passed every gate ({} rejected)", .rejected.len())]
    NoVerifiedFix { rejected: Vec<RejectedFix> },
    #[error("registry refused the deployment: {0}")]
    Registry(#[from] RegistryError),
}

/// Run the loop for one violation. Returns the deployed fix, or every rejection.
///
/// `fork` builds an independent copy of a world; `World` is not `Clone`.
/// `preserve` projects the state the fix must *not* change, so it is compared
/// against the active driver's output.
pub fn repair<Fork, Preserve>(
    site: &RepairSite<'_>,
    invariant: &dyn Invariant,
    violation: &Violation,
    proposer: &dyn Proposer,
    memory: &mut FixMemory,
    fork: Fork,
    preserve: Preserve,
) -> Result<RepairOutcome, RepairError>
where
    Fork: Fn(&World) -> World,
    Preserve: Fn(&World, Entity) -> serde_json::Value,
{
    let start = Instant::now();
    let baseline = site
        .registry
        .snapshot()
        .active(site.component)
        .ok_or(RepairError::NoActiveDriver)?;

    // The learned fix goes first. It still passes every gate.
    let mut candidates: Vec<Candidate> = memory
        .learned(invariant.name())
        .cloned()
        .into_iter()
        .collect();
    candidates.extend(proposer.propose(violation, invariant));

    let mut rejected = Vec::new();
    for candidate in candidates {
        let version = candidate.driver.version();
        let verified = match gate(site, invariant, &baseline, &candidate, &fork, &preserve) {
            Ok(verified) => verified,
            Err(reason) => {
                rejected.push(RejectedFix { version, reason });
                continue;
            }
        };

        match site.registry.register(candidate.driver.clone()) {
            Ok(()) | Err(RegistryError::Duplicate { .. }) => {}
            Err(e) => return Err(e.into()),
        }
        site.registry
            .activate_verified(site.component, version, &verified)?;
        memory
            .learned
            .insert(invariant.name().to_string(), candidate);

        return Ok(RepairOutcome {
            version,
            verified,
            latency: start.elapsed(),
            rejected,
        });
    }

    Err(RepairError::NoVerifiedFix { rejected })
}

/// Run one candidate through verify, shadow, and preserve, in that order.
fn gate<Fork, Preserve>(
    site: &RepairSite<'_>,
    invariant: &dyn Invariant,
    baseline: &DriverHandle,
    candidate: &Candidate,
    fork: &Fork,
    preserve: &Preserve,
) -> Result<Verified, RejectReason>
where
    Fork: Fn(&World) -> World,
    Preserve: Fn(&World, Entity) -> serde_json::Value,
{
    let bounds = invariant.bounds();
    let verified = verify_affine(candidate.model, bounds, bounds).map_err(RejectReason::Verify)?;

    let mut after = fork(site.checkpoint);
    candidate
        .driver
        .update(site.ctx, &mut after)
        .map_err(RejectReason::Driver)?;
    let still = invariant
        .check(&after)
        .iter()
        .filter(|v| site.sample.contains(&v.entity))
        .count();
    if still > 0 {
        return Err(RejectReason::StillViolates(still));
    }

    let report = shadow_compare(
        baseline.as_ref(),
        candidate.driver.as_ref(),
        site.checkpoint,
        site.ctx,
        site.sample,
        fork,
        preserve,
    )
    .map_err(|e| match e {
        ShadowError::Driver { source, .. } => RejectReason::Driver(source),
        other => RejectReason::Shadow(other.to_string()),
    })?;
    // The fix is expected to change the invariant's field, so divergence is
    // only fatal for the fields `preserve` selects. Here `preserve` is the
    // unrelated state, so any divergence is a real side effect.
    if !report.is_clean() {
        return Err(RejectReason::Diverged(report.divergent.len()));
    }

    Ok(verified)
}

/// A driver built from a closure, for fixes and tests. Its `requested`
/// capabilities are the ones the closure needs.
pub struct ClosureDriver<F>
where
    F: Fn(&DriverContext, &mut World) -> Result<(), DriverError> + Send + Sync,
{
    id: DriverId,
    version: DriverVersion,
    component: ComponentTypeId,
    caps: CapabilitySet,
    update: F,
}

impl<F> ClosureDriver<F>
where
    F: Fn(&DriverContext, &mut World) -> Result<(), DriverError> + Send + Sync,
{
    pub fn new(
        id: DriverId,
        version: DriverVersion,
        component: ComponentTypeId,
        caps: CapabilitySet,
        update: F,
    ) -> Self {
        ClosureDriver {
            id,
            version,
            component,
            caps,
            update,
        }
    }
}

impl<F> Driver for ClosureDriver<F>
where
    F: Fn(&DriverContext, &mut World) -> Result<(), DriverError> + Send + Sync,
{
    fn id(&self) -> DriverId {
        self.id.clone()
    }
    fn version(&self) -> DriverVersion {
        self.version
    }
    fn component(&self) -> ComponentTypeId {
        self.component
    }
    fn requested_capabilities(&self) -> CapabilitySet {
        self.caps.clone()
    }
    fn update(&self, ctx: &DriverContext, world: &mut World) -> Result<(), DriverError> {
        (self.update)(ctx, world)
    }
}

/// Convenience to wrap a closure driver with its model as a [`Candidate`].
pub fn candidate<F>(driver: ClosureDriver<F>, model: AffineModel) -> Candidate
where
    F: Fn(&DriverContext, &mut World) -> Result<(), DriverError> + Send + Sync + 'static,
{
    Candidate {
        driver: Arc::new(driver),
        model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{Monitor, ValueBound};
    use crate::verify::Bounds;
    use tpt_nova_ecs::Component;

    /// Per-entity velocity. `v` is repaired; `tag` must never change.
    #[derive(Debug, Default, Clone)]
    struct Velocity {
        v: i64,
        tag: u32,
    }
    impl Component for Velocity {}

    const VMAX: i64 = 10;
    const COMPONENT_CAPS: fn() -> CapabilitySet = CapabilitySet::empty;

    fn component() -> ComponentTypeId {
        ComponentTypeId::of::<Velocity>()
    }

    /// Applies `v -> num/den * v` with integer `v`, kept in integer arithmetic
    /// so the driver matches the declared model exactly.
    fn scale(num: i64, den: i64) -> impl Fn(&DriverContext, &mut World) -> Result<(), DriverError> {
        move |_ctx, world| {
            let entities = world.entities();
            for e in entities {
                if let Some(vel) = world.get_component_mut::<Velocity>(e) {
                    let scaled = (vel.v as i128 * num as i128).div_euclid(den as i128);
                    vel.v = scaled as i64;
                }
            }
            Ok(())
        }
    }

    fn driver(major: u32, num: i64, den: i64) -> Candidate {
        candidate(
            ClosureDriver::new(
                DriverId::new("velocity-scale").unwrap(),
                DriverVersion::new(major, 0, 0),
                component(),
                COMPONENT_CAPS(),
                scale(num, den),
            ),
            AffineModel::new(num, den, 0).unwrap(),
        )
    }

    fn invariant() -> impl Invariant {
        ValueBound::new(
            "velocity-bounded",
            Bounds::new(-VMAX, VMAX).unwrap(),
            |w: &World, e| w.get_component::<Velocity>(e).map(|v| v.v as f64),
        )
    }

    /// Two entities at 8 (in bounds). The buggy active driver scales by 3/2,
    /// which pushes 8 to 12.
    fn checkpoint() -> (World, Vec<Entity>) {
        let mut world = World::new();
        let entities: Vec<Entity> = (0..2)
            .map(|i| {
                let e = world.spawn();
                world.add_component(e, Velocity { v: 8, tag: 100 + i });
                e
            })
            .collect();
        (world, entities)
    }

    fn fork_of() -> impl Fn(&World) -> World {
        |_| checkpoint().0
    }

    fn tag(w: &World, e: Entity) -> serde_json::Value {
        serde_json::json!(w.get_component::<Velocity>(e).map(|v| v.tag))
    }

    /// Registry with the buggy 3/2 driver active, and a monitor that has seen the
    /// violation after one buggy tick.
    fn scenario() -> (DriverRegistry, Vec<Entity>, Violation, World, DriverContext) {
        let registry = DriverRegistry::with_policy(COMPONENT_CAPS());
        registry.register(driver(1, 3, 2).driver).unwrap();
        registry
            .activate(component(), DriverVersion::new(1, 0, 0))
            .unwrap();

        let (world, sample) = checkpoint();
        let mut after = fork_of()(&world);
        let buggy = registry.snapshot().active(component()).unwrap();
        let ctx = DriverContext::new(0, 1.0 / 60.0, COMPONENT_CAPS());
        buggy.update(&ctx, &mut after).unwrap();

        let mut monitor = Monitor::new();
        monitor.add(invariant());
        let violation = monitor.check(&after).remove(0);
        (registry, sample, violation, world, ctx)
    }

    #[test]
    fn a_verified_safe_fix_is_deployed_and_the_bug_is_gone() {
        let (registry, sample, violation, world, ctx) = scenario();
        let proposer = |_v: &Violation, _i: &dyn Invariant| vec![driver(2, 9, 10)];
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let inv = invariant();
        let out = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .expect("repairs");

        assert_eq!(out.version, DriverVersion::new(2, 0, 0));
        assert_eq!(out.verified.model(), AffineModel::new(9, 10, 0).unwrap());
        assert!(out.rejected.is_empty());
        assert_eq!(
            registry.snapshot().active_version(component()),
            Some(DriverVersion::new(2, 0, 0))
        );
        // The learned fix is recorded for next time.
        assert!(memory.learned("velocity-bounded").is_some());
    }

    #[test]
    fn a_fix_that_the_verifier_rejects_is_never_deployed() {
        let (registry, sample, violation, world, ctx) = scenario();
        // The buggy 3/2 model again, offered as the "fix". The verifier rejects it.
        let proposer = |_v: &Violation, _i: &dyn Invariant| vec![driver(2, 3, 2)];
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let inv = invariant();
        let err = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .unwrap_err();
        match err {
            RepairError::NoVerifiedFix { rejected } => {
                assert_eq!(rejected.len(), 1);
                assert!(matches!(rejected[0].reason, RejectReason::Verify(_)));
            }
            other => panic!("expected NoVerifiedFix, got {other:?}"),
        }
        assert_eq!(
            registry.snapshot().active_version(component()),
            Some(DriverVersion::new(1, 0, 0))
        );
        assert!(memory.learned("velocity-bounded").is_none());
    }

    #[test]
    fn a_verified_model_whose_code_disagrees_is_caught_by_the_shadow_gate() {
        let (registry, sample, violation, world, ctx) = scenario();
        // Declares 9/10 (so it verifies) but its code actually scales by 3/2.
        let lying = candidate(
            ClosureDriver::new(
                DriverId::new("velocity-scale").unwrap(),
                DriverVersion::new(2, 0, 0),
                component(),
                COMPONENT_CAPS(),
                scale(3, 2),
            ),
            AffineModel::new(9, 10, 0).unwrap(),
        );
        let proposer = move |_v: &Violation, _i: &dyn Invariant| vec![lying.clone()];
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let inv = invariant();
        let err = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .unwrap_err();
        match err {
            RepairError::NoVerifiedFix { rejected } => {
                assert_eq!(rejected[0].reason, RejectReason::StillViolates(2));
            }
            other => panic!("expected NoVerifiedFix, got {other:?}"),
        }
        assert_eq!(
            registry.snapshot().active_version(component()),
            Some(DriverVersion::new(1, 0, 0))
        );
    }

    #[test]
    fn a_fix_that_touches_unrelated_state_is_rejected_by_the_preserve_gate() {
        let (registry, sample, violation, world, ctx) = scenario();
        // Correct velocity scaling, but also rewrites `tag`.
        let sloppy = candidate(
            ClosureDriver::new(
                DriverId::new("velocity-scale").unwrap(),
                DriverVersion::new(2, 0, 0),
                component(),
                COMPONENT_CAPS(),
                |ctx: &DriverContext, world: &mut World| {
                    scale(9, 10)(ctx, world)?;
                    for e in world.entities() {
                        if let Some(v) = world.get_component_mut::<Velocity>(e) {
                            v.tag += 1;
                        }
                    }
                    Ok(())
                },
            ),
            AffineModel::new(9, 10, 0).unwrap(),
        );
        let proposer = move |_v: &Violation, _i: &dyn Invariant| vec![sloppy.clone()];
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let inv = invariant();
        let err = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .unwrap_err();
        match err {
            RepairError::NoVerifiedFix { rejected } => {
                assert_eq!(rejected[0].reason, RejectReason::Diverged(2));
            }
            other => panic!("expected NoVerifiedFix, got {other:?}"),
        }
    }

    #[test]
    fn the_learned_fix_is_tried_first_and_still_passes_the_gates() {
        let (registry, sample, violation, world, ctx) = scenario();
        let good = driver(2, 9, 10);
        let inv = invariant();
        let mut memory = FixMemory::new();
        memory.learned.insert(inv.name().to_string(), good.clone());

        // A proposer that offers nothing, so only the learned fix can win.
        let proposer = |_v: &Violation, _i: &dyn Invariant| Vec::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let out = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .expect("the learned fix redeploys");
        assert_eq!(out.version, DriverVersion::new(2, 0, 0));
    }

    #[test]
    fn repair_reports_latency() {
        let (registry, sample, violation, world, ctx) = scenario();
        let proposer = |_v: &Violation, _i: &dyn Invariant| vec![driver(2, 9, 10)];
        let mut memory = FixMemory::new();
        let site = RepairSite {
            registry: &registry,
            component: component(),
            checkpoint: &world,
            ctx: &ctx,
            sample: &sample,
        };
        let inv = invariant();
        let out = repair(
            &site,
            &inv,
            &violation,
            &proposer,
            &mut memory,
            fork_of(),
            tag,
        )
        .unwrap();
        assert!(out.latency < Duration::from_secs(5));
    }
}
