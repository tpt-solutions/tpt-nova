//! C ABI boundary + hot-reload lifecycle contract test for `nova-scripting`.
//!
//! Loads the *real* compiled `cdylib` produced by `nova-gameplay-example`
//! through [`HotModule`], verifying the stable C ABI surface and the load ->
//! reload -> unload lifecycle. It locates the artifact next to the test
//! executable and skips (rather than fails) if it is not present.
//!
//! Note: assertions about *gameplay effects* (e.g. the player moving) live in
//! `nova-gameplay-example`'s own tests. Here we only exercise the host ABI
//! surface — loading the dylib, resolving the three C exports, reloading on a
//! file change, and unloading in order — which is the contract `nova-scripting`
//! owns and must keep stable.

use std::path::PathBuf;
use std::time::Duration;

use nova_ecs::World;
use nova_scripting::{HotModule, ABI_VERSION};

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
fn find_example_cdylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let profile_dir = exe.parent()?.parent()?; // deps -> profile
    let candidate = profile_dir.join(dylib_file_name("nova_gameplay_example"));
    candidate.exists().then_some(candidate)
}

/// Copy the example cdylib to a test-owned temp file so we can simulate a
/// recompile (rewrite) without ever touching the locked artifact in
/// `target/`. `HotModule` loads a temp copy of *whatever* path it is given, so
/// rewriting our own throwaway file is safe even where DLLs are locked (Windows).
fn stage_temp_copy() -> Option<PathBuf> {
    let original = find_example_cdylib()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ext = original.extension().and_then(|e| e.to_str()).unwrap_or("mod");
    let work = std::env::temp_dir().join(format!("nova_reload_test_{nanos}.{ext}"));
    std::fs::copy(&original, &work).ok()?;
    Some(work)
}

#[test]
fn c_abi_boundary_version_matches_host() {
    let path = match find_example_cdylib() {
        Some(p) => p,
        None => {
            eprintln!("skipping: example cdylib not found next to test exe");
            return;
        }
    };
    // The exported `abi_version` symbol must report the host's canonical version
    // — that is the stable contract `HotModule` enforces on load.
    let lib = unsafe { libloading::Library::new(&path) }.expect("load example lib");
    unsafe {
        let abi: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib
            .get(nova_scripting::symbols::ABI_VERSION)
            .expect("abi_version symbol exported");
        assert_eq!(abi(), ABI_VERSION);
    }

    // And the full load (which resolves create/destroy and checks the version)
    // must succeed through the host API.
    let _module = HotModule::load(&path).expect("HotModule accepts the module");
}

#[test]
fn lifecycle_load_reload_unload() {
    let path = match find_example_cdylib() {
        Some(p) => p,
        None => {
            eprintln!("skipping: example cdylib not found next to test exe");
            return;
        }
    };

    let work = match stage_temp_copy() {
        Some(w) => w,
        None => {
            eprintln!("skipping: could not stage temp copy of example cdylib");
            return;
        }
    };

    let mut module = HotModule::load(&work).expect("load example cdylib");

    // on_load / update must not panic through the boundary.
    let mut world = World::new();
    module.on_load(&mut world);
    module.update(&mut world, 1.0, 0);

    // Simulate a recompile by refreshing our own temp copy (HotModule copies it
    // to a temp file on load, so the working copy is never locked and can be
    // overwritten freely — including on Windows, where rewriting a loaded DLL
    // directly is rejected by the OS).
    std::thread::sleep(Duration::from_millis(20));
    let original = find_example_cdylib().expect("example cdylib present");
    std::fs::copy(&original, &work).expect("refresh working copy");

    let reloaded = module
        .reload_if_changed(&mut world)
        .expect("reload succeeds");
    assert!(reloaded, "expected a reload after touching the source file");

    // Still drivable after the swap, and dropping unloads instance+library.
    module.update(&mut world, 1.0, 1);

    let _ = std::fs::remove_file(&work);
}
