//! One fixed-point decimal: `decimal128(38, 18)` already applied, so a price
//! and a quantity add, multiply and compare as integers do.

use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};
use std::str::FromStr;

use smol_str::format_smolstr;

use super::scalars::Decimal128;
use crate::types::i256::i256;
use crate::{DataType, Error, Result, Scalar};

/// The units one whole is: `10^18`.
const ONE_UNITS: i128 = 1_000_000_000_000_000_000;

/// The most units a value holds: `10^38 - 1`, what a 38-digit coefficient
/// states.
const MAX_UNITS: i128 = 99_999_999_999_999_999_999_999_999_999_999_999_999;

/// The mantissa a reading stops adding digits to: past `10^38 - 1` no
/// value holds a further digit ahead of the point, and one behind it is
/// truncated.
const MANTISSA_LIMIT: u128 = 10_000_000_000_000_000_000_000_000_000_000_000_000;

/// The exponent one `e` tail states: an optional sign and digits, or
/// nothing where the tail is not that or is past what an `i32` holds.
fn parse_exponent(tail: &[u8]) -> Option<i32> {
    let (negative, digits) = match tail.split_first()? {
        (b'-', digits) => (true, digits),
        (b'+', digits) => (false, digits),
        _ => (false, tail),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut exponent: i32 = 0;
    for digit in digits {
        exponent = exponent
            .checked_mul(10)?
            .checked_add(i32::from(digit - b'0'))?;
    }
    Some(if negative { -exponent } else { exponent })
}

/// One exact decimal at eighteen fractional digits and thirty-eight digits
/// of precision - Arrow's `decimal128(38, 18)`, preapplied.
///
/// A market's numbers are decimals: a price of `82.5` is exactly that, and
/// a float would hold `82.5000000000000071`. The crate's exact decimals hold
/// any coefficient at any scale, and every operation on two of them first
/// asks which scale they meet at. This one fixes the scale once, at
/// eighteen - more than any venue quotes, enough for a rate compounded over
/// a year - so the value is one `i128` of units and every operation is an
/// integer's: add and subtract are one addition, compare is one comparison,
/// hash is the integer's, and multiply and divide widen to 256 bits for the
/// one product and come back. Nothing is allocated, and nothing passes
/// through a float unless a caller asks for one.
///
/// The value is bounded to thirty-eight digits, `MIN` to `MAX`, so it lands
/// in a `decimal128(38, 18)` column exactly; a result past that is an
/// overflow, which the checked operations answer as `None` and the
/// operators refuse the way the integers' do. Multiplication and division
/// keep eighteen digits and truncate the rest toward zero.
///
/// ```
/// use yggdryl::types::Decimal;
///
/// # fn main() -> yggdryl::Result<()> {
/// let px: Decimal = "82.5".parse()?;
/// let qty = Decimal::from_int(1_000);
/// assert_eq!((px * qty).to_string(), "82500");
/// assert_eq!((px / Decimal::from_int(4)).to_string(), "20.625");
/// assert_eq!(px + Decimal::from_int(1), "83.5".parse()?);
/// assert!(px > Decimal::ZERO && -px < Decimal::ZERO);
/// // Exactly the column's value, and back.
/// assert_eq!(px.units(), 82_500_000_000_000_000_000);
/// assert_eq!(Decimal::from_units(px.units()), Some(px));
/// assert_eq!(px.to_f64(), 82.5);
/// // Past thirty-eight digits there is no value, only an overflow.
/// assert_eq!(Decimal::MAX.checked_add(Decimal::ONE), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal(i128);

impl Decimal {
    /// The fractional digits every value holds.
    pub const SCALE: i8 = 18;

    /// The digits of precision every value holds.
    pub const PRECISION: u8 = 38;

    /// Nothing.
    pub const ZERO: Self = Self(0);

    /// One whole.
    pub const ONE: Self = Self(ONE_UNITS);

    /// The greatest value: thirty-eight nines, eighteen of them fractional.
    pub const MAX: Self = Self(MAX_UNITS);

    /// The least value: `MAX` negated.
    pub const MIN: Self = Self(-MAX_UNITS);

    /// The value `units * 10^-18` states, or nothing past the precision.
    #[must_use]
    pub const fn from_units(units: i128) -> Option<Self> {
        if units > MAX_UNITS || units < -MAX_UNITS {
            None
        } else {
            Some(Self(units))
        }
    }

    /// The units this value is: the `decimal128(38, 18)` coefficient.
    #[must_use]
    pub const fn units(self) -> i128 {
        self.0
    }

    /// A whole number, which every `i64` is within the precision.
    #[must_use]
    pub const fn from_int(value: i64) -> Self {
        Self(value as i128 * ONE_UNITS)
    }

    /// The nearest value to a float, or nothing for one that is not finite
    /// or is past the precision.
    ///
    /// A float carries about sixteen significant digits, so what comes back
    /// is the float's reading rounded to eighteen fractional digits, never
    /// more exact than the float was.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        // The shortest text that reads back as the same float is the number
        // the float was meant to be: `3000000`, never the
        // `2999999.999999999949668352` that scaling the binary fraction by a
        // power of ten answers.
        Self::parse(&format!("{value}")).ok()
    }

    /// The float nearest this value.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / 1e18
    }

    /// The value one decimal text states, read as leniently as a number can
    /// be read without guessing.
    ///
    /// One pass over the bytes, no allocation: surrounding ASCII whitespace
    /// is ignored; an empty text is nothing, `0`; a `+` or `-` may lead; the
    /// digits may be grouped with `,`, `_`, `'` or a space ahead of the
    /// point; the point may lead (`.5`), trail (`5.`) or be absent; digits
    /// past the eighteenth fractional one are truncated toward zero; and an
    /// exponent - `1e3`, `2.5E-2` - moves the point. What is refused is text
    /// that states no number - a bare sign, two points, a letter, `NaN`,
    /// `inf` - and a value past thirty-eight digits, which no reading could
    /// hold.
    ///
    /// ```
    /// use yggdryl::types::Decimal;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(Decimal::parse(" 1,250.50 ")?.to_string(), "1250.5");
    /// assert_eq!(Decimal::parse("")?, Decimal::ZERO);
    /// assert_eq!(Decimal::parse(".5")?.to_string(), "0.5");
    /// assert_eq!(Decimal::parse("2.5e3")?.to_string(), "2500");
    /// assert_eq!(Decimal::parse("1E-2")?.to_string(), "0.01");
    /// assert_eq!(Decimal::parse("0.1234567890123456789")?.to_string(), "0.123456789012345678");
    /// assert!(Decimal::parse("1.2.3").is_err() && Decimal::parse("NaN").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for text that states no number and for a
    /// value past the precision.
    pub fn parse(text: &str) -> Result<Self> {
        let refused = |reason: &str| Error::Parse {
            target: "decimal",
            position: 0,
            reason: format_smolstr!("{reason}: {text:?}"),
        };
        let bytes = text.trim_ascii().as_bytes();
        let Some((&first, mut rest)) = bytes.split_first() else {
            return Ok(Self::ZERO);
        };
        let negative = match first {
            b'-' => true,
            b'+' => false,
            _ => {
                rest = bytes;
                false
            }
        };
        // The digits read, as an integer, and how many of them fell behind
        // the point; a digit past what the integer holds is one too many
        // ahead of the point and one truncated behind it.
        let mut mantissa: u128 = 0;
        let mut fraction: i32 = 0;
        let mut seen_digit = false;
        let mut seen_point = false;
        let mut exponent: i32 = 0;
        let mut at = 0;
        while at < rest.len() {
            match rest[at] {
                digit @ b'0'..=b'9' => {
                    seen_digit = true;
                    if mantissa < MANTISSA_LIMIT {
                        mantissa = mantissa * 10 + u128::from(digit - b'0');
                        if seen_point {
                            fraction += 1;
                        }
                    } else if !seen_point {
                        return Err(refused("expected at most 38 digits"));
                    }
                }
                b'.' if !seen_point => seen_point = true,
                b',' | b'_' | b'\'' | b' ' if seen_digit && !seen_point => {}
                b'e' | b'E' if seen_digit => {
                    exponent = parse_exponent(&rest[at + 1..])
                        .ok_or_else(|| refused("expected an exponent"))?;
                    break;
                }
                _ => return Err(refused("expected a decimal")),
            }
            at += 1;
        }
        if !seen_digit {
            return Err(refused("expected a decimal"));
        }
        // The units are the mantissa moved to eighteen fractional digits:
        // up by what the point and the exponent leave short, down - truncated
        // toward zero - by what they leave over.
        let shift = i32::from(Self::SCALE) + exponent - fraction;
        let units = if shift >= 0 {
            u32::try_from(shift)
                .ok()
                .and_then(|shift| 10_u128.checked_pow(shift))
                .and_then(|scale| mantissa.checked_mul(scale))
                .ok_or_else(|| refused("expected at most 38 digits"))?
        } else {
            match u32::try_from(-shift)
                .ok()
                .and_then(|shift| 10_u128.checked_pow(shift))
            {
                Some(scale) => mantissa / scale,
                None => 0,
            }
        };
        let units = i128::try_from(units).map_err(|_| refused("expected at most 38 digits"))?;
        Self::from_units(if negative { -units } else { units })
            .ok_or_else(|| refused("expected at most 38 digits"))
    }

    /// The value a scalar holds, where it is an exact decimal that restates
    /// at eighteen fractional digits, a whole integer, a finite float, or
    /// text [`Self::parse`] reads.
    #[must_use]
    pub fn from_scalar(value: &Scalar) -> Option<Self> {
        if let Some(units) = value.decimal_unscaled_at(Self::SCALE) {
            return Self::from_units(units);
        }
        if let Some(whole) = value.as_i64() {
            return Some(Self::from_int(whole));
        }
        if let Some(text) = value.as_str() {
            return Self::parse(text).ok();
        }
        value.as_f64().and_then(Self::from_f64)
    }

    /// The datatype every value is: `decimal128(38, 18)`.
    #[must_use]
    pub const fn dtype() -> DataType {
        DataType::DECIMAL
    }

    /// Whether this value is nothing.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Whether this value is below nothing.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Whether this value is above nothing.
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// This value without its sign.
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// The sum, or nothing past the precision.
    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).and_then(Self::from_units)
    }

    /// The difference, or nothing past the precision.
    #[must_use]
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).and_then(Self::from_units)
    }

    /// The product at eighteen digits, truncated toward zero, or nothing
    /// past the precision.
    #[must_use]
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        i256::from(self.0)
            .checked_mul(i256::from(other.0))?
            .checked_div(i256::from(ONE_UNITS))?
            .as_i128()
            .and_then(Self::from_units)
    }

    /// The quotient at eighteen digits, truncated toward zero, or nothing
    /// for a divisor of nothing or a quotient past the precision.
    #[must_use]
    pub fn checked_div(self, other: Self) -> Option<Self> {
        if other.is_zero() {
            return None;
        }
        i256::from(self.0)
            .checked_mul(i256::from(ONE_UNITS))?
            .checked_div(i256::from(other.0))?
            .as_i128()
            .and_then(Self::from_units)
    }

    /// The value the float nearest this one rounds to at `places` fractional
    /// digits, truncating the rest toward zero; `places` past the scale is
    /// the value itself.
    #[must_use]
    pub fn truncated(self, places: u8) -> Self {
        if places >= Self::SCALE as u8 {
            return self;
        }
        let keep = 10_i128.pow(u32::from(Self::SCALE as u8 - places));
        Self(self.0 / keep * keep)
    }

    /// The `Decimal128` this value is, at its own scale.
    #[must_use]
    pub const fn into_decimal128(self) -> Decimal128 {
        Decimal128::new(self.0, Self::SCALE)
    }
}

