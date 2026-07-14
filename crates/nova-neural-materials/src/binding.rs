//! Binding a live neural material onto an in-scene surface.
//!
//! `NeuralMaterialRegistry` owns the streamed frames and `NeuralTexture`
//! uploads them to the GPU. This module is the missing link: a
//! [`MaterialBinding`] records "material `<id>` drives entity `<target>`", so a
//! renderer can, each frame, pull `registry.latest(id)` and upload it onto that
//! entity's PBR material. With this in place the engine can paint a live
//! video-LLM texture onto an actual mesh rather than only holding frames in the
//! registry.

use nova_ecs::Entity;

use crate::frame::Frame;
use crate::registry::NeuralMaterialRegistry;

/// A single "material drives entity" association.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialBinding {
    /// The registered material id (matches a [`crate::MaterialPrompt`] id in the
    /// [`NeuralMaterialRegistry`]).
    pub material_id: String,
    /// The entity whose PBR material should receive this material's latest frame.
    pub target: Entity,
}

impl MaterialBinding {
    pub fn new(material_id: impl Into<String>, target: Entity) -> Self {
        MaterialBinding {
            material_id: material_id.into(),
            target,
        }
    }

    /// The latest frame for the bound material, if the registry has produced one.
    /// A renderer calls this each frame and uploads the result onto `target`.
    pub fn latest_frame<'a>(&self, registry: &'a NeuralMaterialRegistry) -> Option<&'a Frame> {
        registry.latest(&self.material_id)
    }
}

/// An ECS resource (or host-owned collection) of every active material binding.
#[derive(Debug, Clone, Default)]
pub struct MaterialBindings {
    bindings: Vec<MaterialBinding>,
}

impl MaterialBindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `material_id` should drive `target`'s surface.
    pub fn bind(&mut self, material_id: impl Into<String>, target: Entity) {
        self.bindings
            .push(MaterialBinding::new(material_id, target));
    }

    /// All active bindings.
    pub fn iter(&self) -> impl Iterator<Item = &MaterialBinding> {
        self.bindings.iter()
    }

    /// The bindings whose target is `entity` (an entity may be driven by several
    /// materials, e.g. base color + emissive).
    pub fn bindings_for(&self, target: Entity) -> Vec<&MaterialBinding> {
        self.bindings
            .iter()
            .filter(|b| b.target == target)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{FeedSource, MaterialPrompt};
    use nova_ecs::Entity;

    #[test]
    fn binding_resolves_latest_frame() {
        let mut reg = NeuralMaterialRegistry::default();
        let prompt = MaterialPrompt::new("billboard", "neon rain", FeedSource::CaptureDevice(0))
            .with_resolution(4, 4);
        reg.register(prompt).unwrap();
        reg.update();

        let target = Entity {
            index: 3,
            generation: 0,
        };
        let binding = MaterialBinding::new("billboard", target);
        let frame = binding.latest_frame(&reg).expect("frame should exist");
        assert_eq!((frame.width, frame.height), (4, 4));
    }

    #[test]
    fn bindings_collection_tracks_per_entity() {
        let mut bindings = MaterialBindings::new();
        assert!(bindings.is_empty());
        let a = Entity {
            index: 1,
            generation: 0,
        };
        let b = Entity {
            index: 2,
            generation: 0,
        };
        bindings.bind("color", a);
        bindings.bind("emissive", a);
        bindings.bind("color", b);
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings.bindings_for(a).len(), 2);
        assert_eq!(bindings.bindings_for(b).len(), 1);
    }
}
