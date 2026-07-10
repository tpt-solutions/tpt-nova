//! 2D audio for TPT Nova: SFX one-shots, looping music, and bus mixing.
//!
//! The [`Mixer`] models volume buses (master/SFX/music) with no device
//! dependency, so it is fully unit-testable. [`AudioEngine`] wires that mixer
//! to a real output device via `rodio`. Opening the device is best-effort: on a
//! headless machine (CI, servers) the engine degrades to a no-op so game code
//! can call `play_*` unconditionally.

pub mod mixer;

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use rodio::source::Source;
use rodio::stream::{DeviceSinkBuilder, MixerDeviceSink};
use rodio::{Decoder, Player};

pub use mixer::{Bus, Mixer};

/// An in-memory decodable sound (WAV/OGG/FLAC/MP3, per rodio's decoders).
///
/// The raw encoded bytes are kept and decoded on each play, so one `Sound` can
/// back many simultaneous voices cheaply (no giant PCM buffer per instance).
#[derive(Clone)]
pub struct Sound {
    bytes: Arc<Vec<u8>>,
}

impl Sound {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Sound {
            bytes: Arc::new(bytes),
        }
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        Ok(Sound::from_bytes(std::fs::read(path)?))
    }

    fn cursor(&self) -> Cursor<Vec<u8>> {
        Cursor::new((*self.bytes).clone())
    }

    /// Decode into a one-shot source.
    pub fn decode(&self) -> Result<Decoder<Cursor<Vec<u8>>>, rodio::decoder::DecoderError> {
        Decoder::new(self.cursor())
    }

    /// Total number of samples (across channels). Handy for tests and for
    /// estimating length without playing.
    pub fn sample_count(&self) -> Result<usize, rodio::decoder::DecoderError> {
        Ok(self.decode()?.count())
    }
}

/// A single playing SFX voice plus the per-voice volume it was started at.
struct Voice {
    player: Player,
    volume: f32,
    bus: Bus,
}

/// Owns the audio output device and all currently-playing sounds.
///
/// Not an ECS resource: it holds a platform audio stream that is not `Send`/
/// `Sync`, so it lives directly on the application/main thread.
pub struct AudioEngine {
    device: Option<MixerDeviceSink>,
    mixer: Mixer,
    music: Option<Voice>,
    sfx: Vec<Voice>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        AudioEngine::new()
    }
}

impl AudioEngine {
    /// Open the default output device. If none is available the engine still
    /// works as a silent no-op.
    pub fn new() -> Self {
        let device = match DeviceSinkBuilder::open_default_sink() {
            Ok(d) => Some(d),
            Err(e) => {
                log::warn!("nova-audio: no output device ({e}); running silent");
                None
            }
        };
        AudioEngine {
            device,
            mixer: Mixer::new(),
            music: None,
            sfx: Vec::new(),
        }
    }

    /// True if a real output device is available.
    pub fn is_active(&self) -> bool {
        self.device.is_some()
    }

    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// Number of currently-tracked SFX voices (call [`AudioEngine::update`]
    /// first to drop finished ones).
    pub fn active_sfx(&self) -> usize {
        self.sfx.len()
    }

    fn new_player(&self) -> Option<Player> {
        self.device.as_ref().map(|d| Player::connect_new(d.mixer()))
    }

    /// Play a one-shot effect at `volume` (relative to the SFX bus).
    /// Returns true if a voice was started.
    pub fn play_sfx(&mut self, sound: &Sound, volume: f32) -> bool {
        let player = match self.new_player() {
            Some(p) => p,
            None => return false,
        };
        let source = match sound.decode() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("nova-audio: failed to decode sfx: {e}");
                return false;
            }
        };
        player.append(source);
        player.set_volume(self.mixer.gain(Bus::Sfx, volume));
        self.sfx.push(Voice {
            player,
            volume,
            bus: Bus::Sfx,
        });
        true
    }

    /// Start (or replace) the looping/one-shot background music.
    /// Returns true if playback started.
    pub fn play_music(&mut self, sound: &Sound, looping: bool) -> bool {
        self.stop_music();
        let player = match self.new_player() {
            Some(p) => p,
            None => return false,
        };
        let decoded = match sound.decode() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("nova-audio: failed to decode music: {e}");
                return false;
            }
        };
        if looping {
            player.append(decoded.repeat_infinite());
        } else {
            player.append(decoded);
        }
        player.set_volume(self.mixer.gain(Bus::Music, 1.0));
        self.music = Some(Voice {
            player,
            volume: 1.0,
            bus: Bus::Music,
        });
        true
    }

    pub fn stop_music(&mut self) {
        if let Some(v) = self.music.take() {
            v.player.stop();
        }
    }

    pub fn set_master_volume(&mut self, v: f32) {
        self.mixer.set_master(v);
        self.reapply_volumes();
    }

    pub fn set_bus_volume(&mut self, bus: Bus, v: f32) {
        self.mixer.set_bus(bus, v);
        self.reapply_volumes();
    }

    /// Re-apply mixer gains to every live voice (after a volume change).
    fn reapply_volumes(&mut self) {
        for v in &self.sfx {
            v.player.set_volume(self.mixer.gain(v.bus, v.volume));
        }
        if let Some(v) = &self.music {
            v.player.set_volume(self.mixer.gain(v.bus, v.volume));
        }
    }

    /// Drop finished SFX voices. Call once per frame.
    pub fn update(&mut self) {
        self.sfx.retain(|v| !v.player.empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal 16-bit PCM mono WAV in memory.
    fn make_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
        let channels: u16 = 1;
        let bits: u16 = 16;
        let byte_rate = sample_rate * channels as u32 * (bits / 8) as u32;
        let block_align = channels * (bits / 8);
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&sample_rate.to_le_bytes());
        v.extend_from_slice(&byte_rate.to_le_bytes());
        v.extend_from_slice(&block_align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_generated_wav() {
        // 0.1s of a sine at 44.1kHz.
        let sr = 44_100;
        let n = sr / 10;
        let samples: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
            })
            .collect();
        let sound = Sound::from_bytes(make_wav(&samples, sr));
        let count = sound.sample_count().expect("decode");
        assert!(count > 0, "expected decoded samples, got {count}");
    }

    #[test]
    fn engine_is_silent_without_panicking() {
        // On CI there is no audio device; play_* must not panic and should
        // simply report that nothing started.
        let mut engine = AudioEngine::new();
        let sound = Sound::from_bytes(make_wav(&[0i16; 100], 44_100));
        let _ = engine.play_sfx(&sound, 0.8);
        let _ = engine.play_music(&sound, true);
        engine.set_master_volume(0.5);
        engine.set_bus_volume(Bus::Music, 0.3);
        engine.update();
        engine.stop_music();
        // Mixer math is always correct regardless of device availability.
        assert_eq!(engine.mixer().master(), 0.5);
    }
}
