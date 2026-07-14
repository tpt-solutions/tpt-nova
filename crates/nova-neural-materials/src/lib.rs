//! Live video-LLM neural material feeds for TPT Nova.
//!
//! A *neural material* is a texture whose pixels are produced by a generative
//! model (a Video LLM, a webcam, a streamed clip) rather than a static image
//! file. This crate defines the **API contract** between the engine and any
//! such feed, plus the machinery to pump streamed [`Frame`]s onto a
//! [`wgpu::Texture`] in real time.
//!
//! The contract is deliberately transport-agnostic:
//!
//! - A [`MaterialPrompt`] describes *what* to generate (prompt text, source,
//!   resolution, fps).
//! - A [`NeuralMaterialProvider`] turns a prompt into a [`FrameSource`] that
//!   yields decoded [`Frame`]s as they arrive.
//! - [`NeuralTexture`] uploads those frames onto the GPU, and
//!   [`NeuralMaterialRegistry`] (an ECS resource) tracks active materials and
//!   their latest frames each tick.
//!
//! [`MockProvider`] implements the contract without any network: it synthesizes
//! an animated gradient, which is enough to prove the full round-trip end to
//! end (prompt → frames → texture) in tests and in the headless demo.

pub mod binding;
pub mod frame;
pub mod prompt;
pub mod provider;
pub mod registry;
pub mod source;
pub mod texture;

