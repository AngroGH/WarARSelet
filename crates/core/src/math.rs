//! Deterministic fixed-point math.
//!
//! The whole simulation uses integer math only: no `f32`/`f64` is allowed
//! inside gameplay logic, so two machines (or a replay) always compute the
//! exact same result. Floating point exists only in presentation helpers
//! (`to_f32`) used by clients.
//!
//! `Fixed` stores a Q24.8 number: 8 fractional bits, one tile == 256 units.

use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

pub const FRAC_BITS: u32 = 8;
pub const SCALE: i32 = 1 << FRAC_BITS;

/// Q24.8 fixed-point number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub struct Fixed(pub i32);

impl Fixed {
    pub const ZERO: Self = Fixed(0);
    pub const ONE: Self = Fixed(SCALE);

    #[inline]
    pub const fn from_bits(bits: i32) -> Self {
        Fixed(bits)
    }

    #[inline]
    pub const fn to_bits(self) -> i32 {
        self.0
    }

    #[inline]
    pub const fn from_int(v: i32) -> Self {
        Fixed(v * SCALE)
    }

    /// Build from thousandths of a tile: `from_milli(1500) == 1.5`.
    pub fn from_milli(v: i32) -> Self {
        Fixed(((v as i64) * SCALE as i64 / 1000) as i32)
    }

    /// Truncating conversion toward zero.
    #[inline]
    pub fn to_int(self) -> i32 {
        self.0 / SCALE
    }

    #[inline]
    pub fn to_milli(self) -> i32 {
        ((self.0 as i64) * 1000 / SCALE as i64) as i32
    }

    #[inline]
    pub fn abs(self) -> Self {
        Fixed(self.0.saturating_abs())
    }

    #[inline]
    pub fn min(self, other: Self) -> Self {
        if self.0 < other.0 {
            self
        } else {
            other
        }
    }

    #[inline]
    pub fn max(self, other: Self) -> Self {
        if self.0 > other.0 {
            self
        } else {
            other
        }
    }

    #[inline]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    #[inline]
    pub fn signum(self) -> i32 {
        self.0.signum()
    }

    /// Presentation only. Never use the result for simulation decisions.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / SCALE as f32
    }
}

impl Add for Fixed {
    type Output = Fixed;
    #[inline]
    fn add(self, rhs: Fixed) -> Fixed {
        Fixed(self.0.saturating_add(rhs.0))
    }
}

impl Sub for Fixed {
    type Output = Fixed;
    #[inline]
    fn sub(self, rhs: Fixed) -> Fixed {
        Fixed(self.0.saturating_sub(rhs.0))
    }
}

impl Neg for Fixed {
    type Output = Fixed;
    #[inline]
    fn neg(self) -> Fixed {
        Fixed(-self.0)
    }
}

impl Mul for Fixed {
    type Output = Fixed;
    #[inline]
    fn mul(self, rhs: Fixed) -> Fixed {
        Fixed(((self.0 as i64 * rhs.0 as i64) >> FRAC_BITS) as i32)
    }
}

impl Mul<i32> for Fixed {
    type Output = Fixed;
    #[inline]
    fn mul(self, rhs: i32) -> Fixed {
        Fixed(self.0.saturating_mul(rhs))
    }
}

impl Div for Fixed {
    type Output = Fixed;
    #[inline]
    fn div(self, rhs: Fixed) -> Fixed {
        if rhs.0 == 0 {
            return Fixed(0);
        }
        Fixed((((self.0 as i64) << FRAC_BITS) / rhs.0 as i64) as i32)
    }
}

impl AddAssign for Fixed {
    #[inline]
    fn add_assign(&mut self, rhs: Fixed) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl SubAssign for Fixed {
    #[inline]
    fn sub_assign(&mut self, rhs: Fixed) {
        self.0 = self.0.saturating_sub(rhs.0);
    }
}

/// Integer square root (Newton method). Exact for all `u64` inputs.
pub fn isqrt(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    x
}

/// 2D vector in fixed-point tile space.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: Fixed,
    pub y: Fixed,
}

