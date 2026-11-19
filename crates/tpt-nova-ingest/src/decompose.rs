//! Approximate convex decomposition for collision colliders.
//!
//! A single triangle mesh is usually non-convex, so a rigid body built from it
//! directly would collide incorrectly. This module splits the mesh into a
//! handful of approximately-convex [`ConvexPart`]s, each of which becomes one
//! convex hull in a compound collider (see [`crate::physics`]).
//!
//! The algorithm is a top-down recursive bisection:
//!
//! 1. Take a group of triangles (initially the whole mesh) and compute its
//!    convex hull.
//! 2. Measure its *concavity*: the deepest any of its vertices sits inside that
//!    hull, relative to the group's bounding-box diagonal. A convex group scores
//!    zero.
//! 3. If the concavity is within tolerance, or the part budget is spent, keep
//!    the hull. Otherwise split the triangles at the median centroid along the
//!    group's widest axis and recurse on both halves.
//!
//! This is a simpler scheme than V-HACD, and it is deterministic (no RNG, stable
//! sort order). Output is a valid set of convex parts, but the part boundaries
//! differ from V-HACD's.

use serde::{Deserialize, Serialize};

use crate::hull::convex_hull;
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

/// Controls how finely a mesh is split into convex parts.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DecompositionParams {
    /// Upper bound on the number of convex parts produced.
    pub max_convex_hulls: u32,
    /// Allowed concavity as a fraction of the group's bounding-box diagonal.
    /// Lower values fit the mesh more tightly at the cost of more parts.
    pub concavity: f32,
}

impl Default for DecompositionParams {
    fn default() -> Self {
        DecompositionParams {
            max_convex_hulls: 16,
            concavity: 0.02,
        }
    }
}

type Tri = [u32; 3];
type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Decompose `mesh` into approximately-convex parts.
///
/// `params` controls the part budget and concavity tolerance. With `None` the
/// defaults are used. Degenerate groups (zero-area or coplanar) produce no
/// part, so the result can be empty for a flat mesh.
pub fn decompose_convex(
    mesh: &MeshData,
    params: Option<DecompositionParams>,
) -> Result<Vec<ConvexPart>, IngestError> {
    if mesh.indices.len() < 3 {
        return Err(IngestError::EmptyMesh);
    }
    let params = params.unwrap_or_default();
    let max_parts = params.max_convex_hulls.max(1) as usize;
    let vertex_count = mesh.vertices.len() as u32;

    let tris: Vec<Tri> = mesh
        .indices
        .windows(3)
        .step_by(3)
        .map(|c| [c[0], c[1], c[2]])
        .filter(|t| t.iter().all(|&i| i < vertex_count))
        .collect();
    if tris.is_empty() {
        return Err(IngestError::EmptyMesh);
    }

    let Some(root) = Group::new(&mesh.vertices, tris) else {
        return Ok(Vec::new());
    };
    let mut groups = vec![root];
    // A frozen group has no acceptable cut left and stays whole.
    let mut frozen = vec![false];

    while groups.len() < max_parts {
        // Split the most concave splittable group next; the first one wins ties.
        let mut pick: Option<usize> = None;
        for (i, g) in groups.iter().enumerate() {
            if frozen[i] || g.concavity <= params.concavity || g.tris.len() < 2 {
                continue;
            }
            if pick.is_none_or(|p| g.concavity > groups[p].concavity) {
                pick = Some(i);
            }
        }
        let Some(i) = pick else { break };

        let children = best_split(&mesh.vertices, &groups[i].tris).and_then(|(a, b)| {
            Some((
                Group::new(&mesh.vertices, a)?,
                Group::new(&mesh.vertices, b)?,
            ))
        });
        match children {
            Some((a, b)) => {
                groups.remove(i);
                frozen.remove(i);
                for g in [a, b] {
                    groups.push(g);
                    frozen.push(false);
                }
            }
            None => frozen[i] = true,
        }
    }
    Ok(groups.into_iter().map(|g| g.hull).collect())
}

/// A triangle group with its convex hull and concavity, cached for splitting.
struct Group {
    tris: Vec<Tri>,
    hull: ConvexPart,
    concavity: f32,
}

