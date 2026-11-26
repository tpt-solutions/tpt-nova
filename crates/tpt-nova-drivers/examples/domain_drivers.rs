//! Example drivers for the three planned domain layers: games, simulation, and
//! visualisation. Each is a declarative fix, compiled and verified by the same path
//! the repair loop uses.
//!
//! The domain repos (`tpt-nova-games`, `tpt-nova-sim`, `tpt-nova-viz`) are at the
//! planning stage. Their TODOs wait on `tpt-nova-drivers`, so these examples show
//! the intended shape of a domain driver against the engine's own API.
//!
//! Run: `cargo run -p tpt-nova-drivers --example domain_drivers`

use std::sync::Arc;

use tpt_nova_drivers::compile::{compile, FixSpec, FloatField};
use tpt_nova_drivers::{
    verify_affine, Bounds, CapabilitySet, DriverContext, DriverId, DriverVersion,
};
use tpt_nova_ecs::{Component, ComponentTypeId, Entity, World};

/// A per-entity `f32` field of one component, read and written through closures.
struct Field<C: Component> {
    get: fn(&C) -> f32,
    set: fn(&mut C, f32),
}

impl<C: Component> FloatField for Field<C> {
    fn read(&self, w: &World, e: Entity) -> Option<f32> {
        w.get_component::<C>(e).map(|c| (self.get)(c))
    }
    fn write(&self, w: &mut World, e: Entity, v: f32) {
        if let Some(c) = w.get_component_mut::<C>(e) {
            (self.set)(c, v);
        }
    }
}

#[derive(Debug, Default)]
struct Speed(f32);
impl Component for Speed {}

#[derive(Debug, Default)]
struct Temperature(f32);
impl Component for Temperature {}

#[derive(Debug, Default)]
struct Intensity(f32);
impl Component for Intensity {}

struct Domain {
    name: &'static str,
    spec: FixSpec,
    /// The range the field lives in. The verifier must show the fix keeps it there.
    range: Bounds,
    describe: &'static str,
}

fn spec<C: Component>(id: &str, num: i64, den: i64, offset: i64) -> FixSpec {
    FixSpec {
        id: DriverId::new(id).unwrap(),
        version: DriverVersion::new(1, 0, 0),
        component: ComponentTypeId::of::<C>(),
        num,
        den,
        offset,
    }
}

fn main() {
    let domains = [
        Domain {
            name: "games",
            spec: spec::<Speed>("speed-damping", 49, 50, 0),
            range: Bounds::new(0, 100).unwrap(),
            describe: "2% linear drag each tick",
        },
        Domain {
            name: "sim",
            // T' = T + 0.1 * (20 - T) = 0.9*T + 2, written as (9*T + 20) / 10.
            spec: spec::<Temperature>("thermal-relaxation", 9, 10, 20),
            range: Bounds::new(0, 100).unwrap(),
            describe: "relax toward 20 degrees at rate 0.1",
        },
        Domain {
            name: "viz",
            spec: spec::<Intensity>("intensity-normalise", 1, 2, 0),
            range: Bounds::new(0, 100).unwrap(),
            describe: "halve the intensity scale",
        },
    ];

    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Speed(80.0));
    world.add_component(e, Temperature(60.0));
    world.add_component(e, Intensity(90.0));

    let ctx = DriverContext::new(0, 1.0 / 60.0, CapabilitySet::empty());
    for d in domains {
        let model_ok = verify_affine(
            tpt_nova_drivers::AffineModel::new(d.spec.num, d.spec.den, d.spec.offset).unwrap(),
            d.range,
            d.range,
        )
        .is_ok();
        println!(
            "[{}] {} -> verified={} (model {}/{} + {})",
            d.name, d.describe, model_ok, d.spec.num, d.spec.den, d.spec.offset
        );
        if !model_ok {
            continue;
        }

        // Each domain writes to its own field. Compile against that component.
        let candidate = match d.name {
            "games" => compile(
                &d.spec,
                Arc::new(Field::<Speed> {
                    get: |c| c.0,
                    set: |c, v| c.0 = v,
                }),
            ),
            "sim" => compile(
                &d.spec,
                Arc::new(Field::<Temperature> {
                    get: |c| c.0,
                    set: |c, v| c.0 = v,
                }),
            ),
            _ => compile(
                &d.spec,
                Arc::new(Field::<Intensity> {
                    get: |c| c.0,
                    set: |c, v| c.0 = v,
                }),
            ),
        }
        .expect("the spec compiles");
        candidate.driver.update(&ctx, &mut world).expect("runs");
    }

    println!(
        "after one tick: speed={:.2} temperature={:.2} intensity={:.2}",
        world.get_component::<Speed>(e).unwrap().0,
        world.get_component::<Temperature>(e).unwrap().0,
        world.get_component::<Intensity>(e).unwrap().0,
    );
}
