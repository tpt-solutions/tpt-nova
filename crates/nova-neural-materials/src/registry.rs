//! Engine-facing registry of active neural materials (an ECS resource).

use std::collections::HashMap;

use crate::frame::Frame;
use crate::prompt::MaterialPrompt;
use crate::provider::{MockProvider, NeuralMaterialProvider, ProviderError};
use crate::source::FrameSource;
use crate::texture::NeuralTexture;

struct Entry {
    #[allow(dead_code)]
    prompt: MaterialPrompt,
    source: Box<dyn FrameSource>,
    latest: Option<Frame>,
}

/// Tracks every live neural material, polls its feed each tick, and keeps the
/// most recent [`Frame`] available for GPU upload.
///
/// Store this as a `World` resource. Call [`update`](Self::update) once per tick
/// (after the feed may have produced frames), then map [`latest`](Self::latest)
/// onto a [`NeuralTexture`] for the matching material.
pub struct NeuralMaterialRegistry {
    provider: Box<dyn NeuralMaterialProvider>,
    materials: HashMap<String, Entry>,
}

impl Default for NeuralMaterialRegistry {
    fn default() -> Self {
        NeuralMaterialRegistry::new(Box::new(MockProvider))
    }
}

impl NeuralMaterialRegistry {
    pub fn new(provider: Box<dyn NeuralMaterialProvider>) -> Self {
        NeuralMaterialRegistry {
            provider,
            materials: HashMap::new(),
        }
    }

    /// Swap in a different provider (e.g. a real Video LLM client). Existing
    /// materials keep their feeds; new registrations use the new provider.
    pub fn set_provider(&mut self, provider: Box<dyn NeuralMaterialProvider>) {
        self.provider = provider;
    }

    /// Open a feed described by `prompt` and begin tracking it under `prompt.id`.
    pub fn register(&mut self, prompt: MaterialPrompt) -> Result<(), ProviderError> {
        let source = self.provider.open(&prompt)?;
        self.materials.insert(
            prompt.id.clone(),
            Entry {
                prompt,
                source,
                latest: None,
            },
        );
        Ok(())
    }

    /// Whether a material with the given id is currently tracked.
    pub fn contains(&self, id: &str) -> bool {
        self.materials.contains_key(id)
    }

    /// Poll every feed once and store the newest frame each produced.
    pub fn update(&mut self) {
        for entry in self.materials.values_mut() {
            if let Some(frame) = entry.source.next_frame() {
                entry.latest = Some(frame);
            }
        }
    }

    /// The most recent frame for a material, if any has arrived yet.
    pub fn latest(&self, id: &str) -> Option<&Frame> {
        self.materials.get(id).and_then(|e| e.latest.as_ref())
    }

    /// Timestamp (ms) of the latest frame for a material, if any.
    pub fn latest_timestamp(&self, id: &str) -> Option<u64> {
        self.materials
            .get(id)
            .and_then(|e| e.latest.as_ref())
            .map(|f| f.timestamp_ms)
    }

    /// Copy the latest frame for `id` onto a GPU [`NeuralTexture`].
    pub fn upload(
        &self,
        id: &str,
        queue: &wgpu::Queue,
        texture: &NeuralTexture,
    ) -> Result<(), crate::frame::FrameError> {
        match self.latest(id) {
            Some(frame) => texture.upload(queue, frame),
            None => Ok(()),
        }
    }
}
