//! A compact signed 256-bit integer for exact decimal coefficients.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Result};

/// A signed two's-complement 256-bit integer.
///
/// The four words are stored least-significant first. The type exists in the
/// native value model so `Scalar::D256` does not depend on Arrow; Arrow
/// conversion copies the same 32 bytes when that feature is enabled.
///
/// ```
/// use yggdryl::I256;
///
/// let value: I256 = "123456789012345678901234567890".parse()?;
/// assert_eq!(value.to_string(), "123456789012345678901234567890");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Default, Eq, Hash, PartialEq)]
#[repr(transparent)]
pub struct I256([u64; 4]);

impl I256 {
    /// Zero.
    pub const ZERO: Self = Self([0; 4]);

    /// Return the deterministic hash of the exact two's-complement value.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
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
        let mut words = [0_u64; 4];
        let mut word = 0;
        while word < 4 {
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

    /// Return the little-endian two's-complement representation.
    pub const fn into_le_bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        let mut word = 0;
        while word < 4 {
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
        if self.0[2] == 0 && self.0[3] == 0 {
            Some(self.0[0] as u128 | ((self.0[1] as u128) << 64))
        } else {
            None
        }
    }

    /// Return whether this value is negative.
    pub const fn is_negative(self) -> bool {
        self.0[3] >> 63 != 0
    }

    /// Negate this integer, returning `None` only for the signed minimum.
    pub fn checked_neg(self) -> Option<Self> {
        if self.0 == [0, 0, 0, 1 << 63] {
            None
        } else {
            Some(Self(wrapping_neg(self.0)))
        }
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
        self.0[0] == 0 && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }

    /// Add without wrapping the signed 256-bit range.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        signed_add(
            self.unsigned_abs(),
            self.is_negative(),
            other.unsigned_abs(),
            other.is_negative(),
        )
    }

    /// Subtract without wrapping the signed 256-bit range.
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        signed_add(
            self.unsigned_abs(),
            self.is_negative(),
            other.unsigned_abs(),
            !other.is_negative(),
        )
    }

    /// Multiply without wrapping the signed 256-bit range.
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        let magnitude = unsigned_mul(self.unsigned_abs(), other.unsigned_abs())?;
        Self::from_magnitude(magnitude, self.is_negative() ^ other.is_negative())
    }

    /// Divide with truncation toward zero, returning `None` for zero or overflow.
    pub fn checked_div(self, other: Self) -> Option<Self> {
        if other.is_zero() {
            return None;
        }
        let (quotient, _) = unsigned_div_rem(self.unsigned_abs(), other.unsigned_abs());
        Self::from_magnitude(quotient, self.is_negative() ^ other.is_negative())
    }

    /// Return the signed remainder, returning `None` only for a zero divisor.
    pub fn checked_rem(self, other: Self) -> Option<Self> {
        if other.is_zero() {
            return None;
        }
        let (_, remainder) = unsigned_div_rem(self.unsigned_abs(), other.unsigned_abs());
        Self::from_magnitude(remainder, self.is_negative())
    }

    pub(crate) fn checked_mul_ten(self) -> Option<Self> {
        let negative = self.is_negative();
        let magnitude = self.unsigned_abs();
        let multiplied = unsigned_mul_small(magnitude, 10)?;
        Self::from_magnitude(multiplied, negative)
    }

    pub(crate) fn divided_by_ten(self) -> Option<Self> {
        let negative = self.is_negative();
        let (magnitude, remainder) = unsigned_div_small(self.unsigned_abs(), 10);
        (remainder == 0)
            .then(|| Self::from_magnitude(magnitude, negative))
            .flatten()
    }

    fn unsigned_abs(self) -> [u64; 4] {
        if self.is_negative() {
            wrapping_neg(self.0)
        } else {
            self.0
        }
    }

    fn from_magnitude(magnitude: [u64; 4], negative: bool) -> Option<Self> {
        let sign_bit = magnitude[3] >> 63;
        if sign_bit != 0 && (!negative || magnitude != [0, 0, 0, 1 << 63]) {
            return None;
        }
        Some(if negative && magnitude != [0; 4] {
            Self(wrapping_neg(magnitude))
        } else {
            Self(magnitude)
        })
    }
}

