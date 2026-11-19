//! In-house 3D rigid body solver for convex-hull compounds.
//!
//! This replaces Rapier3D so the workspace stays free of Apache-2.0-only
//! dependencies. Scope:
//!
//! * Shapes are compounds of convex polyhedra (the [`ConvexPart`]s produced by
//!   decomposition). Each pair of parts is tested with the separating-axis
//!   theorem over face normals and edge cross products.
//! * A face contact yields up to eight points; an edge contact yields one.
//! * Contacts are resolved by a fixed-iteration sequential-impulse solver with
//!   Coulomb friction on two tangents and Baumgarte position correction.
//! * Iteration order is fixed, so identical inputs give identical results.
//!
//! It is not a full-featured engine: there is no sleeping, no warm starting, and
//! no continuous collision detection. Fast-moving small bodies can tunnel.

use std::collections::BTreeSet;

use glam::{Mat3, Quat, Vec3};

use crate::decompose::ConvexPart;

/// Sequential-impulse iterations per step.
pub const SOLVER_ITERATIONS: usize = 10;
/// Fraction of residual penetration corrected per step.
const BAUMGARTE: f32 = 0.2;
/// Penetration tolerated before position correction kicks in.
const SLOP: f32 = 0.005;
/// Upper bound on the speed at which penetration is pushed out, so a deep
/// overlap (for example a body spawned inside another) is resolved smoothly
/// rather than launched.
const MAX_CORRECTION_SPEED: f32 = 4.0;
/// Approach speed below which restitution is ignored.
const RESTITUTION_THRESHOLD: f32 = 1.0;
/// Maximum incident-face points kept per face contact.
const MAX_FACE_CONTACTS: usize = 8;

/// One convex part in its body's local frame, with outward-facing triangles.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalHull {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[usize; 3]>,
    /// Unique undirected edges as vertex-index pairs.
    pub edges: Vec<[usize; 2]>,
    pub volume: f32,
}

impl LocalHull {
    /// Build from a convex part. Triangle winding is corrected to point outward
    /// (by signed volume), so callers need not guarantee an orientation.
    /// Returns `None` for parts with no enclosed volume.
    pub fn from_part(part: &ConvexPart) -> Option<Self> {
        let verts: Vec<Vec3> = part.vertices.iter().map(|v| Vec3::from_array(*v)).collect();
        let n = verts.len();
        let mut tris: Vec<[usize; 3]> = part
            .indices
            .iter()
            .filter(|t| t.iter().all(|&i| (i as usize) < n))
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        if n < 4 || tris.is_empty() {
            return None;
        }
        let six_volume: f32 = tris
            .iter()
            .map(|t| verts[t[0]].dot(verts[t[1]].cross(verts[t[2]])))
            .sum();
        if six_volume.abs() <= f32::EPSILON {
            return None;
        }
        if six_volume < 0.0 {
            for t in &mut tris {
                t.swap(1, 2);
            }
        }
        let mut edge_set = BTreeSet::new();
        for t in &tris {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edge_set.insert((a.min(b), a.max(b)));
            }
        }
        Some(LocalHull {
            verts,
            tris,
            edges: edge_set.into_iter().map(|(a, b)| [a, b]).collect(),
            volume: six_volume.abs() / 6.0,
        })
    }
}

/// A [`LocalHull`] transformed into world space for one step.
#[derive(Debug, Clone)]
pub struct WorldHull {
    pub verts: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub edge_dirs: Vec<Vec3>,
    pub min: Vec3,
    pub max: Vec3,
}

impl WorldHull {
    fn new(local: &LocalHull, pos: Vec3, rot: Quat) -> Self {
        let verts: Vec<Vec3> = local.verts.iter().map(|&v| pos + rot * v).collect();
        let normals: Vec<Vec3> = local
            .tris
            .iter()
            .map(|t| {
                let (a, b, c) = (verts[t[0]], verts[t[1]], verts[t[2]]);
                (b - a).cross(c - a).normalize_or_zero()
            })
            .filter(|n| *n != Vec3::ZERO)
            .collect();
        let edge_dirs: Vec<Vec3> = local
            .edges
            .iter()
            .map(|e| (verts[e[1]] - verts[e[0]]).normalize_or_zero())
            .filter(|d| *d != Vec3::ZERO)
            .collect();
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in &verts {
            min = min.min(*v);
            max = max.max(*v);
        }
        WorldHull {
            verts,
            normals,
            edge_dirs,
            min,
            max,
        }
    }

