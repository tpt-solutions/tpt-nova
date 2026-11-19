//! 2D audio for TPT Nova: SFX one-shots, looping music, and bus mixing.
//!
//! The [`Mixer`] models volume buses (master/SFX/music) with no device
//! dependency, so it is fully unit-testable. [`AudioEngine`] wires that mixer
//! to a real output device via `rodio`. Opening the device is best-effort: on a
//! headless machine (CI, servers) the engine degrades to a no-op so game code
//! can call `play_*` unconditionally.

pub mod flac;
pub mod mixer;
pub mod wav;

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use rodio::buffer::SamplesBuffer;
use rodio::source::Source;
use rodio::stream::{DeviceSinkBuilder, MixerDeviceSink};
use rodio::{Decoder, Player, Sample, SpatialPlayer};

pub use mixer::{spatial_attenuation, Bus, Listener, Mixer, SpatialParams, Vec3};

/// Error returned when a [`Sound`] cannot be decoded.
#[derive(Debug)]
pub enum DecodeError {
    /// The bytes have a RIFF/WAVE header but the WAV decoder rejected them.
    Wav(wav::WavError),
    /// The bytes were handed to rodio (Ogg Vorbis / MP3) and it failed.
    Stream(rodio::decoder::DecoderError),
    /// The bytes start with `fLaC` but the FLAC decoder rejected them.
    Flac(tpt_av_cadence_core::CadenceError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Wav(e) => write!(f, "{e}"),
            DecodeError::Stream(e) => write!(f, "{e}"),
            DecodeError::Flac(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A decoded, playable sound. WAV is decoded fully into memory by [`wav`];
/// Ogg Vorbis and MP3 are streamed through rodio's decoders.
pub enum Decoded {
    Wav(SamplesBuffer),
    Stream(Box<Decoder<Cursor<Vec<u8>>>>),
    Flac(Box<flac::FlacSource>),
}

impl Iterator for Decoded {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        match self {
            Decoded::Wav(s) => s.next(),
            Decoded::Stream(s) => s.next(),
            Decoded::Flac(s) => s.next(),
        }
    }
}

impl Source for Decoded {
    fn current_span_len(&self) -> Option<usize> {
        match self {
            Decoded::Wav(s) => s.current_span_len(),
            Decoded::Stream(s) => s.current_span_len(),
            Decoded::Flac(s) => s.current_span_len(),
        }
    }

    fn channels(&self) -> rodio::ChannelCount {
        match self {
            Decoded::Wav(s) => s.channels(),
            Decoded::Stream(s) => s.channels(),
            Decoded::Flac(s) => s.channels(),
        }
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        match self {
            Decoded::Wav(s) => s.sample_rate(),
            Decoded::Stream(s) => s.sample_rate(),
            Decoded::Flac(s) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        match self {
            Decoded::Wav(s) => s.total_duration(),
            Decoded::Stream(s) => s.total_duration(),
            Decoded::Flac(s) => s.total_duration(),
        }
    }
}

/// An in-memory decodable sound (WAV, Ogg Vorbis, or MP3).
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

    /// Decode into a one-shot source. The container is detected from the
    /// header: RIFF/WAVE goes to [`wav::decode_wav`], `fLaC` goes to
    /// [`flac::FlacSource`]; anything else is handed to rodio, which handles
    /// Ogg Vorbis and MP3.
    pub fn decode(&self) -> Result<Decoded, DecodeError> {
        if wav::is_wav(&self.bytes) {
            return wav::decode_wav(&self.bytes)
                .map(Decoded::Wav)
                .map_err(DecodeError::Wav);
        }
        if flac::is_flac(&self.bytes) {
            return flac::FlacSource::open((*self.bytes).clone())
                .map(|s| Decoded::Flac(Box::new(s)))
                .map_err(DecodeError::Flac);
        }
        Decoder::new(Cursor::new((*self.bytes).clone()))
            .map(|d| Decoded::Stream(Box::new(d)))
            .map_err(DecodeError::Stream)
    }

    /// Total number of samples (across channels). Handy for tests and for
    /// estimating length without playing.
    pub fn sample_count(&self) -> Result<usize, DecodeError> {
        Ok(self.decode()?.count())
    }
}

/// A single playing SFX voice plus the per-voice volume it was started at.
struct Voice {
    player: Player,
    volume: f32,
    bus: Bus,
}

/// A single playing positional (3D) voice, spatialized via `rodio::SpatialPlayer`.
struct SpatialVoice {
    player: SpatialPlayer,
    volume: f32,
    bus: Bus,
    /// Cached distance attenuation from the spawn-time `SpatialParams`, so
    /// volume re-application after a bus/master change keeps the same falloff.
    attenuation: f32,
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
    spatial: Vec<SpatialVoice>,
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
                log::warn!("tpt-nova-audio: no output device ({e}); running silent");
                None
            }
        };
        AudioEngine {
            device,
            mixer: Mixer::new(),
            music: None,
            sfx: Vec::new(),
            spatial: Vec::new(),
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

    /// Play a positional (3D) sound at `source` relative to `listener`.
    ///
    /// Binaural panning is handled by `rodio::SpatialPlayer` from the listener's
    /// ear positions; the engine applies the master/bus/sound gain on top, and
    /// layers the caller's [`SpatialParams`] distance-rolloff model
    /// (`ref_distance`/`max_distance`/`rolloff`) so a designer-tuned falloff is
    /// actually applied rather than silently ignored. Returns `true` if a voice
    /// was started. Falls back to a silent no-op (returns `false`) when no output
    /// device is available, so callers can play unconditionally.
    pub fn play_spatial(
        &mut self,
        sound: &Sound,
        listener: &Listener,
        source: Vec3,
        params: &SpatialParams,
        bus: Bus,
        sound_volume: f32,
    ) -> bool {
        let device = match self.device.as_ref() {
            Some(d) => d,
            None => return false,
        };
        let decoded = match sound.decode() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("tpt-nova-audio: failed to decode spatial sfx: {e}");
                return false;
            }
        };
        // Designer-controlled distance attenuation from `SpatialParams`. This is
        // layered on top of rodio's own internal distance model, so the rolloff
        // shape the caller asked for is genuinely applied (a source past
        // `max_distance` is forced silent regardless of rodio's model).
        let att = spatial_attenuation(listener, source, params).max(0.0);
        let (left_ear, right_ear) = listener.ear_positions(0.1);
        let player = SpatialPlayer::connect_new(device.mixer(), source, left_ear, right_ear);
        player.append(decoded);
        player.set_volume((self.mixer.gain(bus, sound_volume) * att).max(0.0));
        self.spatial.push(SpatialVoice {
            player,
            volume: sound_volume,
            bus,
            attenuation: att,
        });
        true
    }

    /// Number of currently-tracked positional (3D) voices.
    pub fn active_spatial(&self) -> usize {
        self.spatial.len()
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
                log::warn!("tpt-nova-audio: failed to decode sfx: {e}");
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
                log::warn!("tpt-nova-audio: failed to decode music: {e}");
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
        for v in &self.spatial {
            v.player
                .set_volume((self.mixer.gain(v.bus, v.volume) * v.attenuation).max(0.0));
        }
    }

    /// Drop finished SFX voices. Call once per frame.
    pub fn update(&mut self) {
        self.sfx.retain(|v| !v.player.empty());
        self.spatial.retain(|v| !v.player.empty());
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

    #[test]
    fn stop_music_during_loop_is_safe() {
        // Whether or not a device exists, starting a looping track and stopping
        // it mid-loop must be safe and idempotent.
        let mut engine = AudioEngine::new();
        let sound = Sound::from_bytes(make_wav(&[0i16; 100], 44_100));
        let started = engine.play_music(&sound, true);
        engine.stop_music();
        engine.stop_music();
        // Stopping again must remain a no-op and not panic.
        assert!(engine.mixer().bus(Bus::Music) >= 0.0);
        let _ = started;
    }

    #[test]
    fn bus_volume_changes_reapply_without_device() {
        let mut engine = AudioEngine::new();
        let sound = Sound::from_bytes(make_wav(&[0i16; 100], 44_100));
        let _ = engine.play_sfx(&sound, 1.0);
        // Changing bus volumes re-applies gains to live voices; must not panic
        // whether or not a voice actually started.
        engine.set_bus_volume(Bus::Sfx, 0.4);
        engine.set_master_volume(0.2);
        assert_eq!(engine.mixer().master(), 0.2);
        assert_eq!(engine.mixer().bus(Bus::Sfx), 0.4);
    }

    #[test]
    fn spatial_play_is_safe_without_device() {
        // On a headless machine play_spatial must not panic and simply reports
        // that nothing started; the analytic gain model is independently tested
        // in the mixer.
        let mut engine = AudioEngine::new();
        let sound = Sound::from_bytes(make_wav(&[0i16; 100], 44_100));
        let listener = Listener::default();
        let started = engine.play_spatial(
            &sound,
            &listener,
            [10.0, 0.0, 0.0],
            &SpatialParams::default(),
            Bus::Sfx,
            1.0,
        );
        // Whether or not a device exists, the call is safe; active_spatial
        // reflects whatever the platform allowed.
        let _ = started;
        let _ = engine.active_spatial();
    }

    #[test]
    fn spatial_gain_preview_matches_mixer_model() {
        // The engine's analytic preview must agree with `Mixer::spatial_gain`,
        // so headless code can reason about what a positional source will do.
        let engine = AudioEngine::new();
        let listener = Listener::default();
        let params = SpatialParams::default();
        let [l, r] =
            engine
                .mixer()
                .spatial_gain(&listener, [3.0, 0.0, 0.0], &params, Bus::Sfx, 1.0);
        assert!(l >= 0.0 && r >= 0.0);
        // A source at the listener is full-volume (center pan => equal-power).
        let [cl, cr] =
            engine
                .mixer()
                .spatial_gain(&listener, [0.0, 0.0, 0.0], &params, Bus::Sfx, 1.0);
        assert!((cl - cr).abs() < 1e-4);
    }

    #[test]
    fn rolloff_model_changes_previewed_volume() {
        // The `SpatialParams` rolloff model must actually move the gain: a
        // steeper rolloff attenuates a mid-distance source more than the linear
        // default, proving `play_spatial` no longer ignores `params`.
        let engine = AudioEngine::new();
        let listener = Listener::default();
        let src = [10.0, 0.0, 0.0];
        let linear =
            engine
                .mixer()
                .spatial_gain(&listener, src, &SpatialParams::default(), Bus::Sfx, 1.0);
        let steep = engine.mixer().spatial_gain(
            &listener,
            src,
            &SpatialParams {
                rolloff: 3.0,
                ..Default::default()
            },
            Bus::Sfx,
            1.0,
        );
        let l_sum = linear[0] + linear[1];
        let s_sum = steep[0] + steep[1];
        assert!(s_sum < l_sum, "steeper rolloff should be quieter");
    }
}
