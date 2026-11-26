//! Multi-part collision proxies for splat clouds.
//!
//! The single convex hull in [`crate::collider`] is cheap but wraps concavities
//! (a cave, an L-shaped wall) in solid air. This module builds a tighter proxy:
//!
//! 1. **Floater filtering** — drop low-opacity splats, oversized splats and
//!    k-nearest density outliers (stray haze that would inflate the hull).
//! 2. **Opacity-weighted voxelization** — accumulate each surviving splat's
//!    opacity into a voxel grid and keep voxels whose total passes a threshold.
//! 3. **Surface mesh** — emit the boundary faces of the occupied voxel set.
//! 4. **Convex decomposition** — run `tpt_nova_ingest::decompose` on that
//!    mesh to obtain a handful of approximately-convex parts, which become one
//!    multi-part [`Collider3D`]. If decomposition yields nothing, each connected voxel
//!    component falls back to its own convex hull.
//!
//! All parameters live in a serializable [`ProxyParams`] so a scene can record
//! how its proxy was produced and regenerate it later.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use tpt_nova_ecs::transform::Transform;
use tpt_nova_ecs::{Entity, World};
use tpt_nova_ingest::decompose::{decompose_convex, ConvexPart, DecompositionParams};
use tpt_nova_ingest::hull::convex_hull;
use tpt_nova_ingest::loader::MeshData;
use tpt_nova_ingest::Collider3D;

use crate::{SplatCloud, SplatError};

/// Tunable proxy-generation parameters (serializable so they can live in a scene).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProxyParams {
    /// Splats with opacity below this are dropped as floaters.
    pub opacity_threshold: f32,
    /// Splats whose largest std-dev exceeds `max_scale_ratio × median` are dropped.
    pub max_scale_ratio: f32,
    /// Neighbors used for the density outlier test.
    pub knn_k: usize,
    /// A splat whose mean k-NN distance exceeds `knn_std_ratio × median` is dropped.
    pub knn_std_ratio: f32,
    /// Voxel edge length in world units. `0.0` derives it from the cloud size.
    pub voxel_size: f32,
    /// Minimum accumulated opacity (in "opaque splat" units) for a voxel to count.
    pub voxel_density_threshold: f32,
    /// Upper bound on convex parts produced by decomposition.
    pub max_parts: u32,
    /// Decomposition concavity tolerance (lower = tighter fit, more parts).
    pub concavity: f32,
}

impl Default for ProxyParams {
    fn default() -> Self {
        ProxyParams {
            opacity_threshold: 0.2,
            max_scale_ratio: 8.0,
            knn_k: 8,
            knn_std_ratio: 3.0,
            voxel_size: 0.0,
            voxel_density_threshold: 0.5,
            max_parts: 8,
            concavity: 0.01,
        }
    }
}

/// Counts describing how a proxy was produced (useful for telemetry and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyReport {
    pub input_splats: usize,
    pub kept_splats: usize,
    pub occupied_voxels: usize,
    pub parts: usize,
    /// True when decomposition produced nothing and per-component hulls were used.
    pub used_fallback: bool,
}

/// Remove floaters: low opacity, oversized, and k-NN density outliers.
/// Returns the indices (into `cloud.splats`) that survive, in ascending order.
pub fn filter_floaters(cloud: &SplatCloud, params: &ProxyParams) -> Vec<u32> {
    let n = cloud.len();
    let mut keep: Vec<bool> = cloud
        .splats
        .iter()
        .map(|s| s.opacity >= params.opacity_threshold)
        .collect();

    // Oversized splats relative to the cloud's typical size.
    let mut sizes: Vec<f32> = cloud.splats.iter().map(|s| s.max_scale()).collect();
    if let Some(med) = median(&mut sizes) {
        let limit = med * params.max_scale_ratio;
        for (i, s) in cloud.splats.iter().enumerate() {
            if s.max_scale() > limit {
                keep[i] = false;
            }
        }
    }

    // Density outliers: compare each splat's mean k-NN distance to the median.
    if params.knn_k > 0 && n > params.knn_k {
        let positions: Vec<[f32; 3]> = cloud.splats.iter().map(|s| s.position).collect();
        let mean_knn = mean_knn_distances(&positions, params.knn_k);
        let mut sorted = mean_knn.clone();
        if let Some(med) = median(&mut sorted) {
            let limit = med * params.knn_std_ratio;
            for i in 0..n {
                if mean_knn[i] > limit {
                    keep[i] = false;
                }
            }
        }
    }

    keep.iter()
        .enumerate()
        .filter_map(|(i, &k)| k.then_some(i as u32))
        .collect()
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(|a, b| a.total_cmp(b));
    Some(values[values.len() / 2])
}

