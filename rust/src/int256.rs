//! The compact 256-bit integer pair the exact decimals are built on.
//!
//! [`u256`] is the unsigned magnitude every wide computation runs in, and
//! [`i256`] is the signed two's-complement value [`Decimal256`] stores. Both
//! keep four 64-bit words least-significant first, so a little-endian buffer
//! is the same 32 bytes in either direction and Arrow conversion copies rather
//! than re-encodes.
//!
//! They are spelled like the native integers they extend - `i256`, `u256`,
//! beside `i128` and `u128` - because that is what a declaration reads as.
//!
//! [`Decimal256`]: crate::decimal::Decimal256

#![allow(non_camel_case_types)]

use std::cmp::Ordering;
use std::fmt;
use std::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Result};

/// The number of 64-bit words a 256-bit integer is stored in.
const WORDS: usize = 4;

/// An unsigned 256-bit integer.
///
/// The four words are stored least-significant first. It is the magnitude
/// [`i256`] computes in - an unsigned add, multiply, or division carries no
/// sign to reason about - and a value in its own right wherever a count
/// exceeds 128 bits.
///
/// ```
/// use yggdryl::u256;
///
/// let value: u256 = "115792089237316195423570985008687907853269984665640564039457584007913129639935"
///     .parse()?;
/// assert_eq!(value, u256::MAX);
/// assert_eq!(u256::from_u128(10).checked_mul(u256::from_u128(10)), Some(u256::from_u128(100)));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct u256([u64; WORDS]);

impl u256 {
    /// Zero.
    pub const ZERO: Self = Self([0; WORDS]);

    /// The largest representable value.
    pub const MAX: Self = Self([u64::MAX; WORDS]);

    /// Return the deterministic hash of the exact 256-bit value.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// Build from an unsigned 128-bit integer.
    pub const fn from_u128(value: u128) -> Self {
        Self([value as u64, (value >> 64) as u64, 0, 0])
    }

    /// Build from a little-endian representation.
    pub const fn from_le_bytes(bytes: [u8; 32]) -> Self {
        let mut words = [0_u64; WORDS];
        let mut word = 0;
        while word < WORDS {
            let start = word * 8;
            words[word] = u64::from_le_bytes([
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
                bytes[start + 4],
                bytes[start + 5],
                bytes[start + 6],
                bytes[start + 7],
            ]);
            word += 1;
        }
        Self(words)
    }

