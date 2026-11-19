//! In-house 2D rigid body solver: shapes, collision detection, and a
//! sequential-impulse contact solver.
//!
//! This replaces Rapier2D so the workspace stays free of Apache-2.0-only
//! dependencies. The design is deliberately small and deterministic:
//!
//! * Shapes are circles or convex polygons (capsules are approximated by a
//!   stadium polygon). Polygon pairs use separating-axis tests with
//!   Sutherland–Hodgman clipping to produce up to two contact points.
//! * The solver iterates contacts in a fixed order with a fixed iteration count,
//!   so identical inputs always produce identical outputs.
//! * Penetration is corrected with a velocity bias (Baumgarte stabilisation);
//!   no sleeping or warm starting.

use glam::Vec2;

/// Sequential-impulse iterations per step.
pub const SOLVER_ITERATIONS: usize = 10;
/// Fraction of residual penetration corrected per step.
const BAUMGARTE: f32 = 0.2;
/// Penetration tolerated before position correction kicks in.
const SLOP: f32 = 0.01;
/// Upper bound on the speed at which penetration is pushed out, so a deep
/// overlap is resolved smoothly rather than launched.
const MAX_CORRECTION_SPEED: f32 = 4.0;
/// Approach speed below which restitution is ignored (avoids jitter).
const RESTITUTION_THRESHOLD: f32 = 1.0;
/// Segments per cap when approximating a capsule as a polygon.
pub const CAPSULE_CAP_SEGMENTS: usize = 8;

/// World-space geometry of one collider for a single step.
#[derive(Debug, Clone, PartialEq)]
pub enum Geom {
    Circle {
        center: Vec2,
        radius: f32,
    },
    /// Convex, counter-clockwise vertices in world space.
    Polygon(Vec<Vec2>),
}

/// A contact manifold between two geometries. `normal` points from the first
/// geometry towards the second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manifold {
    pub normal: Vec2,
    pub points: [Vec2; 2],
    pub depths: [f32; 2],
    pub count: usize,
}

impl Manifold {
    fn single(normal: Vec2, point: Vec2, depth: f32) -> Self {
        Manifold {
            normal,
            points: [point, Vec2::ZERO],
            depths: [depth, 0.0],
            count: 1,
        }
    }

    fn flipped(self) -> Self {
        Manifold {
            normal: -self.normal,
            ..self
        }
    }
}

/// Outward unit normal of edge `i` (from `v[i]` to `v[i+1]`) of a CCW polygon.
fn edge_normal(v: &[Vec2], i: usize) -> Vec2 {
    let a = v[i];
    let b = v[(i + 1) % v.len()];
    let d = b - a;
    let n = Vec2::new(d.y, -d.x);
    n.normalize_or_zero()
}

