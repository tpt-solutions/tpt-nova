//! Node types: meshes, sprites, cameras, and lights.

use glam::{Mat4, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::id::NodeId;
use crate::transform::Transform;

/// What a node represents in the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Pure transform container (pivots, groups, sockets).
    Group,
    /// A renderable built-in mesh.
    Mesh(MeshNode),
    /// A 2D sprite drawn by the sprite pipeline.
    Sprite(SpriteNode),
    /// A perspective camera. A scene selects one as active.
    Camera(PerspectiveCamera),
    /// A light source (directional, point, or ambient).
    Light(LightSource),
}

impl NodeKind {
    pub fn is_mesh(&self) -> bool {
        matches!(self, NodeKind::Mesh(_))
    }

    pub fn is_sprite(&self) -> bool {
        matches!(self, NodeKind::Sprite(_))
    }

    pub fn is_camera(&self) -> bool {
        matches!(self, NodeKind::Camera(_))
    }

    pub fn is_light(&self) -> bool {
        matches!(self, NodeKind::Light(_))
    }

    /// A short stable label, used by the render plan and telemetry.
    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Group => "group",
            NodeKind::Mesh(_) => "mesh",
            NodeKind::Sprite(_) => "sprite",
            NodeKind::Camera(_) => "camera",
            NodeKind::Light(LightSource::Directional(_)) => "directional-light",
            NodeKind::Light(LightSource::Point(_)) => "point-light",
            NodeKind::Light(LightSource::Ambient(_)) => "ambient-light",
        }
    }
}

/// A node in the scene graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub transform: Transform,
    pub visible: bool,
    pub kind: NodeKind,
    /// Ordered child ids. Kept in sync by [`Scene`](crate::Scene).
    pub children: Vec<NodeId>,
    /// Parent id, or `None` for a root. Kept in sync by [`Scene`](crate::Scene).
    pub parent: Option<NodeId>,
}

impl Node {
    pub fn group() -> Self {
        Node::new(NodeKind::Group)
    }

    pub fn mesh(mesh: MeshNode) -> Self {
        Node::new(NodeKind::Mesh(mesh))
    }

    /// A single built-in unit cube — the renderer's only `MeshKind` today.
    pub fn cube() -> Self {
        Node::mesh(MeshNode::cube())
    }

    pub fn sprite(sprite: SpriteNode) -> Self {
        Node::new(NodeKind::Sprite(sprite))
    }

    pub fn camera(camera: PerspectiveCamera) -> Self {
        Node::new(NodeKind::Camera(camera))
    }

    pub fn light(light: impl Into<LightSource>) -> Self {
        Node::new(NodeKind::Light(light.into()))
    }

    fn new(kind: NodeKind) -> Self {
        Node {
            name: String::new(),
            transform: Transform::IDENTITY,
            visible: true,
            kind,
            children: Vec::new(),
            parent: None,
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn at(mut self, position: Vec3) -> Self {
        self.transform.position = position;
        self
    }

    pub fn rotated(mut self, rotation: glam::Quat) -> Self {
        self.transform.rotation = rotation;
        self
    }

    pub fn scaled(mut self, scale: Vec3) -> Self {
        self.transform.scale = scale;
        self
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }

    /// The node's authored name, falling back to its kind label.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.kind.label().to_string()
        } else {
            self.name.clone()
        }
    }
}

/// A built-in mesh. `label` records the source asset for tooling, while `kind`
/// selects the geometry the renderer already knows how to build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshNode {
    pub kind: tpt_nova_ecs::MeshKind,
    pub label: Option<String>,
}

impl MeshNode {
    pub fn cube() -> Self {
        MeshNode {
            kind: tpt_nova_ecs::MeshKind::Cube,
            label: None,
        }
    }

    /// Tag the mesh with a source label (e.g. the `.glb` it came from) for the
    /// inspector and telemetry. This does not load the asset.
    pub fn from_label(label: impl Into<String>) -> Self {
        MeshNode {
            kind: tpt_nova_ecs::MeshKind::Cube,
            label: Some(label.into()),
        }
    }

