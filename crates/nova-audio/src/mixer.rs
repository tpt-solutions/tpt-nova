//! Volume buses and mixing math.
//!
//! The mixer is deliberately tiny and dependency-free so it can be unit-tested
//! without an audio device. It models a master gain plus one gain per logical
//! bus; the effective gain applied to a sound is `master * bus`.

/// A logical group of sounds that share a volume control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Short one-shot effects.
    Sfx,
    /// Looping background music.
    Music,
}

/// Master + per-bus volume levels. Volumes are linear gains in `[0, +inf)`,
/// clamped to be non-negative; `1.0` is unity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mixer {
    master: f32,
    sfx: f32,
    music: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            master: 1.0,
            sfx: 1.0,
            music: 1.0,
        }
    }
}

impl Mixer {
    pub fn new() -> Self {
        Mixer::default()
    }

    pub fn master(&self) -> f32 {
        self.master
    }

    pub fn bus(&self, bus: Bus) -> f32 {
        match bus {
            Bus::Sfx => self.sfx,
            Bus::Music => self.music,
        }
    }

    pub fn set_master(&mut self, v: f32) {
        self.master = v.max(0.0);
    }

    pub fn set_bus(&mut self, bus: Bus, v: f32) {
        let v = v.max(0.0);
        match bus {
            Bus::Sfx => self.sfx = v,
            Bus::Music => self.music = v,
        }
    }

    /// Effective linear gain for a sound on `bus` played at `sound_volume`.
    pub fn gain(&self, bus: Bus, sound_volume: f32) -> f32 {
        (self.master * self.bus(bus) * sound_volume.max(0.0)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_by_default() {
        let m = Mixer::new();
        assert_eq!(m.gain(Bus::Sfx, 1.0), 1.0);
        assert_eq!(m.gain(Bus::Music, 1.0), 1.0);
    }

    #[test]
    fn master_scales_all_buses() {
        let mut m = Mixer::new();
        m.set_master(0.5);
        assert_eq!(m.gain(Bus::Sfx, 1.0), 0.5);
        assert_eq!(m.gain(Bus::Music, 0.5), 0.25);
    }

    #[test]
    fn negative_volumes_are_clamped() {
        let mut m = Mixer::new();
        m.set_bus(Bus::Sfx, -3.0);
        assert_eq!(m.gain(Bus::Sfx, 1.0), 0.0);
        assert_eq!(m.gain(Bus::Sfx, -1.0), 0.0);
    }
}
