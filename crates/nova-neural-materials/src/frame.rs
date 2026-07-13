//! A single decoded video frame, in RGBA8 (one byte per channel, row-major).

use serde::{Deserialize, Serialize};

/// The GPU pixel format all neural material frames are normalized to.
pub const FRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A decoded video frame ready to be uploaded to the GPU.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, length must equal `width * height * 4`.
    pub rgba: Vec<u8>,
    /// Monotonic capture timestamp in milliseconds.
    pub timestamp_ms: u64,
}

/// Errors from constructing or validating a [`Frame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// `rgba.len()` did not equal `width * height * 4`.
    SizeMismatch { expected: usize, actual: usize },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::SizeMismatch { expected, actual } => {
                write!(f, "frame byte length {actual}, expected {expected}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

impl Frame {
    /// Build a frame, validating that `rgba` has the right length.
    pub fn new(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        timestamp_ms: u64,
    ) -> Result<Self, FrameError> {
        let expected = expected_len(width, height).ok_or(FrameError::SizeMismatch {
            expected: 0,
            actual: rgba.len(),
        })?;
        if rgba.len() != expected {
            return Err(FrameError::SizeMismatch {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Frame {
            width,
            height,
            rgba,
            timestamp_ms,
        })
    }

    /// Number of texels in the frame.
    pub fn texel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Validate byte length; cheap re-check before a GPU upload.
    pub fn validate(&self) -> Result<(), FrameError> {
        let expected = expected_len(self.width, self.height).ok_or(FrameError::SizeMismatch {
            expected: 0,
            actual: self.rgba.len(),
        })?;
        if self.rgba.len() != expected {
            return Err(FrameError::SizeMismatch {
                expected,
                actual: self.rgba.len(),
            });
        }
        Ok(())
    }
}

/// Compute the expected RGBA8 byte length without overflowing on large
/// (potentially untrusted/deserialized) dimensions.
fn expected_len(width: u32, height: u32) -> Option<usize> {
    let w = width as u64;
    let h = height as u64;
    let len = w.checked_mul(h)?.checked_mul(4)?;
    usize::try_from(len).ok()
}