    fn project(&self, axis: Vec3) -> (f32, f32) {
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for v in &self.verts {
            let d = v.dot(axis);
            lo = lo.min(d);
            hi = hi.max(d);
        }
        (lo, hi)
    }
}

/// A contact between two hulls. `normal` points from the first hull to the second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HullContact {
    pub normal: Vec3,
    pub point: Vec3,
    pub depth: f32,
}

/// Separating-axis test between two convex hulls.
///
/// Returns the contacts when the hulls overlap, or an empty vector when they
/// are separated along some axis.
pub fn collide_hulls(a: &WorldHull, b: &WorldHull) -> Vec<HullContact> {
    // Quick reject on bounding boxes.
    if a.max.x < b.min.x
        || b.max.x < a.min.x
        || a.max.y < b.min.y
        || b.max.y < a.min.y
        || a.max.z < b.min.z
        || b.max.z < a.min.z
    {
        return Vec::new();
    }

    // Find the axis of least penetration. `oriented` points from A towards B.
    let mut best_overlap = f32::INFINITY;
    let mut best_axis = Vec3::ZERO;
    let mut best_is_face = true;

    let mut consider = |axis: Vec3, is_face: bool| -> bool {
        let (amin, amax) = a.project(axis);
        let (bmin, bmax) = b.project(axis);
        let push_b_along_axis = amax - bmin;
        let push_b_against_axis = bmax - amin;
        if push_b_along_axis <= 0.0 || push_b_against_axis <= 0.0 {
            return false;
        }
        let (overlap, oriented) = if push_b_along_axis <= push_b_against_axis {
            (push_b_along_axis, axis)
        } else {
            (push_b_against_axis, -axis)
        };
        if overlap < best_overlap {
            best_overlap = overlap;
            best_axis = oriented;
            best_is_face = is_face;
        }
        true
    };

    for &n in a.normals.iter().chain(b.normals.iter()) {
        if !consider(n, true) {
            return Vec::new();
        }
    }
    for &ea in &a.edge_dirs {
        for &eb in &b.edge_dirs {
            let axis = ea.cross(eb);
            let len = axis.length();
            if len <= 1e-6 {
                continue;
            }
            if !consider(axis / len, false) {
                return Vec::new();
            }
        }
    }
    if !best_overlap.is_finite() {
        return Vec::new();
    }

    let axis = best_axis;
    let (amin, amax) = a.project(axis);
    let (bmin, bmax) = b.project(axis);
    let tol = 1e-3 * (amax - amin).max(bmax - bmin) + 1e-6;

    if best_is_face {
        // Prefer A's top face (B penetrates it); otherwise B's bottom face.
        let contacts = face_contacts(&a.verts, &b.verts, axis, 1.0, tol);
        if !contacts.is_empty() {
            return contacts;
        }
        let contacts = face_contacts(&b.verts, &a.verts, axis, -1.0, tol);
        if !contacts.is_empty() {
            return contacts;
        }
    }

    // Edge-edge axis, or no face contact inside a footprint: one contact at the
    // extremal vertices along the axis.
    let pa = extreme_vertex(&a.verts, axis, true);
    let pb = extreme_vertex(&b.verts, axis, false);
    vec![HullContact {
        normal: axis,
        point: (pa + pb) * 0.5,
        depth: best_overlap,
    }]
}

