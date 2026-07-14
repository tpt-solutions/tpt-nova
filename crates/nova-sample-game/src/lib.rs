//! An end-to-end sample game that exercises the **entire** TPT Nova pipeline.
//!
//! This is the proof point for the Alpha: a playable-shaped world assembled from
//! every subsystem — ECS entities, Rapier3D physics, scene save/load, the
//! external agent control API, Gaussian Splat ingestion, and standalone
//! packaging. Everything here runs headless (no window/GPU) so the same code
//! path is exercised by both the shipped demo and the test suite.

use nova_agent_api::{apply_command, AgentCommand, EntityRef};
use nova_ecs::transform::{GlobalTransform, Transform};
use nova_ecs::{Entity, Vec3, World};
use nova_ingest::decompose::ConvexPart;
use nova_ingest::{step_physics_3d, Collider3D, PhysicsWorld3D, RigidBody3D};
use nova_splat::{build_convex_hull_collider, load_splat_bytes, SplatCloud};

/// One axis-aligned box collider (8 corners, 12 triangles) of half-extents
/// `(hx, hy, hz)`, centered at the origin — the building block for the sample.
pub fn box_collider(hx: f32, hy: f32, hz: f32) -> Collider3D {
    let v = [
        [-hx, -hy, -hz],
        [hx, -hy, -hz],
        [hx, hy, -hz],
        [-hx, hy, -hz],
        [-hx, -hy, hz],
        [hx, -hy, hz],
        [hx, hy, hz],
        [-hx, hy, hz],
    ];
    let idx: [[u32; 3]; 12] = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [1, 5, 6],
        [1, 6, 2],
        [2, 6, 7],
        [2, 7, 3],
        [3, 7, 4],
        [3, 4, 0],
    ];
    Collider3D::from_parts(vec![ConvexPart {
        vertices: v.to_vec(),
        indices: idx.to_vec(),
    }])
}

/// Build the sample world: a dynamic player that will fall onto a fixed
/// ground. Returns the world and the player entity handle.
pub fn build_world() -> (World, Entity) {
    let mut world = World::new();
    world.add_resource(PhysicsWorld3D::default());

    // Ground: a wide, thin fixed slab whose top sits at y = 0.
    let ground = world.spawn();
    world.add_component(
        ground,
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    );
    world.add_component(ground, RigidBody3D::fixed());
    world.add_component(ground, box_collider(10.0, 0.5, 10.0));

    // Player: a 1×1×1 dynamic box starting 3 units above the ground.
    let player = world.spawn();
    world.add_component(
        player,
        Transform::from_translation(Vec3::new(0.0, 3.0, 0.0)),
    );
    world.add_component(player, RigidBody3D::dynamic());
    world.add_component(player, box_collider(0.5, 0.5, 0.5));
    world.add_component(player, GlobalTransform::identity());

    (world, player)
}

/// Advance the sample simulation by `steps` fixed timesteps.
pub fn tick(world: &mut World, steps: u32) {
    for _ in 0..steps {
        step_physics_3d(world, 1.0 / 60.0);
    }
}

/// The player's current Y position (after physics).
pub fn player_y(world: &World, player: Entity) -> f32 {
    world
        .get_component::<Transform>(player)
        .map(|t| t.translation.y)
        .unwrap_or(f32::NAN)
}