/// Mean distance to the `k` nearest other points for every point, using a
/// uniform grid so the cost stays near-linear for captures of realistic size.
fn mean_knn_distances(points: &[[f32; 3]], k: usize) -> Vec<f32> {
    let n = points.len();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in points {
        for i in 0..3 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let extent = (0..3)
        .map(|i| hi[i] - lo[i])
        .fold(0.0f32, f32::max)
        .max(1e-6);
    // Roughly one point per cell on average.
    let cell = (extent / (n as f32).cbrt().max(1.0)).max(1e-6);
    let key = |p: &[f32; 3]| -> (i32, i32, i32) {
        (
            ((p[0] - lo[0]) / cell).floor() as i32,
            ((p[1] - lo[1]) / cell).floor() as i32,
            ((p[2] - lo[2]) / cell).floor() as i32,
        )
    };
    let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
    for (i, p) in points.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i as u32);
    }

    let mut out = vec![0.0f32; n];
    for (i, p) in points.iter().enumerate() {
        let (cx, cy, cz) = key(p);
        let mut best: Vec<f32> = Vec::new();
        let mut ring = 0i32;
        loop {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    for dz in -ring..=ring {
                        // Only the shell of this ring; interior was visited already.
                        if dx.abs() != ring && dy.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        if let Some(cell_pts) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                            for &j in cell_pts {
                                if j as usize == i {
                                    continue;
                                }
                                let q = points[j as usize];
                                let d2 = (p[0] - q[0]).powi(2)
                                    + (p[1] - q[1]).powi(2)
                                    + (p[2] - q[2]).powi(2);
                                best.push(d2);
                            }
                        }
                    }
                }
            }
            best.sort_unstable_by(|a, b| a.total_cmp(b));
            best.truncate(k);
            // Once the k-th neighbor is closer than the searched shell, it is exact.
            let searched = ring as f32 * cell;
            let done = best.len() >= k && best[k - 1].sqrt() <= searched;
            if done || ring as f32 * cell > extent * 2.0 + cell {
                break;
            }
            ring += 1;
        }
        let kk = best.len().max(1);
        out[i] = best.iter().take(k).map(|d| d.sqrt()).sum::<f32>() / kk as f32;
    }
    out
}