    /// Return the little-endian representation.
    pub const fn into_le_bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        let mut word = 0;
        while word < WORDS {
            let encoded = self.0[word].to_le_bytes();
            let mut byte = 0;
            while byte < 8 {
                bytes[word * 8 + byte] = encoded[byte];
                byte += 1;
            }
            word += 1;
        }
        bytes
    }

    /// Return this value as `u128` when it fits.
    pub const fn as_u128(self) -> Option<u128> {
        if self.0[2] == 0 && self.0[3] == 0 {
            Some(self.0[0] as u128 | ((self.0[1] as u128) << 64))
        } else {
            None
        }
    }

    /// Return whether this integer is zero.
    pub const fn is_zero(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    /// Add without wrapping the unsigned 256-bit range.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        let mut output = [0_u64; WORDS];
        let mut carry = false;
        for (index, word) in output.iter_mut().enumerate() {
            let (held, first) = self.0[index].overflowing_add(other.0[index]);
            let (held, second) = held.overflowing_add(u64::from(carry));
            *word = held;
            carry = first || second;
        }
        (!carry).then_some(Self(output))
    }

    /// Subtract without wrapping below zero.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        let (difference, borrow) = self.borrowing_sub(other);
        (!borrow).then_some(difference)
    }

    /// Multiply without wrapping the unsigned 256-bit range.
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        let mut product = [0_u64; WORDS * 2];
        for (left_index, left_word) in self.0.into_iter().enumerate() {
            let mut carry = 0_u128;
            for (right_index, right_word) in other.0.into_iter().enumerate() {
                let index = left_index + right_index;
                let held = u128::from(left_word) * u128::from(right_word)
                    + u128::from(product[index])
                    + carry;
                product[index] = held as u64;
                carry = held >> 64;
            }
            product[left_index + WORDS] = carry as u64;
        }
        (product[WORDS..].iter().all(|word| *word == 0))
            .then(|| Self([product[0], product[1], product[2], product[3]]))
    }

    /// Divide, returning `None` for a zero divisor.
    pub fn checked_div(self, other: Self) -> Option<Self> {
        (!other.is_zero()).then(|| self.div_rem(other).0)
    }

    /// Return the remainder, returning `None` for a zero divisor.
    pub fn checked_rem(self, other: Self) -> Option<Self> {
        (!other.is_zero()).then(|| self.div_rem(other).1)
    }

    /// `10^76`, the first integer no seventy-six-digit coefficient reaches.
    pub(crate) const TEN_POW_76: Self = Self::from_le_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x95, 0x71, 0xf1, 0xa5, 0x75,
        0x77, 0x79, 0x29, 0x65, 0xe8, 0xab, 0xb4, 0x64, 0x07, 0xb5, 0x15, 0x99, 0x11, 0xa7, 0xcc,
        0x1b, 0x16,
    ]);

    /// `self * multiplier / divisor`, truncated toward zero and exact through
    /// a 512-bit product; `None` for a zero divisor or a quotient past 256
    /// bits.
    ///
    /// What a fixed-scale decimal's product and quotient need: `a * b / 10^18`
    /// and `a * 10^18 / b` each pass through an integer up to 512 bits wide
    /// before the one truncation, so neither is a pair of 256-bit operations.
    /// Restoring long division, one bit at a time, as `div_rem` does; the
    /// decimal paths that reach this are per-value, not per-row.
    pub(crate) fn mul_div(self, multiplier: Self, divisor: Self) -> Option<Self> {
        if divisor.is_zero() {
            return None;
        }
        let mut product = [0_u64; WORDS * 2];
        for (left_index, left_word) in self.0.into_iter().enumerate() {
            let mut carry = 0_u128;
            for (right_index, right_word) in multiplier.0.into_iter().enumerate() {
                let index = left_index + right_index;
                let held = u128::from(left_word) * u128::from(right_word)
                    + u128::from(product[index])
                    + carry;
                product[index] = held as u64;
                carry = held >> 64;
            }
            product[left_index + WORDS] = carry as u64;
        }
        let mut quotient = [0_u64; WORDS * 2];
        let mut remainder = Self::ZERO;
        for bit in (0..WORDS * 2 * 64).rev() {
            let incoming = (product[bit / 64] >> (bit % 64)) & 1;
            let mut carry = incoming;
            for word in &mut remainder.0 {
                let next = *word >> 63;
                *word = (*word << 1) | carry;
                carry = next;
            }
            // A bit carried out of the shift means the 257-bit remainder is
            // past the divisor; the wrapped difference is the exact one.
            if carry == 1 || remainder >= divisor {
                remainder = remainder.borrowing_sub(divisor).0;
                quotient[bit / 64] |= 1_u64 << (bit % 64);
            }
        }
        (quotient[WORDS..].iter().all(|word| *word == 0))
            .then(|| Self([quotient[0], quotient[1], quotient[2], quotient[3]]))
    }

    /// Return the quotient and remainder of a non-zero division.
    ///
    /// Restoring long division, one bit at a time: the 256-bit words leave no
    /// wider native type to divide in, and the decimal paths that reach this
    /// are per-value, not per-row.
    fn div_rem(self, divisor: Self) -> (Self, Self) {
        debug_assert!(!divisor.is_zero());
        let mut quotient = [0_u64; WORDS];
        let mut remainder = Self::ZERO;
        for bit in (0..256).rev() {
            let incoming = (self.0[bit / 64] >> (bit % 64)) & 1;
            let mut carry = incoming;
            for word in &mut remainder.0 {
                let next = *word >> 63;
                *word = (*word << 1) | carry;
                carry = next;
            }
            if remainder >= divisor {
                remainder = remainder.borrowing_sub(divisor).0;
                quotient[bit / 64] |= 1_u64 << (bit % 64);
            }
        }
        (Self(quotient), remainder)
    }

    /// Multiply by a single word, refusing to wrap.
    pub(crate) fn checked_mul_word(self, multiplier: u64) -> Option<Self> {
        let mut output = [0_u64; WORDS];
        let mut carry = 0_u128;
        for (index, word) in self.0.into_iter().enumerate() {
            let product = u128::from(word) * u128::from(multiplier) + carry;
            output[index] = product as u64;
            carry = product >> 64;
        }
        (carry == 0).then_some(Self(output))
    }

    /// Add a single word, refusing to wrap.
    pub(crate) fn checked_add_word(mut self, value: u64) -> Option<Self> {
        let (first, mut carry) = self.0[0].overflowing_add(value);
        self.0[0] = first;
        let mut index = 1;
        while carry && index < WORDS {
            let (word, overflow) = self.0[index].overflowing_add(1);
            self.0[index] = word;
            carry = overflow;
            index += 1;
        }
        (!carry).then_some(self)
    }

    /// Divide by a single non-zero word, returning the quotient and remainder.
    pub(crate) fn div_rem_word(self, divisor: u64) -> (Self, u64) {
        debug_assert!(divisor != 0);
        let mut output = [0_u64; WORDS];
        let mut remainder = 0_u128;
        for index in (0..WORDS).rev() {
            let dividend = (remainder << 64) | u128::from(self.0[index]);
            output[index] = (dividend / u128::from(divisor)) as u64;
            remainder = dividend % u128::from(divisor);
        }
        (Self(output), remainder as u64)
    }

    /// The base-10 digits this integer is written in; zero is one digit.
    ///
    /// Within 128 bits it is the native logarithm; past them one word
    /// division by ten per digit, so a width's precision is checked without
    /// rendering the number.
    pub(crate) fn decimal_digits(self) -> u32 {
        let mut magnitude = self;
        let mut digits = 0;
        loop {
            if let Some(narrow) = magnitude.as_u128() {
                return digits + narrow.checked_ilog10().map_or(1, |log| log + 1);
            }
            magnitude = magnitude.div_rem_word(10).0;
            digits += 1;
        }
    }

    /// Subtract, reporting the borrow out of the top word.
    fn borrowing_sub(self, other: Self) -> (Self, bool) {
        let mut output = [0_u64; WORDS];
        let mut borrow = false;
        for (index, word) in output.iter_mut().enumerate() {
            let (held, first) = self.0[index].overflowing_sub(other.0[index]);
            let (held, second) = held.overflowing_sub(u64::from(borrow));
            *word = held;
            borrow = first || second;
        }
        (Self(output), borrow)
    }

    /// Return the two's-complement negation, which wraps at zero.
    const fn wrapping_neg(self) -> Self {
        let mut words = self.0;
        let mut carry = true;
        let mut index = 0;
        while index < WORDS {
            words[index] = !words[index];
            if carry {
                let (next, overflow) = words[index].overflowing_add(1);
                words[index] = next;
                carry = overflow;
            }
            index += 1;
        }
        Self(words)
    }

    /// Return whether the top bit is set, which is the signed minimum's shape.
    const fn has_sign_bit(self) -> bool {
        self.0[WORDS - 1] >> 63 != 0
    }

    /// Write the base-10 digits, most significant first.
    fn write_digits(self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return formatter.write_str("0");
        }
        // 2^256 is 78 digits, so one stack buffer renders any value.
        let mut digits = [0_u8; 78];
        let mut length = 0;
        let mut magnitude = self;
        while !magnitude.is_zero() {
            let (quotient, remainder) = magnitude.div_rem_word(10);
            digits[length] = b'0' + u8::try_from(remainder).unwrap_or(0);
            length += 1;
            magnitude = quotient;
        }
        digits[..length].reverse();
        formatter.write_str(std::str::from_utf8(&digits[..length]).unwrap_or("0"))
    }

    /// Read base-10 digits, refusing anything that does not fit 256 bits.
    ///
    /// `target` names the type the caller is parsing, so a signed parse
    /// reports `i256` rather than the magnitude type it ran through.
    fn from_digits(digits: &str, target: &'static str) -> Result<Self> {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(parse_error(target, "expected base-10 digits"));
        }
        let mut magnitude = Self::ZERO;
        for digit in digits.bytes() {
            magnitude = magnitude
                .checked_mul_word(10)
                .and_then(|held| held.checked_add_word(u64::from(digit - b'0')))
                .ok_or_else(|| parse_error(target, "integer exceeds 256 bits"))?;
        }
        Ok(magnitude)
    }
}

