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

    /// Effective per-channel `[left, right]` gain for a positional source.
    ///
    /// Combines the bus/master gain with the distance [`spatial_attenuation`]
    /// and an equal-power [`spatial_pan`]. This analytic model is what the
    /// engine uses for device-independent previews and unit tests; the live
    /// path defers binaural panning + distance to `rodio::SpatialPlayer`, which
    /// already attenuates by distance, so the engine feeds it the flat
    /// `gain(bus, sound_volume)` and lets it place the sound in 3D.
    pub fn spatial_gain(
        &self,
        listener: &Listener,
        source: Vec3,
        p: &SpatialParams,
        bus: Bus,
        sound_volume: f32,
    ) -> [f32; 2] {
        let att = spatial_attenuation(listener, source, p);
        let pan = spatial_pan(listener, source);
        let theta = (pan + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
        let (l, r) = (theta.cos(), theta.sin());
        let g = att * self.gain(bus, sound_volume);
        [g * l, g * r]
    }
}

/// A 3D vector (dependency-free stand-in for a math library type).
pub type Vec3 = [f32; 3];

fn v_sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn v_add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn v_scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn v_dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn v_len(a: Vec3) -> f32 {
    v_dot(a, a).sqrt().max(0.0)
}
fn v_norm(a: Vec3) -> Vec3 {
    let l = v_len(a);
    if l > 1e-8 {
        v_scale(a, 1.0 / l)
    } else {
        [0.0, 0.0, 0.0]
    }
}
fn v_cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Where the listener (the player's head/ears) is and which way they face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// Listener world position.
    pub position: Vec3,
    /// Unit forward direction the listener is looking along.
    pub forward: Vec3,
    /// Unit up direction (head top).
    pub up: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Listener {
            position: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        }
    }
}

impl Listener {
    /// Unit vector pointing out of the listener's right ear.
    pub fn right(&self) -> Vec3 {
        v_norm(v_cross(v_norm(self.forward), v_norm(self.up)))
    }

    /// World-space `(left_ear, right_ear)` positions for a given inter-aural
    /// distance. Feed these to `rodio::SpatialPlayer` to place a source.
    pub fn ear_positions(&self, ear_distance: f32) -> (Vec3, Vec3) {
        let r = self.right();
        (
            v_sub(self.position, v_scale(r, ear_distance)),
            v_add(self.position, v_scale(r, ear_distance)),
        )
    }
}

/// Tuning for positional-audio distance falloff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialParams {
    /// Within this distance the source is at full volume.
    pub ref_distance: f32,
    /// At or beyond this distance the source is silent.
    pub max_distance: f32,
    /// Exponent shaping the falloff (`1` = linear, `>1` = steeper).
    pub rolloff: f32,
}

impl Default for SpatialParams {
    fn default() -> Self {
        SpatialParams {
            ref_distance: 1.0,
            max_distance: 50.0,
            rolloff: 1.0,
        }
    }
}

/// Distance attenuation in `[0,1]`: full volume inside `ref_distance`, linear
/// falloff to silence at `max_distance`, shaped by `rolloff`.
pub fn spatial_attenuation(listener: &Listener, source: Vec3, p: &SpatialParams) -> f32 {
    let d = v_len(v_sub(source, listener.position));
    let ref_d = p.ref_distance.max(0.0);
    if d <= ref_d {
        return 1.0;
    }
    let max_d = p.max_distance.max(ref_d);
    if d >= max_d {
        return 0.0;
    }
    let t = (max_d - d) / (max_d - ref_d);
    t.max(0.0).min(1.0).powf(p.rolloff.max(0.0))
}

