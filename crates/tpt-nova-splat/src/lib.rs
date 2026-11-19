//! Gaussian Splat (3D Gaussian Splatting) support for TPT Nova.
//!
//! A 3DGS capture is a cloud of anisotropic 3D Gaussians ("splats"), each with a
//! position, a per-axis scale, a rotation quaternion, an RGBA color and
//! (optionally) higher-order spherical-harmonic color bands. This crate:
//!
//! * parses the on-disk formats — the flat `.splat` layout, the `.ply` 3DGS
//!   export (including `f_rest_*` SH bands, see [`loader`]), and a quantized
//!   chunked `.compressed.ply` layout (see [`compress`]),
//! * exposes the cloud as an ECS [`SplatCloud`] component,
//! * derives collision proxies: a single [`build_convex_hull_collider`] or a
//!   multi-part VHACD proxy from [`proxy`],
//! * edits clouds in place ([`edit`]): AABB / screen-rect selection, delete,
//!   crop and affine transform,
//! * and (behind the `render` feature) provides a wgpu [`SplatPipeline`] that
//!   draws anisotropic, depth-sorted Gaussians with view-dependent SH color,
//!   frustum culling and distance LOD ([`lod`]).
//!
//! Position/color/opacity/rotation activations follow the de-facto 3DGS
//! conventions (sigmoid opacity, `exp` scale, normalized quaternion, SH color),
//! so captured data round-trips through the same decode used to train it.

use glam::{Mat3, Mat4, Quat, Vec3, Vec4};
use tpt_nova_ecs::component::Component;

pub mod collider;
pub mod compress;
pub mod edit;
pub mod loader;
pub mod lod;
pub mod proxy;
pub mod sh;
pub mod telemetry;

#[cfg(feature = "render")]
pub mod render;

pub use collider::{attach_hull_collider, build_convex_hull_collider, hull_triangle_count};
pub use compress::{encode_compressed_ply, load_compressed_ply_bytes};
pub use edit::{SplatSelection, SplatSummary};
pub use loader::{load_file, load_ply_bytes, load_splat_bytes};
pub use lod::{LodConfig, SplatChunk};
pub use proxy::{attach_proxy_collider, build_proxy_collider, ProxyParams};
pub use sh::SplatQuality;

/// Errors raised while loading or processing a splat cloud.
#[derive(Debug, thiserror::Error)]
pub enum SplatError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported splat file extension (expected .splat, .ply or .compressed.ply)")]
    UnknownExtension,
    #[error("malformed .splat file: {0}")]
    MalformedSplat(String),
    #[error("malformed .ply file: {0}")]
    MalformedPly(String),
    #[error("empty splat cloud (no Gaussians)")]
    EmptyCloud,
}

/// One 3D Gaussian.
///
/// All fields are in the activation-decoded ("render-ready") space: scales are
/// already `exp`-activated std-devs, the rotation quaternion is unit-length, and
/// `color` carries linear RGBA in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Splat {
    pub position: [f32; 3],
    pub scale: [f32; 3],
    pub rotation: [f32; 4], // w, x, y, z
    pub color: [f32; 4],    // r, g, b, a (linear, 0..=1)
    pub opacity: f32,       // 0..=1
}

impl Splat {
    /// Decode the antimatter15 `.splat` 32-byte record.
    ///
    /// Layout: 3×f32 position, 3×f32 `ln` scale, 4×u8 RGBA, 4×u8 quaternion
    /// packed as `round(q·128 + 128)` with `q` a unit quaternion.
    pub fn from_splat_bytes(b: &[u8]) -> Result<Splat, SplatError> {
        if b.len() < 32 {
            return Err(SplatError::MalformedSplat(format!(
                "record is {} bytes, expected 32",
                b.len()
            )));
        }
        let position = [
            f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            f32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            f32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        ];
        let scale_log = [
            f32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            f32::from_le_bytes([b[16], b[17], b[18], b[19]]),
            f32::from_le_bytes([b[20], b[21], b[22], b[23]]),
        ];
        let scale = [scale_log[0].exp(), scale_log[1].exp(), scale_log[2].exp()];
        let color = [
            b[24] as f32 / 255.0,
            b[25] as f32 / 255.0,
            b[26] as f32 / 255.0,
            b[27] as f32 / 255.0,
        ];
        // Quaternion: (w, x, y, z) = (b[28..31], b[31]) in packed form.
        let quat = [
            (b[28] as f32 - 128.0) / 128.0,
            (b[29] as f32 - 128.0) / 128.0,
            (b[30] as f32 - 128.0) / 128.0,
            (b[31] as f32 - 128.0) / 128.0,
        ];
        let rotation = normalize_quat(quat);
        Ok(Splat {
            position,
            scale,
            rotation,
            color,
            opacity: color[3],
        })
    }

