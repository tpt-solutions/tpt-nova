//! The scene container: node arena, hierarchy, active camera, and environment.

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::id::NodeId;
use crate::node::{AmbientLight, Node, NodeKind, PerspectiveCamera};
use crate::post::{PostEffectKind, PostProcessEffect, PostProcessPipeline};

/// A declarative 3D scene.
///
/// The scene is a plain in-memory graph (spec3 §7 default): cheap to build,
/// serializable, and independent of the ECS. Call [`bake`](crate::bake) to turn
/// it into the components `tpt-nova-render`'s PBR and sprite pipelines read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    nodes: Vec<Option<Node>>,
    roots: Vec<NodeId>,
    active_camera: Option<NodeId>,
    ambient: AmbientLight,
    background: Color,
    post: PostProcessPipeline,
}

impl Default for Scene {
    fn default() -> Self {
        Scene::new()
    }
}

impl Scene {
    /// An empty scene with "stunning defaults" post-processing enabled and the
    /// renderer's dark blue-gray clear color.
    pub fn new() -> Self {
        Scene {
            nodes: Vec::new(),
            roots: Vec::new(),
            active_camera: None,
            ambient: AmbientLight::default(),
            background: Color::rgb(0.05, 0.07, 0.10),
            post: PostProcessPipeline::stunning_defaults(),
        }
    }

    pub fn with_background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    // ---- nodes ---------------------------------------------------------