    pub fn to_ecs(&self) -> tpt_nova_ecs::Mesh {
        tpt_nova_ecs::Mesh { kind: self.kind }
    }
}

/// Sprite parameters, mapped onto `tpt-nova-render`'s `Sprite` component.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpriteNode {
    /// Atlas region id.
    pub region: u32,
    /// Size in world/screen units.
    pub size: Vec2,
    /// RGBA tint multiplied with the atlas texel.
    pub color: [f32; 4],
    /// Draw order; higher draws on top.
    pub z: f32,
}

impl Default for SpriteNode {
    fn default() -> Self {
        SpriteNode {
            region: 0,
            size: Vec2::splat(32.0),
            color: [1.0; 4],
            z: 0.0,
        }
    }
}

impl SpriteNode {
    pub fn new(region: u32, size: Vec2) -> Self {
        SpriteNode {
            region,
            size,
            ..Default::default()
        }
    }

    pub fn with_tint(mut self, color: [f32; 4]) -> Self {
        self.color = color;
        self
    }

    pub fn with_z(mut self, z: f32) -> Self {
        self.z = z;
        self
    }

    pub fn to_ecs(&self) -> tpt_nova_render::Sprite {
        tpt_nova_render::Sprite {
            region: self.region,
            size: self.size,
            color: self.color,
            z: self.z,
        }
    }
}

/// A perspective camera. `fov_y_degrees` is authored in degrees because that is
/// how designers think about field of view; the ECS projection takes radians.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerspectiveCamera {
    pub fov_y_degrees: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for PerspectiveCamera {
    fn default() -> Self {
        PerspectiveCamera {
            fov_y_degrees: 60.0,
            aspect: 16.0 / 9.0,
            near: 0.01,
            far: 1000.0,
        }
    }
}

impl PerspectiveCamera {
    pub fn new(fov_y_degrees: f32, aspect: f32, near: f32, far: f32) -> Self {
        PerspectiveCamera {
            fov_y_degrees,
            aspect,
            near,
            far,
        }
    }

    pub fn with_aspect(mut self, aspect: f32) -> Self {
        self.aspect = aspect;
        self
    }

    pub fn with_fov_degrees(mut self, fov_y_degrees: f32) -> Self {
        self.fov_y_degrees = fov_y_degrees;
        self
    }

    /// Projection matrix for the authored aspect (or a render-target override).
    pub fn projection(&self, aspect_override: Option<f32>) -> Mat4 {
        Mat4::perspective_rh(
            self.fov_y_degrees.to_radians(),
            aspect_override.unwrap_or(self.aspect).max(1e-4),
            self.near,
            self.far,
        )
    }

    /// View-projection for a camera whose world matrix is `world`.
    pub fn view_proj(&self, world: Mat4, aspect_override: Option<f32>) -> Mat4 {
        self.projection(aspect_override) * world.inverse()
    }

    pub fn to_ecs(&self) -> tpt_nova_ecs::Camera {
        tpt_nova_ecs::Camera {
            fov_y: self.fov_y_degrees.to_radians(),
            near: self.near,
            far: self.far,
            aspect: self.aspect,
        }
    }

    pub fn from_ecs(cam: &tpt_nova_ecs::Camera) -> Self {
        PerspectiveCamera {
            fov_y_degrees: cam.fov_y.to_degrees(),
            aspect: cam.aspect,
            near: cam.near,
            far: cam.far,
        }
    }
}

/// Sun-like light: parallel rays along `direction` (world space).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DirectionalLight {
    pub color: Color,
    pub intensity: f32,
    /// Direction the light *travels* (from the sun toward the scene).
    pub direction: Vec3,
    pub cast_shadows: bool,
    /// Half-size of the orthographic shadow frustum, in world units.
    pub shadow_extent: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        DirectionalLight {
            color: Color::WHITE,
            intensity: 1.0,
            direction: Vec3::new(-0.4, -0.8, -0.3).normalize(),
            cast_shadows: false,
            shadow_extent: 20.0,
        }
    }
}

