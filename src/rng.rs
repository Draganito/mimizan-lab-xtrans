//! Noise for the tests, the same on every run.

use std::f64::consts::PI;

/// Deterministic Gaussian generator (xorshift64* + Box-Muller).
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seeded generator; a zero seed is replaced by one (xorshift has no
    /// zero state).
    pub fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }
    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `[0, 1)` with 53 random bits.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// One draw per call. The Box-Muller pair is deliberately not reused:
    /// both members share the radius, so neighbouring pixels would have
    /// correlated noise power.
    pub fn gaussian(&mut self) -> f64 {
        let u1 = self.uniform().max(1e-300);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        r * (2.0 * PI * u2).cos()
    }
}