impl From<u128> for u256 {
    fn from(value: u128) -> Self {
        Self::from_u128(value)
    }
}

impl fmt::Debug for u256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for u256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_digits(formatter)
    }
}

impl Ord for u256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for index in (0..WORDS).rev() {
            let ordering = self.0[index].cmp(&other.0[index]);
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for u256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FromStr for u256 {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        let digits = value.strip_prefix('+').unwrap_or(value);
        Self::from_digits(digits, "u256")
    }
}

/// A signed two's-complement 256-bit integer.
///
/// The four words are stored least-significant first. The type exists in the
/// native value model so [`crate::decimal::Decimal256`] does not depend
/// on Arrow; Arrow conversion copies the same 32 bytes when that feature is
/// enabled.
///
/// ```
/// use yggdryl::i256;
///
/// let value: i256 = "123456789012345678901234567890".parse()?;
/// assert_eq!(value.to_string(), "123456789012345678901234567890");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct i256([u64; WORDS]);

impl i256 {
    /// Zero.
    pub const ZERO: Self = Self([0; WORDS]);

    /// The most negative representable value, whose magnitude has no positive
    /// counterpart.
    const MIN_MAGNITUDE: u256 = u256([0, 0, 0, 1 << 63]);

    /// Return the deterministic hash of the exact two's-complement value.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// Build from a signed 128-bit integer.
    pub const fn from_i128(value: i128) -> Self {
        let low = value as u128;
        let fill = if value < 0 { u64::MAX } else { 0 };
        Self([low as u64, (low >> 64) as u64, fill, fill])
    }