/// Closest point on segment `a..b` to `p`.
fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= f32::EPSILON {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

/// Collide two world-space geometries. Returns `None` when they do not overlap.
pub fn collide(a: &Geom, b: &Geom) -> Option<Manifold> {
    match (a, b) {
        (
            Geom::Circle {
                center: ca,
                radius: ra,
            },
            Geom::Circle {
                center: cb,
                radius: rb,
            },
        ) => circle_circle(*ca, *ra, *cb, *rb),
        (Geom::Circle { center, radius }, Geom::Polygon(poly)) => {
            circle_polygon(*center, *radius, poly)
        }
        (Geom::Polygon(poly), Geom::Circle { center, radius }) => {
            circle_polygon(*center, *radius, poly).map(Manifold::flipped)
        }
        (Geom::Polygon(pa), Geom::Polygon(pb)) => polygon_polygon(pa, pb),
    }
}

fn circle_circle(ca: Vec2, ra: f32, cb: Vec2, rb: f32) -> Option<Manifold> {
    let d = cb - ca;
    let dist = d.length();
    if dist >= ra + rb {
        return None;
    }
    let n = if dist > f32::EPSILON {
        d / dist
    } else {
        Vec2::X
    };
    let depth = ra + rb - dist;
    // Midpoint of the two surfaces along the contact normal.
    let point = ((ca + n * ra) + (cb - n * rb)) * 0.5;
    Some(Manifold::single(n, point, depth))
}

/// Circle `(c, r)` against polygon `poly`; normal points from the circle to the polygon.
fn circle_polygon(c: Vec2, r: f32, poly: &[Vec2]) -> Option<Manifold> {
    let n_verts = poly.len();
    if n_verts < 2 {
        return None;
    }
    // Signed distance from the centre to each edge plane (positive = outside).
    let mut best_s = f32::NEG_INFINITY;
    let mut best_i = 0;
    let mut inside = true;
    for i in 0..n_verts {
        let n = edge_normal(poly, i);
        let s = n.dot(c - poly[i]);
        if s > 0.0 {
            inside = false;
        }
        if s > best_s {
            best_s = s;
            best_i = i;
        }
    }

    if inside {
        // Centre is inside: push the circle out through the nearest face.
        // The polygon is the second shape, so the normal (circle -> polygon) is -n.
        let n = edge_normal(poly, best_i);
        let depth = r + best_s;
        let point = c - n * (r + best_s) * 0.5;
        return Some(Manifold::single(-n, point, depth));
    }

    // Centre is outside: closest boundary point.
    let mut best_q = Vec2::ZERO;
    let mut best_d = f32::INFINITY;
    for i in 0..n_verts {
        let q = closest_on_segment(c, poly[i], poly[(i + 1) % n_verts]);
        let d = (c - q).length();
        if d < best_d {
            best_d = d;
            best_q = q;
        }
    }
    if best_d >= r {
        return None;
    }
    let dir = best_q - c;
    let normal = if best_d > f32::EPSILON {
        dir / best_d
    } else {
        Vec2::X
    };
    Some(Manifold::single(normal, best_q, r - best_d))
}

/// Clip the segment `p0..p1` to the half-plane `n·x <= c`.
fn clip_segment(p0: Vec2, p1: Vec2, n: Vec2, c: f32) -> Vec<Vec2> {
    let d0 = n.dot(p0) - c;
    let d1 = n.dot(p1) - c;
    let mut out = Vec::with_capacity(2);
    if d0 <= 0.0 {
        out.push(p0);
    }
    if d1 <= 0.0 {
        out.push(p1);
    }
    if d0 * d1 < 0.0 {
        let t = d0 / (d0 - d1);
        out.push(p0 + (p1 - p0) * t);
    }
    out
}

/// Separating-axis test with clipping for two convex polygons.
fn polygon_polygon(pa: &[Vec2], pb: &[Vec2]) -> Option<Manifold> {
    if pa.len() < 3 || pb.len() < 3 {
        return None;
    }
    // Find the face (of either polygon) with the least penetration.
    let mut best_sep = f32::NEG_INFINITY;
    let mut best: Option<(bool, usize)> = None; // (ref is A?, edge index)
    for i in 0..pa.len() {
        let n = edge_normal(pa, i);
        if n == Vec2::ZERO {
            continue;
        }
        let h = n.dot(pa[i]);
        let sep = pb
            .iter()
            .map(|&p| n.dot(p) - h)
            .fold(f32::INFINITY, f32::min);
        if sep > best_sep {
            best_sep = sep;
            best = Some((true, i));
        }
    }
    for i in 0..pb.len() {
        let n = edge_normal(pb, i);
        if n == Vec2::ZERO {
            continue;
        }
        let h = n.dot(pb[i]);
        let sep = pa
            .iter()
            .map(|&p| n.dot(p) - h)
            .fold(f32::INFINITY, f32::min);
        if sep > best_sep {
            best_sep = sep;
            best = Some((false, i));
        }
    }
    if best_sep > 0.0 {
        return None;
    }
    let (ref_is_a, ri) = best?;
    let (ref_poly, inc_poly) = if ref_is_a { (pa, pb) } else { (pb, pa) };

    // Reference face (outward normal `n` of the reference polygon).
    let n = edge_normal(ref_poly, ri);
    let v1 = ref_poly[ri];
    let v2 = ref_poly[(ri + 1) % ref_poly.len()];
    let h = n.dot(v1);

    // Incident edge: the incident polygon edge most anti-parallel to `n`.
    let mut inc_i = 0;
    let mut inc_dot = f32::INFINITY;
    for i in 0..inc_poly.len() {
        let d = edge_normal(inc_poly, i).dot(n);
        if d < inc_dot {
            inc_dot = d;
            inc_i = i;
        }
    }
    let p1 = inc_poly[inc_i];
    let p2 = inc_poly[(inc_i + 1) % inc_poly.len()];

    // Clip the incident edge to the reference face's side planes.
    let t = (v2 - v1).normalize_or_zero();
    if t == Vec2::ZERO {
        return None;
    }
    // Clip against both side planes: t·x <= t·v2 and -t·x <= -t·v1.
    let seg = clip_segment(p1, p2, t, t.dot(v2));
    let seg = if seg.len() == 2 {
        clip_segment(seg[0], seg[1], -t, -t.dot(v1))
    } else {
        Vec::new()
    };

    // Keep points below the reference face and turn them into contacts.
    let mut manifold: Option<Manifold> = None;
    let normal_ab = if ref_is_a { n } else { -n };
    for p in seg {
        let sep = n.dot(p) - h;
        if sep > 0.0 {
            continue;
        }
        let depth = -sep;
        match manifold.as_mut() {
            None => manifold = Some(Manifold::single(normal_ab, p, depth)),
            Some(m) if m.count < 2 => {
                m.points[1] = p;
                m.depths[1] = depth;
                m.count = 2;
            }
            Some(_) => {}
        }
    }
    manifold
}

/// Mass properties of a shape in its local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProps {
    pub mass: f32,
    pub inertia: f32,
}