impl Group {
    fn new(vertices: &[[f32; 3]], tris: Vec<Tri>) -> Option<Self> {
        let hull = hull_of_group(vertices, &tris)?;
        let concavity = concavity(vertices, &tris, &hull);
        Some(Group {
            tris,
            hull,
            concavity,
        })
    }
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

/// Convex hull of the vertices referenced by `group`.
fn hull_of_group(vertices: &[[f32; 3]], group: &[Tri]) -> Option<ConvexPart> {
    let mut used: Vec<u32> = group.iter().flat_map(|t| t.iter().copied()).collect();
    used.sort_unstable();
    used.dedup();
    let points: Vec<[f32; 3]> = used.iter().map(|&i| vertices[i as usize]).collect();
    if let Some(hull) = convex_hull(&points) {
        return Some(hull);
    }

    // A flat group (zero volume) has no 3D hull. It still needs a collider, so
    // thicken it into a thin slab along the normal of its largest triangle.
    let (lo, hi) = bounds(&points);
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let normal = group
        .iter()
        .map(|t| {
            let a = vertices[t[0] as usize].map(f64::from);
            let b = vertices[t[1] as usize].map(f64::from);
            let c = vertices[t[2] as usize].map(f64::from);
            let n = cross(sub(b, a), sub(c, a));
            (n, length(n))
        })
        .max_by(|x, y| x.1.total_cmp(&y.1))?;
    if normal.1 <= 0.0 || diag <= 0.0 || !diag.is_finite() {
        return None;
    }
    let n = normal.0.map(|c| c / normal.1);
    let thickness = diag as f64 * 1e-3;
    let mut slab: Vec<[f32; 3]> = Vec::with_capacity(points.len() * 2);
    for p in &points {
        for s in [-1.0, 1.0] {
            slab.push([
                (p[0] as f64 + n[0] * thickness * s) as f32,
                (p[1] as f64 + n[1] * thickness * s) as f32,
                (p[2] as f64 + n[2] * thickness * s) as f32,
            ]);
        }
    }
    convex_hull(&slab)
}

fn bounds(points: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// Largest distance from the hull surface to the group's triangles, relative to
/// the group's bounding-box diagonal. Zero for a group that is already convex.
///
/// The probe points are the hull's triangle centroids and edge midpoints. A
/// convex group's hull lies on its own surface, so every probe is on the mesh.
/// A concave group leaves hull surface "bridging" empty space, and those probes
/// sit far from every triangle. Mesh triangles are subsampled with a fixed
/// stride above [`MAX_CONCAVITY_TRIANGLES`] to keep large meshes tractable.
fn concavity(vertices: &[[f32; 3]], group: &[Tri], hull: &ConvexPart) -> f32 {
    let mut used: Vec<u32> = group.iter().flat_map(|t| t.iter().copied()).collect();
    used.sort_unstable();
    used.dedup();

    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for &i in &used {
        let p = vertices[i as usize];
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    if diag <= 0.0 || !diag.is_finite() {
        return 0.0;
    }

    let stride = group.len().div_ceil(MAX_CONCAVITY_TRIANGLES).max(1);
    let sampled: Vec<[V3; 3]> = group
        .iter()
        .step_by(stride)
        .map(|t| t.map(|i| vertices[i as usize].map(f64::from)))
        .collect();

    let mut worst_sq = 0.0f64;
    for t in &hull.indices {
        let a = hull.vertices[t[0] as usize].map(f64::from);
        let b = hull.vertices[t[1] as usize].map(f64::from);
        let c = hull.vertices[t[2] as usize].map(f64::from);
        let probes = [
            [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ],
            [
                (a[0] + b[0]) / 2.0,
                (a[1] + b[1]) / 2.0,
                (a[2] + b[2]) / 2.0,
            ],
            [
                (b[0] + c[0]) / 2.0,
                (b[1] + c[1]) / 2.0,
                (b[2] + c[2]) / 2.0,
            ],
            [
                (c[0] + a[0]) / 2.0,
                (c[1] + a[1]) / 2.0,
                (c[2] + a[2]) / 2.0,
            ],
        ];
        for p in probes {
            let nearest_sq = sampled
                .iter()
                .map(|tri| point_triangle_distance_sq(p, tri))
                .fold(f64::INFINITY, f64::min);
            worst_sq = worst_sq.max(nearest_sq);
        }
    }
    (worst_sq.sqrt() / diag as f64) as f32
}

/// Triangle count above which [`concavity`] subsamples the group.
const MAX_CONCAVITY_TRIANGLES: usize = 2048;

/// Squared distance from `p` to triangle `tri` (closest-point, after Ericson,
/// *Real-Time Collision Detection* §5.1.5).
fn point_triangle_distance_sq(p: V3, tri: &[V3; 3]) -> f64 {
    let [a, b, c] = *tri;
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let closest = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let d3 = dot(ab, bp);
        let d4 = dot(ac, bp);
        let cp = sub(p, c);
        let d5 = dot(ab, cp);
        let d6 = dot(ac, cp);
        let vc = d1 * d4 - d3 * d2;
        let vb = d5 * d2 - d1 * d6;
        let va = d3 * d6 - d5 * d4;
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else if d6 >= 0.0 && d5 <= d6 {
            c
        } else if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            add_scaled(a, ab, v)
        } else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            add_scaled(a, ac, w)
        } else if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            add_scaled(b, sub(c, b), w)
        } else {
            let denom = va + vb + vc;
            if denom.abs() <= f64::MIN_POSITIVE {
                // Degenerate triangle: fall back to the nearest vertex.
                a
            } else {
                let v = vb / denom;
                let w = vc / denom;
                add_scaled(add_scaled(a, ab, v), ac, w)
            }
        }
    };
    let d = sub(p, closest);
    dot(d, d)
}