    /// Build from an unsigned 128-bit integer.
    pub const fn from_u128(value: u128) -> Self {
        Self([value as u64, (value >> 64) as u64, 0, 0])
    }

    /// Build from a little-endian two's-complement representation.
    pub const fn from_le_bytes(bytes: [u8; 32]) -> Self {
        Self(u256::from_le_bytes(bytes).0)
    }

    /// Return the little-endian two's-complement representation.
    pub const fn into_le_bytes(self) -> [u8; 32] {
        u256(self.0).into_le_bytes()
    }

    /// Return this value as `i128` when it fits.
    pub const fn as_i128(self) -> Option<i128> {
        let low = self.0[0] as u128 | ((self.0[1] as u128) << 64);
        let negative = self.0[1] >> 63 != 0;
        let fill = if negative { u64::MAX } else { 0 };
        if self.0[2] == fill && self.0[3] == fill {
            Some(low as i128)
        } else {
            None
        }
    }

    /// Return this value as `u128` when it is non-negative and fits.
    pub const fn as_u128(self) -> Option<u128> {
        u256(self.0).as_u128()
    }

    /// Return whether this value is negative.
    pub const fn is_negative(self) -> bool {
        u256(self.0).has_sign_bit()
    }

    /// Negate this integer, returning `None` only for the signed minimum.
    pub fn checked_neg(self) -> Option<Self> {
        (u256(self.0) != Self::MIN_MAGNITUDE).then(|| Self(u256(self.0).wrapping_neg().0))
    }

    /// Return the non-negative magnitude, or `None` for the signed minimum.
    pub fn checked_abs(self) -> Option<Self> {
        if self.is_negative() {
            self.checked_neg()
        } else {
            Some(self)
        }
    }

    /// Return whether this integer is zero.
    pub const fn is_zero(self) -> bool {
        u256(self.0).is_zero()
    }