impl From<i128> for I256 {
    fn from(value: i128) -> Self {
        Self::from_i128(value)
    }
}

impl fmt::Debug for I256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for I256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            return formatter.write_str("0");
        }
        let negative = self.is_negative();
        let mut magnitude = self.unsigned_abs();
        let mut digits = [0_u8; 78];
        let mut length = 0;
        while magnitude != [0; 4] {
            let (quotient, remainder) = unsigned_div_small(magnitude, 10);
            digits[length] = b'0' + u8::try_from(remainder).unwrap_or(0);
            length += 1;
            magnitude = quotient;
        }
        if negative {
            formatter.write_str("-")?;
        }
        while length != 0 {
            length -= 1;
            formatter.write_str(std::str::from_utf8(&digits[length..=length]).unwrap_or("0"))?;
        }
        Ok(())
    }
}

impl Ord for I256 {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0[3] as i64)
            .cmp(&(other.0[3] as i64))
            .then_with(|| self.0[2].cmp(&other.0[2]))
            .then_with(|| self.0[1].cmp(&other.0[1]))
            .then_with(|| self.0[0].cmp(&other.0[0]))
    }
}

impl PartialOrd for I256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

macro_rules! checked_binary_operator {
    ($trait:ident, $method:ident, $checked:ident, $message:literal) => {
        impl $trait for I256 {
            type Output = Self;

            fn $method(self, other: Self) -> Self::Output {
                self.$checked(other).expect($message)
            }
        }
    };
}

checked_binary_operator!(Add, add, checked_add, "signed 256-bit addition overflow");
checked_binary_operator!(Sub, sub, checked_sub, "signed 256-bit subtraction overflow");
checked_binary_operator!(
    Mul,
    mul,
    checked_mul,
    "signed 256-bit multiplication overflow"
);
checked_binary_operator!(Div, div, checked_div, "signed 256-bit division failed");
checked_binary_operator!(Rem, rem, checked_rem, "signed 256-bit remainder by zero");

impl Neg for I256 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self.checked_neg()
            .expect("signed 256-bit negation overflow")
    }
}

macro_rules! assign_operator {
    ($trait:ident, $method:ident, $operator:tt) => {
        impl $trait for I256 {
            fn $method(&mut self, other: Self) {
                *self = *self $operator other;
            }
        }
    };
}

assign_operator!(AddAssign, add_assign, +);
assign_operator!(SubAssign, sub_assign, -);
assign_operator!(MulAssign, mul_assign, *);
assign_operator!(DivAssign, div_assign, /);
assign_operator!(RemAssign, rem_assign, %);

impl FromStr for I256 {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        let (negative, digits) = match value.as_bytes().first() {
            Some(b'-') => (true, &value[1..]),
            Some(b'+') => (false, &value[1..]),
            _ => (false, value),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(parse_error("expected base-10 digits"));
        }
        let mut magnitude = [0_u64; 4];
        for digit in digits.bytes() {
            magnitude = unsigned_mul_small(magnitude, 10)
                .and_then(|held| unsigned_add_small(held, u64::from(digit - b'0')))
                .ok_or_else(|| parse_error("integer exceeds 256 bits"))?;
        }
        Self::from_magnitude(magnitude, negative)
            .ok_or_else(|| parse_error("integer is outside the signed 256-bit range"))
    }
}

impl Serialize for I256 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for I256 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = I256;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a signed 256-bit base-10 integer")
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(I256::from_i128(i128::from(value)))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(I256::from_i128(i128::from(value)))
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

fn wrapping_neg(mut words: [u64; 4]) -> [u64; 4] {
    let mut carry = true;
    for word in &mut words {
        *word = !*word;
        if carry {
            let (next, overflow) = word.overflowing_add(1);
            *word = next;
            carry = overflow;
        }
    }
    words
}

fn signed_add(
    left: [u64; 4],
    left_negative: bool,
    right: [u64; 4],
    right_negative: bool,
) -> Option<I256> {
    if left_negative == right_negative {
        return I256::from_magnitude(unsigned_add(left, right)?, left_negative);
    }
    match unsigned_cmp(left, right) {
        Ordering::Greater => I256::from_magnitude(unsigned_sub(left, right), left_negative),
        Ordering::Less => I256::from_magnitude(unsigned_sub(right, left), right_negative),
        Ordering::Equal => Some(I256::ZERO),
    }
}

