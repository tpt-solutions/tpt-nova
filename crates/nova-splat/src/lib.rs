//! Gaussian Splat (3D Gaussian Splatting) support for TPT Nova.
//!
//! A 3DGS capture is a cloud of anisotropic 3D Gaussians ("splats"), each with a
//! position, a per-axis scale, a rotation quaternion, and an RGBA color. This
//! crate:
//!
//! * parses the two dominant on-disk formats — the flat `.splat` layout and the
//!   `.ply` 3DGS export (see [`loader`]),
//! * exposes the cloud as an ECS [`SplatCloud`] component,
//! * derives a **low-poly convex hull collider** from the splat positions
//!   ([`build_convex_hull_collider`]) so a captured scene participates in
//!   physics without per-splat cost,
//! * and (behind the `render` feature) provides a wgpu [`SplatPipeline`] that
//!   draws the cloud as camera-facing billboards integrated with `nova-render`.
//!
//! Position/color/opacity/rotation activations follow the de-facto 3DGS
//! conventions (sigmoid opacity, `exp` scale, normalized quaternion, SH-DC
//! color), so captured data round-trips through the same decode used to train
//! it.

use nova_ecs::component::Component;

pub mod collider;
pub mod loader;

#[cfg(feature = "render")]
pub mod render;

pub use collider::{attach_hull_collider, build_convex_hull_collider, hull_triangle_count};
pub use loader::{load_file, load_ply_bytes, load_splat_bytes};

/// Errors raised while loading or processing a splat cloud.
#[derive(Debug, thiserror::Error)]
pub enum SplatError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported splat file extension (expected .splat or .ply)")]
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
}

/// A collection of [`Splat`]s stored as an ECS component.
///
/// Kept as a flat `Vec` (structure-of-arrays is overkill at this layer; the
/// render pipeline and collider generation both want random access by index).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SplatCloud {
    pub splats: Vec<Splat>,
}

impl SplatCloud {
    pub fn new(splats: Vec<Splat>) -> Self {
        SplatCloud { splats }
    }

    pub fn len(&self) -> usize {
        self.splats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.splats.is_empty()
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

    #[test]
    fn sh_dc_maps_zero_dc_to_mid_gray() {
        assert!((sh_dc_to_linear(0.0) - 0.5).abs() < 1e-6);
    }
}