    /// Largest per-axis std-dev; a useful single "radius" for billboard sizing.
    pub fn max_scale(&self) -> f32 {
        self.scale[0].max(self.scale[1]).max(self.scale[2])
    }

    /// World-space 3D covariance `Σ = R S Sᵀ Rᵀ` of this Gaussian.
    ///
    /// `rotation` is stored `(w, x, y, z)`; glam takes `(x, y, z, w)`.
    pub fn covariance3d(&self) -> Mat3 {
        let [w, x, y, z] = self.rotation;
        let r = Mat3::from_quat(Quat::from_xyzw(x, y, z, w));
        let m = r * Mat3::from_diagonal(Vec3::from(self.scale));
        m * m.transpose()
    }

    /// Replace rotation and per-axis scale so [`Splat::covariance3d`] equals
    /// `cov` (symmetric, positive semidefinite). Preserves non-uniform scaling
    /// exactly after an affine transform.
    pub fn set_covariance(&mut self, cov: Mat3) {
        let (vals, mut v) = symmetric_eigen3(cov);
        // Keep a right-handed basis so the quaternion is a proper rotation.
        if v.determinant() < 0.0 {
            v = Mat3::from_cols(v.x_axis, v.y_axis, -v.z_axis);
        }
        let q = Quat::from_mat3(&v).normalize();
        self.rotation = [q.w, q.x, q.y, q.z];
        self.scale = [
            vals[0].max(1e-12).sqrt(),
            vals[1].max(1e-12).sqrt(),
            vals[2].max(1e-12).sqrt(),
        ];
    }
}

/// Eigen-decomposition of a symmetric 3×3 matrix by cyclic Jacobi rotations.
///
/// Returns `(eigenvalues, eigenvectors)`; the eigenvectors are the columns of
/// the returned matrix, ordered by descending eigenvalue.
// Index loops are intentional: each sweep rotates a fixed (p, q) pair of rows
// and columns, which iterator forms would obscure.
#[allow(clippy::needless_range_loop)]
pub fn symmetric_eigen3(m: Mat3) -> ([f32; 3], Mat3) {
    let c = m.to_cols_array_2d();
    // Row-major working copy: a[row][col].
    let mut a = [
        [c[0][0], c[1][0], c[2][0]],
        [c[0][1], c[1][1], c[2][1]],
        [c[0][2], c[1][2], c[2][2]],
    ];
    let mut v = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..24 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        if off < 1e-20 {
            break;
        }
        for (p, q) in [(0usize, 1usize), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-30 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = if theta == 0.0 {
                1.0
            } else {
                theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
            };
            let cs = 1.0 / (t * t + 1.0).sqrt();
            let sn = t * cs;
            for k in 0..3 {
                let (akp, akq) = (a[k][p], a[k][q]);
                a[k][p] = cs * akp - sn * akq;
                a[k][q] = sn * akp + cs * akq;
            }
            for k in 0..3 {
                let (apk, aqk) = (a[p][k], a[q][k]);
                a[p][k] = cs * apk - sn * aqk;
                a[q][k] = sn * apk + cs * aqk;
            }
            for row in v.iter_mut() {
                let (vkp, vkq) = (row[p], row[q]);
                row[p] = cs * vkp - sn * vkq;
                row[q] = sn * vkp + cs * vkq;
            }
        }
    }
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| a[j][j].total_cmp(&a[i][i]));
    let vals = [
        a[order[0]][order[0]],
        a[order[1]][order[1]],
        a[order[2]][order[2]],
    ];
    let col = |i: usize| Vec3::new(v[0][order[i]], v[1][order[i]], v[2][order[i]]);
    (vals, Mat3::from_cols(col(0), col(1), col(2)))
}

/// A collection of [`Splat`]s stored as an ECS component.
///
/// Kept as a flat `Vec` (structure-of-arrays is overkill at this layer; the
/// render pipeline and collider generation both want random access by index).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplatCloud {
    pub splats: Vec<Splat>,
    /// Per-splat SH coefficients (see [`sh`]). Empty when the source carried no
    /// higher-order bands; otherwise one block per splat.
    pub sh: Vec<[f32; sh::SH_FLOATS]>,
    /// Highest SH degree present in `sh` (0 when `sh` is empty).
    pub sh_degree: u8,
}

