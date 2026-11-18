//! Turning a [`MaterialPrompt`] into a live [`FrameSource`].
//!
//! [`MockProvider`] is the reference implementation: it needs no network and
//! synthesizes an animated RGBA gradient, which is enough to prove the entire
//! round-trip (prompt → frames → texture) in tests and the headless demo. A
//! production deployment would swap in a provider that talks to a real Video
//! LLM endpoint.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::frame::Frame;
use crate::prompt::MaterialPrompt;
use crate::source::FrameSource;

/// Errors returned when a provider cannot open a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The prompt described an unsupported source/endpoint.
    UnsupportedSource(String),
    /// The requested resolution is outside what the provider can produce.
    InvalidResolution { width: u32, height: u32 },
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::UnsupportedSource(s) => write!(f, "unsupported feed source: {s}"),
            ProviderError::InvalidResolution { width, height } => {
                write!(f, "unsupported resolution: {width}x{height}")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

/// Opens feeds described by [`MaterialPrompt`].
pub trait NeuralMaterialProvider: Send + Sync {
    /// Open a feed, returning a [`FrameSource`] that yields decoded frames.
    fn open(&self, prompt: &MaterialPrompt) -> Result<Box<dyn FrameSource>, ProviderError>;
}

/// A deterministic, network-free provider used for tests and the headless demo.
///
/// It renders an animated gradient whose phase advances one step per frame, so
/// callers can verify that frames actually change over time and that each frame
/// matches the requested resolution.
pub struct MockProvider;

struct MockFrameSource {
    width: u32,
    height: u32,
    counter: Arc<AtomicU64>,
    hue_offset: u8,
    buf: Vec<u8>,
}

impl FrameSource for MockFrameSource {
    fn next_frame(&mut self) -> Option<Frame> {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        // Reuse a single retained scratch buffer: clear + reserve keeps the
        // allocation across calls instead of reallocating per frame. (`Frame`
        // owns its `rgba` by value, so a brand-new `Vec` is still handed to the
        // consumer each call; production providers should likewise recycle a
        // scratch buffer rather than building fresh pixel vectors.)
        let tight = self.width as usize * self.height as usize * 4;
        self.buf.clear();
        self.buf.reserve(tight);
        for y in 0..self.height {
            for x in 0..self.width {
                let t = (n as f32 * 0.05
                    + (x as f32 / self.width.max(1) as f32) * std::f32::consts::TAU
                    + (y as f32 / self.height.max(1) as f32) * std::f32::consts::PI)
                    .sin()
                    * 0.5
                    + 0.5;
                let (r, g, b) = hsv_to_rgb(
                    ((self.hue_offset as f32 + t * 360.0) % 360.0).round() as u16,
                    0.7,
                    0.9,
                );
                self.buf.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Frame::new(self.width, self.height, std::mem::take(&mut self.buf), n).ok()
    }
}

impl NeuralMaterialProvider for MockProvider {
    fn open(&self, prompt: &MaterialPrompt) -> Result<Box<dyn FrameSource>, ProviderError> {
        if prompt.width == 0 || prompt.height == 0 || prompt.width > 4096 || prompt.height > 4096 {
            return Err(ProviderError::InvalidResolution {
                width: prompt.width,
                height: prompt.height,
            });
        }
        // Vary the gradient phase by the first tag so different materials look
        // distinct in tests/demos.
        let hue_offset = prompt
            .tags
            .first()
            .map(|t| t.bytes().fold(0u8, |a, b| a.wrapping_add(b)))
            .unwrap_or(0);
        Ok(Box::new(MockFrameSource {
            width: prompt.width,
            height: prompt.height,
            counter: Arc::new(AtomicU64::new(0)),
            hue_offset,
            buf: Vec::with_capacity((prompt.width as usize * prompt.height as usize * 4).max(1)),
        }))
    }
}

/// Convert HSV (h in [0,360), s/v in [0,1]) to 8-bit RGB.
fn hsv_to_rgb(h: u16, s: f32, v: f32) -> (u8, u8, u8) {
    let h = h as f32;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
