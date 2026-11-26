//! Appfront bridge (feature `appfront`): a scene as a `SceneDelegate`.
//!
//! [`AppfrontScene`] implements `tpt_appfront_canvas_3d::SceneDelegate`, so a
//! scene can be hosted in an appfront viewport.
//!
//! - **Picking** goes through [`crate::pick`].
//! - **Presenting** renders the scene offscreen on the GPU
//!   (`tpt_nova_render::OffscreenRenderer`) and keeps the frame. The host reads it
//!   with [`AppfrontScene::last_frame`] and draws it into the viewport. The
//!   appfront contract only reports a status, so the pixels travel this way, not
//!   through `present`'s return value. With no GPU adapter, `present` reports
//!   `PresentStatus::Unsupported` and picking still works.

use glam::Vec2;
use tpt_appfront_canvas_3d::{Hit3D, PresentStatus, SceneDelegate, ViewportPoint, ViewportSize};
use tpt_nova_ecs::world::World;
use tpt_nova_render::{Frame, OffscreenRenderer};

use crate::{bake::bake, pick::pick, NodeId, Scene};

/// A scene viewed through one of its cameras, ready to be hosted in an appfront viewport.
pub struct AppfrontScene {
    scene: Scene,
    camera: NodeId,
    elapsed: f32,
    /// Created on the first frame, so a host that never presents never touches the GPU.
    renderer: Option<OffscreenRenderer>,
    /// Set after a failed attempt, so `present` does not retry the GPU every frame.
    renderer_failed: bool,
    last_frame: Option<Frame>,
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
            renderer: None,
            renderer_failed: false,
            last_frame: None,
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

    /// The most recent frame `present` rendered, or `None` if none has been
    /// rendered (no GPU, or `present` not yet called).
    pub fn last_frame(&self) -> Option<&Frame> {
        self.last_frame.as_ref()
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

    fn present(&mut self, size: ViewportSize) -> PresentStatus {
        // Native only: creating the renderer blocks on the GPU, which a browser
        // cannot do. On the web, presenting reports `Unsupported` for now.
        #[cfg(not(target_arch = "wasm32"))]
        if self.renderer.is_none() && !self.renderer_failed {
            match OffscreenRenderer::new() {
                Ok(r) => self.renderer = Some(r),
                Err(_) => self.renderer_failed = true,
            }
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return PresentStatus::Unsupported;
        };
        let width = size.width.round().max(1.0) as u32;
        let height = size.height.round().max(1.0) as u32;

        let mut world = World::new();
        bake(&self.scene).spawn_into(&mut world);
        match renderer.render(&world, width, height) {
            Ok(frame) => {
                self.last_frame = Some(frame);
                PresentStatus::Presented
            }
            Err(_) => PresentStatus::Unsupported,
        }
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
    fn frames_advance_time() {
        let (scene, cam, _) = scene_with_cube();
        let mut vp = Viewport3D::new(
            AppfrontScene::new(scene, cam).unwrap(),
            ViewportSize {
                width: 800.0,
                height: 600.0,
            },
        );
        vp.frame(0.5);
        vp.frame(0.25);
        assert!((vp.delegate().elapsed() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn present_renders_the_cube_into_a_frame_the_host_can_read() {
        let (scene, cam, _) = scene_with_cube();
        let mut vp = Viewport3D::new(
            AppfrontScene::new(scene, cam).unwrap(),
            ViewportSize {
                width: 96.0,
                height: 64.0,
            },
        );
        let status = vp.frame(0.0);
        if status == PresentStatus::Unsupported {
            // No GPU adapter here: presenting is reported as unsupported, and no frame exists.
            assert!(vp.delegate().last_frame().is_none());
            return;
        }
        assert_eq!(status, PresentStatus::Presented);
        let frame = vp
            .delegate()
            .last_frame()
            .expect("a presented frame is kept");
        assert_eq!((frame.width, frame.height), (96, 64));
        let background = frame.pixel(0, 0).unwrap();
        assert_ne!(
            frame.pixel(48, 32).unwrap(),
            background,
            "the cube is at the centre of the view"
        );
    }

    #[test]
    fn a_non_camera_node_is_rejected_as_the_viewport_camera() {
        let (scene, _, cube) = scene_with_cube();
        assert!(AppfrontScene::new(scene, cube).is_err());
    }
}