/// Local-frame shape description (what a body stores).
#[derive(Debug, Clone, PartialEq)]
pub enum LocalShape {
    Circle {
        radius: f32,
    },
    /// Convex CCW polygon in local coordinates, centred on the body origin.
    Polygon(Vec<Vec2>),
}

impl LocalShape {
    /// Mass and polar inertia for `density`, about the body origin.
    pub fn mass_props(&self, density: f32) -> MassProps {
        match self {
            LocalShape::Circle { radius } => {
                let area = std::f32::consts::PI * radius * radius;
                let mass = density * area;
                MassProps {
                    mass,
                    inertia: 0.5 * mass * radius * radius,
                }
            }
            LocalShape::Polygon(v) => {
                // Triangle fan about the polygon centroid (vertex average, which is
                // the true centroid for the regular and axis-aligned shapes used here).
                let n = v.len() as f32;
                let c = v.iter().copied().sum::<Vec2>() / n;
                let mut area = 0.0;
                let mut inertia = 0.0;
                for i in 0..v.len() {
                    let a = v[i] - c;
                    let b = v[(i + 1) % v.len()] - c;
                    let cross = a.perp_dot(b).abs();
                    let tri_area = 0.5 * cross;
                    area += tri_area;
                    // Second moment of a triangle about its vertex-to-centroid origin.
                    inertia += cross * (a.dot(a) + a.dot(b) + b.dot(b)) / 12.0;
                }
                let mass = density * area;
                MassProps {
                    mass,
                    inertia: density * inertia,
                }
            }
        }
    }

