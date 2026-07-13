//! Demonstrates the embedded scripting sandbox: an AI-style script is run under
//! a restricted capability set, then its commands are applied to a `World`.
//! Run with `cargo run -p nova-scripting-embedded --example sandbox`.

use nova_ecs::World;
use nova_scripting_embedded::{Capabilities, Capability, EmbeddedRuntime};

fn main() {
    // An AI-generated script is only trusted with spawn + write + log + net.
    let caps = Capabilities::none()
        .grant(Capability::Spawn)
        .grant(Capability::WriteWorld)
        .grant(Capability::Log)
        .grant(Capability::Net)
        .clone();

    let mut rt = EmbeddedRuntime::new(caps);
    let mut world = World::new();

    let script = r#"
        let hero = spawn_entity();
        set_transform(hero, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0);
        let crate = spawn_entity();
        set_transform(crate, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5);
        log("spawned hero + crate");
        emit_event("level_ready");
    "#;

    match rt.run_and_apply(script, &mut world) {
        Ok(()) => {
            println!("applied script; entities = {}", world.entity_count());
            println!("captured logs: {:?}", rt.take_logs());
        }
        Err(e) => {
            println!("script rejected by sandbox: {e}");
        }
    }

    // A script that exceeds its capabilities is simply rejected at compile time.
    let mut denied_rt = EmbeddedRuntime::new(Capabilities::none().grant(Capability::Log).clone());
    let mut other_world = World::new();
    let err = denied_rt
        .run_and_apply("spawn_entity();", &mut other_world)
        .unwrap_err();
    println!("denied-capability script failed as expected: {err}");
}