pub use binding::{MaterialBinding, MaterialBindings};
pub use frame::{Frame, FrameError};
pub use prompt::{FeedSource, MaterialPrompt};
pub use provider::{MockProvider, NeuralMaterialProvider, ProviderError};
pub use registry::NeuralMaterialRegistry;
pub use source::{FrameSource, PushingSource, StaticImageSource};
pub use texture::NeuralTexture;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::FeedSource;
    use crate::source::StaticImageSource;

    fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> Frame {
        let rgba = [r, g, b, 255].repeat(width as usize * height as usize);
        Frame::new(width, height, rgba, 0).unwrap()
    }

    #[test]
    fn frame_rejects_wrong_length() {
        let err = Frame::new(2, 2, vec![0u8; 10], 0).unwrap_err();
        assert_eq!(
            err,
            FrameError::SizeMismatch {
                expected: 16,
                actual: 10
            }
        );
    }

    #[test]
    fn prompt_serializes_round_trip() {
        let p = MaterialPrompt::new(
            "billboard",
            "neon rain, cyberpunk",
            FeedSource::VideoLlm {
                endpoint: "llm://sora".into(),
            },
        )
        .with_resolution(512, 256)
        .with_fps(24.0)
        .with_tag("cinematic");
        let json = serde_json::to_string(&p).unwrap();
        let back: MaterialPrompt = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "billboard");
        assert_eq!(back.width, 512);
        assert_eq!(back.height, 256);
        assert_eq!(back.fps, 24.0);
        assert_eq!(back.tags, vec!["cinematic".to_string()]);
    }

    #[test]
    fn mock_provider_yields_correctly_sized_frames() {
        let provider = MockProvider;
        let prompt = MaterialPrompt::new("m", "gradient", FeedSource::CaptureDevice(0))
            .with_resolution(8, 4);
        let mut src = provider.open(&prompt).unwrap();
        let f0 = src.next_frame().unwrap();
        let f1 = src.next_frame().unwrap();
        assert_eq!((f0.width, f0.height), (8, 4));
        assert_eq!(f0.rgba.len(), 8 * 4 * 4);
        assert_ne!(f0.rgba, f1.rgba, "frames should animate over time");
        assert_eq!(f1.timestamp_ms, 1);
    }

    #[test]
    fn mock_provider_rejects_bad_resolution() {
        let provider = MockProvider;
        let prompt =
            MaterialPrompt::new("m", "x", FeedSource::CaptureDevice(0)).with_resolution(0, 0);
        assert!(matches!(
            provider.open(&prompt),
            Err(ProviderError::InvalidResolution { .. })
        ));
    }

    #[test]
    fn pushing_source_buffers_frames() {
        let mut src = PushingSource::new();
        assert_eq!(src.pending(), 0);
        src.push(solid(2, 2, 1, 2, 3));
        src.push(solid(2, 2, 4, 5, 6));
        assert_eq!(src.pending(), 2);
        let first = src.next_frame().unwrap();
        assert_eq!(first.rgba[0], 1);
        assert_eq!(src.pending(), 1);
    }

    #[test]
    fn static_image_source_loops_or_ends() {
        let frame = solid(1, 1, 9, 9, 9);
        let mut once = StaticImageSource::new(frame.clone(), false);
        assert!(once.next_frame().is_some());
        assert!(once.next_frame().is_none());

        let mut looping = StaticImageSource::new(frame, true);
        assert!(looping.next_frame().is_some());
        assert!(looping.next_frame().is_some());
    }

    #[test]
    fn registry_polls_and_stores_latest_frame() {
        let mut reg = NeuralMaterialRegistry::default();
        let prompt = MaterialPrompt::new("b", "rainbow", FeedSource::CaptureDevice(0))
            .with_resolution(4, 4)
            .with_tag("a");
        reg.register(prompt).unwrap();
        assert!(reg.contains("b"));
        assert!(reg.latest("b").is_none());
        reg.update();
        reg.update();
        let latest = reg.latest("b").expect("frame should have arrived");
        assert_eq!((latest.width, latest.height), (4, 4));
        assert!(reg.latest_timestamp("b").unwrap() >= 1);
    }

    #[test]
    fn frame_overflow_is_safe() {
        // Oversized dimensions must be rejected by the overflow-safe guard
        // rather than panic or silently wrap.
        let err = Frame::new(u32::MAX, u32::MAX, Vec::new(), 0).unwrap_err();
        assert!(matches!(err, FrameError::SizeMismatch { .. }));
    }

    #[test]
    fn set_provider_reopens_feeds() {
        let mut reg = NeuralMaterialRegistry::default();
        let prompt =
            MaterialPrompt::new("m", "x", FeedSource::CaptureDevice(0)).with_resolution(4, 4);
        reg.register(prompt).unwrap();
        reg.update();
        assert!(reg.latest("m").is_some());
        // Swapping the provider re-opens the existing feed and keeps it live.
        reg.set_provider(Box::new(MockProvider)).unwrap();
        reg.update();
        assert!(reg.latest("m").is_some());
    }

    /// A feed that yields a fixed list of frames then ends (simulates a dropped
    /// / finished stream), to exercise the registry's dropped-stream handling.
    struct DroppingSource {
        frames: Vec<Frame>,
        idx: usize,
    }

    impl crate::source::FrameSource for DroppingSource {
        fn next_frame(&mut self) -> Option<Frame> {
            if self.idx < self.frames.len() {
                let f = self.frames[self.idx].clone();
                self.idx += 1;
                Some(f)
            } else {
                None
            }
        }
    }

    struct DroppingProvider {
        frames: Vec<Frame>,
    }

    impl NeuralMaterialProvider for DroppingProvider {
        fn open(&self, _prompt: &MaterialPrompt) -> Result<Box<dyn FrameSource>, ProviderError> {
            Ok(Box::new(DroppingSource {
                frames: self.frames.clone(),
                idx: 0,
            }))
        }
    }

    #[test]
    fn dropped_stream_keeps_last_frame() {
        let frames = vec![
            solid(2, 2, 1, 1, 1),
            solid(2, 2, 2, 2, 2),
            solid(2, 2, 3, 3, 3),
        ];
        let mut reg = NeuralMaterialRegistry::new(Box::new(DroppingProvider { frames }));
        let prompt =
            MaterialPrompt::new("d", "x", FeedSource::CaptureDevice(0)).with_resolution(2, 2);
        reg.register(prompt).unwrap();

        reg.update();
        reg.update();
        reg.update(); // source now exhausted
        let last = reg
            .latest("d")
            .expect("last frame retained after stream drops");
        assert_eq!(last.rgba[0], 3);

        // Further polls must not panic and must keep the last frame available.
        reg.update();
        assert!(reg.latest("d").is_some());
    }
}
