//! `nova-export` CLI — package shipped Nova games.

use std::path::PathBuf;

use nova_export::{
    bundle_application, pack_directory, pack_to_file, unpack_from_file, PackError, PlatformTarget,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Ok(());
    }
    match args[1].as_str() {
        "pack" => {
            // nova-export pack <dir> <out.novapack> [ext,...]
            let dir = args.get(2).ok_or("missing <dir>")?;
            let out = args.get(3).ok_or("missing <out.novapack>")?;
            let exts: Vec<&str> = args
                .get(4)
                .map(|s| s.split(',').collect())
                .unwrap_or_default();
            let entries = pack_directory(std::path::Path::new(dir), &exts)?;
            pack_to_file(&entries, std::path::Path::new(out))?;
            println!("packed {} files -> {out}", entries.len());
        }
        "unpack" => {
            // nova-export unpack <in.novapack> <outdir>
            let in_p = args.get(2).ok_or("missing <in.novapack>")?;
            let out = args.get(3).ok_or("missing <outdir>")?;
            let entries = unpack_from_file(std::path::Path::new(in_p))?;
            std::fs::create_dir_all(out)?;
            for e in &entries {
                let dest = PathBuf::from(out).join(&e.name);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, &e.data)?;
            }
            println!("unpacked {} files -> {out}", entries.len());
        }
        "bundle" => {
            // nova-export bundle <binary> <assets> <out> [win|linux|macos]
            let binary = args.get(2).ok_or("missing <binary>")?;
            let assets = args.get(3).ok_or("missing <assets>")?;
            let out = args.get(4).ok_or("missing <out>")?;
            let target = match args.get(5).map(|s| s.as_str()) {
                Some("win") | Some("windows") => PlatformTarget::Windows,
                Some("linux") => PlatformTarget::Linux,
                Some("mac") | Some("macos") => PlatformTarget::MacOS,
                _ => PlatformTarget::Windows,
            };
            let game = std::path::Path::new(binary)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("nova-game")
                .to_string();
            let manifest = bundle_application(
                &game,
                std::path::Path::new(binary),
                std::path::Path::new(assets),
                std::path::Path::new(out),
                target,
            )?;
            println!(
                "bundled {} ({} assets) -> {}/{}",
                manifest.game_name, manifest.asset_count, out, manifest.target
            );
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            print_usage();
        }
    }
    Ok(())
}

fn print_usage() {
    println!(
        "nova-export — package shipped Nova games\n\
usage:\n  \
nova-export pack   <dir> <out.novapack> [ext,ext,...]\n  \
nova-export unpack <in.novapack> <outdir>\n  \
nova-export bundle <binary> <assets> <out> [win|linux|macos]"
    );
}

// Keep `PackError` referenced so the binary links against the crate's error type
// even if a code path is not exercised by the default CLI flow.
#[allow(dead_code)]
fn _assert_error_type(_: PackError) {}