fn unsigned_cmp(left: [u64; 4], right: [u64; 4]) -> Ordering {
    for index in (0..4).rev() {
        let ordering = left[index].cmp(&right[index]);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn unsigned_add(left: [u64; 4], right: [u64; 4]) -> Option<[u64; 4]> {
    let mut output = [0_u64; 4];
    let mut carry = false;
    for index in 0..4 {
        let (held, first) = left[index].overflowing_add(right[index]);
        let (held, second) = held.overflowing_add(u64::from(carry));
        output[index] = held;
        carry = first || second;
    }
    (!carry).then_some(output)
}

/// Subtract unsigned limbs where `left >= right`.
fn unsigned_sub(left: [u64; 4], right: [u64; 4]) -> [u64; 4] {
    let mut output = [0_u64; 4];
    let mut borrow = false;
    for index in 0..4 {
        let (held, first) = left[index].overflowing_sub(right[index]);
        let (held, second) = held.overflowing_sub(u64::from(borrow));
        output[index] = held;
        borrow = first || second;
    }
    debug_assert!(!borrow);
    output
}

fn unsigned_mul(left: [u64; 4], right: [u64; 4]) -> Option<[u64; 4]> {
    let mut product = [0_u64; 8];
    for (left_index, left_word) in left.into_iter().enumerate() {
        let mut carry = 0_u128;
        for (right_index, right_word) in right.into_iter().enumerate() {
            let index = left_index + right_index;
            let held =
                u128::from(left_word) * u128::from(right_word) + u128::from(product[index]) + carry;
            product[index] = held as u64;
            carry = held >> 64;
        }
        product[left_index + 4] = carry as u64;
    }
    (product[4..].iter().all(|word| *word == 0))
        .then(|| [product[0], product[1], product[2], product[3]])
}

fn unsigned_div_rem(dividend: [u64; 4], divisor: [u64; 4]) -> ([u64; 4], [u64; 4]) {
    debug_assert!(divisor != [0; 4]);
    let mut quotient = [0_u64; 4];
    let mut remainder = [0_u64; 4];
    for bit in (0..256).rev() {
        let incoming = (dividend[bit / 64] >> (bit % 64)) & 1;
        let mut carry = incoming;
        for word in &mut remainder {
            let next = *word >> 63;
            *word = (*word << 1) | carry;
            carry = next;
        }
        if unsigned_cmp(remainder, divisor) != Ordering::Less {
            remainder = unsigned_sub(remainder, divisor);
            quotient[bit / 64] |= 1_u64 << (bit % 64);
        }
    }
    (quotient, remainder)
}

fn unsigned_mul_small(words: [u64; 4], multiplier: u64) -> Option<[u64; 4]> {
    let mut output = [0_u64; 4];
    let mut carry = 0_u128;
    for (index, word) in words.into_iter().enumerate() {
        let product = u128::from(word) * u128::from(multiplier) + carry;
        output[index] = product as u64;
        carry = product >> 64;
    }
    (carry == 0).then_some(output)
}

fn unsigned_add_small(mut words: [u64; 4], value: u64) -> Option<[u64; 4]> {
    let (first, mut carry) = words[0].overflowing_add(value);
    words[0] = first;
    let mut index = 1;
    while carry && index < words.len() {
        let (word, overflow) = words[index].overflowing_add(1);
        words[index] = word;
        carry = overflow;
        index += 1;
    }
    (!carry).then_some(words)
}

fn unsigned_div_small(words: [u64; 4], divisor: u64) -> ([u64; 4], u64) {
    let mut output = [0_u64; 4];
    let mut remainder = 0_u128;
    for index in (0..4).rev() {
        let dividend = (remainder << 64) | u128::from(words[index]);
        output[index] = (dividend / u128::from(divisor)) as u64;
        remainder = dividend % u128::from(divisor);
    }
    (output, remainder as u64)
}

fn parse_error(reason: &'static str) -> Error {
    Error::Parse {
        target: "i256",
        position: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::I256;

    #[test]
    fn signed_boundaries_round_trip() {
        for text in [
            "0",
            "-1",
            "170141183460469231731687303715884105727",
            "-170141183460469231731687303715884105728",
            "57896044618658097711785492504343953926634992332820282019728792003956564819967",
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968",
        ] {
            let value: I256 = text.parse().unwrap();
            assert_eq!(value.to_string(), text);
        }
    }

    #[test]
    fn out_of_range_values_are_refused() {
        assert!(
            "57896044618658097711785492504343953926634992332820282019728792003956564819968"
                .parse::<I256>()
                .is_err()
        );
        assert!(
            "-57896044618658097711785492504343953926634992332820282019728792003956564819969"
                .parse::<I256>()
                .is_err()
        );
    }

    #[test]
    fn ordering_is_signed() {
        let values = [
            "-100000000000000000000",
            "-1",
            "0",
            "1",
            "100000000000000000000",
        ]
        .map(|value| value.parse::<I256>().unwrap());
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn stable_hash_covers_all_four_limbs_deterministically() {
        let values = [0, 8, 16, 24].map(|byte| {
            let mut bytes = [0_u8; 32];
            bytes[byte] = 7;
            I256::from_le_bytes(bytes)
        });

        assert_eq!(values[0].stable_hash(), I256::from_i128(7).stable_hash());
        for (index, value) in values.iter().enumerate() {
            assert_eq!(value.stable_hash(), value.stable_hash());
            for other in &values[index + 1..] {
                assert_ne!(value, other);
                assert_ne!(value.stable_hash(), other.stable_hash());
            }
        }
    }

    #[test]
    fn checked_addition_and_subtraction_cover_signs_and_carries() {
        let held = |value: &str| value.parse::<I256>().unwrap();
        assert_eq!(held("5").checked_add(held("-3")), Some(held("2")));
        assert_eq!(held("-5").checked_add(held("3")), Some(held("-2")));
        assert_eq!(held("-5").checked_add(held("-3")), Some(held("-8")));
        assert_eq!(held("5").checked_sub(held("-3")), Some(held("8")));
        assert_eq!(held("-5").checked_sub(held("3")), Some(held("-8")));
        assert_eq!(
            held("18446744073709551615").checked_add(held("1")),
            Some(held("18446744073709551616"))
        );

        let maximum =
            held("57896044618658097711785492504343953926634992332820282019728792003956564819967");
        let minimum =
            held("-57896044618658097711785492504343953926634992332820282019728792003956564819968");
        assert_eq!(maximum.checked_add(held("1")), None);
        assert_eq!(minimum.checked_sub(held("1")), None);
        assert_eq!(minimum.checked_neg(), None);
        assert_eq!(minimum.checked_abs(), None);
    }

    #[test]
    fn multiplication_detects_the_full_signed_boundary() {
        let held = |value: &str| value.parse::<I256>().unwrap();
        assert_eq!(
            held("340282366920938463463374607431768211456").checked_mul(held("2")),
            Some(held("680564733841876926926749214863536422912"))
        );
        let maximum =
            held("57896044618658097711785492504343953926634992332820282019728792003956564819967");
        assert_eq!(maximum.checked_mul(held("2")), None);
        assert_eq!(held("-12").checked_mul(held("-11")), Some(held("132")));
    }

    #[test]
    fn division_and_remainder_recompose_every_sign_combination() {
        let held = |value: &str| value.parse::<I256>().unwrap();
        let magnitude = held("12345678901234567890123456789012345678901234567890");
        for (left, right) in [
            (magnitude, held("97")),
            (-magnitude, held("97")),
            (magnitude, held("-97")),
            (-magnitude, held("-97")),
        ] {
            let quotient = left.checked_div(right).unwrap();
            let remainder = left.checked_rem(right).unwrap();
            assert_eq!(quotient.checked_mul(right).unwrap() + remainder, left);
            assert!(remainder.is_zero() || remainder.is_negative() == left.is_negative());
        }
        assert_eq!(magnitude.checked_div(I256::ZERO), None);
        assert_eq!(magnitude.checked_rem(I256::ZERO), None);

        let minimum =
            held("-57896044618658097711785492504343953926634992332820282019728792003956564819968");
        assert_eq!(minimum.checked_div(held("-1")), None);
        assert_eq!(minimum.checked_rem(held("-1")), Some(I256::ZERO));
    }
}