/// Opacity-weighted voxelization: returns the occupied voxel keys and the voxel edge length.
pub fn voxelize(
    cloud: &SplatCloud,
    indices: &[u32],
    params: &ProxyParams,
) -> (HashSet<(i32, i32, i32)>, f32, [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for &i in indices {
        let p = cloud.splats[i as usize].position;
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    if indices.is_empty() {
        return (HashSet::new(), 1.0, [0.0; 3]);
    }
    let extent = (0..3)
        .map(|k| hi[k] - lo[k])
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let vs = if params.voxel_size > 0.0 {
        params.voxel_size
    } else {
        extent / 48.0
    };
    let mut weight: HashMap<(i32, i32, i32), f32> = HashMap::new();
    for &i in indices {
        let s = &cloud.splats[i as usize];
        let key = (
            ((s.position[0] - lo[0]) / vs).floor() as i32,
            ((s.position[1] - lo[1]) / vs).floor() as i32,
            ((s.position[2] - lo[2]) / vs).floor() as i32,
        );
        *weight.entry(key).or_insert(0.0) += s.opacity;
    }
    let occupied = weight
        .into_iter()
        .filter(|(_, w)| *w >= params.voxel_density_threshold)
        .map(|(k, _)| k)
        .collect();
    (occupied, vs, lo)
}

/// Boundary surface of a voxel set as a triangle mesh (outward-facing winding).
pub fn voxel_surface_mesh(
    voxels: &HashSet<(i32, i32, i32)>,
    vs: f32,
    origin: [f32; 3],
    name: &str,
) -> MeshData {
    let mut vertex_ids: HashMap<(i32, i32, i32), u32> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut vid = |c: (i32, i32, i32), vertices: &mut Vec<[f32; 3]>| -> u32 {
        *vertex_ids.entry(c).or_insert_with(|| {
            vertices.push([
                origin[0] + c.0 as f32 * vs,
                origin[1] + c.1 as f32 * vs,
                origin[2] + c.2 as f32 * vs,
            ]);
            (vertices.len() - 1) as u32
        })
    };
    // Each face: axis, sign of the neighbor direction (+1 or -1).
    for &(x, y, z) in voxels {
        for axis in 0..3 {
            for sign in [-1i32, 1] {
                let mut nb = (x, y, z);
                match axis {
                    0 => nb.0 += sign,
                    1 => nb.1 += sign,
                    _ => nb.2 += sign,
                }
                if voxels.contains(&nb) {
                    continue;
                }
                // Face corners on the voxel's boundary plane facing `sign`.
                let base = [x, y, z];
                let plane = base[axis] + if sign > 0 { 1 } else { 0 };
                let (u, v) = match axis {
                    0 => (1usize, 2usize),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                let corner = |du: i32, dv: i32| -> (i32, i32, i32) {
                    let mut c = [0i32; 3];
                    c[axis] = plane;
                    c[u] = base[u] + du;
                    c[v] = base[v] + dv;
                    (c[0], c[1], c[2])
                };
                let p0 = corner(0, 0);
                let p1 = corner(1, 0);
                let p2 = corner(1, 1);
                let p3 = corner(0, 1);
                let ids = [
                    vid(p0, &mut vertices),
                    vid(p1, &mut vertices),
                    vid(p2, &mut vertices),
                    vid(p3, &mut vertices),
                ];
                // Orient the quad so its normal points away from the voxel set.
                let a = vertices[ids[0] as usize];
                let b = vertices[ids[1] as usize];
                let c = vertices[ids[2] as usize];
                let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let normal = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let outward = sign as f32
                    * if axis == 0 {
                        normal[0]
                    } else if axis == 1 {
                        normal[1]
                    } else {
                        normal[2]
                    };
                let (t0, t1, t2) = if outward >= 0.0 {
                    (ids[0], ids[1], ids[2])
                } else {
                    (ids[0], ids[2], ids[1])
                };
                let (s0, s1, s2) = if outward >= 0.0 {
                    (ids[0], ids[2], ids[3])
                } else {
                    (ids[0], ids[3], ids[2])
                };
                indices.extend_from_slice(&[t0, t1, t2, s0, s1, s2]);
            }
        }
    }
    MeshData {
        name: name.to_string(),
        vertices,
        indices,
    }
}

/// Convex hull of one connected voxel component, as a [`ConvexPart`].
fn hull_of_voxels(voxels: &[(i32, i32, i32)], vs: f32, origin: [f32; 3]) -> Option<ConvexPart> {
    let mut pts: Vec<[f32; 3]> = Vec::new();
    for &(x, y, z) in voxels {
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    pts.push([
                        origin[0] + (x + dx) as f32 * vs,
                        origin[1] + (y + dy) as f32 * vs,
                        origin[2] + (z + dz) as f32 * vs,
                    ]);
                }
            }
        }
    }
    let part = convex_hull(&pts)?;
    if part.vertices.len() < 4 || part.indices.is_empty() {
        return None;
    }
    Some(part)
}

/// Split occupied voxels into 6-connected components.
fn connected_components(voxels: &HashSet<(i32, i32, i32)>) -> Vec<Vec<(i32, i32, i32)>> {
    let mut seen: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut comps = Vec::new();
    let mut keys: Vec<_> = voxels.iter().copied().collect();
    keys.sort_unstable();
    for start in keys {
        if !seen.insert(start) {
            continue;
        }
        let mut comp = Vec::new();
        let mut queue = VecDeque::from([start]);
        while let Some(c) = queue.pop_front() {
            comp.push(c);
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let nb = (c.0 + dx, c.1 + dy, c.2 + dz);
                if voxels.contains(&nb) && seen.insert(nb) {
                    queue.push_back(nb);
                }
            }
        }
        comps.push(comp);
    }
    comps
}