impl SplatCloud {
    pub fn new(splats: Vec<Splat>) -> Self {
        SplatCloud {
            splats,
            sh: Vec::new(),
            sh_degree: 0,
        }
    }

    /// Build a cloud with SH coefficients attached. `sh` must have one block per splat.
    pub fn with_sh(
        splats: Vec<Splat>,
        sh: Vec<[f32; sh::SH_FLOATS]>,
        sh_degree: u8,
    ) -> Result<Self, SplatError> {
        if sh.len() != splats.len() {
            return Err(SplatError::MalformedSplat(format!(
                "SH block count {} does not match splat count {}",
                sh.len(),
                splats.len()
            )));
        }
        Ok(SplatCloud {
            splats,
            sh,
            sh_degree: sh_degree.min(sh::SH_MAX_DEGREE),
        })
    }

    pub fn len(&self) -> usize {
        self.splats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.splats.is_empty()
    }

    /// Whether SH coefficients are attached to this cloud.
    pub fn has_sh(&self) -> bool {
        !self.sh.is_empty() && self.sh.len() == self.splats.len()
    }

    /// Indices of all splats ordered **back to front** from a camera whose
    /// world→view matrix is `view`. Farthest first, so premultiplied "over"
    /// blending composites correctly. Ties break on index for determinism.
    pub fn back_to_front_order(&self, view: Mat4) -> Vec<u32> {
        let mut keyed: Vec<(f32, u32)> = self
            .splats
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let v = view * Vec4::new(s.position[0], s.position[1], s.position[2], 1.0);
                (v.z, i as u32)
            })
            .collect();
        // View space looks down -Z, so the most negative z is the farthest.
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        keyed.into_iter().map(|(_, i)| i).collect()
    }

    /// Keep only the splats whose index passes `keep` (SH blocks follow).
    pub fn retain_indices(&mut self, keep: impl Fn(usize) -> bool) {
        let mask: Vec<bool> = (0..self.splats.len()).map(&keep).collect();
        // Check before `splats` shrinks, or `has_sh` would compare mismatched lengths.
        let had_sh = self.has_sh();
        let mut i = 0usize;
        self.splats.retain(|_| {
            let k = mask[i];
            i += 1;
            k
        });
        if had_sh {
            let mut j = 0usize;
            self.sh.retain(|_| {
                let k = mask[j];
                j += 1;
                k
            });
        } else {
            self.sh.clear();
        }
        if self.splats.is_empty() {
            self.sh_degree = 0;
        }
    }

    /// Axis-aligned bounds of every splat center, expanded by each splat's
    /// largest std-dev so it encloses the visible cloud, not just the centers.
    pub fn bounds(&self) -> Aabb {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for s in &self.splats {
            let r = s.max_scale();
            for i in 0..3 {
                min[i] = min[i].min(s.position[i] - r);
                max[i] = max[i].max(s.position[i] + r);
            }
        }
        if min[0].is_infinite() {
            return Aabb {
                min: [0.0; 3],
                max: [0.0; 3],
            };
        }
        Aabb { min, max }
    }
}

impl Component for SplatCloud {}

/// An axis-aligned bounding box ([`SplatCloud::bounds`] output).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    /// Center point of the box.
    pub fn center(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    /// Size of the box on each axis.
    pub fn size(&self) -> [f32; 3] {
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }

    /// Whether `p` lies inside the box (inclusive on all faces).
    pub fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| p[i] >= self.min[i] && p[i] <= self.max[i])
    }
}

/// Normalize a `(w, x, y, z)` quaternion to unit length, falling back to the
/// identity quaternion for a zero-length input (degenerate captures).
pub fn normalize_quat(q: [f32; 4]) -> [f32; 4] {
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if len < 1e-8 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    [q[0] / len, q[1] / len, q[2] / len, q[3] / len]
}

/// Sigmoid activation used for 3DGS opacity.
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// First spherical-harmonics DC band to linear color: `0.5 + SH_C0 · dc`.
pub fn sh_dc_to_linear(dc: f32) -> f32 {
    const SH_C0: f32 = 0.2820948;
    let c = 0.5 + SH_C0 * dc;
    c.clamp(0.0, 1.0)
}

