//! Smart mesh ingestion demo: "drag a .glb in, get a collider + rig".
//!
//! Runs the full ingestion pipeline on a mesh file passed on the command line,
//! spawns an ECS entity with the auto-generated Rapier3D collider and a rigid
//! body, drops it onto a fixed ground plane, and prints a short report proving
//! the collider and auto-rig were produced and are simulated.
//!
//! Usage:
//!   cargo run -p nova-ingest --bin ingest_demo [path/to/model.glb]
//!
//! With no argument it loads the shipped `assets/cube.glb` sample, so the demo
//! is runnable zero-config.

use std::path::PathBuf;
use std::process::ExitCode;

use nova_ecs::transform::Transform;
use nova_ecs::{Vec3, World};
use nova_ingest::{ingest, Collider3D, PhysicsWorld3D, RigidBody3D};

fn default_asset() -> PathBuf {
    // The committed sample lives at the workspace root's `assets/` directory.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("assets")
        .join("cube.glb")
}

fn main() -> ExitCode {
    let path: PathBuf = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => {
            let d = default_asset();
            println!("no path given; using shipped sample: {}", d.display());
            d
        }
    };

    println!("Ingesting {}...", path.display());
    let result = match ingest(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ingestion failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let vert_count: usize = result.meshes.iter().map(|m| m.vertices.len()).sum();
    let part_count = result.convex_parts.len();
    println!("  meshes:         {}", result.meshes.len());
    println!("  vertices:       {vert_count}");
    println!(
        "  bounds:         min {:?}  max {:?}",
        result.bounds.min.to_array(),
        result.bounds.max.to_array()
    );
    println!("  collider parts: {part_count} convex hull(s)");
    println!("  auto-rig bones: {}", result.skeleton.bones.len());

    // Build a tiny scene: the ingested mesh as a dynamic body above a ground.
    let mut world = World::new();
    world.add_resource(PhysicsWorld3D::default());

    let ground = world.spawn();
    world.add_component(
        ground,
        Transform::from_translation(Vec3::new(0.0, -1.0, 0.0)),
    );
    world.add_component(ground, RigidBody3D::fixed());
    world.add_component(ground, Collider3D::from_parts(ground_plane_parts()));

    let start_y = result.bounds.max.y + 5.0;
    let mesh_entity = world.spawn();
    world.add_component(
        mesh_entity,
        Transform::from_translation(Vec3::new(0.0, start_y, 0.0)),
    );
    world.add_component(mesh_entity, RigidBody3D::dynamic());
    world.add_component(
        mesh_entity,
        Collider3D::from_parts(result.convex_parts.clone()),
    );

    println!("\nSimulating fall onto ground (fixed 60 Hz)...");
    for frame in 0..180 {
        nova_ingest::step_physics_3d(&mut world, 1.0 / 60.0);
        if frame % 60 == 0 {
            let y = world
                .get_component::<Transform>(mesh_entity)
                .map(|t| t.translation.y)
                .unwrap_or(f32::NAN);
            println!("  t={:>4.2}s  y={y:>7.3}", frame as f32 / 60.0);
        }
    }
    let final_y = world
        .get_component::<Transform>(mesh_entity)
        .map(|t| t.translation.y)
        .unwrap_or(f32::NAN);
    let bodies = world.resource::<PhysicsWorld3D>().unwrap().body_count();
    println!("\nDone: {bodies} bodies in sim, ingested mesh came to rest near y={final_y:.3}.");
    println!("Collider + auto-rig were generated automatically from the source mesh.");
    ExitCode::SUCCESS
}

/// A large, flat convex slab used as the ground plane.
fn ground_plane_parts() -> Vec<nova_ingest::ConvexPart> {
    let hx = 50.0;
    let hy = 0.5;
    let hz = 50.0;
    let v = vec![
        [-hx, -hy, -hz],
        [hx, -hy, -hz],
        [hx, hy, -hz],
        [-hx, hy, -hz],
        [-hx, -hy, hz],
        [hx, -hy, hz],
        [hx, hy, hz],
        [-hx, hy, hz],
    ];
    let indices = vec![
        [0u32, 1, 2],
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
    vec![nova_ingest::ConvexPart {
        vertices: v,
        indices,
    }]
}