impl Vec2 {
    pub const ZERO: Self = Vec2 {
        x: Fixed::ZERO,
        y: Fixed::ZERO,
    };

    #[inline]
    pub const fn new(x: Fixed, y: Fixed) -> Self {
        Vec2 { x, y }
    }

    #[inline]
    pub fn from_ints(x: i32, y: i32) -> Self {
        Vec2::new(Fixed::from_int(x), Fixed::from_int(y))
    }

    #[inline]
    pub fn from_milli(x: i32, y: i32) -> Self {
        Vec2::new(Fixed::from_milli(x), Fixed::from_milli(y))
    }

    pub fn length(self) -> Fixed {
        let x = self.x.to_bits() as i64;
        let y = self.y.to_bits() as i64;
        Fixed(isqrt((x * x + y * y) as u64) as i32)
    }

    pub fn dist_sq(a: Self, b: Self) -> i64 {
        let dx = (a.x.to_bits() - b.x.to_bits()) as i64;
        let dy = (a.y.to_bits() - b.y.to_bits()) as i64;
        dx * dx + dy * dy
    }

    pub fn dist(a: Self, b: Self) -> Fixed {
        Fixed(isqrt(Vec2::dist_sq(a, b) as u64) as i32)
    }

    /// Unit-length direction, or `ZERO` when the vector is degenerate.
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len.to_bits() <= 0 {
            return Vec2::ZERO;
        }
        Vec2::new(self.x / len, self.y / len)
    }

    /// Move `self` toward `target` by at most `max_step`.
    pub fn step_toward(self, target: Self, max_step: Fixed) -> (Self, bool) {
        let delta = target - self;
        let dist = delta.length();
        if dist <= max_step || dist.to_bits() == 0 {
            (target, true)
        } else {
            (self + delta.normalized() * max_step, false)
        }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<Fixed> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, rhs: Fixed) -> Vec2 {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Tile coordinate (integer grid cell).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize)]
pub struct TilePos {
    pub x: i32,
    pub y: i32,
}

impl TilePos {
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        TilePos { x, y }
    }

    pub fn center(self) -> Vec2 {
        Vec2::from_ints(self.x * 2 + 1, self.y * 2 + 1) * Fixed::from_milli(500)
    }

    pub fn from_vec(v: Vec2) -> Self {
        TilePos::new(v.x.to_int(), v.y.to_int())
    }

    pub fn dist(self, other: TilePos) -> i32 {
        let dx = (self.x - other.x).abs();
        let dy = (self.y - other.y).abs();
        dx.max(dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_roundtrip() {
        let a = Fixed::from_int(3) + Fixed::from_milli(500);
        assert_eq!(a.to_milli(), 3500);
        assert_eq!(a.to_int(), 3);
        assert_eq!((a - Fixed::from_int(1)).to_milli(), 2500);
        assert_eq!((Fixed::from_int(2) * Fixed::from_milli(250)).to_milli(), 500);
        assert_eq!(Fixed::from_int(1).to_f32(), 1.0);
    }

    #[test]
    fn integer_sqrt() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(1_000_000), 1000);
    }

    #[test]
    fn vector_math() {
        let a = Vec2::from_ints(0, 0);
        let b = Vec2::from_ints(3, 4);
        assert_eq!(Vec2::dist(a, b).to_int(), 5);
        let (pos, arrived) = a.step_toward(b, Fixed::from_int(1));
        assert!(!arrived);
        // 3/5 and 4/5 of a tile, with 1/256 truncation error at most.
        assert!((pos.x.to_milli() - 600).abs() <= 4, "x = {}", pos.x.to_milli());
        assert!((pos.y.to_milli() - 800).abs() <= 4, "y = {}", pos.y.to_milli());
        assert!((pos.length().to_milli() - 1000).abs() <= 4);
    }
}