/// Project a world-space 3D covariance to the 2D screen-space covariance of the
/// splat's footprint, in **pixels**, using the perspective Jacobian (EWA
/// splatting). `view` is world→view, `fx`/`fy` are focal lengths in pixels.
///
/// Returns `(a, b, c)` for the symmetric 2×2 matrix `[[a, b], [b, c]]`, with the
/// standard +0.3 px² low-pass term added so every splat covers at least about a
/// pixel. The screen axes are x right and y up, matching the renderer's offsets.
pub fn project_covariance_2d(
    cov3: Mat3,
    center: [f32; 3],
    view: Mat4,
    fx: f32,
    fy: f32,
) -> [f32; 3] {
    let t = view * Vec4::new(center[0], center[1], center[2], 1.0);
    // Camera looks down -Z: depth is -t.z (positive in front of the camera).
    let depth = (-t.z).max(1e-4);
    let (tx, ty) = (t.x, t.y);
    // Rows of the Jacobian of (fx·tx/d, fy·ty/d) with d = -tz.
    let j0 = Vec3::new(fx / depth, 0.0, fx * tx / (depth * depth));
    let j1 = Vec3::new(0.0, fy / depth, fy * ty / (depth * depth));
    let w = Mat3::from_mat4(view);
    let cov_cam = w * cov3 * w.transpose();
    let a = j0.dot(cov_cam * j0) + 0.3;
    let b = j0.dot(cov_cam * j1);
    let c = j1.dot(cov_cam * j1) + 0.3;
    [a, b, c]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splat_from_splat_bytes_decodes_known_record() {
        // Position (1,2,3), zero log-scale (-> scale 1), white opaque color,
        // identity quaternion packed (w=1 -> 255, others -> 128).
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&1.0f32.to_le_bytes());
        b[4..8].copy_from_slice(&2.0f32.to_le_bytes());
        b[8..12].copy_from_slice(&3.0f32.to_le_bytes());
        // log scale = ln(1) = 0.
        b[12..16].copy_from_slice(&0.0f32.to_le_bytes());
        b[16..20].copy_from_slice(&0.0f32.to_le_bytes());
        b[20..24].copy_from_slice(&0.0f32.to_le_bytes());
        b[24] = 255;
        b[25] = 128;
        b[26] = 64;
        b[27] = 255;
        b[28] = 255; // w = 1
        b[29] = 128;
        b[30] = 128;
        b[31] = 128;

        let s = Splat::from_splat_bytes(&b).unwrap();
        assert_eq!(s.position, [1.0, 2.0, 3.0]);
        assert!((s.scale[0] - 1.0).abs() < 1e-5);
        assert!((s.color[0] - 1.0).abs() < 1e-5);
        assert!((s.color[1] - 128.0 / 255.0).abs() < 1e-5);
        // Identity quaternion -> rotation ≈ (1,0,0,0).
        assert!((s.rotation[0] - 1.0).abs() < 1e-5);
        assert_eq!(s.opacity, 1.0);
    }

    #[test]
    fn splat_bytes_reject_short_input() {
        assert!(Splat::from_splat_bytes(&[0u8; 16]).is_err());
    }

    #[test]
    fn normalize_quat_handles_zero() {
        assert_eq!(normalize_quat([0.0; 4]), [1.0, 0.0, 0.0, 0.0]);
        let q = normalize_quat([2.0, 0.0, 0.0, 0.0]);
        assert!((q[0] - 1.0).abs() < 1e-6);
    }

    fn unit_splat(position: [f32; 3], scale: [f32; 3], rotation: [f32; 4]) -> Splat {
        Splat {
            position,
            scale,
            rotation,
            color: [1.0; 4],
            opacity: 1.0,
        }
    }

    #[test]
    fn covariance_axis_aligned_is_diagonal_of_squared_scales() {
        let s = unit_splat([0.0; 3], [1.0, 2.0, 3.0], [1.0, 0.0, 0.0, 0.0]);
        let c = s.covariance3d();
        assert!((c.x_axis.x - 1.0).abs() < 1e-5);
        assert!((c.y_axis.y - 4.0).abs() < 1e-5);
        assert!((c.z_axis.z - 9.0).abs() < 1e-5);
        assert!(c.x_axis.y.abs() < 1e-5 && c.x_axis.z.abs() < 1e-5 && c.y_axis.z.abs() < 1e-5);
    }

    #[test]
    fn covariance_rotated_90_about_z_swaps_axes() {
        // 90° about +Z: x-axis std-dev 2 becomes the world-Y std-dev.
        let h = std::f32::consts::FRAC_PI_4;
        let s = unit_splat([0.0; 3], [2.0, 1.0, 1.0], [h.cos(), 0.0, 0.0, h.sin()]);
        let c = s.covariance3d();
        assert!((c.y_axis.y - 4.0).abs() < 1e-4, "yy={}", c.y_axis.y);
        assert!((c.x_axis.x - 1.0).abs() < 1e-4, "xx={}", c.x_axis.x);
    }

    #[test]
    fn set_covariance_round_trips() {
        let s = unit_splat([0.0; 3], [2.0, 0.5, 1.0], [0.9238795, 0.0, 0.3826834, 0.0]);
        let target = s.covariance3d();
        let mut r = unit_splat([0.0; 3], [1.0; 3], [1.0, 0.0, 0.0, 0.0]);
        r.set_covariance(target);
        let back = r.covariance3d();
        for (a, b) in target
            .to_cols_array()
            .iter()
            .zip(back.to_cols_array().iter())
        {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn jacobi_eigen_recovers_diagonal_values() {
        let m = Mat3::from_diagonal(Vec3::new(1.0, 5.0, 3.0));
        let (vals, _) = symmetric_eigen3(m);
        assert!((vals[0] - 5.0).abs() < 1e-5);
        assert!((vals[1] - 3.0).abs() < 1e-5);
        assert!((vals[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn projected_covariance_grows_with_distance_shrinks() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let cov = Mat3::from_diagonal(Vec3::splat(0.04));
        let near = project_covariance_2d(cov, [0.0, 0.0, -2.0], view, 500.0, 500.0);
        let far = project_covariance_2d(cov, [0.0, 0.0, -20.0], view, 500.0, 500.0);
        assert!(near[0] > far[0]);
        // Isotropic splat seen head-on stays isotropic.
        assert!((near[0] - near[2]).abs() < 1e-3);
        assert!(near[1].abs() < 1e-5);
    }

    #[test]
    fn projected_covariance_is_elongated_across_view_when_stretched() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let cov = Mat3::from_diagonal(Vec3::new(4.0, 0.01, 0.01));
        let [a, _, c] = project_covariance_2d(cov, [0.0, 0.0, -5.0], view, 500.0, 500.0);
        assert!(a > 10.0 * c, "a={a} c={c}");
    }

    #[test]
    fn back_to_front_orders_far_splats_first() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        let cloud = SplatCloud::new(vec![
            unit_splat([0.0, 0.0, -1.0], [0.1; 3], [1.0, 0.0, 0.0, 0.0]),
            unit_splat([0.0, 0.0, -10.0], [0.1; 3], [1.0, 0.0, 0.0, 0.0]),
            unit_splat([0.0, 0.0, -5.0], [0.1; 3], [1.0, 0.0, 0.0, 0.0]),
        ]);
        assert_eq!(cloud.back_to_front_order(view), vec![1, 2, 0]);
    }

    #[test]
    fn retain_keeps_sh_aligned() {
        let splats = (0..4)
            .map(|i| unit_splat([i as f32, 0.0, 0.0], [0.1; 3], [1.0, 0.0, 0.0, 0.0]))
            .collect();
        let sh = (0..4).map(|i| [i as f32; sh::SH_FLOATS]).collect();
        let mut cloud = SplatCloud::with_sh(splats, sh, 3).unwrap();
        cloud.retain_indices(|i| i % 2 == 1);
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.sh.len(), 2);
        assert_eq!(cloud.sh[0][0], 1.0);
        assert_eq!(cloud.sh[1][0], 3.0);
    }

    #[test]
    fn with_sh_rejects_mismatched_lengths() {
        let splats = vec![unit_splat([0.0; 3], [1.0; 3], [1.0, 0.0, 0.0, 0.0])];
        assert!(SplatCloud::with_sh(splats, Vec::new(), 3).is_err());
    }

    #[test]
    fn sh_dc_maps_zero_dc_to_mid_gray() {
        assert!((sh_dc_to_linear(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn bounds_encloses_padded_centers() {
        let cloud = SplatCloud::new(vec![
            Splat {
                position: [0.0, 0.0, 0.0],
                scale: [1.0, 1.0, 1.0],
                rotation: [1.0, 0.0, 0.0, 0.0],
                color: [1.0; 4],
                opacity: 1.0,
            },
            Splat {
                position: [2.0, 4.0, 6.0],
                scale: [0.5, 0.5, 0.5],
                rotation: [1.0, 0.0, 0.0, 0.0],
                color: [1.0; 4],
                opacity: 1.0,
            },
        ]);
        let b = cloud.bounds();
        assert_eq!(b.min, [-1.0, -1.0, -1.0]);
        assert_eq!(b.max, [2.5, 4.5, 6.5]);
        assert_eq!(b.center(), [0.75, 1.75, 2.75]);
    }

    #[test]
    fn empty_cloud_bounds_is_zero() {
        let b = SplatCloud::default().bounds();
        assert_eq!(b.min, [0.0; 3]);
        assert_eq!(b.max, [0.0; 3]);
    }
}
