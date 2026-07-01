//! The custom 256-bit integers [`I256`] and [`U256`] — foundational value types for
//! callers (such as the schema layer's 256-bit types) that need a native `i256` /
//! `u256`, which Rust lacks. Each is four little-endian 64-bit limbs (limb `0` least
//! significant) and offers the integer basics: ordering, conversion from the smaller
//! integers, wrapping add / sub, byte round-tripping, the `ZERO` / `ONE` / `MIN` /
//! `MAX` constants, and (via [`Bytes`](crate::Bytes)) serialization through an `Io`.
//!
//! ```
//! use yggdryl_core::U256;
//!
//! assert_eq!(U256::from(1u8) + U256::from(2u8), U256::from(3u8));
//! assert_eq!(U256::default(), U256::ZERO);
//! assert!(U256::MAX > U256::from(u64::MAX));
//! ```

use std::cmp::Ordering;

/// Defines a 256-bit integer as four little-endian 64-bit limbs, with the arithmetic
/// shared by the signed and unsigned variants (two's complement add / sub are the
/// same bit operations).
macro_rules! int256 {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name([u64; 4]);

        impl $name {
            /// The value `0`.
            pub const ZERO: Self = Self([0; 4]);
            /// The value `1`.
            pub const ONE: Self = Self([1, 0, 0, 0]);

            /// From four little-endian 64-bit limbs (limb `0` least significant).
            pub const fn from_limbs(limbs: [u64; 4]) -> Self {
                Self(limbs)
            }

            /// The four little-endian 64-bit limbs.
            pub const fn to_limbs(self) -> [u64; 4] {
                self.0
            }

            /// The little-endian 32-byte representation.
            pub const fn to_le_bytes(self) -> [u8; 32] {
                let mut out = [0u8; 32];
                let mut i = 0;
                while i < 4 {
                    let limb = self.0[i].to_le_bytes();
                    let mut j = 0;
                    while j < 8 {
                        out[i * 8 + j] = limb[j];
                        j += 1;
                    }
                    i += 1;
                }
                out
            }

            /// From a little-endian 32-byte representation.
            pub const fn from_le_bytes(bytes: [u8; 32]) -> Self {
                let mut limbs = [0u64; 4];
                let mut i = 0;
                while i < 4 {
                    limbs[i] = u64::from_le_bytes([
                        bytes[i * 8],
                        bytes[i * 8 + 1],
                        bytes[i * 8 + 2],
                        bytes[i * 8 + 3],
                        bytes[i * 8 + 4],
                        bytes[i * 8 + 5],
                        bytes[i * 8 + 6],
                        bytes[i * 8 + 7],
                    ]);
                    i += 1;
                }
                Self(limbs)
            }

            /// Wrapping (modular) addition.
            pub const fn wrapping_add(self, rhs: Self) -> Self {
                let mut limbs = [0u64; 4];
                let mut carry = 0u64;
                let mut i = 0;
                while i < 4 {
                    let (sum, c1) = self.0[i].overflowing_add(rhs.0[i]);
                    let (sum, c2) = sum.overflowing_add(carry);
                    limbs[i] = sum;
                    carry = c1 as u64 + c2 as u64;
                    i += 1;
                }
                Self(limbs)
            }

            /// Wrapping (modular) subtraction.
            pub const fn wrapping_sub(self, rhs: Self) -> Self {
                let mut limbs = [0u64; 4];
                let mut borrow = 0u64;
                let mut i = 0;
                while i < 4 {
                    let (diff, b1) = self.0[i].overflowing_sub(rhs.0[i]);
                    let (diff, b2) = diff.overflowing_sub(borrow);
                    limbs[i] = diff;
                    borrow = b1 as u64 + b2 as u64;
                    i += 1;
                }
                Self(limbs)
            }
        }

        impl std::ops::Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                self.wrapping_add(rhs)
            }
        }

        impl std::ops::Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                self.wrapping_sub(rhs)
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

int256!(
    U256,
    "A 256-bit unsigned integer, stored as four little-endian 64-bit limbs."
);

impl U256 {
    /// The smallest value (`0`).
    pub const MIN: Self = Self::ZERO;
    /// The largest value.
    pub const MAX: Self = Self([u64::MAX; 4]);
}

impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        // Most-significant limb first; all limbs unsigned.
        self.0[3]
            .cmp(&other.0[3])
            .then_with(|| self.0[2].cmp(&other.0[2]))
            .then_with(|| self.0[1].cmp(&other.0[1]))
            .then_with(|| self.0[0].cmp(&other.0[0]))
    }
}

/// `From` the smaller unsigned integers (zero-extended).
macro_rules! u256_from {
    ($($ty:ty),+) => {$(
        impl From<$ty> for U256 {
            fn from(value: $ty) -> Self {
                Self([value as u64, 0, 0, 0])
            }
        }
    )+};
}
u256_from!(u8, u16, u32, u64);

impl From<u128> for U256 {
    fn from(value: u128) -> Self {
        Self([value as u64, (value >> 64) as u64, 0, 0])
    }
}

int256!(
    I256,
    "A 256-bit signed (two's complement) integer, stored as four little-endian 64-bit limbs."
);

impl I256 {
    /// The most negative value.
    pub const MIN: Self = Self([0, 0, 0, 1u64 << 63]);
    /// The largest value.
    pub const MAX: Self = Self([u64::MAX, u64::MAX, u64::MAX, i64::MAX as u64]);

    /// The two's-complement negation.
    pub const fn wrapping_neg(self) -> Self {
        Self::ZERO.wrapping_sub(self)
    }
}

impl Ord for I256 {
    fn cmp(&self, other: &Self) -> Ordering {
        // The most-significant limb carries the sign, so compare it as `i64`; the
        // lower limbs are magnitude bits, compared unsigned.
        (self.0[3] as i64)
            .cmp(&(other.0[3] as i64))
            .then_with(|| self.0[2].cmp(&other.0[2]))
            .then_with(|| self.0[1].cmp(&other.0[1]))
            .then_with(|| self.0[0].cmp(&other.0[0]))
    }
}

impl std::ops::Neg for I256 {
    type Output = Self;
    fn neg(self) -> Self {
        self.wrapping_neg()
    }
}

/// `From` the smaller signed integers (sign-extended).
macro_rules! i256_from {
    ($($ty:ty),+) => {$(
        impl From<$ty> for I256 {
            fn from(value: $ty) -> Self {
                let ext = if value < 0 { u64::MAX } else { 0 };
                Self([value as i64 as u64, ext, ext, ext])
            }
        }
    )+};
}
i256_from!(i8, i16, i32, i64);

impl From<i128> for I256 {
    fn from(value: i128) -> Self {
        let ext = if value < 0 { u64::MAX } else { 0 };
        Self([value as u64, (value >> 64) as u64, ext, ext])
    }
}
