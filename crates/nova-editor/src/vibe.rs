//! The "Vibe GUI": a visual cubic-Bézier curve editor for driving a single
//! parameter (here, gravity) — and pushing edits back into the live simulation.
//!
//! The curve is authored in normalized space (`x,y in [0,1]`). A UI maps that to
//! a screen rectangle for display and drags control points; the sampled curve
//! value then feeds a physics parameter so tweaks are felt immediately.

use glam::Vec2;
use nova_physics::PhysicsWorld;
use nova_ui::Rect;

/// A cubic Bézier defined by four control points in normalized `[0,1]` space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BezierCurve {
    pub points: [Vec2; 4],
}

impl Default for BezierCurve {
    fn default() -> Self {
        // A gentle ease from 0 up to 1.
        BezierCurve {
            points: [
                Vec2::new(0.0, 0.0),
                Vec2::new(0.33, 0.0),
                Vec2::new(0.66, 1.0),
                Vec2::new(1.0, 1.0),
            ],
        }
    }
}

impl BezierCurve {
    /// Evaluate the curve at parameter `t in [0,1]` (de Casteljau).
    pub fn sample(&self, t: f32) -> Vec2 {
        let t = t.clamp(0.0, 1.0);
        let [p0, p1, p2, p3] = self.points;
        let u = 1.0 - t;
        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;
        p0 * b0 + p1 * b1 + p2 * b2 + p3 * b3
    }

    /// The curve's Y value at parameter `t` (the authored parameter value).
    pub fn value_at(&self, t: f32) -> f32 {
        self.sample(t).y
    }

    /// A polyline approximation with `segments + 1` points, for rendering.
    pub fn polyline(&self, segments: usize) -> Vec<Vec2> {
        let n = segments.max(1);
        (0..=n).map(|i| self.sample(i as f32 / n as f32)).collect()
    }

    /// Move control point `index` by `delta` (normalized), keeping it in bounds.
    pub fn move_point(&mut self, index: usize, delta: Vec2) {
        if let Some(p) = self.points.get_mut(index) {
            *p = (*p + delta).clamp(Vec2::ZERO, Vec2::ONE);
        }
    }

    /// Set control point `index` directly (normalized, clamped).
    pub fn set_point(&mut self, index: usize, value: Vec2) {
        if let Some(p) = self.points.get_mut(index) {
            *p = value.clamp(Vec2::ZERO, Vec2::ONE);
        }
    }
}

/// Convert a normalized point to a position inside a screen rect (y is flipped
/// so higher values appear higher on screen).
pub fn normalized_to_screen(rect: Rect, p: Vec2) -> Vec2 {
    Vec2::new(
        rect.min.x + p.x * rect.width(),
        rect.max.y - p.y * rect.height(),
    )
}

/// Inverse of [`normalized_to_screen`].
pub fn screen_to_normalized(rect: Rect, p: Vec2) -> Vec2 {
    let w = rect.width().max(1e-6);
    let h = rect.height().max(1e-6);
    Vec2::new((p.x - rect.min.x) / w, (rect.max.y - p.y) / h)
}

/// The interactive curve editor: the curve plus which handle is being dragged.
#[derive(Debug, Clone)]
pub struct CurveEditor {
    pub curve: BezierCurve,
    /// The control-point index currently grabbed, if any.
    pub dragging: Option<usize>,
    /// Handle pick radius in screen pixels.
    pub handle_radius: f32,
}

impl Default for CurveEditor {
    fn default() -> Self {
        CurveEditor {
            curve: BezierCurve::default(),
            dragging: None,
            handle_radius: 10.0,
        }
    }
}

impl CurveEditor {
    /// Screen-space rectangles for each control-point handle.
    pub fn handle_rects(&self, area: Rect) -> Vec<Rect> {
        self.curve
            .points
            .iter()
            .map(|&p| {
                let c = normalized_to_screen(area, p);
                Rect::from_min_size(
                    c - Vec2::splat(self.handle_radius),
                    Vec2::splat(self.handle_radius * 2.0),
                )
            })
            .collect()
    }

    /// Index of the handle under `pointer`, if any.
    pub fn pick_handle(&self, area: Rect, pointer: Vec2) -> Option<usize> {
        self.curve
            .points
            .iter()
            .enumerate()
            .map(|(i, &p)| (i, normalized_to_screen(area, p).distance(pointer)))
            .filter(|&(_, d)| d <= self.handle_radius)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(i, _)| i)
    }

    /// Drive interaction for one frame. `pressed` is the click edge, `down` is
    /// whether the pointer is held. Returns true if the curve changed.
    pub fn interact(&mut self, area: Rect, pointer: Vec2, pressed: bool, down: bool) -> bool {
        if pressed {
            self.dragging = self.pick_handle(area, pointer);
        }
        if !down {
            self.dragging = None;
            return false;
        }
        if let Some(idx) = self.dragging {
            self.curve
                .set_point(idx, screen_to_normalized(area, pointer));
            return true;
        }
        false
    }
}

