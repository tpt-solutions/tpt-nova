//! Smart mesh ingestion for TPT Nova.
//!
//! The ingestion pipeline turns an arbitrary artist mesh into something the
//! engine can simulate and animate:
//!
//! 1. **Load** — [`load`] reads `.glb`/`.gltf`/`.obj` into plain [`MeshData`]
//!    (see [`loader`]).
//! 2. **Decompose** — [`decompose_convex`] runs V-HACD to split concave meshes
//!    into approximately-convex [`ConvexPart`]s for collision (see [`decompose`]).
//! 3. **Auto-rig** — [`auto_rig`] derives a procedural skeleton from the bounds
//!    (see [`rig`]).
//! 4. **Collide** — [`Collider3D`] + [`PhysicsWorld3D`] feed those parts to
//!    Rapier3D (see [`physics`]).
//!
//! [`ingest`] runs the whole chain and returns an [`IngestResult`].

pub mod decompose;
pub mod loader;
pub mod physics;
pub mod rig;

use std::path::Path;

use glam::Vec3;

pub use decompose::{decompose_convex, decompose_meshes, ConvexPart};
pub use loader::{load, load_gltf, load_obj, Aabb, IngestError, MeshData};
pub use physics::{step_physics_3d, BodyKind3D, Collider3D, PhysicsWorld3D, RigidBody3D};
pub use rig::auto_rig;

/// The combined output of ingesting one mesh file.
#[derive(Debug, Clone)]
pub struct IngestResult {
    pub meshes: Vec<MeshData>,
    pub convex_parts: Vec<ConvexPart>,
    pub skeleton: nova_anim::Skeleton,
    pub bounds: Aabb,
}

/// Run the full ingestion pipeline on a mesh file: load, V-HACD decompose, and
/// auto-rig from the combined bounds.
pub fn ingest(path: &Path) -> Result<IngestResult, IngestError> {
    let meshes = load(path)?;
    let convex_parts = decompose_meshes(&meshes)?;

    let mut bounds = Aabb {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    for m in &meshes {
        bounds = bounds.union(&m.bounds());
    }
    if meshes.is_empty() {
        bounds = Aabb {
            min: Vec3::ZERO,
            max: Vec3::ZERO,
        };
    }
    let skeleton = auto_rig(&bounds, 4);

    Ok(IngestResult {
        meshes,
        convex_parts,
        skeleton,
        bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_anim::{compute_skinning, BonePose};

    /// A unit cube mesh for decomposition tests.
    fn cube_mesh() -> MeshData {
        MeshData {
            name: "cube".into(),
            vertices: vec![
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            indices: vec![
                0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 4, 5, 0, 5, 1, 1, 5, 6, 1, 6, 2, 2, 6, 7, 2,
                7, 3, 3, 7, 4, 3, 4, 0,
            ],
        }
    }

    #[test]
    fn decompose_convex_produces_at_least_one_part() {
        let parts = decompose_convex(&cube_mesh(), None).unwrap();
        assert!(
            !parts.is_empty(),
            "VHACD should return at least one convex part for a cube"
        );
    }

    #[test]
    fn load_obj_reads_vertices_and_indices() {
        let obj = "v 0.0 0.0 0.0\nv 1.0 0.0 0.0\nv 0.0 1.0 0.0\nf 1 2 3\n";
        let dir = std::env::temp_dir();
        let path = dir.join("nova_ingest_test.obj");
        std::fs::write(&path, obj).unwrap();
        let meshes = load_obj(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].vertices.len(), 3);
        assert_eq!(meshes[0].indices.len(), 3);
    }

    #[test]
    fn ingest_pipeline_yields_skeleton_and_colliders() {
        // Exercise the whole chain on an in-memory GLB triangle (loader path).
        let glb = make_triangle_glb();
        let dir = std::env::temp_dir();
        let path = dir.join("nova_ingest_test.glb");
        std::fs::write(&path, &glb).unwrap();
        let result = ingest(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(result.meshes.len(), 1);
        assert!(
            !result.convex_parts.is_empty(),
            "ingestion should produce convex collider parts"
        );
        assert_eq!(result.skeleton.bones.len(), 4);

        // The auto-rigged skeleton must skin to identity at its rest pose.
        let poses: Vec<BonePose> = result
            .skeleton
            .bones
            .iter()
            .map(|b| {
                let (s, r, t) = b.rest_local.to_scale_rotation_translation();
                BonePose {
                    translation: t,
                    rotation: r,
                    scale: s,
                }
            })
            .collect();
        let skins = compute_skinning(&result.skeleton, &poses);
        for s in &skins {
            assert!(s.abs_diff_eq(glam::Mat4::IDENTITY, 1e-4));
        }
    }

    /// Build a minimal binary glTF (.glb) containing a single triangle, entirely
    /// in memory, to exercise the glTF loader path without external assets.
    fn make_triangle_glb() -> Vec<u8> {
        // One triangle: indices [0,1,2] (u32) + three positions (vec3 f32).
        let mut bin = Vec::new();
        bin.extend_from_slice(&0u32.to_le_bytes());
        bin.extend_from_slice(&1u32.to_le_bytes());
        bin.extend_from_slice(&2u32.to_le_bytes());
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in p {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }

        let json = r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{"POSITION":1},"indices":0}]}],"buffers":[{"byteLength":48}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":12,"target":34963},{"buffer":0,"byteOffset":12,"byteLength":36,"target":34962}],"accessors":[{"bufferView":0,"componentType":5125,"count":3,"type":"SCALAR"},{"bufferView":1,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}]}"#;

        let json_padded = pad(json.as_bytes(), 0x20);
        let bin_padded = pad(&bin, 0x00);
        let total = 12 + 8 + json_padded.len() + 8 + bin_padded.len();

        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json_padded.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json_padded);
        out.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin_padded);
        out
    }

    fn pad(data: &[u8], pad_byte: u8) -> Vec<u8> {
        let mut v = data.to_vec();
        while !v.len().is_multiple_of(4) {
            v.push(pad_byte);
        }
        v
    }
}
