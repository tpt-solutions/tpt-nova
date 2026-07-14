//! Video-to-ECS ingestion: depth + segmentation → collision proxies.
//!
//! A single RGB frame alone is not enough to place a captured video into a
//! physics world, but a **depth map** (per-pixel distance) plus a
//! **segmentation mask** (per-pixel object class/instance) is: each labeled
//! region can be unprojected into a cloud of 3D points, and that cloud becomes
//! a low-poly **collision proxy** — a convex hull (or, as a fallback, an AABB
//! box) wrapped in a `nova_ingest::Collider3D`. The result drops straight into
//! the Rapier3D step (`nova_ingest::step_physics_3d`), so a videoed scene gains
//! solid collision without hand-authoring every collider.
//!
//! The pure geometry here is fully testable offline: feed synthetic depth/mask
//! arrays and assert the generated proxies and colliders.

use nova_ecs::transform::Transform;
use nova_ecs::{Entity, Mat4, Vec3, World};
use nova_ingest::decompose::ConvexPart;
use nova_ingest::Collider3D;
use parry3d::math::Vector;
use parry3d::transformation::try_convex_hull;
use serde::{Deserialize, Serialize};

/// Pinhole camera intrinsics (pixels).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraIntrinsics {
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
}

impl CameraIntrinsics {
    /// Common case: square pixels and the principal point at the image center.
    pub fn centered(width: u32, height: u32, fx: f32, fy: f32) -> Self {
        CameraIntrinsics {
            fx,
            fy,
            cx: width as f32 / 2.0,
            cy: height as f32 / 2.0,
        }
    }
}

/// A single-channel depth map. `data[i]` is the depth (meters, positive) at
/// pixel row `i / width`, column `i % width`.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl DepthMap {
    pub fn new(width: u32, height: u32, data: Vec<f32>) -> Self {
        DepthMap {
            width,
            height,
            data,
        }
    }

    /// Depth at pixel (x, y); `0.0` (treated as "no depth") when out of range.
    pub fn at(&self, x: u32, y: u32) -> f32 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        self.data[(y * self.width + x) as usize]
    }

    pub fn len(&self) -> usize {
        (self.width * self.height) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// A per-pixel object label map, same resolution as the depth map. `0` is
/// conventionally "background" / "no object".
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentationMask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u32>,
}

impl SegmentationMask {
    pub fn new(width: u32, height: u32, data: Vec<u32>) -> Self {
        SegmentationMask {
            width,
            height,
            data,
        }
    }

    pub fn at(&self, x: u32, y: u32) -> u32 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.data[(y * self.width + x) as usize]
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Errors raised while generating collision proxies.
#[derive(Debug, thiserror::Error)]
pub enum VideoCapError {
    #[error("depth map and segmentation mask resolution mismatch")]
    ResolutionMismatch,
    #[error("no segments found (mask is empty or all background)")]
    NoSegments,
}

/// An axis-aligned bounding box of a generated proxy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProxyAabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ProxyAabb {
    pub fn center(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }
    pub fn size(&self) -> [f32; 3] {
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }
}

/// A collision proxy for one segmented object: its class, the unprojected 3D
/// points, their bounds, and a ready-to-use `Collider3D`.
#[derive(Debug, Clone)]
pub struct CollisionProxy {
    pub class_id: u32,
    pub point_count: usize,
    pub aabb: ProxyAabb,
    pub collider: Collider3D,
}

/// Unproject a pixel (u, v) at `depth` meters into world space.
///
/// Camera space follows the usual computer-vision convention (camera looks down
/// -Z). `pose` maps camera space to world space (identity => world == camera).
pub fn unproject(u: f32, v: f32, depth: f32, intr: &CameraIntrinsics, pose: Mat4) -> Vec3 {
    let x = (u - intr.cx) * depth / intr.fx;
    let y = (v - intr.cy) * depth / intr.fy;
    let z = -depth; // sensor looks down -Z
    let p = pose * glam::Vec4::new(x, y, z, 1.0);
    Vec3::new(p.x, p.y, p.z)
}