/// Build the multi-part collision proxy for `cloud`.
pub fn build_proxy_collider(
    cloud: &SplatCloud,
    params: &ProxyParams,
) -> Result<(Collider3D, ProxyReport), SplatError> {
    if cloud.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    let kept = filter_floaters(cloud, params);
    if kept.is_empty() {
        return Err(SplatError::EmptyCloud);
    }
    let (voxels, vs, origin) = voxelize(cloud, &kept, params);
    if voxels.is_empty() {
        return Err(SplatError::MalformedSplat(
            "no voxels pass the density threshold".into(),
        ));
    }

    let mesh = voxel_surface_mesh(&voxels, vs, origin, "splat-proxy");
    let decomposition = DecompositionParams {
        max_convex_hulls: params.max_parts.max(1),
        concavity: params.concavity,
    };
    let mut parts = if mesh.indices.len() >= 3 {
        decompose_convex(&mesh, Some(decomposition)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut used_fallback = false;
    if parts.is_empty() {
        used_fallback = true;
        parts = connected_components(&voxels)
            .iter()
            .filter_map(|c| hull_of_voxels(c, vs, origin))
            .collect();
    }
    if parts.is_empty() {
        return Err(SplatError::MalformedSplat(
            "could not form any convex part from the splat voxels".into(),
        ));
    }
    let report = ProxyReport {
        input_splats: cloud.len(),
        kept_splats: kept.len(),
        occupied_voxels: voxels.len(),
        parts: parts.len(),
        used_fallback,
    };
    Ok((Collider3D::from_parts(parts), report))
}

/// Attach the multi-part proxy to `entity` (adds a [`Transform`] if missing).
pub fn attach_proxy_collider(
    world: &mut World,
    entity: Entity,
    cloud: &SplatCloud,
    params: &ProxyParams,
) -> Result<ProxyReport, SplatError> {
    let (collider, report) = build_proxy_collider(cloud, params)?;
    if world.get_component::<Transform>(entity).is_none() {
        world.add_component(entity, Transform::default());
    }
    world.add_component(entity, collider);
    Ok(report)
}

/// Sum of the volumes of a closed triangle set, via signed tetrahedra about the origin.
pub fn mesh_volume(vertices: &[[f32; 3]], triangles: &[[u32; 3]]) -> f32 {
    let mut v = 0.0f64;
    for t in triangles {
        let a = vertices[t[0] as usize];
        let b = vertices[t[1] as usize];
        let c = vertices[t[2] as usize];
        let det = a[0] as f64 * (b[1] as f64 * c[2] as f64 - b[2] as f64 * c[1] as f64)
            - a[1] as f64 * (b[0] as f64 * c[2] as f64 - b[2] as f64 * c[0] as f64)
            + a[2] as f64 * (b[0] as f64 * c[1] as f64 - b[1] as f64 * c[0] as f64);
        v += det / 6.0;
    }
    v.abs() as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Splat;

    fn splat(p: [f32; 3], opacity: f32) -> Splat {
        Splat {
            position: p,
            scale: [0.2; 3],
            rotation: [1.0, 0.0, 0.0, 0.0],
            color: [1.0, 1.0, 1.0, opacity],
            opacity,
        }
    }

    /// Solid-ish blob: a dense cubic lattice of opaque splats.
    fn blob(center: [f32; 3], half: i32) -> Vec<Splat> {
        let mut out = Vec::new();
        for x in -half..=half {
            for y in -half..=half {
                for z in -half..=half {
                    out.push(splat(
                        [
                            center[0] + x as f32 * 0.5,
                            center[1] + y as f32 * 0.5,
                            center[2] + z as f32 * 0.5,
                        ],
                        0.9,
                    ));
                }
            }
        }
        out
    }

    fn params() -> ProxyParams {
        ProxyParams {
            voxel_size: 0.25,
            voxel_density_threshold: 0.3,
            ..Default::default()
        }
    }

    #[test]
    fn floaters_are_removed() {
        let mut splats = blob([0.0; 3], 3);
        let real = splats.len();
        // Faint haze far away: low opacity, should go.
        for i in 0..5 {
            splats.push(splat([50.0 + i as f32, 0.0, 0.0], 0.01));
        }
        // An isolated, normal-opacity splat far away: density outlier.
        splats.push(splat([-200.0, 0.0, 0.0], 0.9));
        let cloud = SplatCloud::new(splats);
        let kept = filter_floaters(&cloud, &ProxyParams::default());
        assert!(kept.len() <= real, "kept {} of {real}", kept.len());
        assert!(!kept
            .iter()
            .any(|&i| cloud.splats[i as usize].position[0] > 40.0));
        assert!(!kept
            .iter()
            .any(|&i| cloud.splats[i as usize].position[0] < -100.0));
    }

    #[test]
    fn separated_blobs_yield_multiple_parts() {
        let mut splats = blob([0.0; 3], 3);
        splats.extend(blob([20.0, 0.0, 0.0], 3));
        let cloud = SplatCloud::new(splats);
        let (collider, report) = build_proxy_collider(&cloud, &params()).unwrap();
        assert!(
            collider.parts.len() >= 2,
            "parts = {}",
            collider.parts.len()
        );
        assert_eq!(report.parts, collider.parts.len());
    }

    #[test]
    fn l_shape_proxy_is_tighter_than_single_hull() {
        // An L made of two slabs.
        let mut splats = Vec::new();
        for x in 0..16 {
            for y in 0..4 {
                for z in 0..4 {
                    splats.push(splat(
                        [x as f32 * 0.25, y as f32 * 0.25, z as f32 * 0.25],
                        0.9,
                    ));
                }
            }
        }
        for x in 0..4 {
            for y in 4..16 {
                for z in 0..4 {
                    splats.push(splat(
                        [x as f32 * 0.25, y as f32 * 0.25, z as f32 * 0.25],
                        0.9,
                    ));
                }
            }
        }
        let cloud = SplatCloud::new(splats);
        // Baseline: a single hull over the same voxel geometry the parts are
        // built from. Splat centres sit half a voxel inside that geometry, so
        // they are not a like-for-like baseline.
        let p = params();
        let kept = filter_floaters(&cloud, &p);
        let (voxels, vs, origin) = voxelize(&cloud, &kept, &p);
        let voxel_mesh = voxel_surface_mesh(&voxels, vs, origin, "l-shape");
        let single = convex_hull(&voxel_mesh.vertices).unwrap();
        let hull_vol = mesh_volume(&single.vertices, &single.indices);
        let (proxy, _) = build_proxy_collider(&cloud, &p).unwrap();
        let parts_vol: f32 = proxy
            .parts
            .iter()
            .map(|p| mesh_volume(&p.vertices, &p.indices))
            .sum();
        assert!(
            parts_vol < 0.9 * hull_vol,
            "parts {parts_vol} should be well below hull {hull_vol}"
        );
    }

    #[test]
    fn surface_mesh_of_single_voxel_is_closed_box() {
        let mut voxels = HashSet::new();
        voxels.insert((0, 0, 0));
        let mesh = voxel_surface_mesh(&voxels, 1.0, [0.0; 3], "t");
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len() / 3, 12);
        let tris: Vec<[u32; 3]> = mesh.indices.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        assert!((mesh_volume(&mesh.vertices, &tris) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn components_are_split_by_face_connectivity() {
        let mut voxels = HashSet::new();
        voxels.insert((0, 0, 0));
        voxels.insert((1, 0, 0));
        voxels.insert((5, 5, 5));
        assert_eq!(connected_components(&voxels).len(), 2);
    }

    #[test]
    fn empty_cloud_is_rejected() {
        assert!(matches!(
            build_proxy_collider(&SplatCloud::default(), &ProxyParams::default()),
            Err(SplatError::EmptyCloud)
        ));
    }

    #[test]
    fn attach_adds_collider_and_transform() {
        let mut world = World::new();
        let e = world.spawn();
        let cloud = SplatCloud::new(blob([0.0; 3], 2));
        let report = attach_proxy_collider(&mut world, e, &cloud, &params()).unwrap();
        assert!(report.parts >= 1);
        assert!(world.get_component::<Collider3D>(e).is_some());
        assert!(world.get_component::<Transform>(e).is_some());
    }

    #[test]
    fn params_round_trip_through_serde() {
        let p = ProxyParams {
            max_parts: 5,
            ..Default::default()
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: ProxyParams = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