/// Contacts for a reference hull's face against an incident hull.
///
/// `sign = +1` uses the reference's maximum along `axis` (the incident hull
/// lies on the negative side); `sign = -1` uses its minimum (the incident hull
/// lies on the positive side). An incident vertex is a contact when it is past
/// the reference plane **and** its projection falls inside the reference face's
/// footprint. The footprint check is what makes a small box resting on a large
/// slab produce contacts under the box, not at the slab's far corners.
fn face_contacts(
    ref_verts: &[Vec3],
    inc_verts: &[Vec3],
    axis: Vec3,
    sign: f32,
    tol: f32,
) -> Vec<HullContact> {
    let proj = |v: Vec3| v.dot(axis);
    let plane = if sign > 0.0 {
        ref_verts
            .iter()
            .map(|&v| proj(v))
            .fold(f32::NEG_INFINITY, f32::max)
    } else {
        ref_verts
            .iter()
            .map(|&v| proj(v))
            .fold(f32::INFINITY, f32::min)
    };

    // Reference face: the vertices lying on the plane, ordered CCW around `axis`.
    let mut face: Vec<Vec3> = ref_verts
        .iter()
        .copied()
        .filter(|&v| (proj(v) - plane).abs() <= tol)
        .collect();
    if face.len() < 3 {
        return Vec::new();
    }
    let centre = face.iter().copied().sum::<Vec3>() / face.len() as f32;
    let [u, w] = perpendicular_basis(axis);
    let angle = |p: &Vec3| {
        let d = *p - centre;
        d.dot(w).atan2(d.dot(u))
    };
    face.sort_by(|p, q| angle(p).total_cmp(&angle(q)));

    // Degenerate (collinear) faces have no footprint.
    let mut area2 = 0.0f32;
    for i in 0..face.len() {
        let (p, q) = (face[i] - centre, face[(i + 1) % face.len()] - centre);
        area2 += p.cross(q).dot(axis);
    }
    if area2 <= tol * tol {
        return Vec::new();
    }

    let inside = |p: Vec3| -> bool {
        (0..face.len()).all(|i| {
            let (a, b) = (face[i], face[(i + 1) % face.len()]);
            let edge = b - a;
            edge.cross(p - a).dot(axis) >= -tol * edge.length()
        })
    };

    let mut out: Vec<(f32, HullContact)> = inc_verts
        .iter()
        .filter_map(|&p| {
            let depth = sign * (plane - proj(p));
            if depth <= 0.0 {
                return None;
            }
            let on_plane = p - axis * (proj(p) - plane);
            if !inside(on_plane) {
                return None;
            }
            Some((
                depth,
                HullContact {
                    normal: axis,
                    point: p + axis * (sign * depth * 0.5),
                    depth,
                },
            ))
        })
        .collect();
    out.sort_by(|x, y| y.0.total_cmp(&x.0));
    out.truncate(MAX_FACE_CONTACTS);
    out.into_iter().map(|(_, c)| c).collect()
}

/// The vertex with the largest (`max = true`) or smallest projection on `dir`.
fn extreme_vertex(verts: &[Vec3], dir: Vec3, max: bool) -> Vec3 {
    let key = |v: &Vec3| v.dot(dir);
    let pick = if max {
        verts
            .iter()
            .copied()
            .max_by(|p, q| key(p).total_cmp(&key(q)))
    } else {
        verts
            .iter()
            .copied()
            .min_by(|p, q| key(p).total_cmp(&key(q)))
    };
    pick.unwrap_or(Vec3::ZERO)
}

/// A body as seen by the 3D solver.
#[derive(Debug, Clone)]
pub struct Body3D {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub angvel: Vec3,
    pub inv_mass: f32,
    /// Inverse principal inertia in the body frame.
    pub inv_inertia_local: Vec3,
    pub parts: Vec<LocalHull>,
    pub restitution: f32,
    pub friction: f32,
    pub sensor: bool,
    /// True for fixed and kinematic bodies: they take no impulses.
    pub is_static: bool,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
}

#[derive(Debug, Clone, Copy)]
struct Contact {
    a: usize,
    b: usize,
    normal: Vec3,
    point: Vec3,
    depth: f32,
    restitution: f32,
    friction: f32,
    ra: Vec3,
    rb: Vec3,
    tangents: [Vec3; 2],
    normal_mass: f32,
    tangent_mass: [f32; 2],
    bias: f32,
    normal_impulse: f32,
    tangent_impulse: [f32; 2],
}

fn inv_or_zero(x: f32) -> f32 {
    if x > 0.0 {
        1.0 / x
    } else {
        0.0
    }
}