/// Build a `Collider3D` from a point set: a convex hull when the cloud is
/// 3D, otherwise an AABB box (eight corners) so every proxy still collides.
fn collider_from_points(points: &[[f32; 3]]) -> Collider3D {
    if points.len() >= 4 {
        let vecs: Vec<Vector> = points
            .iter()
            .map(|p| Vector::new(p[0], p[1], p[2]))
            .collect();
        if let Ok((hull, indices)) = try_convex_hull(&vecs) {
            if hull.len() >= 4 && !indices.is_empty() {
                let vertices: Vec<[f32; 3]> = hull.iter().map(|p| [p.x, p.y, p.z]).collect();
                return Collider3D::from_parts(vec![ConvexPart { vertices, indices }]);
            }
        }
    }
    // Fallback: a box from the point bounds.
    box_collider(points)
}

/// An axis-aligned box collider built from a point cloud's bounds.
fn box_collider(points: &[[f32; 3]]) -> Collider3D {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in points {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    if min[0].is_infinite() {
        min = [0.0; 3];
        max = [0.0; 3];
    }
    let (x0, y0, z0) = (min[0], min[1], min[2]);
    let (x1, y1, z1) = (max[0], max[1], max[2]);
    let verts = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    let idx: [[u32; 3]; 12] = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [1, 5, 6],
        [1, 6, 2],
        [2, 6, 7],
        [2, 7, 3],
        [3, 7, 4],
        [3, 4, 0],
    ];
    Collider3D::from_parts(vec![ConvexPart {
        vertices: verts.to_vec(),
        indices: idx.to_vec(),
    }])
}