    /// Add a root node.
    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Some(node));
        self.roots.push(id);
        id
    }

    /// Add `node` as the last child of `parent`.
    pub fn add_child(&mut self, parent: NodeId, node: Node) -> Result<NodeId, SceneError> {
        self.require(parent)?;
        // Allocate directly rather than via `add_node`, which would also list the
        // child as a root and make it visited twice by traversals.
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Some(node));
        self.attach(parent, id);
        Ok(id)
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0 as usize)?.as_mut()
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.node_count() == 0
    }

    /// Iterate `(id, node)` over every live node, in creation order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.as_ref().map(|n| (NodeId(i as u32), n)))
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn children(&self, id: NodeId) -> Option<&[NodeId]> {
        self.get(id).map(|n| n.children.as_slice())
    }

    /// Live node ids in depth-first order (roots, then descendants).
    pub fn depth_first(&self) -> Vec<NodeId> {
        let mut out = Vec::with_capacity(self.node_count());
        for &root in &self.roots {
            self.push_depth_first(root, &mut out);
        }
        out
    }

    fn push_depth_first(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let Some(node) = self.get(id) else { return };
        out.push(id);
        for &child in &node.children {
            self.push_depth_first(child, out);
        }
    }

    /// Remove `id` and its whole subtree. Returns `false` for unknown ids.
    pub fn remove(&mut self, id: NodeId) -> bool {
        let Some(node) = self.get(id) else {
            return false;
        };
        let parent = node.parent;
        // Collect the subtree before mutating anything.
        let mut doomed = Vec::new();
        self.collect_subtree(id, &mut doomed);

        if let Some(parent) = parent {
            if let Some(p) = self.get_mut(parent) {
                p.children.retain(|c| *c != id);
            }
        } else {
            self.roots.retain(|r| *r != id);
        }
        for d in doomed {
            self.nodes[d.0 as usize] = None;
        }
        if let Some(active) = self.active_camera {
            if !self.contains(active) {
                self.active_camera = None;
            }
        }
        true
    }

    fn collect_subtree(&self, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        if let Some(node) = self.get(id) {
            let children = node.children.clone();
            for child in children {
                self.collect_subtree(child, out);
            }
        }
    }

    /// Move `child` under `parent` (or make it a root when `parent` is `None`).
    ///
    /// Rejects self-parenting and cycles; a node can never become its own
    /// ancestor, so [`Scene::world_matrix`] always terminates.
    pub fn set_parent(&mut self, child: NodeId, parent: Option<NodeId>) -> Result<(), SceneError> {
        self.require(child)?;
        if let Some(parent) = parent {
            self.require(parent)?;
            if parent == child {
                return Err(SceneError::SelfParent(child));
            }
            if self.is_ancestor(child, parent) {
                return Err(SceneError::Cycle { child, parent });
            }
        }
        self.detach(child);
        match parent {
            Some(parent) => self.attach(parent, child),
            None => {
                self.roots.push(child);
                if let Some(node) = self.get_mut(child) {
                    node.parent = None;
                }
            }
        }
        Ok(())
    }

    fn attach(&mut self, parent: NodeId, child: NodeId) {
        if let Some(p) = self.get_mut(parent) {
            p.children.push(child);
        }
        if let Some(c) = self.get_mut(child) {
            c.parent = Some(parent);
        }
    }

    fn detach(&mut self, child: NodeId) {
        let Some(parent) = self.get(child).and_then(|n| n.parent) else {
            self.roots.retain(|r| *r != child);
            return;
        };
        if let Some(p) = self.get_mut(parent) {
            p.children.retain(|c| *c != child);
        }
    }

    /// Is `maybe_ancestor` on `id`'s parent chain?
    pub fn is_ancestor(&self, maybe_ancestor: NodeId, id: NodeId) -> bool {
        let mut cursor = Some(id);
        let mut guard = 0;
        while let Some(current) = cursor {
            if current == maybe_ancestor {
                return true;
            }
            guard += 1;
            if guard > 4096 {
                break;
            }
            cursor = self.get(current).and_then(|n| n.parent);
        }
        false
    }

    fn require(&self, id: NodeId) -> Result<&Node, SceneError> {
        self.get(id).ok_or(SceneError::UnknownNode(id))
    }

    // ---- transforms ----------------------------------------------------

    /// The node's world matrix, accumulated down its parent chain.
    pub fn world_matrix(&self, id: NodeId) -> Option<Mat4> {
        self.get(id)?;
        let mut chain = Vec::new();
        let mut cursor = Some(id);
        let mut guard = 0;
        while let Some(current) = cursor {
            chain.push(current);
            guard += 1;
            if guard > 4096 {
                break;
            }
            cursor = self.get(current).and_then(|n| n.parent);
        }
        let mut world = Mat4::IDENTITY;
        for node_id in chain.iter().rev() {
            if let Some(node) = self.get(*node_id) {
                world *= node.transform.matrix();
            }
        }
        Some(world)
    }

    pub fn world_position(&self, id: NodeId) -> Option<Vec3> {
        self.world_matrix(id).map(|m| m.w_axis.truncate())
    }

    /// Whether `id` and every ancestor is visible.
    pub fn visible_in_tree(&self, id: NodeId) -> bool {
        let mut cursor = Some(id);
        let mut guard = 0;
        while let Some(current) = cursor {
            let Some(node) = self.get(current) else {
                return false;
            };
            if !node.visible {
                return false;
            }
            guard += 1;
            if guard > 4096 {
                return false;
            }
            cursor = node.parent;
        }
        true
    }

    // ---- cameras -------------------------------------------------------

    /// Choose the camera the renderer should look through.
    ///
    /// Pass `None` to fall back to the first visible camera node. Selecting a
    /// non-camera node is an error rather than a silent no-op, so misuse shows
    /// up in tests instead of a black viewport.
    pub fn set_active_camera(&mut self, id: Option<NodeId>) -> Result<(), SceneError> {
        match id {
            Some(id) => match self.require(id)?.kind {
                NodeKind::Camera(_) => {
                    self.active_camera = Some(id);
                    Ok(())
                }
                _ => Err(SceneError::NotACamera(id)),
            },
            None => {
                self.active_camera = None;
                Ok(())
            }
        }
    }

    /// The explicitly selected camera, or the first visible camera node.
    pub fn active_camera(&self) -> Option<NodeId> {
        if let Some(id) = self.active_camera {
            if self.get(id).map(|n| n.kind.is_camera()).unwrap_or(false) {
                return Some(id);
            }
        }
        self.iter()
            .find(|(id, n)| n.kind.is_camera() && self.visible_in_tree(*id))
            .map(|(id, _)| id)
    }

    pub fn camera(&self, id: NodeId) -> Option<&PerspectiveCamera> {
        match &self.get(id)?.kind {
            NodeKind::Camera(cam) => Some(cam),
            _ => None,
        }
    }

    pub fn camera_mut(&mut self, id: NodeId) -> Option<&mut PerspectiveCamera> {
        match &mut self.get_mut(id)?.kind {
            NodeKind::Camera(cam) => Some(cam),
            _ => None,
        }
    }

    /// Override the active camera's aspect ratio (e.g. when its viewport is
    /// resized). No-op when there is no active camera.
    pub fn set_active_camera_aspect(&mut self, aspect: f32) {
        if let Some(id) = self.active_camera() {
            if let Some(cam) = self.camera_mut(id) {
                cam.aspect = aspect.max(1e-4);
            }
        }
    }

    // ---- environment & post-processing ---------------------------------

    pub fn ambient(&self) -> AmbientLight {
        self.ambient
    }

    pub fn set_ambient(&mut self, ambient: AmbientLight) {
        self.ambient = ambient;
    }

    pub fn background(&self) -> Color {
        self.background
    }

    pub fn set_background(&mut self, background: Color) {
        self.background = background;
    }

    pub fn post_process(&self) -> &PostProcessPipeline {
        &self.post
    }

    pub fn post_process_mut(&mut self) -> &mut PostProcessPipeline {
        &mut self.post
    }

    /// Enable an effect (replacing any existing effect of the same kind).
    pub fn enable_post_process(&mut self, effect: PostProcessEffect) {
        self.post.enable(effect);
    }

    /// Disable an effect by kind.
    pub fn disable_post_process(&mut self, kind: PostEffectKind) -> bool {
        self.post.disable(kind)
    }
}