    /// World-space geometry for a body at `pos` rotated by `angle`.
    pub fn world(&self, pos: Vec2, angle: f32) -> Geom {
        match self {
            LocalShape::Circle { radius } => Geom::Circle {
                center: pos,
                radius: *radius,
            },
            LocalShape::Polygon(v) => {
                let (s, c) = angle.sin_cos();
                Geom::Polygon(
                    v.iter()
                        .map(|p| pos + Vec2::new(c * p.x - s * p.y, s * p.x + c * p.y))
                        .collect(),
                )
            }
        }
    }
}

/// Stadium polygon approximating a capsule with its axis along Y.
pub fn capsule_polygon(half_height: f32, radius: f32) -> Vec<Vec2> {
    let n = CAPSULE_CAP_SEGMENTS;
    let mut out = Vec::with_capacity(2 * (n + 1));
    // Bottom cap, angles pi..2pi (left -> bottom -> right).
    for k in 0..=n {
        let t = std::f32::consts::PI + std::f32::consts::PI * k as f32 / n as f32;
        out.push(Vec2::new(radius * t.cos(), -half_height + radius * t.sin()));
    }
    // Top cap, angles 0..pi (right -> top -> left).
    for k in 0..=n {
        let t = std::f32::consts::PI * k as f32 / n as f32;
        out.push(Vec2::new(radius * t.cos(), half_height + radius * t.sin()));
    }
    out
}

/// A body as seen by the solver. Positions are world-space and updated in place.
#[derive(Debug, Clone)]
pub struct SolverBody {
    pub pos: Vec2,
    pub angle: f32,
    pub vel: Vec2,
    pub angvel: f32,
    pub inv_mass: f32,
    pub inv_inertia: f32,
    pub shape: Option<LocalShape>,
    pub restitution: f32,
    pub friction: f32,
    pub sensor: bool,
    /// True for bodies that never move (fixed and kinematic): they take no impulses.
    pub is_static: bool,
}

#[derive(Debug, Clone, Copy)]
struct Contact {
    a: usize,
    b: usize,
    normal: Vec2,
    point: Vec2,
    depth: f32,
    restitution: f32,
    friction: f32,
    ra: Vec2,
    rb: Vec2,
    normal_mass: f32,
    tangent_mass: f32,
    bias: f32,
    normal_impulse: f32,
    tangent_impulse: f32,
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.perp_dot(b)
}

/// Velocity of a point at offset `r` on a body with linear `v` and angular `w`.
fn point_velocity(v: Vec2, w: f32, r: Vec2) -> Vec2 {
    v + Vec2::new(-w * r.y, w * r.x)
}

/// Detect contacts between all non-static pairs, in body insertion order.
fn detect(bodies: &[SolverBody]) -> Vec<Contact> {
    let geoms: Vec<Option<Geom>> = bodies
        .iter()
        .map(|b| b.shape.as_ref().map(|s| s.world(b.pos, b.angle)))
        .collect();
    let mut contacts = Vec::new();
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (bi, bj) = (&bodies[i], &bodies[j]);
            if (bi.is_static && bj.is_static) || bi.sensor || bj.sensor {
                continue;
            }
            let (Some(gi), Some(gj)) = (&geoms[i], &geoms[j]) else {
                continue;
            };
            let Some(m) = collide(gi, gj) else { continue };
            let restitution = bi.restitution.max(bj.restitution);
            let friction = (bi.friction * bj.friction).max(0.0).sqrt();
            for k in 0..m.count {
                contacts.push(Contact {
                    a: i,
                    b: j,
                    normal: m.normal,
                    point: m.points[k],
                    depth: m.depths[k],
                    restitution,
                    friction,
                    ra: Vec2::ZERO,
                    rb: Vec2::ZERO,
                    normal_mass: 0.0,
                    tangent_mass: 0.0,
                    bias: 0.0,
                    normal_impulse: 0.0,
                    tangent_impulse: 0.0,
                });
            }
        }
    }
    contacts
}