impl DataType {
    /// The decimal a market's numbers are held as: `decimal128(38, 18)`,
    /// what every [`Decimal`] is.
    pub const DECIMAL: Self = Self::Decimal128 {
        precision: 38,
        scale: 18,
    };
}

impl fmt::Display for Decimal {
    /// The decimal text, with no trailing zero behind the point.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = super::scalars::decimal_text(i256::from(self.0), Self::SCALE);
        let trimmed = match text.find('.') {
            Some(_) => text.trim_end_matches('0').trim_end_matches('.'),
            None => text.as_str(),
        };
        formatter.write_str(if trimmed.is_empty() { "0" } else { trimmed })
    }
}

impl fmt::Debug for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Decimal({self})")
    }
}

impl FromStr for Decimal {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}

impl From<Decimal> for Decimal128 {
    fn from(value: Decimal) -> Self {
        value.into_decimal128()
    }
}

impl From<Decimal> for Scalar {
    fn from(value: Decimal) -> Self {
        Self::Decimal128(value.into_decimal128())
    }
}

impl From<i64> for Decimal {
    fn from(value: i64) -> Self {
        Self::from_int(value)
    }
}

macro_rules! operator {
    ($trait:ident, $method:ident, $assign:ident, $assign_method:ident, $checked:ident, $what:literal) => {
        impl $trait for Decimal {
            type Output = Self;

            fn $method(self, other: Self) -> Self {
                self.$checked(other)
                    .unwrap_or_else(|| panic!(concat!("decimal ", $what, " overflows 38 digits")))
            }
        }

        impl $assign for Decimal {
            fn $assign_method(&mut self, other: Self) {
                *self = $trait::$method(*self, other);
            }
        }
    };
}

operator!(Add, add, AddAssign, add_assign, checked_add, "addition");
operator!(Sub, sub, SubAssign, sub_assign, checked_sub, "subtraction");
operator!(
    Mul,
    mul,
    MulAssign,
    mul_assign,
    checked_mul,
    "multiplication"
);
operator!(Div, div, DivAssign, div_assign, checked_div, "division");

impl Neg for Decimal {
    type Output = Self;

    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl std::iter::Sum for Decimal {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}
