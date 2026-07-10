//! End-to-end hot-reload test: load the *actual* compiled `cdylib` for this
//! crate through `nova_scripting::HotModule`, drive the player, then trigger a
//! reload by touching the library file and confirm it still works.
//!
//! The test locates the build artifact relative to the test executable
//! (`target/<profile>/`), so it runs as part of a normal `cargo test` after the
//! workspace (including this crate's cdylib) has been built. If the artifact is
//! not present it skips rather than failing.

use std::path::PathBuf;
use std::time::Duration;

use nova_ecs::transform::Transform;
use nova_ecs::{Vec3, World};
use nova_gameplay_example::Player;
use nova_input::{default_action_map, InputState, KeyCode};
use nova_scripting::HotModule;

fn dylib_file_name(stem: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("{stem}.dll")
    }
    #[cfg(target_os = "macos")]
    {
        format!("lib{stem}.dylib")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        format!("lib{stem}.so")
    }
}

/// `.../target/<profile>/deps/<test-exe>` -> `.../target/<profile>/<dylib>`
fn find_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?; // deps -> profile
    let candidate = profile_dir.join(dylib_file_name("nova_gameplay_example"));
    candidate.exists().then_some(candidate)
}

fn world_pressing_right() -> (World, nova_ecs::Entity) {
    let mut world = World::new();
    let mut input = InputState::default();
    input.keys.insert(KeyCode::KeyD);
    world.add_resource(input);
    world.add_resource(default_action_map());
    let player = world.spawn();
    world.add_component(player, Transform::from_translation(Vec3::ZERO));
    world.add_component(player, Player::default());
    (world, player)
}

#[test]
fn loads_and_reloads_gameplay_dylib() {
    let path = match find_cdylib() {
        Some(p) => p,
        None => {
            eprintln!("skipping: gameplay cdylib not found next to test exe");
            return;
        }
    };

    let mut module = HotModule::load(&path).expect("load gameplay dylib");
    let (mut world, player) = world_pressing_right();

    module.on_load(&mut world);
    module.update(&mut world, 1.0, 0);
    let x1 = world
        .get_component::<Transform>(player)
        .unwrap()
        .translation
        .x;
    assert!(x1 > 4.9, "player should have moved right, x={x1}");

    // Trigger a reload by touching the library file (rewrite identical bytes).
    // HotModule loads a temp copy, so the original file is not locked.
    std::thread::sleep(Duration::from_millis(20));
    let bytes = std::fs::read(&path).expect("read dylib");
    std::fs::write(&path, &bytes).expect("rewrite dylib");

    let reloaded = module
        .reload_if_changed(&mut world)
        .expect("reload succeeds");
    assert!(reloaded, "expected a reload after touching the file");

    // Still functional after reload.
    let before = world
        .get_component::<Transform>(player)
        .unwrap()
        .translation
        .x;
    module.update(&mut world, 1.0, 1);
    let after = world
        .get_component::<Transform>(player)
        .unwrap()
        .translation
        .x;
    assert!(after > before, "player should keep moving after reload");
}
