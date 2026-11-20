//! Spatial chunking, deterministic decimation, frustum culling and distance LOD.
//!
//! Very large captures are split into **spatial chunks** (contiguous runs in a
//! Morton/Z-order curve, so each chunk is compact in space). Each chunk has its
//! own bounding box, which lets the renderer:
//!
//! * skip chunks outside the view frustum ([`Frustum::intersects_aabb`]),
//! * pick a decimation level from the chunk's camera distance ([`LodConfig`]),
//! * and upload each chunk to the GPU independently (`render.rs`).
//!
//! Decimation is **seeded and per-splat**: whether splat `i` survives a ratio
//! depends only on `(seed, i)`. A splat kept at a coarse LOD is therefore also
//! kept at every finer LOD, so switching levels never flickers unrelated splats.

use glam::{Mat4, Vec3, Vec4};

use crate::{Aabb, SplatCloud};

/// One spatial chunk: indices into the source cloud plus its bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct SplatChunk {
    /// Global splat indices belonging to this chunk, in chunk order.
    pub indices: Vec<u32>,
    /// Bounds of the chunk's splats (centers padded by their std-dev).
    pub aabb: Aabb,
}

/// Distance-based level-of-detail and chunking parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodConfig {
    /// Target splats per chunk.
    pub chunk_size: usize,
    /// Chunk distance (world units) beyond which the mid LOD is used.
    pub mid_distance: f32,
    /// Chunk distance beyond which the far LOD is used.
    pub far_distance: f32,
    /// Fraction of splats kept at the mid LOD.
    pub mid_ratio: f32,
    /// Fraction of splats kept at the far LOD.
    pub far_ratio: f32,
    /// Seed for deterministic decimation.
    pub seed: u64,
}

impl Default for LodConfig {
    fn default() -> Self {
        LodConfig {
            chunk_size: 4096,
            mid_distance: 20.0,
            far_distance: 60.0,
            mid_ratio: 0.5,
            far_ratio: 0.25,
            seed: 0x5EED,
        }
    }
}

impl LodConfig {
    /// Keep-ratio for a chunk at `distance` from the camera.
    pub fn ratio_for_distance(&self, distance: f32) -> f32 {
        if distance < self.mid_distance {
            1.0
        } else if distance < self.far_distance {
            self.mid_ratio
        } else {
            self.far_ratio
        }
    }
}

/// SplitMix64 finalizer: a cheap, well-mixed 64-bit hash.
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic keep test: whether splat `index` survives `ratio` under `seed`.
pub fn keep_splat(seed: u64, index: u32, ratio: f32) -> bool {
    if ratio >= 1.0 {
        return true;
    }
    if ratio <= 0.0 {
        return false;
    }
    let h = mix64(seed ^ (index as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    // Map the top 24 bits to [0, 1).
    let u = (h >> 40) as f32 / (1u64 << 24) as f32;
    u < ratio
}

impl SplatCloud {
    /// A deterministic subset keeping roughly `ratio` of the splats.
    ///
    /// The same `(cloud, ratio, seed)` always yields the same output, and a
    /// larger ratio's output is a superset of a smaller ratio's. SH blocks are
    /// carried along with their splats.
    pub fn decimate(&self, ratio: f32, seed: u64) -> SplatCloud {
        let mut out = self.clone();
        out.retain_indices(|i| keep_splat(seed, i as u32, ratio));
        out
    }
}

/// Spread the low 10 bits of `v` so there are two zero bits between each.
fn spread10(v: u32) -> u32 {
    let mut x = v & 0x3ff;
    x = (x | (x << 16)) & 0x0300_00ff;
    x = (x | (x << 8)) & 0x0300_f00f;
    x = (x | (x << 4)) & 0x030c_30c3;
    x = (x | (x << 2)) & 0x0924_9249;
    x
}

/// 30-bit Morton (Z-order) code of a point normalized into `[0, 1]³`.
pub fn morton3(p: [f32; 3]) -> u32 {
    let q = |v: f32| ((v.clamp(0.0, 1.0)) * 1023.0).round() as u32;
    spread10(q(p[0])) | (spread10(q(p[1])) << 1) | (spread10(q(p[2])) << 2)
}

/// Split a cloud into spatial chunks of about `cfg.chunk_size` splats each,
/// ordered along a Morton curve. Every splat appears in exactly one chunk.
pub fn build_chunks(cloud: &SplatCloud, cfg: &LodConfig) -> Vec<SplatChunk> {
    if cloud.is_empty() {
        return Vec::new();
    }
    let size = cfg.chunk_size.max(1);
    // Normalize centers into the cloud's center bounds for the Morton code.
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for s in &cloud.splats {
        for i in 0..3 {
            lo[i] = lo[i].min(s.position[i]);
            hi[i] = hi[i].max(s.position[i]);
        }
    }
    let span: [f32; 3] = std::array::from_fn(|i| (hi[i] - lo[i]).max(1e-9));
    let mut keyed: Vec<(u32, u32)> = cloud
        .splats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let n: [f32; 3] = std::array::from_fn(|k| (s.position[k] - lo[k]) / span[k]);
            (morton3(n), i as u32)
        })
        .collect();
    keyed.sort_unstable();

    keyed
        .chunks(size)
        .map(|run| {
            let indices: Vec<u32> = run.iter().map(|&(_, i)| i).collect();
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for &i in &indices {
                let s = &cloud.splats[i as usize];
                let r = s.max_scale();
                for k in 0..3 {
                    min[k] = min[k].min(s.position[k] - r);
                    max[k] = max[k].max(s.position[k] + r);
                }
            }
            SplatChunk {
                indices,
                aabb: Aabb { min, max },
            }
        })
        .collect()
}

