//! Appfront bridge (feature `appfront`): a scene as a `SceneDelegate`.
//!
//! [`AppfrontScene`] implements `tpt_appfront_canvas_3d::SceneDelegate`, so a
//! scene can be hosted in an appfront viewport. Picking goes through
//! [`crate::pick`] and is fully supported. Presenting is not: the GPU renderer has
//! no readback path into the appfront canvas, so `present` reports
//! `PresentStatus::Unsupported`. The host can still route pointer presses to 3D
//! objects.

use glam::Vec2;
use tpt_appfront_canvas_3d::{Hit3D, PresentStatus, SceneDelegate, ViewportPoint, ViewportSize};

use crate::{pick::pick, NodeId, Scene};

/// A scene viewed through one of its cameras, ready to be hosted in an appfront viewport.
pub struct AppfrontScene {
    scene: Scene,
    camera: NodeId,
    elapsed: f32,
}

impl AppfrontScene {
    /// Host `scene` through `camera`. Fails if `camera` is not a camera node.
    pub fn new(scene: Scene, camera: NodeId) -> Result<Self, &'static str> {
        if scene.camera(camera).is_none() {
            return Err("the viewport camera must be a camera node");
        }
        Ok(AppfrontScene {
            scene,
            camera,
            elapsed: 0.0,
        })
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// Seconds of simulation time the host has advanced.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }
}

impl SceneDelegate for AppfrontScene {
    fn update(&mut self, dt: f32) {
        self.elapsed += dt;
    }

    fn pick(&self, size: ViewportSize, at: ViewportPoint) -> Option<Hit3D> {
        let hit = pick(
            &self.scene,
            self.camera,
            Vec2::new(size.width, size.height),
            Vec2::new(at.x, at.y),
        )?;
        Some(Hit3D {
            id: u64::from(hit.node.0),
            point: hit.point.to_array(),
            distance: hit.distance,
        })
    }

    fn present(&mut self, _size: ViewportSize) -> PresentStatus {
        PresentStatus::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, PerspectiveCamera, Transform};
    use glam::Vec3;
    use tpt_appfront_canvas_3d::Viewport3D;

    fn scene_with_cube() -> (Scene, NodeId, NodeId) {
        let mut scene = Scene::new();
        let cam = scene.add_node(Node::camera(PerspectiveCamera::default()).with_transform(
            Transform::from_position(Vec3::new(0.0, 0.0, 5.0)).looking_at(Vec3::ZERO),
        ));
        scene.set_active_camera(Some(cam)).unwrap();
        let cube = scene.add_node(Node::cube());
        (scene, cam, cube)
    }

    #[test]
    fn a_pointer_press_on_the_cube_returns_its_node_id() {
        let (scene, cam, cube) = scene_with_cube();
        let vp = Viewport3D::new(
            AppfrontScene::new(scene, cam).unwrap(),
            ViewportSize {
                width: 800.0,
                height: 600.0,
            },
        );
        let hit = vp
            .pointer_down(ViewportPoint { x: 400.0, y: 300.0 })
            .expect("the centre of the view hits the cube");
        assert_eq!(hit.id, u64::from(cube.0));
        assert!(hit.distance > 0.0);
    }

    #[test]
    fn a_press_off_the_cube_picks_nothing() {
        let (scene, cam, _) = scene_with_cube();
        let vp = Viewport3D::new(
            AppfrontScene::new(scene, cam).unwrap(),
            ViewportSize {
                width: 800.0,
                height: 600.0,
            },
        );
        assert!(vp.pointer_down(ViewportPoint { x: 5.0, y: 5.0 }).is_none());
    }

    #[test]
    fn frames_advance_time_and_report_unsupported_presentation() {
        let (scene, cam, _) = scene_with_cube();
        let mut vp = Viewport3D::new(
            AppfrontScene::new(scene, cam).unwrap(),
            ViewportSize {
                width: 800.0,
                height: 600.0,
            },
        );
        assert_eq!(vp.frame(0.5), PresentStatus::Unsupported);
        vp.frame(0.25);
        assert!((vp.delegate().elapsed() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn a_non_camera_node_is_rejected_as_the_viewport_camera() {
        let (scene, _, cube) = scene_with_cube();
        assert!(AppfrontScene::new(scene, cube).is_err());
    }
}
