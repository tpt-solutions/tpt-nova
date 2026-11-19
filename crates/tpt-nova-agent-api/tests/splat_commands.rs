//! Agent commands for splat entities: crop and proxy regeneration.

use tpt_nova_agent_api::{apply_command, AgentCommand, EntityRef};
use tpt_nova_ecs::transform::Transform;
use tpt_nova_ecs::World;
use tpt_nova_ingest::Collider3D;
use tpt_nova_splat::{Splat, SplatCloud};

fn splat(x: f32) -> Splat {
    Splat {
        position: [x, 0.0, 0.0],
        scale: [0.1; 3],
        rotation: [1.0, 0.0, 0.0, 0.0],
        color: [1.0; 4],
        opacity: 1.0,
    }
}

fn spawn_splat_entity(world: &mut World, n: usize) -> EntityRef {
    let e = world.spawn();
    world.add_component(e, Transform::default());
    world.add_component(
        e,
        SplatCloud::new((0..n).map(|i| splat(i as f32)).collect()),
    );
    EntityRef::Id(format!("{e}"))
}

#[test]
fn crop_splat_keeps_only_boxed_splats() {
    let mut world = World::new();
    let target = spawn_splat_entity(&mut world, 10);
    apply_command(
        &mut world,
        &AgentCommand::CropSplat {
            target: target.clone(),
            min: [2.5, -1.0, -1.0],
            max: [5.5, 1.0, 1.0],
        },
    )
    .unwrap();
    let e = tpt_nova_agent_api::EntityRef::from_id_string(match &target {
        EntityRef::Id(s) => s,
        EntityRef::Name(_) => unreachable!(),
    })
    .unwrap();
    assert_eq!(world.get_component::<SplatCloud>(e).unwrap().len(), 3);
}

#[test]
fn crop_on_non_splat_entity_is_rejected() {
    let mut world = World::new();
    let e = world.spawn();
    world.add_component(e, Transform::default());
    let err = apply_command(
        &mut world,
        &AgentCommand::CropSplat {
            target: EntityRef::Id(format!("{e}")),
            min: [0.0; 3],
            max: [1.0; 3],
        },
    );
    assert!(err.is_err());
}

#[test]
fn regenerate_proxy_attaches_collider() {
    let mut world = World::new();
    let target = spawn_splat_entity(&mut world, 64);
    apply_command(
        &mut world,
        &AgentCommand::RegenerateSplatProxy {
            target: target.clone(),
            params: None,
        },
    )
    .unwrap();
    let e = tpt_nova_agent_api::EntityRef::from_id_string(match &target {
        EntityRef::Id(s) => s,
        EntityRef::Name(_) => unreachable!(),
    })
    .unwrap();
    assert!(world.get_component::<Collider3D>(e).is_some());
}
