//! A request to a generative feed: *what* to generate and at what resolution.

use serde::{Deserialize, Serialize};

/// Where a neural material's pixels come from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FeedSource {
    /// A Video LLM endpoint that turns `prompt` into a live clip (URL/identifier).
    VideoLlm { endpoint: String },
    /// A local capture device (webcam / capture card), addressed by index.
    CaptureDevice(u32),
    /// A looping local media file.
    File { path: String },
}

/// A declarative description of a neural material feed.
///
/// This is the surface an external AI agent fills in; the engine hands it to a
/// [`NeuralMaterialProvider`](crate::NeuralMaterialProvider) which is
/// responsible for producing matching [`Frame`](crate::Frame)s.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaterialPrompt {
    /// Stable identifier used to look the material up later (e.g. "billboard_01").
    pub id: String,
    /// Natural-language / structured prompt sent to the generative model.
    pub prompt: String,
    /// Where the pixels originate.
    pub source: FeedSource,
    /// Requested frame width in texels.
    pub width: u32,
    /// Requested frame height in texels.
    pub height: u32,
    /// Requested frames per second; used by the engine to pace polling.
    pub fps: f32,
    /// Free-form style/capability tags (e.g. "cinematic", "no-nsfw").
    pub tags: Vec<String>,
}

impl MaterialPrompt {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>, source: FeedSource) -> Self {
        MaterialPrompt {
            id: id.into(),
            prompt: prompt.into(),
            source,
            width: 256,
            height: 256,
            fps: 30.0,
            tags: Vec::new(),
        }
    }

    /// Approximate frame interval in milliseconds for pacing the poll loop.
    pub fn frame_interval_ms(&self) -> u64 {
        if self.fps <= 0.0 {
            return 33;
        }
        (1000.0 / self.fps as f64).max(1.0) as u64
    }

    pub fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn with_fps(mut self, fps: f32) -> Self {
        self.fps = fps;
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}