/// The six planes of a view frustum, as `(normal, d)` with `n·p + d >= 0` inside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extract the frustum from a world→clip matrix (Gribb–Hartmann).
    ///
    /// The near plane is loosened to `z ≥ -w`, so the test is conservative for
    /// both the `[0, 1]` (wgpu/glam `perspective_rh`) and `[-1, 1]` depth ranges.
    pub fn from_view_proj(vp: Mat4) -> Frustum {
        let r0 = vp.row(0);
        let r1 = vp.row(1);
        let r2 = vp.row(2);
        let r3 = vp.row(3);
        let raw = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r3 + r2, r3 - r2];
        let planes = raw.map(|p| {
            let len = p.truncate().length();
            if len > 1e-12 {
                p / len
            } else {
                p
            }
        });
        Frustum { planes }
    }

    /// Conservative AABB test: `false` only when the box is fully outside.
    pub fn intersects_aabb(&self, aabb: &Aabb) -> bool {
        self.planes.iter().all(|p| {
            // Positive vertex: the corner furthest along the plane normal.
            let x = if p.x >= 0.0 { aabb.max[0] } else { aabb.min[0] };
            let y = if p.y >= 0.0 { aabb.max[1] } else { aabb.min[1] };
            let z = if p.z >= 0.0 { aabb.max[2] } else { aabb.min[2] };
            p.x * x + p.y * y + p.z * z + p.w >= 0.0
        })
    }
}

/// Visible, LOD-selected local indices for one chunk, **unsorted**.
///
/// This is the CPU half of the GPU sort path: the expensive part (per-splat
/// frustum + LOD filtering) stays on the CPU, while the per-camera depth sort is
/// left to [`crate::render::SplatPipeline`]'s GPU radix sort. The CPU path
/// ([`visible_order`]) calls this and then sorts.
///
/// `chunk_centers` are the chunk's splat centers (local order), `global_indices`
/// the matching cloud indices used for the deterministic keep test.
pub fn visible_candidates(
    chunk_centers: &[[f32; 3]],
    global_indices: &[u32],
    chunk_aabb: &Aabb,
    cam_pos: Vec3,
    frustum: &Frustum,
    cfg: &LodConfig,
) -> Vec<u32> {
    if !frustum.intersects_aabb(chunk_aabb) {
        return Vec::new();
    }
    let c = Vec3::from(chunk_aabb.center());
    let distance = (c - cam_pos).length();
    let ratio = cfg.ratio_for_distance(distance);
    chunk_centers
        .iter()
        .zip(global_indices)
        .enumerate()
        .filter(|(_, (_, &global))| keep_splat(cfg.seed, global, ratio))
        .map(|(local, _)| local as u32)
        .collect()
}

/// Visible, LOD-selected local indices for one chunk, ordered back to front.
///
/// `chunk_centers` are the chunk's splat centers (local order), `global_indices`
/// the matching cloud indices used for the deterministic keep test.
pub fn visible_order(
    chunk_centers: &[[f32; 3]],
    global_indices: &[u32],
    chunk_aabb: &Aabb,
    cam_pos: Vec3,
    view: Mat4,
    frustum: &Frustum,
    cfg: &LodConfig,
) -> Vec<u32> {
    let candidates = visible_candidates(
        chunk_centers,
        global_indices,
        chunk_aabb,
        cam_pos,
        frustum,
        cfg,
    );
    let mut keyed: Vec<(f32, u32)> = candidates
        .into_iter()
        .map(|local| {
            let center = chunk_centers[local as usize];
            let v = view * Vec4::new(center[0], center[1], center[2], 1.0);
            (v.z, local)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, i)| i).collect()
}

