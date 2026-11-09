//! Simulation clock.

use serde::{Deserialize, Serialize};

/// Simulation rate. The spec asks for >= 10 Hz; 20 Hz is a good RTS default
/// (smooth enough for melee, cheap enough for thousands of entities).
pub const TICKS_PER_SECOND: u32 = 20;

/// Duration of one simulation tick in milliseconds.
pub const TICK_MS: u32 = 1000 / TICKS_PER_SECOND;

/// Number of ticks in one in-game second.
pub const TICKS_PER_MINUTE: u32 = TICKS_PER_SECOND * 60;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub struct Tick(pub u64);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    #[inline]
    pub fn next(self) -> Tick {
        Tick(self.0 + 1)
    }

    #[inline]
    pub fn seconds(self) -> u32 {
        (self.0 / TICKS_PER_SECOND as u64) as u32
    }

    #[inline]
    pub fn is_multiple_of(self, n: u32) -> bool {
        n > 0 && self.0 % n as u64 == 0
    }
}

impl core::fmt::Display for Tick {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "t{}", self.0)
    }
}
