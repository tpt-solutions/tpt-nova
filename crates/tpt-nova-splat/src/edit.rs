//! In-place editing of splat clouds: selection, delete, crop and transform.
//!
//! Selections are plain sorted index lists ([`SplatSelection`]) so they can be
//! serialized into agent commands, telemetry, and "Highlight & Fix" prompts.
//! Screen-space selection reuses `tpt-nova-overlay`'s projection and rectangle
//! types so the viewport rectangle picks splats exactly as it picks entities.

use glam::{Mat3, Mat4, Vec3};
use serde::{Deserialize, Serialize};
use tpt_nova_overlay::{project_to_screen, ScreenRect};

use crate::{Aabb, SplatCloud};

/// A set of splat indices (sorted, unique) into a [`SplatCloud`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SplatSelection {
    pub indices: Vec<u32>,
}

impl SplatSelection {
    /// Build from arbitrary indices; sorts and de-duplicates.
    pub fn from_indices(mut indices: Vec<u32>) -> Self {
        indices.sort_unstable();
        indices.dedup();
        SplatSelection { indices }
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Inclusive `(first, last)` runs of consecutive indices.
    pub fn ranges(&self) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for &i in &self.indices {
            match out.last_mut() {
                Some(last) if last.1 + 1 == i => last.1 = i,
                _ => out.push((i, i)),
            }
        }
        out
    }

    /// Compact text form, e.g. `"120 splats in 2 ranges: 0-119, 240"`.
    pub fn describe(&self) -> String {
        let ranges = self.ranges();
        let parts: Vec<String> = ranges
            .iter()
            .map(|&(a, b)| {
                if a == b {
                    format!("{a}")
                } else {
                    format!("{a}-{b}")
                }
            })
            .collect();
        format!(
            "{} splats in {} ranges: {}",
            self.len(),
            ranges.len(),
            parts.join(", ")
        )
    }

    /// One prompt line for an AI fix request, naming the entity and its splat subset.
    pub fn prompt_entry(&self, entity_label: &str) -> String {
        format!(
            "splat entity '{entity_label}' selection: {}",
            self.describe()
        )
    }
}

impl SplatCloud {
    /// Splats whose center lies inside `aabb`.
    pub fn select_in_aabb(&self, aabb: &Aabb) -> SplatSelection {
        let indices = self
            .splats
            .iter()
            .enumerate()
            .filter(|(_, s)| aabb.contains(s.position))
            .map(|(i, _)| i as u32)
            .collect();
        SplatSelection { indices }
    }

    /// Splats whose center projects inside the viewport rectangle `rect`.
    ///
    /// `model` places the cloud in the world; `view_proj` is the camera matrix
    /// (the same one `tpt-nova-render` uses); `viewport` is the size in pixels.
    /// Splats behind the camera are never selected.
    pub fn select_in_screen_rect(
        &self,
        model: Mat4,
        view_proj: Mat4,
        viewport: (u32, u32),
        rect: ScreenRect,
    ) -> SplatSelection {
        let mvp = view_proj * model;
        let indices = self
            .splats
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let (x, y) = project_to_screen(Vec3::from(s.position), mvp, viewport)?;
                rect.contains(x, y).then_some(i as u32)
            })
            .collect();
        SplatSelection { indices }
    }

    /// Remove every splat in `sel`.
    pub fn delete(&mut self, sel: &SplatSelection) {
        let mut mask = vec![true; self.len()];
        for &i in &sel.indices {
            if let Some(m) = mask.get_mut(i as usize) {
                *m = false;
            }
        }
        self.retain_indices(|i| mask[i]);
    }

    /// Keep only the splats inside `aabb`; everything else is removed.
    pub fn crop(&mut self, aabb: &Aabb) {
        let mask: Vec<bool> = self
            .splats
            .iter()
            .map(|s| aabb.contains(s.position))
            .collect();
        self.retain_indices(|i| mask[i]);
    }

    /// Apply an affine transform to every splat.
    ///
    /// Centers move by `m`; each covariance becomes `A Σ Aᵀ` (with `A` the
    /// linear part of `m`), re-expressed as rotation + per-axis scale, so
    /// non-uniform scaling is exact. **Limitation:** SH color coefficients are
    /// not rotated, so view-dependent color is approximate after a rotation.
    pub fn transform(&mut self, m: Mat4) {
        let a = Mat3::from_mat4(m);
        for s in &mut self.splats {
            let c = m.transform_point3(Vec3::from(s.position));
            s.position = c.to_array();
            let cov = a * s.covariance3d() * a.transpose();
            s.set_covariance(cov);
        }
    }

    /// Summary statistics for telemetry and agents.
    pub fn summary(&self) -> SplatSummary {
        let bounds = self.bounds();
        let mut histogram = [0u32; 10];
        for s in &self.splats {
            let bin = ((s.opacity.clamp(0.0, 1.0) * 10.0) as usize).min(9);
            histogram[bin] += 1;
        }
        SplatSummary {
            count: self.len(),
            min: bounds.min,
            max: bounds.max,
            sh_degree: if self.has_sh() { self.sh_degree } else { 0 },
            opacity_histogram: histogram,
        }
    }
}

