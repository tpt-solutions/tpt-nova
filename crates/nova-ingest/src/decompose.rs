//! VHACD convex decomposition for collision colliders.
//!
//! A single triangle mesh is usually non-convex, so a rigid body built from it
//! directly would collide incorrectly. We run the Volumetric Hierarchical
//! Approximate Convex Decomposition (V-HACD) algorithm (via `parry3d`) to split
//! the mesh into a handful of approximately-convex [`ConvexPart`]s. Each part
//! becomes one convex hull in a Rapier compound collider (see [`physics`]).

use parry3d::math::Vector;
use parry3d::transformation::vhacd::{VHACDParameters, VHACD};
use serde::{Deserialize, Serialize};

use crate::loader::{IngestError, MeshData};

/// One approximately-convex chunk of a decomposed mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvexPart {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
}

impl ConvexPart {
    pub fn triangle_indices(&self) -> Vec<u32> {
        self.indices.iter().flat_map(|t| t.to_vec()).collect()
    }
}

/// Decompose `mesh` into approximately-convex parts using V-HACD.
///
/// `params` overrides the decomposition quality (resolution, max hull count,
/// concavity tolerance). With `None` the library defaults are used.
pub fn decompose_convex(
    mesh: &MeshData,
    params: Option<VHACDParameters>,
) -> Result<Vec<ConvexPart>, IngestError> {
    if mesh.indices.len() < 3 {
        return Err(IngestError::EmptyMesh);
    }
    let points: Vec<Vector> = mesh
        .vertices
        .iter()
        .map(|v| Vector::new(v[0], v[1], v[2]))
        .collect();
    let indices: Vec<[u32; 3]> = mesh.indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();

    let params = params.unwrap_or_default();
    let decomposition = VHACD::decompose(&params, &points, &indices, false);
    let hulls = decomposition.compute_convex_hulls(4);

    let parts = hulls
        .into_iter()
        .map(|(verts, idx)| ConvexPart {
            vertices: verts.iter().map(|p| [p.x, p.y, p.z]).collect(),
            indices: idx,
        })
        .collect();
    Ok(parts)
}

/// Convenience: decompose every mesh in a file-load result and flatten.
pub fn decompose_meshes(meshes: &[MeshData]) -> Result<Vec<ConvexPart>, IngestError> {
    let mut all = Vec::new();
    for m in meshes {
        all.extend(decompose_convex(m, None)?);
    }
    if all.is_empty() {
        return Err(IngestError::EmptyMesh);
    }
    Ok(all)
}