fn add_scaled(base: V3, dir: V3, t: f64) -> V3 {
    [
        base[0] + dir[0] * t,
        base[1] + dir[1] * t,
        base[2] + dir[2] * t,
    ]
}

/// Choose the cut that leaves the two child groups with the smallest total hull
/// volume. Candidates are taken from triangle-centroid orderings along each
/// axis at several quantiles, so a cut lands near a concave corner when one
/// exists. Returns `None` when no cut yields two non-degenerate children.
fn best_split(vertices: &[[f32; 3]], tris: &[Tri]) -> Option<(Vec<Tri>, Vec<Tri>)> {
    const QUANTILES: [f32; 9] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    let centroids: Vec<[f32; 3]> = tris.iter().map(|t| centroid(vertices, t)).collect();

    let mut best: Option<(f32, Vec<Tri>, Vec<Tri>)> = None;
    for axis in [0usize, 1, 2] {
        let mut order: Vec<usize> = (0..tris.len()).collect();
        order.sort_by(|&a, &b| centroids[a][axis].total_cmp(&centroids[b][axis]));
        for q in QUANTILES {
            let cut = (tris.len() as f32 * q).round() as usize;
            if cut == 0 || cut >= tris.len() {
                continue;
            }
            let left: Vec<Tri> = order[..cut].iter().map(|&i| tris[i]).collect();
            let right: Vec<Tri> = order[cut..].iter().map(|&i| tris[i]).collect();
            let (Some(lh), Some(rh)) = (
                hull_of_group(vertices, &left),
                hull_of_group(vertices, &right),
            ) else {
                continue;
            };
            let score = part_volume(&lh) + part_volume(&rh);
            if best.as_ref().is_none_or(|(s, _, _)| score < *s) {
                best = Some((score, left, right));
            }
        }
    }
    best.map(|(_, l, r)| (l, r))
}

fn centroid(vertices: &[[f32; 3]], t: &Tri) -> [f32; 3] {
    let a = vertices[t[0] as usize];
    let b = vertices[t[1] as usize];
    let c = vertices[t[2] as usize];
    [
        (a[0] + b[0] + c[0]) / 3.0,
        (a[1] + b[1] + c[1]) / 3.0,
        (a[2] + b[2] + c[2]) / 3.0,
    ]
}