/// Compact description of a splat cloud for telemetry and agent context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplatSummary {
    pub count: usize,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub sh_degree: u8,
    /// Ten equal-width opacity bins over `[0, 1]`.
    pub opacity_histogram: [u32; 10],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Splat;

    fn splat(p: [f32; 3], scale: [f32; 3], opacity: f32) -> Splat {
        Splat {
            position: p,
            scale,
            rotation: [1.0, 0.0, 0.0, 0.0],
            color: [1.0; 4],
            opacity,
        }
    }

    fn line() -> SplatCloud {
        SplatCloud::new(
            (0..10)
                .map(|i| splat([i as f32, 0.0, 0.0], [0.1; 3], 0.5))
                .collect(),
        )
    }

    #[test]
    fn selection_ranges_compact_consecutive_runs() {
        let sel = SplatSelection::from_indices(vec![5, 1, 2, 3, 3, 9]);
        assert_eq!(sel.indices, vec![1, 2, 3, 5, 9]);
        assert_eq!(sel.ranges(), vec![(1, 3), (5, 5), (9, 9)]);
        assert_eq!(sel.describe(), "5 splats in 3 ranges: 1-3, 5, 9");
        assert!(sel.prompt_entry("crate").contains("splat entity 'crate'"));
    }

    #[test]
    fn aabb_selection_and_crop() {
        let mut c = line();
        let aabb = Aabb {
            min: [2.5, -1.0, -1.0],
            max: [5.5, 1.0, 1.0],
        };
        let sel = c.select_in_aabb(&aabb);
        assert_eq!(sel.indices, vec![3, 4, 5]);
        c.crop(&aabb);
        assert_eq!(c.len(), 3);
        assert_eq!(c.splats[0].position[0], 3.0);
    }

    #[test]
    fn delete_removes_selected_only() {
        let mut c = line();
        c.delete(&SplatSelection::from_indices(vec![0, 9, 4]));
        assert_eq!(c.len(), 7);
        assert!(c.splats.iter().all(|s| s.position[0] != 4.0));
    }

    #[test]
    fn screen_rect_selects_visible_splats() {
        let view = Mat4::look_at_rh(
            glam::Vec3::new(4.5, 0.0, 10.0),
            glam::Vec3::new(4.5, 0.0, 0.0),
            glam::Vec3::Y,
        );
        let proj = Mat4::perspective_rh(60f32.to_radians(), 1.0, 0.1, 100.0);
        let vp = proj * view;
        let c = line();
        // Whole viewport selects every splat in front of the camera.
        let full = ScreenRect::new((0, 0), (100, 100));
        let all = c.select_in_screen_rect(Mat4::IDENTITY, vp, (100, 100), full);
        assert_eq!(all.len(), 10);
        // A tiny corner selects none.
        let corner = ScreenRect::new((0, 0), (2, 2));
        let none = c.select_in_screen_rect(Mat4::IDENTITY, vp, (100, 100), corner);
        assert!(none.is_empty());
    }

    #[test]
    fn transform_translates_and_scales_covariance_exactly() {
        let mut c = SplatCloud::new(vec![splat([1.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0)]);
        c.transform(
            Mat4::from_translation(glam::Vec3::new(0.0, 5.0, 0.0))
                * Mat4::from_scale(glam::Vec3::new(2.0, 1.0, 1.0)),
        );
        let s = c.splats[0];
        assert!((s.position[0] - 2.0).abs() < 1e-5);
        assert!((s.position[1] - 5.0).abs() < 1e-5);
        // Scaling x by 2 doubles the x std-dev and leaves y/z alone.
        let mut sorted = s.scale;
        sorted.sort_by(|a, b| b.total_cmp(a));
        assert!((sorted[0] - 2.0).abs() < 1e-4, "{:?}", s.scale);
        assert!((sorted[1] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn summary_counts_and_histogram() {
        let c = SplatCloud::new(vec![
            splat([0.0; 3], [0.1; 3], 0.05),
            splat([1.0; 3], [0.1; 3], 0.95),
            splat([2.0; 3], [0.1; 3], 1.0),
        ]);
        let s = c.summary();
        assert_eq!(s.count, 3);
        assert_eq!(s.opacity_histogram[0], 1);
        assert_eq!(s.opacity_histogram[9], 2);
        assert_eq!(s.sh_degree, 0);
    }
}