fn perpendicular_basis(n: Vec3) -> [Vec3; 2] {
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let t1 = n.cross(helper).normalize_or_zero();
    let t2 = n.cross(t1);
    [t1, t2]
}

fn point_velocity(v: Vec3, w: Vec3, r: Vec3) -> Vec3 {
    v + w.cross(r)
}

/// Inverse world inertia `R · diag(inv_local) · Rᵀ`.
fn inv_inertia_world(b: &Body3D) -> Mat3 {
    let r = Mat3::from_quat(b.rot);
    r * Mat3::from_diagonal(b.inv_inertia_local) * r.transpose()
}

/// Effective inverse mass of a contact along direction `d`.
fn effective_mass(ima: f32, imb: f32, iia: Mat3, iib: Mat3, ra: Vec3, rb: Vec3, d: Vec3) -> f32 {
    let ta = (iia * ra.cross(d)).cross(ra);
    let tb = (iib * rb.cross(d)).cross(rb);
    inv_or_zero(ima + imb + d.dot(ta + tb))
}

fn apply_impulse(bodies: &mut [Body3D], iw: &[Mat3], c: &Contact, p: Vec3) {
    {
        let body = &mut bodies[c.a];
        body.vel -= p * body.inv_mass;
        body.angvel -= iw[c.a] * c.ra.cross(p);
    }
    {
        let body = &mut bodies[c.b];
        body.vel += p * body.inv_mass;
        body.angvel += iw[c.b] * c.rb.cross(p);
    }
}

/// Maximum distance between a contact point and a cached point for the two to
/// be treated as the same contact across steps.
const WARM_START_DISTANCE: f32 = 0.05;

/// Accumulated impulses from the previous step, keyed by body pair and
/// approximate contact point. Carrying these into the next step ("warm
/// starting") is what keeps stacked bodies from tumbling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedImpulse {
    pub a: usize,
    pub b: usize,
    pub point: Vec3,
    pub normal: Vec3,
    pub normal_impulse: f32,
    pub tangent_impulse: [f32; 2],
}