/// Stereo pan in `[-1,1]` for `source` relative to the listener: `+1` = hard
/// right, `-1` = hard left, `0` = dead ahead.
pub fn spatial_pan(listener: &Listener, source: Vec3) -> f32 {
    let rel = v_sub(source, listener.position);
    let right = listener.right();
    v_dot(v_norm(rel), right).clamp(-1.0, 1.0)
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

    #[test]
    fn buses_are_independent() {
        let mut m = Mixer::new();
        m.set_bus(Bus::Sfx, 0.5);
        // Music bus is untouched by an SFX change.
        assert_eq!(m.gain(Bus::Sfx, 1.0), 0.5);
        assert_eq!(m.gain(Bus::Music, 1.0), 1.0);
        m.set_bus(Bus::Music, 0.25);
        // And the SFX bus is untouched by a music change.
        assert_eq!(m.gain(Bus::Music, 1.0), 0.25);
        assert_eq!(m.gain(Bus::Sfx, 1.0), 0.5);
    }

    #[test]
    fn gain_combines_master_bus_and_sound_volume() {
        let mut m = Mixer::new();
        m.set_master(0.5);
        m.set_bus(Bus::Music, 0.5);
        // 1.0 master->0.5, music bus 0.5, sound volume 0.5 => 0.5*0.5*0.5 = 0.125
        assert_eq!(m.gain(Bus::Music, 0.5), 0.125);
    }

    #[test]
    fn listener_right_is_perpendicular_to_forward() {
        let l = Listener::default();
        // Default forward = -Z, up = +Y => right = +X.
        assert!((l.right()[0] - 1.0).abs() < 1e-5);
        assert!(l.right()[1].abs() < 1e-5);
        assert!(l.right()[2].abs() < 1e-5);
    }

    #[test]
    fn ear_positions_sit_on_right_axis() {
        let l = Listener::default();
        let (left, right) = l.ear_positions(0.2);
        assert_eq!(right, [0.2, 0.0, 0.0]);
        assert_eq!(left, [-0.2, 0.0, 0.0]);
    }

    #[test]
    fn attenuation_is_unity_inside_ref_distance() {
        let l = Listener::default();
        let p = SpatialParams::default();
        assert_eq!(spatial_attenuation(&l, [0.0, 0.0, 0.0], &p), 1.0);
        assert_eq!(spatial_attenuation(&l, [0.5, 0.0, 0.0], &p), 1.0);
    }

    #[test]
    fn attenuation_falls_to_zero_past_max() {
        let l = Listener::default();
        let p = SpatialParams::default(); // max_distance = 50
        assert_eq!(spatial_attenuation(&l, [100.0, 0.0, 0.0], &p), 0.0);
    }

    #[test]
    fn attenuation_decreases_monotonically_with_distance() {
        let l = Listener::default();
        let p = SpatialParams::default();
        let near = spatial_attenuation(&l, [5.0, 0.0, 0.0], &p);
        let far = spatial_attenuation(&l, [20.0, 0.0, 0.0], &p);
        assert!(far < near, "farther source must be quieter: {near} vs {far}");
        assert!(near > 0.0 && far > 0.0);
    }

    #[test]
    fn rolloff_steepens_the_falloff() {
        let l = Listener::default();
        let mut p = SpatialParams::default();
        p.rolloff = 1.0;
        let linear = spatial_attenuation(&l, [25.0, 0.0, 0.0], &p);
        p.rolloff = 3.0;
        let steep = spatial_attenuation(&l, [25.0, 0.0, 0.0], &p);
        assert!(steep < linear, "higher rolloff should attenuate more");
    }

    #[test]
    fn pan_is_centered_ahead_and_full_to_the_sides() {
        let l = Listener::default();
        // Dead ahead (along -Z) => 0 pan.
        assert!(spatial_pan(&l, [0.0, 0.0, -5.0]).abs() < 1e-5);
        // To the listener's right (+X) => +1.
        assert!((spatial_pan(&l, [5.0, 0.0, 0.0]) - 1.0).abs() < 1e-5);
        // To the left (-X) => -1.
        assert!((spatial_pan(&l, [-5.0, 0.0, 0.0]) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn spatial_gain_applies_master_bus_distance_and_pan() {
        let mut m = Mixer::new();
        m.set_master(0.5);
        let p = SpatialParams::default();
        // Source dead ahead at the listener (full att, center pan).
        let [l, r] = m.spatial_gain(
            &Listener::default(),
            [0.0, 0.0, 0.0],
            &p,
            Bus::Sfx,
            1.0,
        );
        // Equal-power center: each channel ~0.707 * master(0.5) * att(1).
        assert!((l - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
        assert!((r - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);

        // A far source behind the listener is silent.
        let [fl, fr] = m.spatial_gain(
            &Listener::default(),
            [0.0, 0.0, 100.0],
            &p,
            Bus::Sfx,
            1.0,
        );
        assert_eq!(fl, 0.0);
        assert_eq!(fr, 0.0);
    }
}
