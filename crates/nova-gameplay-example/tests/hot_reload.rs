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
///
/// The gameplay `cdylib` can exist both at the crate's primary artifact path
/// (`target/<profile>/<name>.dll`) and in `target/<profile>/deps/`; cargo does
/// not always keep the two copies' mtimes in sync, so a stale primary copy can
/// linger and be picked up here. To always exercise the *current* build we
/// consider both locations and return the most recently modified match.
fn find_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?.to_path_buf(); // deps -> profile
    let name = dylib_file_name("nova_gameplay_example");
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for dir in [profile_dir.clone(), profile_dir.join("deps")] {
        let candidate = dir.join(name.as_str());
        if let Some(t) = candidate.metadata().ok().and_then(|m| m.modified().ok()) {
            match &best {
                Some((_, bt)) if *bt >= t => {}
                _ => best = Some((candidate, t)),
            }
        }
    }
    best.map(|(p, _)| p)
}

/// Copy the gameplay cdylib to a test-owned temp file so we can simulate a
/// recompile (rewrite) without ever touching the locked artifact in `target/`.
/// `HotModule` loads a temp copy of *whatever* path it is given, so rewriting our
/// own throwaway file is safe even where DLLs are locked (Windows). The temp
/// copy inherits the original's (older) build mtime, so the later rewrite
/// reliably advances the mtime past the load-time capture — avoiding a
/// mtime-granularity race that would make `reload_if_changed` miss the change.
fn stage_temp_copy(original: &PathBuf) -> Option<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mod");
    let work = std::env::temp_dir().join(format!("nova_hr_test_{nanos}.{ext}"));
    std::fs::copy(original, &work).ok()?;
    Some(work)
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

    let work = match stage_temp_copy(&path) {
        Some(w) => w,
        None => {
            eprintln!("skipping: could not stage temp copy of gameplay cdylib");
            return;
        }
    };

    let mut module = HotModule::load(&work).expect("load gameplay dylib");
    let (mut world, player) = world_pressing_right();

    module.on_load(&mut world);
    module.update(&mut world, 1.0, 0);
    let x1 = world
        .get_component::<Transform>(player)
        .unwrap()
        .translation
        .x;
    assert!(x1 > 4.9, "player should have moved right, x={x1}");

    // Trigger a reload by touching the working copy (rewriting identical bytes).
    // HotModule loads a temp copy of `work`, so `work` itself is never locked and
    // can be rewritten freely — including on Windows, where rewriting a loaded
    // DLL directly is rejected by the OS.
    //
    // Poll until the reload is observed: on a freshly-linked artifact the load
    // time and the rewrite time can land inside the filesystem's mtime
    // granularity, so a single touch can look unchanged. Re-touching and
    // retrying (wall-clock advancing) makes detection immune to any timestamp
    // resolution.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut reloaded = false;
    while std::time::Instant::now() < deadline {
        let bytes = std::fs::read(&path).expect("read dylib");
        std::fs::write(&work, bytes).expect("rewrite dylib");
        if module
            .reload_if_changed(&mut world)
            .expect("reload succeeds")
        {
            reloaded = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

    let _ = std::fs::remove_file(&work);
}