/// Advance `bodies` by `dt` seconds: integrate velocities, detect contacts,
/// solve impulses, then integrate positions and orientations.
///
/// `cache` holds the previous step's impulses. It must be cleared by the caller
/// whenever the body list changes, because cache entries refer to body indices.
pub fn step(bodies: &mut [Body3D], gravity: Vec3, dt: f32, cache: &mut Vec<CachedImpulse>) {
    if dt <= 0.0 {
        return;
    }
    for b in bodies.iter_mut() {
        if b.is_static || b.inv_mass == 0.0 {
            continue;
        }
        b.vel += gravity * b.gravity_scale * dt;
        b.vel /= 1.0 + dt * b.linear_damping;
        b.angvel /= 1.0 + dt * b.angular_damping;
    }

    // World-space hulls per body, then contacts per body pair (fixed order).
    let hulls: Vec<Vec<WorldHull>> = bodies
        .iter()
        .map(|b| {
            b.parts
                .iter()
                .map(|p| WorldHull::new(p, b.pos, b.rot))
                .collect()
        })
        .collect();
    let iw: Vec<Mat3> = bodies.iter().map(inv_inertia_world).collect();

    let mut contacts: Vec<Contact> = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (bi, bj) = (&bodies[i], &bodies[j]);
            if (bi.is_static && bj.is_static) || bi.sensor || bj.sensor {
                continue;
            }
            for ha in &hulls[i] {
                for hb in &hulls[j] {
                    for hc in collide_hulls(ha, hb) {
                        contacts.push(Contact {
                            a: i,
                            b: j,
                            normal: hc.normal,
                            point: hc.point,
                            depth: hc.depth,
                            restitution: bi.restitution.max(bj.restitution),
                            friction: (bi.friction * bj.friction).max(0.0).sqrt(),
                            ra: Vec3::ZERO,
                            rb: Vec3::ZERO,
                            tangents: [Vec3::ZERO; 2],
                            normal_mass: 0.0,
                            tangent_mass: [0.0; 2],
                            bias: 0.0,
                            normal_impulse: 0.0,
                            tangent_impulse: [0.0; 2],
                        });
                    }
                }
            }
        }
    }

    // Warm start: seed each contact with last step's impulses when it matches.
    for c in contacts.iter_mut() {
        let hit = cache.iter().find(|p| {
            p.a == c.a
                && p.b == c.b
                && p.normal.dot(c.normal) > 0.9
                && p.point.distance(c.point) <= WARM_START_DISTANCE
        });
        if let Some(p) = hit {
            c.normal_impulse = p.normal_impulse;
            c.tangent_impulse = p.tangent_impulse;
        }
    }

    for c in contacts.iter_mut() {
        let (ba, bb) = (&bodies[c.a], &bodies[c.b]);
        c.ra = c.point - ba.pos;
        c.rb = c.point - bb.pos;
        let n = c.normal;
        c.tangents = perpendicular_basis(n);
        let (ima, imb) = (ba.inv_mass, bb.inv_mass);
        c.normal_mass = effective_mass(ima, imb, iw[c.a], iw[c.b], c.ra, c.rb, n);
        for k in 0..2 {
            c.tangent_mass[k] =
                effective_mass(ima, imb, iw[c.a], iw[c.b], c.ra, c.rb, c.tangents[k]);
        }
        let vrel =
            point_velocity(bb.vel, bb.angvel, c.rb) - point_velocity(ba.vel, ba.angvel, c.ra);
        let vn = vrel.dot(n);
        let restitution_bias = if vn < -RESTITUTION_THRESHOLD {
            -c.restitution * vn
        } else {
            0.0
        };
        // Penetration is corrected by moving positions after integration (see
        // below), not by injecting velocity, which would make bodies bounce.
        c.bias = restitution_bias;
    }

    // Apply the warm-start impulses before iterating.
    for c in contacts.iter() {
        if c.normal_impulse == 0.0 && c.tangent_impulse == [0.0; 2] {
            continue;
        }
        let p = c.normal * c.normal_impulse
            + c.tangents[0] * c.tangent_impulse[0]
            + c.tangents[1] * c.tangent_impulse[1];
        apply_impulse(bodies, &iw, c, p);
    }

    for _ in 0..SOLVER_ITERATIONS {
        for c in contacts.iter_mut() {
            let n = c.normal;
            let vrel = point_velocity(bodies[c.b].vel, bodies[c.b].angvel, c.rb)
                - point_velocity(bodies[c.a].vel, bodies[c.a].angvel, c.ra);
            let vn = vrel.dot(n);
            let new_impulse = (c.normal_impulse + (c.bias - vn) * c.normal_mass).max(0.0);
            let delta = new_impulse - c.normal_impulse;
            c.normal_impulse = new_impulse;
            apply_impulse(bodies, &iw, c, n * delta);

            for k in 0..2 {
                let t = c.tangents[k];
                let vrel = point_velocity(bodies[c.b].vel, bodies[c.b].angvel, c.rb)
                    - point_velocity(bodies[c.a].vel, bodies[c.a].angvel, c.ra);
                let vt = vrel.dot(t);
                let limit = c.friction * c.normal_impulse;
                let new_t = (c.tangent_impulse[k] - vt * c.tangent_mass[k]).clamp(-limit, limit);
                let delta_t = new_t - c.tangent_impulse[k];
                c.tangent_impulse[k] = new_t;
                apply_impulse(bodies, &iw, c, t * delta_t);
            }
        }
    }

    // Positional correction: push overlapping bodies apart by a fraction of the
    // remaining penetration, split by inverse mass. Contacts for one body pair
    // share a manifold, so the correction is averaged over that pair's points.
    let max_push = MAX_CORRECTION_SPEED * dt;
    let mut i = 0;
    while i < contacts.len() {
        let (a, b) = (contacts[i].a, contacts[i].b);
        let mut j = i;
        while j < contacts.len() && contacts[j].a == a && contacts[j].b == b {
            j += 1;
        }
        let count = (j - i) as f32;
        let (ima, imb) = (bodies[a].inv_mass, bodies[b].inv_mass);
        let total = ima + imb;
        if total > 0.0 {
            for c in &contacts[i..j] {
                let push = (BAUMGARTE * (c.depth - SLOP).max(0.0) / count).min(max_push);
                if push <= 0.0 {
                    continue;
                }
                bodies[a].pos -= c.normal * push * (ima / total);
                bodies[b].pos += c.normal * push * (imb / total);
            }
        }
        i = j;
    }

    // Remember this step's impulses for the next step's warm start.
    cache.clear();
    cache.extend(contacts.iter().map(|c| CachedImpulse {
        a: c.a,
        b: c.b,
        point: c.point,
        normal: c.normal,
        normal_impulse: c.normal_impulse,
        tangent_impulse: c.tangent_impulse,
    }));

    for b in bodies.iter_mut() {
        b.pos += b.vel * dt;
        let w = b.angvel;
        if w != Vec3::ZERO {
            let spin = Quat::from_xyzw(w.x, w.y, w.z, 0.0) * b.rot;
            b.rot = (b.rot + spin * (0.5 * dt)).normalize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit-half-extent cube as a convex part (outward triangles).
    fn cube_part(half: f32) -> ConvexPart {
        let v = vec![
            [-half, -half, -half],
            [half, -half, -half],
            [half, half, -half],
            [-half, half, -half],
            [-half, -half, half],
            [half, -half, half],
            [half, half, half],
            [-half, half, half],
        ];
        let idx = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        ConvexPart {
            vertices: v,
            indices: idx,
        }
    }

    fn hull_at(part: &ConvexPart, pos: Vec3) -> WorldHull {
        let local = LocalHull::from_part(part).unwrap();
        WorldHull::new(&local, pos, Quat::IDENTITY)
    }

    #[test]
    fn local_hull_orientation_is_corrected() {
        // Deliberately inward-wound cube: the constructor must flip it outward.
        let mut part = cube_part(0.5);
        for t in &mut part.indices {
            t.swap(1, 2);
        }
        let local = LocalHull::from_part(&part).unwrap();
        assert!((local.volume - 1.0).abs() < 1e-5);
        for t in &local.tris {
            let (a, b, c) = (local.verts[t[0]], local.verts[t[1]], local.verts[t[2]]);
            let n = (b - a).cross(c - a);
            assert!(n.dot(a) > 0.0, "triangle should face outward");
        }
    }

    #[test]
    fn flat_part_has_no_hull() {
        let part = ConvexPart {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            indices: vec![[0, 1, 2], [1, 3, 2]],
        };
        assert!(LocalHull::from_part(&part).is_none());
    }

    #[test]
    fn separated_cubes_do_not_collide() {
        let part = cube_part(0.5);
        let a = hull_at(&part, Vec3::ZERO);
        let b = hull_at(&part, Vec3::new(2.0, 0.0, 0.0));
        assert!(collide_hulls(&a, &b).is_empty());
    }

    #[test]
    fn overlapping_cubes_report_face_contacts() {
        let part = cube_part(0.5);
        let a = hull_at(&part, Vec3::ZERO);
        let b = hull_at(&part, Vec3::new(0.0, 0.9, 0.0)); // overlaps by 0.1 on +y
        let contacts = collide_hulls(&a, &b);
        assert_eq!(contacts.len(), 4, "face-on-face should give four points");
        assert!((contacts[0].normal - Vec3::Y).length() < 1e-5);
        for c in &contacts {
            assert!((c.depth - 0.1).abs() < 1e-4, "depth {}", c.depth);
        }
    }

    #[test]
    fn body_struct_defaults_are_dynamic() {
        let mut b = Body3D {
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            vel: Vec3::ZERO,
            angvel: Vec3::ZERO,
            inv_mass: 1.0,
            inv_inertia_local: Vec3::ONE,
            parts: Vec::new(),
            restitution: 0.0,
            friction: 0.5,
            sensor: false,
            is_static: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
        };
        step(
            std::slice::from_mut(&mut b),
            Vec3::new(0.0, -10.0, 0.0),
            0.1,
            &mut Vec::new(),
        );
        assert!(b.vel.y < -0.99 && b.vel.y > -1.01);
        assert!(b.pos.y < 0.0);
    }
}