/// Generate one [`CollisionProxy`] per non-background segment.
///
/// `background` is the mask value treated as "no object" (default `0`). Each
/// segment's pixels are unprojected (skipping zero-depth) and turned into a
/// collider. Returns an error if the maps disagree on size or no segment is
/// found.
pub fn generate_proxies(
    depth: &DepthMap,
    mask: &SegmentationMask,
    intr: &CameraIntrinsics,
    pose: Mat4,
    background: u32,
) -> Result<Vec<CollisionProxy>, VideoCapError> {
    if depth.width != mask.width || depth.height != mask.height {
        return Err(VideoCapError::ResolutionMismatch);
    }

    // Bucket pixel indices by class.
    let mut buckets: std::collections::BTreeMap<u32, Vec<(u32, u32)>> =
        std::collections::BTreeMap::new();
    for y in 0..depth.height {
        for x in 0..depth.width {
            let c = mask.at(x, y);
            if c == background {
                continue;
            }
            buckets.entry(c).or_default().push((x, y));
        }
    }
    if buckets.is_empty() {
        return Err(VideoCapError::NoSegments);
    }

    let mut proxies = Vec::new();
    for (class_id, pixels) in buckets {
        let mut points: Vec<[f32; 3]> = Vec::with_capacity(pixels.len());
        for &(x, y) in &pixels {
            let d = depth.at(x, y);
            if d <= 0.0 {
                continue;
            }
            let p = unproject(x as f32, y as f32, d, intr, pose);
            points.push([p.x, p.y, p.z]);
        }
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in &points {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        if min[0].is_infinite() {
            min = [0.0; 3];
            max = [0.0; 3];
        }
        let aabb = ProxyAabb { min, max };
        let collider = collider_from_points(&points);
        proxies.push(CollisionProxy {
            class_id,
            point_count: points.len(),
            aabb,
            collider,
        });
    }
    Ok(proxies)
}

/// Convenience: build an ECS [`World`] with one entity per collision proxy
/// (transform at the proxy center, the proxy's collider attached). Ready to hand
/// to `nova_ingest::step_physics_3d`.
pub fn build_segments_world(proxies: &[CollisionProxy]) -> World {
    let mut world = World::new();
    for proxy in proxies {
        let e: Entity = world.spawn();
        let center = proxy.aabb.center();
        world.add_component(
            e,
            Transform::from_translation(Vec3::new(center[0], center[1], center[2])),
        );
        world.add_component(e, proxy.collider.clone());
    }
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_pixel_unprojects_to_centered_depth() {
        let intr = CameraIntrinsics::centered(640, 480, 500.0, 500.0);
        // Principal point -> (0,0) in camera space, depth 2 => (0,0,-2).
        let p = unproject(intr.cx, intr.cy, 2.0, &intr, Mat4::IDENTITY);
        assert!((p.x).abs() < 1e-4);
        assert!((p.y).abs() < 1e-4);
        assert!((p.z + 2.0).abs() < 1e-4);
    }

    #[test]
    fn offcenter_pixel_unprojects_to_side() {
        let intr = CameraIntrinsics::centered(640, 480, 500.0, 500.0);
        // Pixel 140px left of center at depth 5 -> x = -140*5/500 = -1.4.
        let p = unproject(intr.cx - 140.0, intr.cy, 5.0, &intr, Mat4::IDENTITY);
        assert!((p.x + 1.4).abs() < 1e-3);
        assert!((p.z + 5.0).abs() < 1e-3);
    }

    #[test]
    fn generates_one_proxy_per_segment() {
        // 4x1 image: left two px class 1 (near), right two px class 2 (far).
        let w = 4;
        let h = 1;
        let depth = DepthMap::new(w, h, vec![1.0, 1.0, 5.0, 5.0]);
        let mask = SegmentationMask::new(w, h, vec![1, 1, 2, 2]);
        let intr = CameraIntrinsics::centered(w, h, 100.0, 100.0);
        let proxies = generate_proxies(&depth, &mask, &intr, Mat4::IDENTITY, 0).unwrap();
        assert_eq!(proxies.len(), 2);
        let classes: Vec<u32> = proxies.iter().map(|p| p.class_id).collect();
        assert!(classes.contains(&1));
        assert!(classes.contains(&2));
        for p in &proxies {
            assert!(p.point_count >= 2);
            assert!(!p.collider.parts.is_empty());
        }
    }

    #[test]
    fn background_is_excluded() {
        let w = 2;
        let h = 1;
        let depth = DepthMap::new(w, h, vec![1.0, 1.0]);
        let mask = SegmentationMask::new(w, h, vec![0, 0]); // all background
        let intr = CameraIntrinsics::centered(w, h, 100.0, 100.0);
        assert!(matches!(
            generate_proxies(&depth, &mask, &intr, Mat4::IDENTITY, 0),
            Err(VideoCapError::NoSegments)
        ));
    }

    #[test]
    fn resolution_mismatch_errors() {
        let d = DepthMap::new(2, 1, vec![1.0, 1.0]);
        let m = SegmentationMask::new(3, 1, vec![1, 1, 1]);
        let intr = CameraIntrinsics::centered(2, 1, 100.0, 100.0);
        assert!(matches!(
            generate_proxies(&d, &m, &intr, Mat4::IDENTITY, 0),
            Err(VideoCapError::ResolutionMismatch)
        ));
    }

    #[test]
    fn few_points_fall_back_to_box_collider() {
        // A single-pixel segment can't form a 3D hull, so it falls back to a
        // box collider (8 corners, 12 triangles) built from its bounds.
        let depth = DepthMap::new(1, 1, vec![3.0]);
        let mask = SegmentationMask::new(1, 1, vec![1]);
        let intr = CameraIntrinsics::centered(1, 1, 100.0, 100.0);
        let proxies = generate_proxies(&depth, &mask, &intr, Mat4::IDENTITY, 0).unwrap();
        assert_eq!(proxies.len(), 1);
        assert_eq!(proxies[0].collider.parts[0].vertices.len(), 8);
        assert_eq!(proxies[0].collider.parts[0].indices.len(), 12);
    }

    #[test]
    fn three_d_segment_builds_convex_hull() {
        // A 2x2 patch with varying depth spans 3D space, yielding a real convex
        // hull (more than the 4 corners of a flat quad).
        let w = 2;
        let h = 2;
        let depth = DepthMap::new(w, h, vec![1.0, 2.0, 3.0, 4.0]);
        let mask = SegmentationMask::new(w, h, vec![1, 1, 1, 1]);
        let intr = CameraIntrinsics::centered(w, h, 100.0, 100.0);
        let proxies = generate_proxies(&depth, &mask, &intr, Mat4::IDENTITY, 0).unwrap();
        assert_eq!(proxies.len(), 1);
        let part = &proxies[0].collider.parts[0];
        assert!(part.vertices.len() >= 4, "hull should have >= 4 verts");
        assert!(!part.indices.is_empty());
    }

    #[test]
    fn build_segments_world_attaches_colliders() {
        let w = 2;
        let h = 1;
        let depth = DepthMap::new(w, h, vec![1.0, 2.0]);
        let mask = SegmentationMask::new(w, h, vec![1, 2]);
        let intr = CameraIntrinsics::centered(w, h, 100.0, 100.0);
        let proxies = generate_proxies(&depth, &mask, &intr, Mat4::IDENTITY, 0).unwrap();
        let world = build_segments_world(&proxies);
        assert_eq!(world.entity_count(), 2);
        assert_eq!(world.query_1::<Collider3D>().len(), 2);
    }
}