/// A round-trip demonstrating scene save/load + external agent control +
/// splat ingestion + asset packaging, all in one flow. Returns a short summary
/// string suitable for printing from the demo binary.
pub fn run_pipeline() -> String {
    // 1) Physics world.
    let (mut world, player) = build_world();
    tick(&mut world, 120);
    let rested_y = player_y(&world, player);
    assert!(
        (0.0..1.5).contains(&rested_y),
        "player should rest near ground"
    );

    // 2) Save + reload the scene.
    let dir = std::env::temp_dir();
    let scene_path = dir.join("nova_sample_scene.ron");
    nova_scene::save_to_file(&world, &scene_path).unwrap();
    let reloaded = nova_scene::load_from_file(&scene_path).unwrap();
    assert_eq!(reloaded.entity_count(), world.entity_count());

    // 3) External agent spawns a new entity and moves the player.
    apply_command(
        &mut world,
        &AgentCommand::Spawn {
            name: Some("npc".into()),
            translation: [2.0, 0.0, 0.0],
            mesh: Some("cube".into()),
        },
    )
    .unwrap();
    apply_command(
        &mut world,
        &AgentCommand::SetTransform {
            target: EntityRef::Name("npc".into()),
            translation: Some([2.0, 5.0, 0.0]),
            rotation_euler_xyz: None,
            scale: None,
        },
    )
    .unwrap();
    assert_eq!(world.entity_count(), 3);

    // 4) Ingest a Gaussian Splat capture and derive its collision proxy.
    let mut splat_bytes = Vec::new();
    for i in 0..50u32 {
        let p = (i as f32) * 0.1;
        let mut rec = [0u8; 32];
        rec[0..4].copy_from_slice(&p.to_le_bytes());
        rec[4..8].copy_from_slice(&((i % 7) as f32).to_le_bytes());
        rec[8..12].copy_from_slice(&(p * 0.5).to_le_bytes());
        rec[24] = 200;
        rec[28] = 255;
        splat_bytes.extend_from_slice(&rec);
    }
    let cloud: SplatCloud = load_splat_bytes(&splat_bytes).unwrap();
    let splat_collider = build_convex_hull_collider(&cloud).unwrap();
    assert!(!splat_collider.parts.is_empty());

    // 5) Package the scene file into a distributable asset pack.
    let pack_path = dir.join("nova_sample_assets.novapack");
    nova_export::pack_to_file(
        &[nova_export::PackEntry {
            name: "scene.ron".into(),
            data: std::fs::read(&scene_path).unwrap(),
        }],
        &pack_path,
    )
    .unwrap();
    let unpacked = nova_export::unpack_from_file(&pack_path).unwrap();
    assert_eq!(unpacked.len(), 1);

    let _ = std::fs::remove_file(&scene_path);
    let _ = std::fs::remove_file(&pack_path);

    format!(
        "sample ok: player rested at y={:.2}, entities={}, splat hull parts={}, pack entries={}",
        rested_y,
        world.entity_count(),
        splat_collider.parts.len(),
        unpacked.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_falls_and_rests_on_ground() {
        let (mut world, player) = build_world();
        let start = player_y(&world, player);
        assert!((start - 3.0).abs() < 1e-3);
        tick(&mut world, 300);
        let rested = player_y(&world, player);
        // Box half-height 0.5 on ground top y=0 => rests near y=0.5.
        assert!(
            (0.4..0.6).contains(&rested),
            "player should rest at ~0.5, got {rested}"
        );
    }

    #[test]
    fn scene_save_reload_preserves_entities() {
        let (mut world, _player) = build_world();
        tick(&mut world, 10);
        let dir = std::env::temp_dir();
        let path = dir.join("nova_sample_rt.ron");
        nova_scene::save_to_file(&world, &path).unwrap();
        let reloaded = nova_scene::load_from_file(&path).unwrap();
        assert_eq!(reloaded.entity_count(), world.entity_count());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn agent_spawns_and_moves_entity() {
        let (mut world, _player) = build_world();
        apply_command(
            &mut world,
            &AgentCommand::Spawn {
                name: Some("npc".into()),
                translation: [0.0, 10.0, 0.0],
                mesh: None,
            },
        )
        .unwrap();
        assert_eq!(world.entity_count(), 3);
        apply_command(
            &mut world,
            &AgentCommand::SetTransform {
                target: EntityRef::Name("npc".into()),
                translation: Some([7.0, 7.0, 7.0]),
                rotation_euler_xyz: None,
                scale: None,
            },
        )
        .unwrap();
        let reg = world.resource::<nova_agent_api::EntityRegistry>().unwrap();
        let npc = reg.lookup("npc").unwrap();
        let t = world.get_component::<Transform>(npc).unwrap();
        assert_eq!(t.translation, Vec3::new(7.0, 7.0, 7.0));
    }

    #[test]
    fn splat_ingestion_yields_collider() {
        let mut buf = Vec::new();
        for i in 0..30u32 {
            let p = i as f32;
            let mut rec = [0u8; 32];
            rec[0..4].copy_from_slice(&p.to_le_bytes());
            rec[4..8].copy_from_slice(&((i % 5) as f32).to_le_bytes());
            rec[8..12].copy_from_slice(&p.to_le_bytes());
            rec[24] = 255;
            rec[28] = 255;
            buf.extend_from_slice(&rec);
        }
        let cloud = load_splat_bytes(&buf).unwrap();
        let col = build_convex_hull_collider(&cloud).unwrap();
        assert!(!col.parts.is_empty());
    }

    #[test]
    fn full_pipeline_runs_end_to_end() {
        let summary = run_pipeline();
        assert!(summary.starts_with("sample ok:"), "got: {summary}");
    }
}
