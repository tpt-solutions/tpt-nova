//! Content-addressed driver bundles (Phase 8, cross-cutting packaging).
//!
//! A driver artifact (a `.wasm` module or a native library) and its manifest are
//! stored under `drivers/<sha256>/`. The directory name is the SHA-256 of the
//! artifact bytes. That makes the bundle tamper-evident: [`unpack_driver`]
//! recomputes the hash and refuses an artifact that does not match its address.
//! Rolling back to an older driver is safe because each version has its own
//! immutable address.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PackEntry, PackError};

/// What kind of artifact the bundle carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    /// A WebAssembly module, run in the sandbox.
    Wasm,
    /// A trusted native `.so`/`.dll`.
    Native,
}

/// The manifest stored next to the artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverBundleManifest {
    pub id: String,
    /// Semantic version, `major.minor.patch`.
    pub version: String,
    /// Human-readable component name the driver targets.
    pub component: String,
    pub kind: ArtifactKind,
    /// Capability names the driver requests, as the drivers crate spells them.
    pub capabilities: Vec<String>,
    /// SHA-256 of the artifact, lowercase hex. Filled in by [`pack_driver`].
    pub artifact_sha256: String,
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Pack `artifact` with `manifest` into entries under its content address.
/// Sets `manifest.artifact_sha256` to the artifact's hash.
pub fn pack_driver(
    manifest: &mut DriverBundleManifest,
    artifact: &[u8],
) -> Result<Vec<PackEntry>, PackError> {
    let hash = sha256_hex(artifact);
    manifest.artifact_sha256 = hash.clone();
    let manifest_bytes = serde_json::to_vec_pretty(manifest)?;
    Ok(vec![
        PackEntry {
            name: format!("drivers/{hash}/manifest.json"),
            data: manifest_bytes,
        },
        PackEntry {
            name: format!("drivers/{hash}/artifact"),
            data: artifact.to_vec(),
        },
    ])
}

/// Read a driver bundle back, verifying its content address.
///
/// Fails if the manifest is missing or unreadable, if the artifact is missing,
/// or if the artifact's hash differs from the manifest or from the directory name.
pub fn unpack_driver(entries: &[PackEntry]) -> Result<(DriverBundleManifest, Vec<u8>), PackError> {
    let manifest_entry = entries
        .iter()
        .find(|e| e.name.starts_with("drivers/") && e.name.ends_with("/manifest.json"))
        .ok_or_else(|| malformed("driver bundle has no manifest"))?;
    let dir = manifest_entry
        .name
        .trim_end_matches("/manifest.json")
        .to_string();
    let artifact_entry = entries
        .iter()
        .find(|e| e.name == format!("{dir}/artifact"))
        .ok_or_else(|| malformed("driver bundle has no artifact"))?;

    let manifest: DriverBundleManifest = serde_json::from_slice(&manifest_entry.data)?;
    let actual = sha256_hex(&artifact_entry.data);
    if actual != manifest.artifact_sha256 {
        return Err(malformed("artifact does not match its manifest hash"));
    }
    if dir != format!("drivers/{actual}") {
        return Err(malformed("artifact does not match its content address"));
    }
    Ok((manifest, artifact_entry.data.clone()))
}

fn malformed(detail: &str) -> PackError {
    PackError::Malformed {
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack, unpack};

    fn manifest() -> DriverBundleManifest {
        DriverBundleManifest {
            id: "gravity-tweak".into(),
            version: "1.2.0".into(),
            component: "Gravity".into(),
            kind: ArtifactKind::Wasm,
            capabilities: vec!["ReadTransform".into()],
            artifact_sha256: String::new(),
        }
    }

    #[test]
    fn sha256_matches_the_known_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_bundle_round_trips_through_a_novapack_archive() {
        let artifact = b"\0asm\x01\0\0\0".to_vec();
        let mut m = manifest();
        let entries = pack_driver(&mut m, &artifact).unwrap();
        assert_eq!(m.artifact_sha256, sha256_hex(&artifact));

        let mut archive = Vec::new();
        pack(&entries, &mut archive).unwrap();
        let read_back = unpack(&archive).unwrap();

        let (got_manifest, got_artifact) = unpack_driver(&read_back).unwrap();
        assert_eq!(got_manifest, m);
        assert_eq!(got_artifact, artifact);
    }

    #[test]
    fn a_tampered_artifact_is_refused() {
        let mut m = manifest();
        let mut entries = pack_driver(&mut m, b"original").unwrap();
        for e in &mut entries {
            if e.name.ends_with("/artifact") {
                e.data = b"tampered".to_vec();
            }
        }
        assert!(unpack_driver(&entries).is_err());
    }

    #[test]
    fn a_missing_manifest_or_artifact_is_refused() {
        let mut m = manifest();
        let entries = pack_driver(&mut m, b"x").unwrap();
        let only_manifest: Vec<_> = entries
            .iter()
            .filter(|e| e.name.ends_with("manifest.json"))
            .cloned()
            .collect();
        assert!(unpack_driver(&only_manifest).is_err());
        let only_artifact: Vec<_> = entries
            .iter()
            .filter(|e| e.name.ends_with("artifact"))
            .cloned()
            .collect();
        assert!(unpack_driver(&only_artifact).is_err());
    }

    #[test]
    fn two_versions_get_distinct_addresses() {
        let mut v1 = manifest();
        let mut v2 = manifest();
        v2.version = "1.3.0".into();
        let a = pack_driver(&mut v1, b"build-a").unwrap();
        let b = pack_driver(&mut v2, b"build-b").unwrap();
        assert_ne!(a[0].name, b[0].name, "each artifact keeps its own address");
    }
}
