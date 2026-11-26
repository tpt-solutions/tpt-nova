//! End-to-end self-debugging loop: observe -> propose -> verify -> shadow ->
//! preserve -> deploy -> learn, with latency measured against a 60 Hz frame.
//!
//! Run: `cargo run -p tpt-nova-drivers --example selfdebug_loop`

use std::time::{Duration, Instant};

use tpt_nova_drivers::{
    repair, AffineModel, Bounds, Candidate, CapabilitySet, ClosureDriver, Driver, DriverContext,
    DriverError, DriverId, DriverRegistry, DriverVersion, FixMemory, Invariant, Monitor,
    RepairSite, ValueBound, Violation,
};
use tpt_nova_ecs::{Component, ComponentTypeId, Entity, World};

const FRAME: Duration = Duration::from_micros(16_667); // 60 Hz
const VMAX: i64 = 10;

#[derive(Debug, Default, Clone)]
struct Velocity {
    v: i64,
    tag: u32,
}
impl Component for Velocity {}

fn component() -> ComponentTypeId {
    ComponentTypeId::of::<Velocity>()
}

/// Multiply every velocity by `num/den` (integer, truncating toward zero).
fn scale(num: i64, den: i64) -> impl Fn(&DriverContext, &mut World) -> Result<(), DriverError> {
    move |_ctx, world| {
        for e in world.entities() {
            if let Some(vel) = world.get_component_mut::<Velocity>(e) {
                vel.v = (vel.v as i128 * num as i128 / den as i128) as i64;
            }
        }
        Ok(())
    }
}

fn candidate(major: u32, num: i64, den: i64) -> Candidate {
    let driver: std::sync::Arc<dyn Driver> = std::sync::Arc::new(ClosureDriver::new(
        DriverId::new("velocity-scale").unwrap(),
        DriverVersion::new(major, 0, 0),
        component(),
        CapabilitySet::empty(),
        scale(num, den),
    ));
    Candidate {
        driver,
        model: AffineModel::new(num, den, 0).unwrap(),
    }
}

fn invariant() -> impl Invariant {
    ValueBound::new(
        "velocity-bounded",
        Bounds::new(-VMAX, VMAX).unwrap(),
        |w: &World, e: Entity| w.get_component::<Velocity>(e).map(|v| v.v as f64),
    )
}

fn main() {
    // A checkpoint with two entities at velocity 8, one tick before the fault.
    let mut checkpoint = World::new();
    let sample: Vec<Entity> = (0..2)
        .map(|i| {
            let e = checkpoint.spawn();
            checkpoint.add_component(e, Velocity { v: 8, tag: 100 + i });
            e
        })
        .collect();
    let fork = |_: &World| {
        let mut w = World::new();
        for i in 0..2 {
            let e = w.spawn();
            w.add_component(e, Velocity { v: 8, tag: 100 + i });
        }
        w
    };
    let tag =
        |w: &World, e: Entity| serde_json::json!(w.get_component::<Velocity>(e).map(|v| v.tag));

    // The active driver is buggy: it scales by 3/2, so 8 becomes 12.
    let registry = DriverRegistry::new();
    let buggy = candidate(1, 3, 2);
    registry.register(buggy.driver.clone()).unwrap();
    registry
        .activate(component(), DriverVersion::new(1, 0, 0))
        .unwrap();

    let ctx = DriverContext::new(0, 1.0 / 60.0, CapabilitySet::empty());
    let mut after = fork(&checkpoint);
    registry
        .snapshot()
        .active(component())
        .unwrap()
        .update(&ctx, &mut after)
        .unwrap();

    let mut monitor = Monitor::new();
    monitor.add(invariant());
    let violation: Violation = monitor.check(&after).remove(0);
    println!(
        "observed: {} on entity {:?} = {}",
        violation.invariant, violation.entity, violation.observed
    );

    // Proposer: offer damping candidates in order. 3/2 is tried first and is
    // rejected by the verifier. 9/10 is the fix.
    let proposer =
        |_v: &Violation, _i: &dyn Invariant| vec![candidate(2, 3, 2), candidate(3, 9, 10)];
    let mut memory = FixMemory::new();
    let site = RepairSite {
        registry: &registry,
        component: component(),
        checkpoint: &checkpoint,
        ctx: &ctx,
        sample: &sample,
    };
    let inv = invariant();

    let started = Instant::now();
    let outcome = repair(&site, &inv, &violation, &proposer, &mut memory, fork, tag)
        .expect("a verified fix exists");
    let latency = started.elapsed();

    println!(
        "deployed {} after {} rejected candidate(s)",
        outcome.version,
        outcome.rejected.len()
    );
    for r in &outcome.rejected {
        println!("  rejected {}: {:?}", r.version, r.reason);
    }
    println!(
        "verified model: next = ({} * cur + {}) / {}",
        outcome.verified.model().num,
        outcome.verified.model().offset,
        outcome.verified.model().den
    );

    // Second fault on the same invariant: the learned fix is tried first.
    let started = Instant::now();
    let again = repair(
        &site,
        &inv,
        &violation,
        &|_: &Violation, _: &dyn Invariant| Vec::new(),
        &mut memory,
        fork,
        tag,
    )
    .expect("learned fix redeploys");
    let relearn = started.elapsed();
    println!(
        "learned fix redeployed as {} in {:?}",
        again.version, relearn
    );

    let verdict = if latency < FRAME { "within" } else { "OVER" };
    println!(
        "repair latency {:?} is {verdict} the 60 Hz frame budget ({:?})",
        latency, FRAME
    );
}
