//! Standalone packaging and asset bundling for shipped Nova games.
//!
//! A released game is *not* a loose pile of files on the user's disk: it is a
//! single self-describing package containing the engine/player binary plus all
//! of its assets (models, splats, textures, scenes) in one redistributable
//! blob. This crate provides:
//!
//! * [`pack`] / [`unpack`] — a small, dependency-free binary container format
//!   (`.novapack`) for bundling many assets into one file,
//! * [`pack_directory`] — bundle an entire assets folder,
//! * [`PlatformTarget`] + [`bundle_application`] — assemble a per-platform
//!   distributable (binary + packed assets + a manifest) for Windows, Linux, or
//!   macOS,
//! * a CLI (`nova-export`) wrapping the same logic for release pipelines.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"NOVAPACK";
const FORMAT_VERSION: u32 = 1;

/// Errors raised while packing or unpacking.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad pack magic (not a .novapack file)")]
    BadMagic,
    #[error("unsupported pack version {found}, expected {FORMAT_VERSION}")]
    UnsupportedVersion { found: u32 },
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// One entry in a packed archive: a file name and its raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// A bundler target platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformTarget {
    Windows,
    Linux,
    MacOS,
}

impl PlatformTarget {
    /// The executable file name on this platform, given a base name.
    pub fn exe_name(self, base: &str) -> String {
        match self {
            PlatformTarget::Windows => format!("{base}.exe"),
            PlatformTarget::Linux | PlatformTarget::MacOS => base.to_string(),
        }
    }

    /// A stable, human-readable directory suffix for the output bundle.
    pub fn dir_suffix(self) -> &'static str {
        match self {
            PlatformTarget::Windows => "win64",
            PlatformTarget::Linux => "linux",
            PlatformTarget::MacOS => "macos",
        }
    }
}

/// The manifest written alongside a bundled application, describing the
/// contents so a launcher/installer or the engine can locate the binary and
/// the asset pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub game_name: String,
    pub target: String,
    pub binary: String,
    pub asset_pack: String,
    pub asset_count: usize,
}

/// Serialize `entries` into the `.novapack` binary container format.
pub fn pack(entries: &[PackEntry], out: &mut Vec<u8>) -> Result<(), PackError> {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        let name_bytes = e.name.as_bytes();
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&(e.data.len() as u64).to_le_bytes());
        out.extend_from_slice(&e.data);
    }
    Ok(())
}

/// Write a `.novapack` to disk.
pub fn pack_to_file(entries: &[PackEntry], path: &Path) -> Result<(), PackError> {
    let mut buf = Vec::new();
    pack(entries, &mut buf)?;
    std::fs::write(path, buf)?;
    Ok(())
}

/// Parse a `.novapack` buffer back into entries.
pub fn unpack(bytes: &[u8]) -> Result<Vec<PackEntry>, PackError> {
    let mut cursor = bytes;
    let mut magic = [0u8; 8];
    cursor.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(PackError::BadMagic);
    }
    let mut version = [0u8; 4];
    cursor.read_exact(&mut version)?;
    let version = u32::from_le_bytes(version);
    if version != FORMAT_VERSION {
        return Err(PackError::UnsupportedVersion { found: version });
    }
    let mut count = [0u8; 4];
    cursor.read_exact(&mut count)?;
    let count = u32::from_le_bytes(count);

    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut name_len = [0u8; 2];
        cursor.read_exact(&mut name_len)?;
        let name_len = u16::from_le_bytes(name_len) as usize;
        let mut name = vec![0u8; name_len];
        cursor.read_exact(&mut name)?;
        let name = String::from_utf8_lossy(&name).into_owned();

        let mut data_len = [0u8; 8];
        cursor.read_exact(&mut data_len)?;
        let data_len = u64::from_le_bytes(data_len) as usize;
        let mut data = vec![0u8; data_len];
        cursor.read_exact(&mut data)?;

        entries.push(PackEntry { name, data });
    }
    Ok(entries)
}

/// Read a `.novapack` file from disk.
pub fn unpack_from_file(path: &Path) -> Result<Vec<PackEntry>, PackError> {
    let bytes = std::fs::read(path)?;
    unpack(&bytes)
}

/// Bundle an entire directory into a `.novapack`, preserving each file's path
/// relative to `root` (forward-slash separated, so packs are cross-platform).
pub fn pack_directory(root: &Path, extensions: &[&str]) -> Result<Vec<PackEntry>, PackError> {
    let mut entries = Vec::new();
    let mut files = Vec::new();
    collect_files(root, &mut files);
    let ext_set: Vec<String> = extensions.iter().map(|e| e.to_ascii_lowercase()).collect();
    for path in files {
        let is_allowed = ext_set.is_empty()
            || path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| ext_set.contains(&e.to_ascii_lowercase()))
                .unwrap_or(false);
        if !is_allowed {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let data = std::fs::read(&path)?;
        entries.push(PackEntry { name: rel, data });
    }
    Ok(entries)
}