/// Errors from scene edits. Every one is a caller bug that is cheap to detect
/// up front, so the scene returns them instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    #[error("node {0} is not a camera")]
    NotACamera(NodeId),
    #[error("node {0} cannot be its own parent")]
    SelfParent(NodeId),
    #[error("cannot parent {child} under its own descendant {parent}")]
    Cycle { child: NodeId, parent: NodeId },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::Node;

    fn camera_node() -> Node {
        Node::camera(PerspectiveCamera::default())
    }

    #[test]
    fn hierarchy_world_matrices_compose() {
        let mut scene = Scene::new();
        let root = scene.add_node(Node::group().at(Vec3::new(0.0, 1.0, 0.0)));
        let child = scene
            .add_child(root, Node::cube().at(Vec3::new(0.0, 0.0, -2.0)))
            .unwrap();
        let grandchild = scene
            .add_child(child, Node::cube().at(Vec3::new(1.0, 0.0, 0.0)))
            .unwrap();

        assert_eq!(scene.children(root).unwrap(), &[child]);
        assert_eq!(scene.get(child).unwrap().parent, Some(root));
        assert_eq!(
            scene.world_position(grandchild).unwrap(),
            Vec3::new(1.0, 1.0, -2.0)
        );
        assert_eq!(scene.depth_first(), vec![root, child, grandchild]);
    }

    #[test]
    fn reparent_moves_subtree_and_rejects_cycles() {
        let mut scene = Scene::new();
        let a = scene.add_node(Node::group());
        let b = scene.add_child(a, Node::group()).unwrap();
        let c = scene.add_node(Node::cube());

        scene.set_parent(b, Some(c)).unwrap();
        assert_eq!(scene.get(b).unwrap().parent, Some(c));
        assert!(scene.children(a).unwrap().is_empty());
        assert_eq!(scene.children(c).unwrap(), &[b]);

        // A child of `b` cannot become `b`'s parent.
        let d = scene.add_child(b, Node::group()).unwrap();
        assert_eq!(
            scene.set_parent(b, Some(d)),
            Err(SceneError::Cycle {
                child: b,
                parent: d
            })
        );
    }

    #[test]
    fn cycle_is_rejected() {
        let mut scene = Scene::new();
        let a = scene.add_node(Node::group());
        let b = scene.add_child(a, Node::group()).unwrap();
        assert_eq!(
            scene.set_parent(a, Some(b)),
            Err(SceneError::Cycle {
                child: a,
                parent: b
            })
        );
        assert_eq!(scene.set_parent(a, Some(a)), Err(SceneError::SelfParent(a)));
        assert_eq!(scene.world_matrix(a), Some(Mat4::IDENTITY));
    }

    #[test]
    fn remove_deletes_the_whole_subtree() {
        let mut scene = Scene::new();
        let a = scene.add_node(Node::group());
        let b = scene.add_child(a, Node::group()).unwrap();
        let _c = scene.add_child(b, Node::cube()).unwrap();
        assert_eq!(scene.node_count(), 3);
        assert!(scene.remove(a));
        assert_eq!(scene.node_count(), 0);
        assert!(scene.roots().is_empty());
        assert!(scene.get(a).is_none());
        assert!(scene.get(b).is_none());
    }

    #[test]
    fn active_camera_falls_back_and_validates() {
        let mut scene = Scene::new();
        let cube = scene.add_node(Node::cube());
        assert_eq!(scene.active_camera(), None);
        assert_eq!(
            scene.set_active_camera(Some(cube)),
            Err(SceneError::NotACamera(cube))
        );

        let cam = scene.add_node(camera_node());
        assert_eq!(scene.active_camera(), Some(cam));
        let cam2 = scene.add_node(camera_node());
        scene.set_active_camera(Some(cam2)).unwrap();
        assert_eq!(scene.active_camera(), Some(cam2));

        scene.get_mut(cam2).unwrap().visible = false;
        // Explicit selection still wins while the node exists.
        assert_eq!(scene.active_camera(), Some(cam2));
    }

    #[test]
    fn hidden_ancestor_hides_children() {
        let mut scene = Scene::new();
        let root = scene.add_node(Node::group());
        let child = scene.add_child(root, Node::cube()).unwrap();
        assert!(scene.visible_in_tree(child));
        scene.get_mut(root).unwrap().visible = false;
        assert!(!scene.visible_in_tree(child));
    }

    #[test]
    fn post_process_defaults_are_stunning() {
        let mut scene = Scene::new();
        let pipeline = scene.post_process();
        assert!(pipeline.is_enabled(PostEffectKind::Bloom));
        assert!(pipeline.is_enabled(PostEffectKind::ToneMapping));
        assert!(pipeline.is_enabled(PostEffectKind::Fxaa));

        assert!(scene.disable_post_process(PostEffectKind::Fxaa));
        assert!(!scene.post_process().is_enabled(PostEffectKind::Fxaa));
        scene.enable_post_process(PostProcessEffect::Fxaa);
        assert!(scene.post_process().is_enabled(PostEffectKind::Fxaa));
    }

    #[test]
    fn ambient_is_scene_level() {
        let mut scene = Scene::new();
        assert_eq!(scene.ambient().intensity, 1.0);
        scene.set_ambient(AmbientLight::new(Color::WHITE, 0.5));
        assert_eq!(scene.ambient().intensity, 0.5);
    }
}
