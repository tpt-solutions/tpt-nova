//! Incremental 3D convex hull (quickhull-style with conflict lists).
//!
//! Replaces `parry3d::transformation::try_convex_hull` so the workspace stays
//! free of Apache-2.0-only dependencies. Arithmetic runs in `f64` for
//! robustness; the output is an outward-facing triangle mesh over the subset of
//! input points that form hull vertices.

use std::collections::HashSet;

use crate::decompose::ConvexPart;

/// A triangle of the working hull, with its outward unit normal and offset
/// such that `normal · p + offset` is the signed distance to its plane.
struct Face {
    v: [usize; 3],
    normal: [f64; 3],
    offset: f64,
    /// Indices of input points that lie outside this face (conflict list).
    outside: Vec<usize>,
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn to_f64(p: [f32; 3]) -> V3 {
    [p[0] as f64, p[1] as f64, p[2] as f64]
}

/// Build a face over `(a, b, c)`, flipping it so its normal points away from
/// `inside` (a point strictly inside the hull).
fn make_face(pts: &[V3], a: usize, b: usize, c: usize, inside: V3) -> Option<Face> {
    let n = cross(sub(pts[b], pts[a]), sub(pts[c], pts[a]));
    let len = length(n);
    if len <= f64::MIN_POSITIVE {
        return None;
    }
    let mut normal = [n[0] / len, n[1] / len, n[2] / len];
    let mut v = [a, b, c];
    let mut offset = -dot(normal, pts[a]);
    if dot(normal, inside) + offset > 0.0 {
        normal = [-normal[0], -normal[1], -normal[2]];
        offset = -offset;
        v = [a, c, b];
    }
    Some(Face {
        v,
        normal,
        offset,
        outside: Vec::new(),
    })
}

fn distance(face: &Face, p: V3) -> f64 {
    dot(face.normal, p) + face.offset
}

/// Index of the point maximising `score`, ignoring non-finite scores.
fn argmax(n: usize, score: impl Fn(usize) -> f64) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for i in 0..n {
        let s = score(i);
        if s.is_finite() && best.is_none_or(|(_, bs)| s > bs) {
            best = Some((i, s));
        }
    }
    best.map(|(i, _)| i)
}