/// Camera world position from a world→view matrix (`-Rᵀ t`).
pub fn camera_position(view: Mat4) -> Vec3 {
    let r = glam::Mat3::from_mat4(view);
    -(r.transpose() * view.w_axis.truncate())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Splat, SplatCloud};

    fn grid(n: usize) -> SplatCloud {
        let mut splats = Vec::new();
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    splats.push(Splat {
                        position: [x as f32, y as f32, z as f32],
                        scale: [0.1; 3],
                        rotation: [1.0, 0.0, 0.0, 0.0],
                        color: [1.0; 4],
                        opacity: 1.0,
                    });
                }
            }
        }
        SplatCloud::new(splats)
    }

    #[test]
    fn decimate_is_deterministic_and_roughly_proportional() {
        let c = grid(10); // 1000 splats
        let a = c.decimate(0.25, 42);
        let b = c.decimate(0.25, 42);
        assert_eq!(a, b);
        let n = a.len() as f32;
        assert!((n - 250.0).abs() < 60.0, "kept {n}");
        // A different seed picks a different subset.
        assert_ne!(a, c.decimate(0.25, 43));
    }

    #[test]
    fn coarser_ratio_is_subset_of_finer_ratio() {
        for i in 0..2000u32 {
            if keep_splat(7, i, 0.25) {
                assert!(keep_splat(7, i, 0.5), "splat {i} dropped at finer ratio");
            }
        }
    }

    #[test]
    fn decimate_keeps_sh_aligned() {
        let c = grid(4);
        let sh = vec![[0.5f32; crate::sh::SH_FLOATS]; c.len()];
        let c = SplatCloud::with_sh(c.splats, sh, 3).unwrap();
        let d = c.decimate(0.5, 1);
        assert_eq!(d.sh.len(), d.len());
    }

    #[test]
    fn chunks_partition_every_splat_once() {
        let c = grid(8); // 512 splats
        let cfg = LodConfig {
            chunk_size: 100,
            ..Default::default()
        };
        let chunks = build_chunks(&c, &cfg);
        assert_eq!(chunks.len(), 6); // 5 full + 1 partial
        let mut seen = vec![false; c.len()];
        for ch in &chunks {
            assert!(ch.indices.len() <= 100);
            for &i in &ch.indices {
                assert!(!seen[i as usize], "duplicate {i}");
                seen[i as usize] = true;
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn morton_orders_nearby_points_together() {
        // Points in the same octant share the high bits of the code.
        let a = morton3([0.01, 0.01, 0.01]);
        let b = morton3([0.02, 0.01, 0.01]);
        let far = morton3([0.99, 0.99, 0.99]);
        assert!((a ^ b) < (a ^ far));
    }

    #[test]
    fn frustum_culls_box_behind_camera_and_keeps_box_in_view() {
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let f = Frustum::from_view_proj(proj * view);
        let ahead = Aabb {
            min: [-1.0, -1.0, -6.0],
            max: [1.0, 1.0, -4.0],
        };
        let behind = Aabb {
            min: [-1.0, -1.0, 4.0],
            max: [1.0, 1.0, 6.0],
        };
        let far_left = Aabb {
            min: [-500.0, -1.0, -6.0],
            max: [-400.0, 1.0, -4.0],
        };
        assert!(f.intersects_aabb(&ahead));
        assert!(!f.intersects_aabb(&behind));
        assert!(!f.intersects_aabb(&far_left));
    }

    #[test]
    fn distance_lod_reduces_far_chunks() {
        let cfg = LodConfig::default();
        assert_eq!(cfg.ratio_for_distance(1.0), 1.0);
        assert_eq!(cfg.ratio_for_distance(30.0), 0.5);
        assert_eq!(cfg.ratio_for_distance(100.0), 0.25);
    }

    #[test]
    fn visible_order_is_back_to_front_and_culls() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let f = Frustum::from_view_proj(proj * view);
        let centers = [[0.0, 0.0, -2.0], [0.0, 0.0, -8.0], [0.0, 0.0, -5.0]];
        let globals = [0u32, 1, 2];
        let aabb = Aabb {
            min: [-1.0, -1.0, -9.0],
            max: [1.0, 1.0, -1.0],
        };
        let cfg = LodConfig {
            mid_distance: 1000.0,
            ..Default::default()
        };
        let order = visible_order(&centers, &globals, &aabb, Vec3::ZERO, view, &f, &cfg);
        assert_eq!(order, vec![1, 2, 0]);

        let behind = Aabb {
            min: [-1.0, -1.0, 2.0],
            max: [1.0, 1.0, 4.0],
        };
        assert!(visible_order(&centers, &globals, &behind, Vec3::ZERO, view, &f, &cfg).is_empty());
    }

    #[test]
    fn visible_candidates_matches_visible_order_set() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let f = Frustum::from_view_proj(proj * view);
        let centers = [[0.0, 0.0, -2.0], [0.0, 0.0, -8.0], [0.0, 0.0, -5.0]];
        let globals = [0u32, 1, 2];
        let aabb = Aabb {
            min: [-1.0, -1.0, -9.0],
            max: [1.0, 1.0, -1.0],
        };
        let cfg = LodConfig::default();
        let mut candidates = visible_candidates(&centers, &globals, &aabb, Vec3::ZERO, &f, &cfg);
        let order = visible_order(&centers, &globals, &aabb, Vec3::ZERO, view, &f, &cfg);
        candidates.sort_unstable();
        let mut sorted_order = order.clone();
        sorted_order.sort_unstable();
        assert_eq!(candidates, sorted_order);
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn camera_position_inverts_view_translation() {
        let eye = Vec3::new(3.0, -2.0, 7.0);
        let view = Mat4::look_at_rh(eye, eye + Vec3::NEG_Z, Vec3::Y);
        assert!((camera_position(view) - eye).length() < 1e-4);
    }
}
