//! Deterministic RNG, plumbed through the world as a resource.
//!
//! Determinism is a core philosophy of TPT Nova: AI agents must be able to
//! replay a simulation from a seed and get identical results. We use a simple,
//! fast xorshift64* generator with no external state.

use std::any::TypeId;

/// A small, fast, fully deterministic PRNG (xorshift64*).
#[derive(Debug, Clone)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        // Avoid a zero state, which would lock the generator.
        let state = if seed == 0 { 0x9E3779B97F4A7C15 } else { seed };
        DeterministicRng { state }
    }

    /// Advance the generator and return a `u64`.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// A `f32` in the range `[0, 1)`.
    pub fn next_f32(&mut self) -> f32 {
        // Take the top 24 bits for a uniform float.
        ((self.next_u64() >> 40) as f32) / (1u32 << 24) as f32
    }

    /// A `f32` in `[min, max)`.
    pub fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        min + self.next_f32() * (max - min)
    }

    pub fn seed(&self) -> u64 {
        self.state
    }
}

/// Resource wrapper so the RNG can live in the world's resource map under a
/// stable key and be retrieved by systems.
#[derive(Debug, Clone)]
pub struct RngResource {
    pub rng: DeterministicRng,
    pub seed: u64,
}

impl RngResource {
    pub fn new(seed: u64) -> Self {
        RngResource {
            rng: DeterministicRng::new(seed),
            seed,
        }
    }
}

/// Stable resource key for the RNG resource.
pub fn rng_resource_id() -> TypeId {
    TypeId::of::<RngResource>()
}