    /// Add without wrapping the signed 256-bit range.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        Self::from_signed_parts(
            self.unsigned_abs(),
            self.is_negative(),
            other.unsigned_abs(),
            other.is_negative(),
        )
    }

    /// Subtract without wrapping the signed 256-bit range.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        Self::from_signed_parts(
            self.unsigned_abs(),
            self.is_negative(),
            other.unsigned_abs(),
            !other.is_negative(),
        )
    }

    /// Multiply without wrapping the signed 256-bit range.
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        let magnitude = self.unsigned_abs().checked_mul(other.unsigned_abs())?;
        Self::from_magnitude(magnitude, self.is_negative() ^ other.is_negative())
    }

    /// Divide with truncation toward zero, returning `None` for zero or overflow.
    pub fn checked_div(self, other: Self) -> Option<Self> {
        let quotient = self.unsigned_abs().checked_div(other.unsigned_abs())?;
        Self::from_magnitude(quotient, self.is_negative() ^ other.is_negative())
    }

    /// Return the signed remainder, returning `None` only for a zero divisor.
    pub fn checked_rem(self, other: Self) -> Option<Self> {
        let remainder = self.unsigned_abs().checked_rem(other.unsigned_abs())?;
        Self::from_magnitude(remainder, self.is_negative())
    }

    /// Return the magnitude, which is unsigned exactly so the signed minimum
    /// has one.
    pub const fn unsigned_abs(self) -> u256 {
        if self.is_negative() {
            u256(self.0).wrapping_neg()
        } else {
            u256(self.0)
        }
    }

    pub(crate) fn checked_mul_ten(self) -> Option<Self> {
        let negative = self.is_negative();
        let multiplied = self.unsigned_abs().checked_mul_word(10)?;
        Self::from_magnitude(multiplied, negative)
    }

    pub(crate) fn divided_by_ten(self) -> Option<Self> {
        let negative = self.is_negative();
        let (magnitude, remainder) = self.unsigned_abs().div_rem_word(10);
        (remainder == 0)
            .then(|| Self::from_magnitude(magnitude, negative))
            .flatten()
    }

    /// Rebuild a signed value from a magnitude and a sign.
    ///
    /// A magnitude whose top bit is set is out of range for a positive value,
    /// and for a negative one only the signed minimum itself fits.
    fn from_magnitude(magnitude: u256, negative: bool) -> Option<Self> {
        if magnitude.has_sign_bit() && (!negative || magnitude != Self::MIN_MAGNITUDE) {
            return None;
        }
        Some(if negative && !magnitude.is_zero() {
            Self(magnitude.wrapping_neg().0)
        } else {
            Self(magnitude.0)
        })
    }

    /// Add two signed values given as magnitude and sign.
    fn from_signed_parts(
        left: u256,
        left_negative: bool,
        right: u256,
        right_negative: bool,
    ) -> Option<Self> {
        if left_negative == right_negative {
            return Self::from_magnitude(left.checked_add(right)?, left_negative);
        }
        match left.cmp(&right) {
            Ordering::Greater => Self::from_magnitude(left.borrowing_sub(right).0, left_negative),
            Ordering::Less => Self::from_magnitude(right.borrowing_sub(left).0, right_negative),
            Ordering::Equal => Some(Self::ZERO),
        }
    }
}

impl From<i128> for i256 {
    fn from(value: i128) -> Self {
        Self::from_i128(value)
    }
}

impl fmt::Debug for i256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for i256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_negative() {
            formatter.write_str("-")?;
        }
        self.unsigned_abs().write_digits(formatter)
    }
}

impl Ord for i256 {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0[3] as i64)
            .cmp(&(other.0[3] as i64))
            .then_with(|| self.0[2].cmp(&other.0[2]))
            .then_with(|| self.0[1].cmp(&other.0[1]))
            .then_with(|| self.0[0].cmp(&other.0[0]))
    }
}

impl PartialOrd for i256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

macro_rules! checked_binary_operator {
    ($type:ident, $trait:ident, $method:ident, $checked:ident, $message:literal) => {
        impl $trait for $type {
            type Output = Self;

            fn $method(self, other: Self) -> Self::Output {
                self.$checked(other).expect($message)
            }
        }
    };
}

