//! Deterministic pseudo-random generator (SplitMix64).
//!
//! Every random decision in the simulation (map generation, meteor strikes,
//! scatter offsets) goes through this generator so replays stay identical.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub const fn seeded(seed: u64) -> Self {
        Rng {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform value in `0..n` (`n == 0` returns 0).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }

    /// Uniform value in `lo..hi`.
    pub fn in_range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            lo
        } else {
            lo + self.below((hi - lo) as u32) as i32
        }
    }

    /// `true` with probability `num / den`.
    pub fn chance(&mut self, num: u32, den: u32) -> bool {
        den > 0 && self.below(den) < num
    }

    /// Roll an index from a weight table.
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        let mut roll = self.below(total);
        for (i, w) in weights.iter().enumerate() {
            if roll < *w {
                return i;
            }
            roll -= *w;
        }
        weights.len().saturating_sub(1)
    }

    pub fn state(&self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_stream() {
        let mut a = Rng::seeded(1234);
        let mut b = Rng::seeded(1234);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut c = Rng::seeded(1235);
        assert_ne!(Rng::seeded(1234).next_u64(), c.next_u64());
    }

    #[test]
    fn ranges() {
        let mut rng = Rng::seeded(7);
        for _ in 0..10_000 {
            let v = rng.in_range(10, 20);
            assert!((10..20).contains(&v));
        }
    }
}
