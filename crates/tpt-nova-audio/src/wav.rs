//! Minimal in-house WAV (RIFF) decoder.
//!
//! rodio's own WAV decoder depends on `hound`, which is Apache-2.0 only and
//! therefore excluded by the project's license policy. This module covers the
//! formats game assets actually use: integer PCM (8/16/24/32-bit) and IEEE
//! float (32/64-bit), mono or multichannel, including `WAVE_FORMAT_EXTENSIBLE`.
//! The whole file is decoded into an interleaved `f32` buffer.

use std::fmt;
use std::num::NonZero;

use rodio::buffer::SamplesBuffer;

const FORMAT_PCM: u16 = 1;
const FORMAT_IEEE_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Error produced when a WAV byte stream cannot be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavError(pub String);

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid WAV: {}", self.0)
    }
}

impl std::error::Error for WavError {}

fn fail<T>(msg: impl Into<String>) -> Result<T, WavError> {
    Err(WavError(msg.into()))
}

/// True when `bytes` starts with a RIFF/WAVE header.
pub fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

#[derive(Clone, Copy)]
struct Format {
    tag: u16,
    channels: u16,
    sample_rate: u32,
    bits: u16,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn parse_fmt(body: &[u8]) -> Result<Format, WavError> {
    if body.len() < 16 {
        return fail("fmt chunk too short");
    }
    let mut tag = u16_at(body, 0);
    let channels = u16_at(body, 2);
    let sample_rate = u32_at(body, 4);
    let bits = u16_at(body, 14);
    if tag == FORMAT_EXTENSIBLE {
        // cbSize(2) validBits(2) channelMask(4) then the 16-byte sub-format GUID,
        // whose first two bytes hold the actual format tag.
        if body.len() < 26 {
            return fail("extensible fmt chunk too short");
        }
        tag = u16_at(body, 24);
    }
    if channels == 0 {
        return fail("zero channels");
    }
    if sample_rate == 0 {
        return fail("zero sample rate");
    }
    Ok(Format {
        tag,
        channels,
        sample_rate,
        bits,
    })
}

/// Decode `bytes` (a complete WAV file) into an in-memory f32 buffer.
pub fn decode_wav(bytes: &[u8]) -> Result<SamplesBuffer, WavError> {
    if !is_wav(bytes) {
        return fail("missing RIFF/WAVE header");
    }
    let mut pos = 12usize;
    let mut format: Option<Format> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_at(bytes, pos + 4) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " => format = Some(parse_fmt(body)?),
            b"data" => {
                data = Some(body);
                if format.is_some() {
                    break;
                }
            }
            _ => {}
        }
        // Chunks are padded to an even byte count.
        pos = body_start.saturating_add(size + (size & 1));
    }
    let Some(format) = format else {
        return fail("no fmt chunk");
    };
    let Some(data) = data else {
        return fail("no data chunk");
    };

    let mut samples = decode_samples(format, data)?;
    let frame = format.channels as usize;
    samples.truncate(samples.len() - samples.len() % frame);
    let channels = NonZero::new(format.channels).expect("checked non-zero above");
    let rate = NonZero::new(format.sample_rate).expect("checked non-zero above");
    Ok(SamplesBuffer::new(channels, rate, samples))
}

fn decode_samples(format: Format, data: &[u8]) -> Result<Vec<f32>, WavError> {
    let out = match (format.tag, format.bits) {
        (FORMAT_PCM, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
        (FORMAT_PCM, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32_768.0)
            .collect(),
        (FORMAT_PCM, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| {
                // Sign-extend the 24-bit little-endian value.
                let v = ((c[0] as i32) | ((c[1] as i32) << 8) | ((c[2] as i32) << 16)) << 8 >> 8;
                v as f32 / 8_388_608.0
            })
            .collect(),
        (FORMAT_PCM, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f32 / 2_147_483_648.0)
            .collect(),
        (FORMAT_IEEE_FLOAT, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        (FORMAT_IEEE_FLOAT, 64) => data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as f32)
            .collect(),
        (tag, bits) => {
            return fail(format!(
                "unsupported sample format tag {tag} with {bits} bits"
            ));
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    fn wav_bytes(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * block as u32).to_le_bytes());
        v.extend_from_slice(&block.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn decodes_16_bit_pcm() {
        let data: Vec<u8> = [0i16, 16_384, -16_384, i16::MIN]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let buf = decode_wav(&wav_bytes(FORMAT_PCM, 1, 22_050, 16, &data)).unwrap();
        assert_eq!(buf.sample_rate().get(), 22_050);
        assert_eq!(buf.channels().get(), 1);
        let out: Vec<f32> = buf.collect();
        assert_eq!(out.len(), 4);
        assert!((out[1] - 0.5).abs() < 1e-4);
        assert!((out[2] + 0.5).abs() < 1e-4);
        assert!((out[3] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn decodes_8_bit_unsigned_and_24_bit() {
        let out: Vec<f32> = decode_wav(&wav_bytes(FORMAT_PCM, 1, 8_000, 8, &[128, 255, 0]))
            .unwrap()
            .collect();
        assert!(out[0].abs() < 1e-6);
        assert!(out[1] > 0.9);
        assert!((out[2] + 1.0).abs() < 1e-6);

        // 24-bit: 0x800000 is the most negative value, 0x400000 is half scale.
        let data = [0x00, 0x00, 0x80, 0x00, 0x00, 0x40];
        let out: Vec<f32> = decode_wav(&wav_bytes(FORMAT_PCM, 1, 8_000, 24, &data))
            .unwrap()
            .collect();
        assert!((out[0] + 1.0).abs() < 1e-6);
        assert!((out[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn decodes_float_and_stereo() {
        let data: Vec<u8> = [0.25f32, -0.25, 0.5, -0.5]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let buf = decode_wav(&wav_bytes(FORMAT_IEEE_FLOAT, 2, 44_100, 32, &data)).unwrap();
        assert_eq!(buf.channels().get(), 2);
        let out: Vec<f32> = buf.collect();
        assert_eq!(out, vec![0.25, -0.25, 0.5, -0.5]);
    }

    #[test]
    fn rejects_non_wav_and_unsupported_formats() {
        assert!(decode_wav(b"not a wav file at all").is_err());
        assert!(!is_wav(b"OggS...."));
        // 12-bit PCM is not a supported container width here.
        assert!(decode_wav(&wav_bytes(FORMAT_PCM, 1, 8_000, 12, &[0, 0])).is_err());
        // Zero channels is rejected.
        assert!(decode_wav(&wav_bytes(FORMAT_PCM, 0, 8_000, 16, &[0, 0])).is_err());
    }

    #[test]
    fn truncated_data_chunk_is_clamped() {
        // Declared data size is larger than what is present; decode what exists.
        let mut bytes = wav_bytes(FORMAT_PCM, 1, 8_000, 16, &[1, 0, 2, 0]);
        bytes.truncate(bytes.len() - 1);
        let out: Vec<f32> = decode_wav(&bytes).unwrap().collect();
        assert_eq!(out.len(), 1);
    }
}
