//! Real spherical-harmonics (SH) color evaluation for Gaussian Splats.
//!
//! 3DGS stores view-dependent color as SH coefficients: degree 0 is the plain
//! DC term, degrees 1-3 add 3, 5 and 7 more bands per color channel (16
//! coefficients per channel at degree 3). The basis below follows the reference
//! 3DGS implementation exactly so captured coefficients decode identically.
//!
//! Storage layout used by [`crate::SplatCloud::sh`]: a flat `[f32; 48]` per
//! splat, channel-major — index `channel * 16 + k` for coefficient `k` of color
//! channel `channel` (0 = R, 1 = G, 2 = B). Coefficient `k = 0` is the *raw*
//! (unclamped, pre-`+0.5`) DC value from the file. The WGSL shader in
//! `render.rs` evaluates the same basis; the CPU version here is the reference
//! used by tests and the telemetry/CPU paths.

/// Number of SH coefficients per color channel at the maximum supported degree.
pub const SH_COEFFS_PER_CHANNEL: usize = 16;
/// Total stored floats per splat (3 channels × 16 coefficients).
pub const SH_FLOATS: usize = 3 * SH_COEFFS_PER_CHANNEL;
/// Maximum supported SH degree.
pub const SH_MAX_DEGREE: u8 = 3;

const SH_C0: f32 = 0.282_094_8;
const SH_C1: f32 = 0.488_602_5;
const SH_C2: [f32; 5] = [
    1.092_548_4,
    -1.092_548_4,
    0.315_391_57,
    -1.092_548_4,
    0.546_274_2,
];
const SH_C3: [f32; 7] = [
    -0.590_043_6,
    2.890_611_4,
    -0.457_045_8,
    0.373_176_33,
    -0.457_045_8,
    1.445_305_7,
    -0.590_043_6,
];

/// Number of non-DC coefficients per channel for a given degree (`(d+1)² - 1`).
pub fn rest_coeffs_per_channel(degree: u8) -> usize {
    let n = (degree as usize + 1) * (degree as usize + 1);
    n.saturating_sub(1)
}

/// Evaluate one color channel of SH at unit view direction `dir`.
///
/// `coeffs` is the 16-entry per-channel slice (`k = 0` is DC). Returns the
/// signed SH sum *before* the 3DGS `+ 0.5` offset and clamp; callers apply
/// [`sh_to_color`] to get the final linear color.
pub fn eval_sh_channel(degree: u8, dir: [f32; 3], coeffs: &[f32]) -> f32 {
    let [x, y, z] = dir;
    let mut r = SH_C0 * coeffs[0];
    if degree == 0 {
        return r;
    }
    r += -SH_C1 * y * coeffs[1] + SH_C1 * z * coeffs[2] - SH_C1 * x * coeffs[3];
    if degree == 1 {
        return r;
    }
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, yz, xz) = (x * y, y * z, x * z);
    r += SH_C2[0] * xy * coeffs[4]
        + SH_C2[1] * yz * coeffs[5]
        + SH_C2[2] * (2.0 * zz - xx - yy) * coeffs[6]
        + SH_C2[3] * xz * coeffs[7]
        + SH_C2[4] * (xx - yy) * coeffs[8];
    if degree == 2 {
        return r;
    }
    r += SH_C3[0] * y * (3.0 * xx - yy) * coeffs[9]
        + SH_C3[1] * xy * z * coeffs[10]
        + SH_C3[2] * y * (4.0 * zz - xx - yy) * coeffs[11]
        + SH_C3[3] * z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * coeffs[12]
        + SH_C3[4] * x * (4.0 * zz - xx - yy) * coeffs[13]
        + SH_C3[5] * z * (xx - yy) * coeffs[14]
        + SH_C3[6] * x * (xx - 3.0 * yy) * coeffs[15];
    r
}

/// Convert a signed SH sum to final linear color: `max(sum + 0.5, 0)`, with the
/// upper bound left to the caller (the renderer clamps to display range).
pub fn sh_to_color(sum: f32) -> f32 {
    (sum + 0.5).max(0.0)
}

/// Evaluate full RGB color for a splat's coefficients from `view_dir` (unit,
/// pointing from the camera toward the splat), capped at `degree`.
pub fn eval_sh_rgb(degree: u8, dir: [f32; 3], sh: &[f32; SH_FLOATS]) -> [f32; 3] {
    let degree = degree.min(SH_MAX_DEGREE);
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let coeffs = &sh[c * SH_COEFFS_PER_CHANNEL..(c + 1) * SH_COEFFS_PER_CHANNEL];
        *o = sh_to_color(eval_sh_channel(degree, dir, coeffs));
    }
    out
}