/// Assemble a per-platform distributable: copy the player `binary` into
/// `<output>/<game>-<target>/<binary_name>`, write the packed `assets` as
/// `<game>-<target>/assets.novapack`, and emit `manifest.json`.
///
/// The binary path is taken as-is (the *build* step that produces it — e.g.
/// `cargo build --release --target …` — is the caller's responsibility; this
/// function packages the already-built artifact for redistribution).
pub fn bundle_application(
    game_name: &str,
    binary: &Path,
    assets: &Path,
    output: &Path,
    target: PlatformTarget,
) -> Result<BundleManifest, PackError> {
    let bundle_dir = output.join(format!("{game_name}-{}", target.dir_suffix()));
    std::fs::create_dir_all(&bundle_dir)?;

    let bin_name = target.exe_name(game_name);
    let dest_bin = bundle_dir.join(&bin_name);
    std::fs::copy(binary, &dest_bin)?;

    let asset_entries = if assets.is_dir() {
        pack_directory(assets, &[])?
    } else if assets.is_file() {
        let data = std::fs::read(assets)?;
        vec![PackEntry {
            name: assets
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("assets.bin")
                .to_string(),
            data,
        }]
    } else {
        Vec::new()
    };
    let pack_path = bundle_dir.join("assets.novapack");
    pack_to_file(&asset_entries, &pack_path)?;

    let manifest = BundleManifest {
        game_name: game_name.to_string(),
        target: target.dir_suffix().to_string(),
        binary: bin_name,
        asset_pack: "assets.novapack".to_string(),
        asset_count: asset_entries.len(),
    };
    let manifest_path = bundle_dir.join("manifest.json");
    let mut f = std::fs::File::create(&manifest_path)?;
    f.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    Ok(manifest)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "target" {
                        continue;
                    }
                }
                collect_files(&path, out);
            } else {
                out.push(path);
            }
        }
    }
}

/// Helper used by the CLI and tests: build a flat name->bytes map from entries.
pub fn entries_to_map(entries: &[PackEntry]) -> HashMap<String, Vec<u8>> {
    entries
        .iter()
        .map(|e| (e.name.clone(), e.data.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrips_bytes_and_names() {
        let entries = vec![
            PackEntry {
                name: "models/cube.glb".into(),
                data: vec![1, 2, 3, 4, 5],
            },
            PackEntry {
                name: "scene.json".into(),
                data: b"{\"a\":1}".to_vec(),
            },
        ];
        let mut buf = Vec::new();
        pack(&entries, &mut buf).unwrap();
        let back = unpack(&buf).unwrap();
        assert_eq!(back, entries);
    }

    #[test]
    fn unpack_rejects_bad_magic() {
        assert!(matches!(unpack(b"NOTAPACK0000"), Err(PackError::BadMagic)));
    }

    #[test]
    fn pack_directory_preserves_relative_paths() {
        let dir = std::env::temp_dir().join("nova_export_dir_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"alpha").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"beta").unwrap();
        std::fs::write(dir.join("ignore.png"), b"img").unwrap();

        let entries = pack_directory(&dir, &["txt"]).unwrap();
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"sub/b.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_to_from_file_roundtrips() {
        let dir = std::env::temp_dir();
        let path = dir.join("nova_export_test.novapack");
        let entries = vec![PackEntry {
            name: "x.bin".into(),
            data: vec![9, 8, 7],
        }];
        pack_to_file(&entries, &path).unwrap();
        let back = unpack_from_file(&path).unwrap();
        assert_eq!(back, entries);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn platform_target_names_binary_correctly() {
        assert_eq!(PlatformTarget::Windows.exe_name("game"), "game.exe");
        assert_eq!(PlatformTarget::Linux.exe_name("game"), "game");
        assert_eq!(PlatformTarget::MacOS.exe_name("game"), "game");
        assert_eq!(PlatformTarget::Windows.dir_suffix(), "win64");
    }

    #[test]
    fn bundle_application_writes_binary_pack_and_manifest() {
        let dir = std::env::temp_dir().join("nova_bundle_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let bin = dir.join("game.exe");
        std::fs::write(&bin, b"PEFAKE").unwrap();
        let assets = dir.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("lvl.json"), b"{}").unwrap();

        let out = dir.join("out");
        let manifest =
            bundle_application("game", &bin, &assets, &out, PlatformTarget::Windows).unwrap();

        let bundle = out.join("game-win64");
        assert!(bundle.join("game.exe").exists());
        assert!(bundle.join("assets.novapack").exists());
        assert!(bundle.join("manifest.json").exists());
        assert_eq!(manifest.asset_count, 1);
        assert_eq!(manifest.target, "win64");

        // The packed assets round-trip.
        let packed = unpack_from_file(&bundle.join("assets.novapack")).unwrap();
        assert_eq!(packed[0].name, "lvl.json");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