impl DirectionalLight {
    pub fn new(direction: Vec3, intensity: f32) -> Self {
        DirectionalLight {
            direction: direction.normalize_or_zero(),
            intensity,
            ..Default::default()
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_shadows(mut self) -> Self {
        self.cast_shadows = true;
        self
    }

    pub fn to_ecs(&self) -> tpt_nova_ecs::Light {
        tpt_nova_ecs::Light {
            kind: tpt_nova_ecs::LightKind::Directional,
            color: self.color.to_vec3(),
            intensity: self.intensity,
            range: 0.0,
            cast_shadows: self.cast_shadows,
            shadow_extent: self.shadow_extent,
        }
    }
}

/// Light radiating from a point, fading to zero at `range`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointLight {
    pub color: Color,
    pub intensity: f32,
    pub range: f32,
    pub cast_shadows: bool,
}

impl Default for PointLight {
    fn default() -> Self {
        PointLight {
            color: Color::WHITE,
            intensity: 1.0,
            range: 10.0,
            cast_shadows: false,
        }
    }
}

impl PointLight {
    pub fn new(intensity: f32, range: f32) -> Self {
        PointLight {
            intensity,
            range,
            ..Default::default()
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn to_ecs(&self) -> tpt_nova_ecs::Light {
        tpt_nova_ecs::Light {
            kind: tpt_nova_ecs::LightKind::Point,
            color: self.color.to_vec3(),
            intensity: self.intensity,
            range: self.range,
            cast_shadows: self.cast_shadows,
            shadow_extent: 0.0,
        }
    }
}

/// Uniform ambient fill. There is no ECS ambient component, so this only feeds
/// [`RenderPlan`](crate::RenderPlan); the PBR shader's own ambient term is used
/// at draw time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AmbientLight {
    pub color: Color,
    pub intensity: f32,
}

impl Default for AmbientLight {
    fn default() -> Self {
        AmbientLight {
            color: Color::rgb(0.16, 0.18, 0.22),
            intensity: 1.0,
        }
    }
}

impl AmbientLight {
    pub fn new(color: Color, intensity: f32) -> Self {
        AmbientLight { color, intensity }
    }

    /// The ambient color scaled by intensity, as a linear RGB triple.
    pub fn radiance(&self) -> Vec3 {
        self.color.to_vec3() * self.intensity
    }
}

/// Any light a [`Node`] can hold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightSource {
    Directional(DirectionalLight),
    Point(PointLight),
    Ambient(AmbientLight),
}

impl From<DirectionalLight> for LightSource {
    fn from(l: DirectionalLight) -> Self {
        LightSource::Directional(l)
    }
}

impl From<PointLight> for LightSource {
    fn from(l: PointLight) -> Self {
        LightSource::Point(l)
    }
}

impl From<AmbientLight> for LightSource {
    fn from(l: AmbientLight) -> Self {
        LightSource::Ambient(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_builders_set_transform_and_name() {
        let node = Node::cube().named("Crate").at(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(node.display_name(), "Crate");
        assert_eq!(node.transform.position, Vec3::new(1.0, 2.0, 3.0));
        assert!(node.kind.is_mesh());
        assert!(node.visible);
    }

    #[test]
    fn light_nodes_convert_to_ecs() {
        let dir = DirectionalLight::new(Vec3::NEG_Y, 2.0).with_shadows();
        let light = dir.to_ecs();
        assert_eq!(light.kind, tpt_nova_ecs::LightKind::Directional);
        assert_eq!(light.intensity, 2.0);
        assert!(light.cast_shadows);
        assert!(matches!(
            LightSource::from(dir),
            LightSource::Directional(_)
        ));
    }

    #[test]
    fn camera_projection_uses_authored_aspect() {
        let cam = PerspectiveCamera::default();
        let proj = cam.projection(None);
        assert!(proj.is_finite());
        let wide = cam.projection(Some(2.0));
        assert_ne!(proj, wide);
    }

    #[test]
    fn camera_round_trips_through_ecs() {
        let cam = PerspectiveCamera::new(45.0, 4.0 / 3.0, 0.5, 500.0);
        let back = PerspectiveCamera::from_ecs(&cam.to_ecs());
        assert!((back.fov_y_degrees - 45.0).abs() < 1e-4);
        assert_eq!(back.near, 0.5);
        assert_eq!(back.far, 500.0);
    }
}