/// Build the per-splat coefficient block from a raw DC triple plus the
/// channel-major `f_rest` values (as stored in a 3DGS `.ply`). `rest` must hold
/// `3 * rest_coeffs_per_channel(degree)` entries; higher bands are left zero.
pub fn pack_sh(dc: [f32; 3], rest: &[f32], degree: u8) -> [f32; SH_FLOATS] {
    let m = rest_coeffs_per_channel(degree);
    let mut out = [0.0f32; SH_FLOATS];
    for c in 0..3 {
        out[c * SH_COEFFS_PER_CHANNEL] = dc[c];
        for k in 0..m {
            out[c * SH_COEFFS_PER_CHANNEL + 1 + k] = rest.get(c * m + k).copied().unwrap_or(0.0);
        }
    }
    out
}

/// Infer the SH degree from the number of `f_rest_*` properties in a `.ply`.
/// Returns `None` when the count matches no supported degree.
pub fn degree_from_rest_count(count: usize) -> Option<u8> {
    (1..=SH_MAX_DEGREE).find(|&d| 3 * rest_coeffs_per_channel(d) == count)
}

/// Quality knob for the splat renderer.
///
/// `sh_degree` caps the SH bands evaluated at runtime. `0` reproduces the
/// original behavior (flat DC color from [`crate::Splat::color`]); higher values
/// use view-dependent color when the cloud carries that data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplatQuality {
    pub sh_degree: u8,
}

impl Default for SplatQuality {
    fn default() -> Self {
        SplatQuality {
            sh_degree: SH_MAX_DEGREE,
        }
    }
}

impl SplatQuality {
    /// The degree actually evaluated for a cloud that stores `cloud_degree`.
    pub fn effective_degree(&self, cloud_degree: u8) -> u8 {
        self.sh_degree.min(cloud_degree).min(SH_MAX_DEGREE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_counts_match_3dgs_layout() {
        assert_eq!(rest_coeffs_per_channel(1), 3);
        assert_eq!(rest_coeffs_per_channel(2), 8);
        assert_eq!(rest_coeffs_per_channel(3), 15);
        assert_eq!(degree_from_rest_count(45), Some(3));
        assert_eq!(degree_from_rest_count(24), Some(2));
        assert_eq!(degree_from_rest_count(9), Some(1));
        assert_eq!(degree_from_rest_count(7), None);
    }

    #[test]
    fn degree_zero_is_dc_only() {
        let mut sh = [0.0f32; SH_FLOATS];
        sh[0] = 1.0 / SH_C0; // DC chosen so SH_C0 * dc = 1.0 → color = 1.5 (before clamp)
        let rgb = eval_sh_rgb(0, [0.0, 0.0, 1.0], &sh);
        assert!((rgb[0] - 1.5).abs() < 1e-5);
        // A zero DC is mid-gray regardless of view.
        let gray = eval_sh_rgb(3, [0.3, 0.4, 0.866], &[0.0; SH_FLOATS]);
        assert!((gray[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn higher_bands_make_color_view_dependent() {
        // Only the degree-1 x-band is non-zero: color should flip sign across x.
        let mut sh = [0.0f32; SH_FLOATS];
        sh[3] = 2.0; // coefficient k=3 (-C1 * x) for red
        let pos = eval_sh_rgb(1, [1.0, 0.0, 0.0], &sh)[0];
        let neg = eval_sh_rgb(1, [-1.0, 0.0, 0.0], &sh)[0];
        assert!(pos < 0.5 && neg > 0.5, "pos={pos} neg={neg}");
        // Lower cap ignores the band entirely.
        let capped = eval_sh_rgb(0, [1.0, 0.0, 0.0], &sh)[0];
        assert!((capped - 0.5).abs() < 1e-6);
    }

    #[test]
    fn pack_sh_places_channels_contiguously() {
        let rest: Vec<f32> = (0..45).map(|i| i as f32).collect();
        let packed = pack_sh([10.0, 20.0, 30.0], &rest, 3);
        assert_eq!(packed[0], 10.0);
        assert_eq!(packed[16], 20.0);
        assert_eq!(packed[32], 30.0);
        // First non-DC red coefficient is rest[0].
        assert_eq!(packed[1], 0.0);
        // First green coefficient is rest[15] (channel-major layout).
        assert_eq!(packed[17], 15.0);
    }

    #[test]
    fn quality_caps_effective_degree() {
        let q = SplatQuality { sh_degree: 2 };
        assert_eq!(q.effective_degree(3), 2);
        assert_eq!(q.effective_degree(1), 1);
        assert_eq!(SplatQuality { sh_degree: 0 }.effective_degree(3), 0);
    }
}