/// Compute the convex hull of `points`.
///
/// Returns `None` when fewer than four non-coplanar points are supplied. The
/// returned part's `vertices` are only the points that are hull corners, and
/// `indices` are outward-facing triangles into them.
pub fn convex_hull(points: &[[f32; 3]]) -> Option<ConvexPart> {
    let pts: Vec<V3> = points.iter().copied().map(to_f64).collect();
    if pts.len() < 4 {
        return None;
    }

    // Tolerance relative to the bounding box diagonal.
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for p in &pts {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let diag = length(sub(max, min));
    if !diag.is_finite() || diag <= 0.0 {
        return None;
    }
    let eps = diag * 1e-9;

    // --- Initial tetrahedron -------------------------------------------------
    let i0 = argmax(pts.len(), |i| -pts[i][0])?;
    let i1 = argmax(pts.len(), |i| length(sub(pts[i], pts[i0])))?;
    let d01 = sub(pts[i1], pts[i0]);
    if length(d01) <= eps {
        return None;
    }
    let i2 = argmax(pts.len(), |i| length(cross(d01, sub(pts[i], pts[i0]))))?;
    let n012 = cross(d01, sub(pts[i2], pts[i0]));
    if length(n012) <= eps * diag {
        return None;
    }
    let i3 = argmax(pts.len(), |i| dot(n012, sub(pts[i], pts[i0])).abs())?;
    if dot(n012, sub(pts[i3], pts[i0])).abs() <= eps * diag {
        return None;
    }
    let simplex = [i0, i1, i2, i3];

    // The simplex centroid is strictly interior to every hull that follows, so
    // it is the reference point for orienting all faces.
    let mut interior = [0.0; 3];
    for &i in &simplex {
        for k in 0..3 {
            interior[k] += pts[i][k] * 0.25;
        }
    }

    let mut faces: Vec<Face> = Vec::with_capacity(16);
    for &(a, b, c) in &[(i0, i1, i2), (i0, i1, i3), (i0, i2, i3), (i1, i2, i3)] {
        faces.push(make_face(&pts, a, b, c, interior)?);
    }
    for i in 0..pts.len() {
        if !simplex.contains(&i) {
            assign_to_face(&mut faces, &pts, i, eps);
        }
    }

    // --- Expand the hull ----------------------------------------------------
    while let Some(fi) = faces.iter().position(|f| !f.outside.is_empty()) {
        // The farthest outside point of the chosen face becomes the eye point.
        let eye = {
            let face = &faces[fi];
            let mut best = face.outside[0];
            let mut best_d = distance(face, pts[best]);
            for &p in &face.outside[1..] {
                let d = distance(face, pts[p]);
                if d > best_d {
                    best = p;
                    best_d = d;
                }
            }
            best
        };
        let eye_p = pts[eye];

        let visible: Vec<bool> = faces.iter().map(|f| distance(f, eye_p) > eps).collect();

        // Horizon edges: directed edges of visible faces whose reverse edge
        // belongs to a non-visible face.
        let mut vis_edges: HashSet<(usize, usize)> = HashSet::new();
        for (f, &vis) in faces.iter().zip(&visible) {
            if vis {
                for k in 0..3 {
                    vis_edges.insert((f.v[k], f.v[(k + 1) % 3]));
                }
            }
        }
        let mut horizon: Vec<(usize, usize)> = vis_edges
            .iter()
            .copied()
            .filter(|&(a, b)| !vis_edges.contains(&(b, a)))
            .collect();
        horizon.sort_unstable();

        // Remove visible faces and collect their outside points for reassignment.
        let mut orphans: Vec<usize> = Vec::new();
        let mut kept: Vec<Face> = Vec::with_capacity(faces.len());
        for (f, vis) in faces.into_iter().zip(visible) {
            if vis {
                orphans.extend(f.outside.into_iter().filter(|&p| p != eye));
            } else {
                kept.push(f);
            }
        }
        faces = kept;

        for (a, b) in horizon {
            if let Some(face) = make_face(&pts, a, b, eye, interior) {
                faces.push(face);
            }
        }
        for p in orphans {
            assign_to_face(&mut faces, &pts, p, eps);
        }
    }

    // --- Emit compacted output ----------------------------------------------
    if faces.len() < 4 {
        return None;
    }
    let mut remap: Vec<Option<u32>> = vec![None; pts.len()];
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::with_capacity(faces.len());
    for f in &faces {
        let mut tri = [0u32; 3];
        for (k, &vi) in f.v.iter().enumerate() {
            tri[k] = *remap[vi].get_or_insert_with(|| {
                vertices.push(points[vi]);
                (vertices.len() - 1) as u32
            });
        }
        indices.push(tri);
    }
    Some(ConvexPart { vertices, indices })
}

/// Attach point `p` to the face it is farthest outside of, if any.
fn assign_to_face(faces: &mut [Face], pts: &[V3], p: usize, eps: f64) {
    let mut best: Option<(usize, f64)> = None;
    for (fi, face) in faces.iter().enumerate() {
        let d = distance(face, pts[p]);
        if d > eps && best.is_none_or(|(_, bd)| d > bd) {
            best = Some((fi, d));
        }
    }
    if let Some((fi, _)) = best {
        faces[fi].outside.push(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part_volume(part: &ConvexPart) -> f64 {
        part.indices
            .iter()
            .map(|t| {
                let a = to_f64(part.vertices[t[0] as usize]);
                let b = to_f64(part.vertices[t[1] as usize]);
                let c = to_f64(part.vertices[t[2] as usize]);
                dot(a, cross(b, c)) / 6.0
            })
            .sum()
    }

    /// Signed distance from `p` to the plane of triangle `t` (positive = outside).
    fn plane_distance(part: &ConvexPart, t: [u32; 3], p: V3) -> f64 {
        let a = to_f64(part.vertices[t[0] as usize]);
        let b = to_f64(part.vertices[t[1] as usize]);
        let c = to_f64(part.vertices[t[2] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        dot(n, sub(p, a)) / length(n)
    }

    #[test]
    fn tetrahedron_hull_has_four_faces() {
        let pts = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let hull = convex_hull(&pts).unwrap();
        assert_eq!(hull.vertices.len(), 4);
        assert_eq!(hull.indices.len(), 4);
        assert!((part_volume(&hull) - 1.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn cube_with_interior_points_keeps_eight_corners() {
        let mut pts = Vec::new();
        for &x in &[-1.0f32, 1.0] {
            for &y in &[-1.0f32, 1.0] {
                for &z in &[-1.0f32, 1.0] {
                    pts.push([x, y, z]);
                }
            }
        }
        pts.push([0.0, 0.0, 0.0]);
        pts.push([0.5, -0.3, 0.2]);
        let hull = convex_hull(&pts).unwrap();
        assert_eq!(hull.vertices.len(), 8);
        assert_eq!(hull.indices.len(), 12);
        assert!((part_volume(&hull) - 8.0).abs() < 1e-4);
        // Every face is outward: the interior centre is on the negative side.
        for &t in &hull.indices {
            assert!(plane_distance(&hull, t, [0.0, 0.0, 0.0]) < 0.0);
        }
    }

    #[test]
    fn coplanar_points_are_rejected() {
        let pts = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ];
        assert!(convex_hull(&pts).is_none());
    }

    #[test]
    fn too_few_points_are_rejected() {
        assert!(convex_hull(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]).is_none());
    }

    #[test]
    fn every_input_point_is_inside_the_hull() {
        let pts: Vec<[f32; 3]> = (0..200)
            .map(|i| {
                let t = i as f32 * 0.37;
                let p = i as f32 * 0.11;
                [t.sin() * (1.0 + 0.3 * p.cos()), t.cos(), p.sin()]
            })
            .collect();
        let hull = convex_hull(&pts).unwrap();
        for p in &pts {
            for &t in &hull.indices {
                assert!(
                    plane_distance(&hull, t, to_f64(*p)) <= 1e-5,
                    "point outside hull"
                );
            }
        }
    }
}