/// Advance `bodies` by `dt` seconds: detect contacts, solve impulses, integrate.
///
/// `gravity` is applied to dynamic bodies only; `gravity_scale`, damping and
/// the velocity/position integration order follow the usual semi-implicit
/// scheme. Callers set kinematic velocities before calling.
pub fn step(
    bodies: &mut [SolverBody],
    gravity: Vec2,
    gravity_scales: &[f32],
    damping: &[(f32, f32)],
    dt: f32,
) {
    if dt <= 0.0 {
        return;
    }
    // Integrate forces and damping into velocities.
    for (i, b) in bodies.iter_mut().enumerate() {
        if b.is_static || b.inv_mass == 0.0 {
            continue;
        }
        b.vel += gravity * gravity_scales[i] * dt;
        let (ld, ad) = damping[i];
        b.vel /= 1.0 + dt * ld;
        b.angvel /= 1.0 + dt * ad;
    }

    // Collision detection on the integrated velocities.
    let mut contacts = detect(bodies);
    for c in contacts.iter_mut() {
        let (pa, pb) = (bodies[c.a].pos, bodies[c.b].pos);
        c.ra = c.point - pa;
        c.rb = c.point - pb;
        let (ima, imb) = (bodies[c.a].inv_mass, bodies[c.b].inv_mass);
        let (iia, iib) = (bodies[c.a].inv_inertia, bodies[c.b].inv_inertia);
        let n = c.normal;
        let t = Vec2::new(-n.y, n.x);
        let rna = cross2(c.ra, n);
        let rnb = cross2(c.rb, n);
        c.normal_mass = inv_or_zero(ima + imb + iia * rna * rna + iib * rnb * rnb);
        let rta = cross2(c.ra, t);
        let rtb = cross2(c.rb, t);
        c.tangent_mass = inv_or_zero(ima + imb + iia * rta * rta + iib * rtb * rtb);

        // Restitution target from the pre-solve approach speed.
        let vrel = point_velocity(bodies[c.b].vel, bodies[c.b].angvel, c.rb)
            - point_velocity(bodies[c.a].vel, bodies[c.a].angvel, c.ra);
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

    // Sequential impulses.
    for _ in 0..SOLVER_ITERATIONS {
        for c in contacts.iter_mut() {
            let n = c.normal;
            let t = Vec2::new(-n.y, n.x);

            // Normal impulse.
            let vrel = point_velocity(bodies[c.b].vel, bodies[c.b].angvel, c.rb)
                - point_velocity(bodies[c.a].vel, bodies[c.a].angvel, c.ra);
            let vn = vrel.dot(n);
            let lambda = (c.bias - vn) * c.normal_mass;
            let new_impulse = (c.normal_impulse + lambda).max(0.0);
            let delta = new_impulse - c.normal_impulse;
            c.normal_impulse = new_impulse;
            apply_impulse(bodies, c, n * delta);

            // Friction, clamped by the Coulomb cone.
            let vrel = point_velocity(bodies[c.b].vel, bodies[c.b].angvel, c.rb)
                - point_velocity(bodies[c.a].vel, bodies[c.a].angvel, c.ra);
            let vt = vrel.dot(t);
            let lambda_t = -vt * c.tangent_mass;
            let max_friction = c.friction * c.normal_impulse;
            let new_t = (c.tangent_impulse + lambda_t).clamp(-max_friction, max_friction);
            let delta_t = new_t - c.tangent_impulse;
            c.tangent_impulse = new_t;
            apply_impulse(bodies, c, t * delta_t);
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

    // Integrate positions. Static bodies have zero velocity, so they stay put;
    // kinematic bodies move by the velocity set from their component.
    for b in bodies.iter_mut() {
        b.pos += b.vel * dt;
        b.angle += b.angvel * dt;
    }
}

fn inv_or_zero(x: f32) -> f32 {
    if x > 0.0 {
        1.0 / x
    } else {
        0.0
    }
}

fn apply_impulse(bodies: &mut [SolverBody], c: &Contact, p: Vec2) {
    let (a, b) = (c.a, c.b);
    {
        let body = &mut bodies[a];
        body.vel -= p * body.inv_mass;
        body.angvel -= body.inv_inertia * cross2(c.ra, p);
    }
    {
        let body = &mut bodies[b];
        body.vel += p * body.inv_mass;
        body.angvel += body.inv_inertia * cross2(c.rb, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(half: f32, centre: Vec2) -> Geom {
        Geom::Polygon(vec![
            centre + Vec2::new(-half, -half),
            centre + Vec2::new(half, -half),
            centre + Vec2::new(half, half),
            centre + Vec2::new(-half, half),
        ])
    }

    #[test]
    fn circles_overlap_and_separate() {
        let a = Geom::Circle {
            center: Vec2::ZERO,
            radius: 1.0,
        };
        let b = Geom::Circle {
            center: Vec2::new(1.5, 0.0),
            radius: 1.0,
        };
        let m = collide(&a, &b).expect("should overlap");
        assert!((m.depths[0] - 0.5).abs() < 1e-5);
        assert!((m.normal - Vec2::X).length() < 1e-5);

        let far = Geom::Circle {
            center: Vec2::new(3.0, 0.0),
            radius: 1.0,
        };
        assert!(collide(&a, &far).is_none());
    }

    #[test]
    fn circle_on_box_top_face() {
        let ground = square(10.0, Vec2::ZERO); // top at y = 10
        let ball = Geom::Circle {
            center: Vec2::new(0.0, 10.4),
            radius: 0.5,
        };
        let m = collide(&ball, &ground).expect("ball touches box");
        // Ball is the first shape, so the normal points down into the box.
        assert!((m.normal.y + 1.0).abs() < 1e-5, "normal {:?}", m.normal);
        assert!((m.depths[0] - 0.1).abs() < 1e-4, "depth {}", m.depths[0]);
    }

    #[test]
    fn box_on_box_gives_two_contacts() {
        let ground = square(5.0, Vec2::new(0.0, -5.0)); // top at y = 0
        let crate_ = square(0.5, Vec2::new(0.0, 0.49)); // bottom at y = -0.01
        let m = collide(&crate_, &ground).expect("boxes overlap");
        assert_eq!(m.count, 2, "face-on-face contact should give two points");
        assert!((m.normal.y + 1.0).abs() < 1e-5);
        for d in &m.depths[..m.count] {
            assert!((d - 0.01).abs() < 1e-4);
        }
    }

    #[test]
    fn separated_boxes_do_not_collide() {
        let a = square(0.5, Vec2::ZERO);
        let b = square(0.5, Vec2::new(2.0, 0.0));
        assert!(collide(&a, &b).is_none());
    }

    #[test]
    fn capsule_polygon_is_convex_ccw() {
        let poly = capsule_polygon(1.0, 0.5);
        // Shoelace area must be positive for CCW winding.
        let mut area = 0.0;
        for i in 0..poly.len() {
            area += poly[i].perp_dot(poly[(i + 1) % poly.len()]);
        }
        assert!(area > 0.0, "capsule polygon must be CCW");
    }

    #[test]
    fn mass_of_unit_box_is_its_area_times_density() {
        let shape = LocalShape::Polygon(vec![
            Vec2::new(-0.5, -0.5),
            Vec2::new(0.5, -0.5),
            Vec2::new(0.5, 0.5),
            Vec2::new(-0.5, 0.5),
        ]);
        let p = shape.mass_props(2.0);
        assert!((p.mass - 2.0).abs() < 1e-5);
        // Square inertia about its centre: m (w^2 + h^2) / 12 = 2 * 2 / 12.
        assert!(
            (p.inertia - 2.0 * 2.0 / 12.0).abs() < 1e-4,
            "inertia {}",
            p.inertia
        );
    }
}