/// Absolute volume of a closed convex part (signed tetrahedra about the origin).
fn part_volume(part: &ConvexPart) -> f32 {
    part.indices
        .iter()
        .map(|t| {
            let a = part.vertices[t[0] as usize].map(f64::from);
            let b = part.vertices[t[1] as usize].map(f64::from);
            let c = part.vertices[t[2] as usize].map(f64::from);
            dot(
                a,
                [
                    b[1] * c[2] - b[2] * c[1],
                    b[2] * c[0] - b[0] * c[2],
                    b[0] * c[1] - b[1] * c[0],
                ],
            ) / 6.0
        })
        .sum::<f64>()
        .abs() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(vertices: &[[f32; 3]], indices: &[u32]) -> MeshData {
        MeshData {
            name: "t".into(),
            vertices: vertices.to_vec(),
            indices: indices.to_vec(),
        }
    }

    /// An axis-aligned box as 12 triangles.
    fn box_mesh(min: [f32; 3], max: [f32; 3]) -> MeshData {
        let v = [
            [min[0], min[1], min[2]],
            [max[0], min[1], min[2]],
            [max[0], max[1], min[2]],
            [min[0], max[1], min[2]],
            [min[0], min[1], max[2]],
            [max[0], min[1], max[2]],
            [max[0], max[1], max[2]],
            [min[0], max[1], max[2]],
        ];
        let idx = [
            0, 2, 1, 0, 3, 2, // -z
            4, 5, 6, 4, 6, 7, // +z
            0, 1, 5, 0, 5, 4, // -y
            3, 7, 6, 3, 6, 2, // +y
            0, 4, 7, 0, 7, 3, // -x
            1, 2, 6, 1, 6, 5, // +x
        ];
        mesh(&v, &idx)
    }

    /// An L-shaped prism: two boxes joined at a right angle, as one mesh.
    fn l_shape_mesh() -> MeshData {
        let a = box_mesh([0.0, 0.0, 0.0], [4.0, 1.0, 1.0]);
        let b = box_mesh([0.0, 1.0, 0.0], [1.0, 4.0, 1.0]);
        let mut verts = a.vertices.clone();
        let offset = verts.len() as u32;
        verts.extend(b.vertices.iter().copied());
        let mut idx = a.indices.clone();
        idx.extend(b.indices.iter().map(|i| i + offset));
        mesh(&verts, &idx)
    }

    #[test]
    fn degenerate_mesh_with_too_few_indices_errors() {
        // A single vertex and no triangles must be rejected, not decomposed.
        let m = mesh(&[[0.0, 0.0, 0.0]], &[]);
        assert!(matches!(
            decompose_convex(&m, None),
            Err(IngestError::EmptyMesh)
        ));
    }

    #[test]
    fn convex_box_stays_a_single_part() {
        let parts = decompose_convex(&box_mesh([0.0; 3], [2.0; 3]), None).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].vertices.len(), 8);
    }

    #[test]
    fn l_shape_splits_into_multiple_parts() {
        let parts = decompose_convex(&l_shape_mesh(), None).unwrap();
        assert!(
            parts.len() >= 2,
            "concave mesh should split, got {}",
            parts.len()
        );
    }

    #[test]
    fn part_budget_is_respected() {
        let params = DecompositionParams {
            max_convex_hulls: 2,
            concavity: 0.0,
        };
        let parts = decompose_convex(&l_shape_mesh(), Some(params)).unwrap();
        assert!(parts.len() <= 2);
    }

    #[test]
    fn decomposition_is_deterministic() {
        let a = decompose_convex(&l_shape_mesh(), None).unwrap();
        let b = decompose_convex(&l_shape_mesh(), None).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn flat_plane_decomposes_without_panic() {
        // A degenerate (zero-volume) quad: 2 triangles in a plane. A flat part
        // has no hull, so the result may legitimately be empty.
        let verts = [
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
        ];
        let idx = [0u32, 1, 2, 0, 2, 3];
        let result = decompose_convex(&mesh(&verts, &idx), None);
        assert!(result.is_ok(), "flat plane decomposition must not panic");
    }

    #[test]
    fn single_triangle_decomposes_without_panic() {
        let verts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = [0u32, 1, 2];
        assert!(decompose_convex(&mesh(&verts, &idx), None).is_ok());
    }

    #[test]
    fn out_of_range_indices_are_ignored() {
        let verts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let idx = [0u32, 1, 9];
        assert!(matches!(
            decompose_convex(&mesh(&verts, &idx), None),
            Err(IngestError::EmptyMesh)
        ));
    }
}
