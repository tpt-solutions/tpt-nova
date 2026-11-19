//! FLAC playback source backed by `tpt-av-cadence-flac`.
//!
//! The decoder yields interleaved `f32` frames in blocks; [`FlacSource`] turns
//! those blocks into a rodio [`Source`] one sample at a time. The codec itself
//! lives in the tpt-cadence suite (MIT OR Apache-2.0), which is pinned by
//! commit in `Cargo.toml`.

use std::io::Cursor;
use std::num::NonZero;
use std::time::Duration;

use rodio::source::Source;
use tpt_av_cadence_core::{CadenceError, Decoder as _};
use tpt_av_cadence_flac::FlacDecoder;

/// Frames decoded per call to the underlying decoder.
const BLOCK_FRAMES: usize = 4096;

/// True when `bytes` begins with the FLAC stream marker `fLaC`.
pub fn is_flac(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"fLaC"
}

/// A FLAC stream exposed as an interleaved f32 [`Source`].
pub struct FlacSource {
    decoder: FlacDecoder,
    channels: NonZero<u16>,
    sample_rate: NonZero<u32>,
    total: Option<Duration>,
    block: Vec<f32>,
    /// Valid samples in `block`.
    filled: usize,
    /// Next sample to hand out from `block`.
    pos: usize,
    finished: bool,
}

impl FlacSource {
    /// Open a FLAC stream held in memory.
    pub fn open(bytes: Vec<u8>) -> Result<Self, CadenceError> {
        let decoder = FlacDecoder::open(Box::new(Cursor::new(bytes)))?;
        let info = decoder.info();
        let channels = NonZero::new(info.channels).ok_or_else(|| {
            CadenceError::InvalidFormat("FLAC stream reports zero channels".into())
        })?;
        let sample_rate = NonZero::new(info.sample_rate).ok_or_else(|| {
            CadenceError::InvalidFormat("FLAC stream reports zero sample rate".into())
        })?;
        let total = info
            .total_frames
            .map(|frames| Duration::from_secs_f64(frames as f64 / info.sample_rate as f64));
        Ok(FlacSource {
            channels,
            sample_rate,
            total,
            block: vec![0.0; BLOCK_FRAMES * channels.get() as usize],
            filled: 0,
            pos: 0,
            finished: false,
            decoder,
        })
    }

    /// Decode the next block. Returns `false` at end of stream.
    fn refill(&mut self) -> bool {
        if self.finished {
            return false;
        }
        match self.decoder.decode(&mut self.block) {
            Ok(0) | Err(_) => {
                self.finished = true;
                self.filled = 0;
                false
            }
            Ok(frames) => {
                self.filled = frames * self.channels.get() as usize;
                self.pos = 0;
                true
            }
        }
    }
}

impl Iterator for FlacSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.filled && !self.refill() {
            return None;
        }
        let sample = self.block[self.pos];
        self.pos += 1;
        Some(sample)
    }
}

impl Source for FlacSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/data/14-wasted-bits.flac");

    #[test]
    fn detects_flac_magic() {
        assert!(is_flac(FIXTURE));
        assert!(!is_flac(b"RIFF....WAVE"));
    }

    #[test]
    fn decodes_fixture_to_samples() {
        let source = FlacSource::open(FIXTURE.to_vec()).expect("open fixture");
        let channels = source.channels().get() as usize;
        let rate = source.sample_rate().get();
        assert!(channels >= 1 && rate > 0);
        let samples: Vec<f32> = source.collect();
        assert!(!samples.is_empty(), "fixture should decode to samples");
        assert_eq!(samples.len() % channels, 0, "whole frames only");
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0001));
    }

    #[test]
    fn rejects_non_flac_bytes() {
        assert!(FlacSource::open(b"definitely not flac".to_vec()).is_err());
    }
}

#[cfg(test)]
mod sound_tests {
    use crate::Sound;

    #[test]
    fn sound_routes_flac_magic_to_flac_decoder() {
        let bytes = include_bytes!("../tests/data/14-wasted-bits.flac").to_vec();
        let decoded = Sound::from_bytes(bytes).decode().expect("flac via Sound");
        assert!(matches!(decoded, crate::Decoded::Flac(_)));
    }
}