checked_binary_operator!(
    i256,
    Add,
    add,
    checked_add,
    "signed 256-bit addition overflow"
);
checked_binary_operator!(
    i256,
    Sub,
    sub,
    checked_sub,
    "signed 256-bit subtraction overflow"
);
checked_binary_operator!(
    i256,
    Mul,
    mul,
    checked_mul,
    "signed 256-bit multiplication overflow"
);
checked_binary_operator!(
    i256,
    Div,
    div,
    checked_div,
    "signed 256-bit division failed"
);
checked_binary_operator!(
    i256,
    Rem,
    rem,
    checked_rem,
    "signed 256-bit remainder by zero"
);
checked_binary_operator!(
    u256,
    Add,
    add,
    checked_add,
    "unsigned 256-bit addition overflow"
);
checked_binary_operator!(
    u256,
    Sub,
    sub,
    checked_sub,
    "unsigned 256-bit subtraction overflow"
);
checked_binary_operator!(
    u256,
    Mul,
    mul,
    checked_mul,
    "unsigned 256-bit multiplication overflow"
);
checked_binary_operator!(
    u256,
    Div,
    div,
    checked_div,
    "unsigned 256-bit division by zero"
);
checked_binary_operator!(
    u256,
    Rem,
    rem,
    checked_rem,
    "unsigned 256-bit remainder by zero"
);

impl Neg for i256 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self.checked_neg()
            .expect("signed 256-bit negation overflow")
    }
}

macro_rules! assign_operator {
    ($type:ident, $trait:ident, $method:ident, $operator:tt) => {
        impl $trait for $type {
            fn $method(&mut self, other: Self) {
                *self = *self $operator other;
            }
        }
    };
}

assign_operator!(i256, AddAssign, add_assign, +);
assign_operator!(i256, SubAssign, sub_assign, -);
assign_operator!(i256, MulAssign, mul_assign, *);
assign_operator!(i256, DivAssign, div_assign, /);
assign_operator!(i256, RemAssign, rem_assign, %);
assign_operator!(u256, AddAssign, add_assign, +);
assign_operator!(u256, SubAssign, sub_assign, -);
assign_operator!(u256, MulAssign, mul_assign, *);
assign_operator!(u256, DivAssign, div_assign, /);
assign_operator!(u256, RemAssign, rem_assign, %);

impl FromStr for i256 {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        let (negative, digits) = match value.as_bytes().first() {
            Some(b'-') => (true, &value[1..]),
            Some(b'+') => (false, &value[1..]),
            _ => (false, value),
        };
        let magnitude = u256::from_digits(digits, "i256")?;
        Self::from_magnitude(magnitude, negative)
            .ok_or_else(|| parse_error("i256", "integer is outside the signed 256-bit range"))
    }
}

macro_rules! text_serde {
    ($type:ident, $expecting:literal, $from_signed:expr) => {
        impl Serialize for $type {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct Visitor;

                impl serde::de::Visitor<'_> for Visitor {
                    type Value = $type;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str($expecting)
                    }

                    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
                    where
                        E: serde::de::Error,
                    {
                        ($from_signed)(i128::from(value)).ok_or_else(|| {
                            E::custom(concat!("expected ", $expecting, ", got a negative integer"))
                        })
                    }

                    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                        Ok(<$type>::from_u128(u128::from(value)))
                    }

                    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
                    where
                        E: serde::de::Error,
                    {
                        value.parse().map_err(E::custom)
                    }
                }

                deserializer.deserialize_any(Visitor)
            }
        }
    };
}

text_serde!(
    i256,
    "a signed 256-bit base-10 integer",
    |value: i128| Some(i256::from_i128(value))
);
text_serde!(
    u256,
    "an unsigned 256-bit base-10 integer",
    |value: i128| u128::try_from(value).ok().map(u256::from_u128)
);

fn parse_error(target: &'static str, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position: 0,
        reason: reason.into(),
    }
}
