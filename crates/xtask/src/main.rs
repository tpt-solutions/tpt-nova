//! `xtask` — a small developer-task runner for the TPT Nova workspace.
//!
//! Invoked via `cargo xtask <subcommand>`. These are build/developer helpers
//! that do not belong inside the engine crates themselves.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("doctor") => doctor(),
        Some("help") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown subcommand `{other}`");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("TPT Nova xtask — developer task runner");
    println!();
    println!("USAGE:");
    println!("    cargo xtask <SUBCOMMAND>");
    println!();
    println!("SUBCOMMANDS:");
    println!("    doctor    Print a pre-flight readiness report");
    println!("    help      Print this help");
}

/// Walk upward from the crate's manifest dir to locate the workspace root
/// (the directory holding a `Cargo.toml` that declares `[workspace]`).
fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if let Ok(contents) = std::fs::read_to_string(&manifest) {
            if contents.contains("[workspace]") {
                return dir;
            }
        }
        if !dir.pop() {
            // Fall back to two levels up from crates/xtask.
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        }
    }
}

/// Static, dependency-light readiness check before someone runs
/// `cargo run -p nova-app` (which needs a real GPU + drivers).
fn doctor() -> ExitCode {
    println!("TPT Nova — developer readiness (doctor)");
    println!("=========================================");
    println!();
    println!(
        "NOTE: This is a STATIC readiness checklist, not a live GPU probe. \
         It cannot detect a GPU without initializing a wgpu adapter (which \
         needs a window/driver). Verify your drivers separately if needed."
    );
    println!();

    let os = std::env::consts::OS;
    println!("Target OS:        {os}");
    println!("Target arch:      {}", std::env::consts::ARCH);
    println!("Cargo target OS:  {}", std::env::consts::OS);

    // Best-effort backend recommendation per OS.
    let recommended = match os {
        "linux" => "Vulkan (preferred on most Linux distros; OpenGL fallback available)",
        "windows" => "Vulkan or DX12 (wgpu's default D3D12 backend)",
        "macos" => "Metal (wgpu's only backend on macOS)",
        _ => "see wgpu docs for backend support on this platform",
    };
    println!("Recommended GPU backend for {os}: {recommended}");
    println!(
        "Env signals: VK_ICD_FILENAMES={}, DXVK=unspecified, LIBGL=unspecified",
        std::env::var("VK_ICD_FILENAMES").unwrap_or_else(|_| "<unset>".into())
    );
    println!("Action: ensure the matching graphics drivers are installed and up to date.");
    println!();

    // Sample asset check — required by the `ingest_demo` zero-config path.
    let root = workspace_root();
    let assets_dir = root.join("assets");
    let required = ["cube.glb", "sample.splat"];
    println!("Sample assets (required by ingest_demo):");
    let mut missing = Vec::new();
    for name in required {
        let path = assets_dir.join(name);
        match path.try_exists() {
            Ok(true) => println!("    [ok]      {name}"),
            _ => {
                println!("    [MISSING] {name}  ({})", path.display());
                missing.push(name);
            }
        }
    }
    if !missing.is_empty() {
        println!();
        println!(
            "Some required sample assets are missing. Regenerate them with: \
             cargo run -p nova-ingest --example gen_sample_assets"
        );
    }
    println!();

    // NOVA_SEED note.
    match std::env::var("NOVA_SEED") {
        Ok(seed) => println!("NOVA_SEED: set (deterministic RNG seed = {seed})"),
        Err(_) => println!(
            "NOVA_SEED: unset (engine will use its default seeded RNG; set for reproducible runs)"
        ),
    }
    println!();

    println!("Readiness report complete.");
    if missing.is_empty() {
        println!("Status: READY (informational — verify GPU drivers before running nova-app).");
        ExitCode::SUCCESS
    } else {
        println!(
            "Status: NOT READY — {} required asset(s) missing. \
             nova-app / ingest_demo may fail to start without them.",
            missing.len()
        );
        // Hard requirement: assets missing => non-zero exit.
        ExitCode::FAILURE
    }
}