/// A binding that maps a curve value to world gravity magnitude.
#[derive(Debug, Clone, Copy)]
pub struct GravityCurveBinding {
    /// Gravity magnitude when the curve value is `1.0`.
    pub max_gravity: f32,
    /// Parameter position `t` used to sample the curve for the current value.
    pub t: f32,
}

impl Default for GravityCurveBinding {
    fn default() -> Self {
        GravityCurveBinding {
            max_gravity: 20.0,
            t: 1.0,
        }
    }
}

impl GravityCurveBinding {
    /// Compute the downward gravity vector this curve currently dictates.
    pub fn gravity(&self, curve: &BezierCurve) -> Vec2 {
        Vec2::new(0.0, -curve.value_at(self.t) * self.max_gravity)
    }

    /// Push the curve's value into the live physics world (round-trip to Rust).
    pub fn apply(&self, curve: &BezierCurve, physics: &mut PhysicsWorld) {
        physics.gravity = self.gravity(curve);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_ecs::transform::Transform;
    use nova_ecs::{Vec3, World};
    use nova_physics::{step_physics, Collider2D, ColliderShape, RigidBody2D};

    #[test]
    fn bezier_endpoints_are_exact() {
        let c = BezierCurve::default();
        assert!(c.sample(0.0).abs_diff_eq(Vec2::new(0.0, 0.0), 1e-6));
        assert!(c.sample(1.0).abs_diff_eq(Vec2::new(1.0, 1.0), 1e-6));
    }

    #[test]
    fn screen_mapping_round_trips() {
        let area = Rect::from_min_size(Vec2::new(10.0, 20.0), Vec2::new(200.0, 100.0));
        let p = Vec2::new(0.3, 0.7);
        let s = normalized_to_screen(area, p);
        let back = screen_to_normalized(area, s);
        assert!(back.abs_diff_eq(p, 1e-5));
    }

    #[test]
    fn dragging_moves_the_grabbed_handle() {
        let area = Rect::from_min_size(Vec2::ZERO, Vec2::new(100.0, 100.0));
        let mut editor = CurveEditor::default();
        // Point 0 is at normalized (0,0) => screen (0,100).
        let grab = normalized_to_screen(area, editor.curve.points[0]);
        assert!(editor.interact(area, grab, true, true));
        // Drag to the middle of the area.
        editor.interact(area, Vec2::new(50.0, 50.0), false, true);
        assert!(editor.curve.points[0].abs_diff_eq(Vec2::new(0.5, 0.5), 1e-5));
    }

    #[test]
    fn gravity_binding_round_trips_into_physics() {
        let mut physics = PhysicsWorld::default();
        let mut curve = BezierCurve::default();
        let binding = GravityCurveBinding {
            max_gravity: 30.0,
            t: 1.0,
        };
        // Default curve value at t=1 is 1.0 => gravity y = -30.
        binding.apply(&curve, &mut physics);
        assert!((physics.gravity.y + 30.0).abs() < 1e-4);

        // Flatten the curve so the value drops; gravity should weaken.
        curve.set_point(2, Vec2::new(0.66, 0.0));
        curve.set_point(3, Vec2::new(1.0, 0.0));
        binding.apply(&curve, &mut physics);
        assert!(physics.gravity.y.abs() < 1e-4);
    }

    #[test]
    fn stronger_gravity_makes_bodies_fall_faster() {
        fn fall_distance(gravity_curve_value: f32) -> f32 {
            let mut world = World::new();
            let mut physics = PhysicsWorld::default();
            let mut curve = BezierCurve::default();
            // Force a constant value by flattening to the requested height.
            for i in 0..4 {
                curve.set_point(i, Vec2::new(i as f32 / 3.0, gravity_curve_value));
            }
            GravityCurveBinding {
                max_gravity: 20.0,
                t: 1.0,
            }
            .apply(&curve, &mut physics);
            world.add_resource(physics);

            let e = world.spawn();
            world.add_component(e, Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
            world.add_component(e, RigidBody2D::dynamic());
            world.add_component(e, Collider2D::new(ColliderShape::ball(0.5)));
            for _ in 0..60 {
                step_physics(&mut world, 1.0 / 60.0);
            }
            -world.get_component::<Transform>(e).unwrap().translation.y
        }

        let weak = fall_distance(0.25);
        let strong = fall_distance(1.0);
        assert!(strong > weak, "strong={strong} should exceed weak={weak}");
    }
}
