//! Exact decimal datatypes.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

pub use fixed::Decimal18;
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::arithmetic::{Arithmetic, invalid_binary};
use crate::invalid;
use crate::parser::Parser;
use crate::value::DataTypeValue;
use crate::value::{DecimalValue, ValidationFailure, expected};
use crate::{DataType, DataTypeId, Error, Result, Scalar, Value, i256};

/// Arrow casts owned by this datatype family.
pub(crate) mod casts {
    use arrow_buffer::i256;

    use crate::DataType;

    pub(crate) struct DecimalText {
        bytes: [u8; 78],
        start: usize,
    }

    impl DecimalText {
        pub(crate) fn new(value: i256) -> Self {
            let negative = value.is_negative();
            let mut raw = value.to_le_bytes();
            if negative {
                let mut carry = true;
                for byte in &mut raw {
                    *byte = !*byte;
                    if carry {
                        let (next, overflow) = byte.overflowing_add(1);
                        *byte = next;
                        carry = overflow;
                    }
                }
            }
            let mut limbs = [0_u64; 4];
            for (limb, chunk) in limbs.iter_mut().zip(raw.chunks_exact(8)) {
                let mut bytes = [0_u8; 8];
                bytes.copy_from_slice(chunk);
                *limb = u64::from_le_bytes(bytes);
            }

            let mut bytes = [0_u8; 78];
            let mut start = bytes.len();
            loop {
                let mut remainder = 0_u128;
                for limb in limbs.iter_mut().rev() {
                    let value = (remainder << 64) | u128::from(*limb);
                    *limb = u64::try_from(value / 10).unwrap_or(u64::MAX);
                    remainder = value % 10;
                }
                start -= 1;
                bytes[start] = b'0' + u8::try_from(remainder).unwrap_or(0);
                if limbs.iter().all(|limb| *limb == 0) {
                    break;
                }
            }
            if negative {
                start -= 1;
                bytes[start] = b'-';
            }
            Self { bytes, start }
        }

        pub(crate) fn as_bytes(&self) -> &[u8] {
            &self.bytes[self.start..]
        }
    }

    /// Whether a target datatype holds decimals, however it encodes them.
    pub(crate) fn holds_decimal(target: &DataType) -> bool {
        matches!(
            crate::cast::text::encoded_value_of(target),
            DataType::Decimal32 { .. }
                | DataType::Decimal64 { .. }
                | DataType::Decimal128 { .. }
                | DataType::Decimal256 { .. }
        )
    }
}

// ------------------------------------------------------------------------
// Decimal construction and precision/scale validation.
// ------------------------------------------------------------------------

/// One exact-decimal datatype and its precision and scale.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DecimalType {
    /// Decimal backed by 32 bits.
    Decimal32 { precision: u8, scale: i8 },
    /// Decimal backed by 64 bits.
    Decimal64 { precision: u8, scale: i8 },
    /// Decimal backed by 128 bits.
    Decimal128 { precision: u8, scale: i8 },
    /// Decimal backed by 256 bits.
    Decimal256 { precision: u8, scale: i8 },
}

impl DecimalType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Decimal32 { .. } => DataTypeId::Decimal32,
            Self::Decimal64 { .. } => DataTypeId::Decimal64,
            Self::Decimal128 { .. } => DataTypeId::Decimal128,
            Self::Decimal256 { .. } => DataTypeId::Decimal256,
        }
    }

    /// The digits this decimal states, whichever width holds them.
    ///
    /// The leaf is how wide the backing integer is; the precision and the
    /// scale are what the column *means*, so a reader asks here and never
    /// branches on the width.
    #[must_use]
    pub const fn precision(self) -> u8 {
        match self {
            Self::Decimal32 { precision, .. }
            | Self::Decimal64 { precision, .. }
            | Self::Decimal128 { precision, .. }
            | Self::Decimal256 { precision, .. } => precision,
        }
    }

    /// The scale this decimal states, whichever width holds it.
    #[must_use]
    pub const fn scale(self) -> i8 {
        match self {
            Self::Decimal32 { scale, .. }
            | Self::Decimal64 { scale, .. }
            | Self::Decimal128 { scale, .. }
            | Self::Decimal256 { scale, .. } => scale,
        }
    }

    /// The name this width states its own refusals under.
    ///
    /// A refusal names the width that could not hold the number, so a caller
    /// reading `precision must be between 1 and 9` knows which one it was.
    pub(crate) const fn refusal_kind(self) -> &'static str {
        match self {
            Self::Decimal32 { .. } => "Decimal32",
            Self::Decimal64 { .. } => "Decimal64",
            Self::Decimal128 { .. } => "Decimal128",
            Self::Decimal256 { .. } => "Decimal256",
        }
    }

    /// The most digits this width holds.
    #[must_use]
    pub const fn maximum(self) -> u8 {
        match self {
            Self::Decimal32 { .. } => 9,
            Self::Decimal64 { .. } => 18,
            Self::Decimal128 { .. } => 38,
            Self::Decimal256 { .. } => 76,
        }
    }

    /// The narrowest width that holds this many digits.
    ///
    /// The one rule behind [`DataType::decimal`]: a caller who states only
    /// what the number means gets the width it fits in.
    #[must_use]
    pub const fn narrowest(precision: u8, scale: i8) -> Self {
        match precision {
            0..=9 => Self::Decimal32 { precision, scale },
            10..=18 => Self::Decimal64 { precision, scale },
            19..=38 => Self::Decimal128 { precision, scale },
            _ => Self::Decimal256 { precision, scale },
        }
    }
}

impl DataTypeValue for DecimalType {
    const FAMILY: &'static str = "decimal";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn validate(&self) -> Result<()> {
        validate_decimal(
            self.refusal_kind(),
            self.precision(),
            self.scale(),
            self.maximum(),
        )
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Decimal32 { precision, scale } => DataType::Decimal32 { precision, scale },
            Self::Decimal64 { precision, scale } => DataType::Decimal64 { precision, scale },
            Self::Decimal128 { precision, scale } => DataType::Decimal128 { precision, scale },
            Self::Decimal256 { precision, scale } => DataType::Decimal256 { precision, scale },
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.decimal_type()
    }
}

impl fmt::Display for DecimalType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}({},{})",
            self.id().as_str(),
            self.precision(),
            self.scale()
        )
    }
}

impl From<DecimalType> for DataType {
    fn from(value: DecimalType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl TryFrom<&DataType> for DecimalType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        value.decimal_type().ok_or_else(|| Error::InvalidDataType {
            kind: "decimal",
            reason: format_smolstr!("expected a decimal datatype, got {value}"),
        })
    }
}

impl DataType {
    /// Creates a Decimal32.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside 1..=9, or when a
    /// positive scale exceeds it.
    pub fn decimal32(precision: u8, scale: i8) -> Result<Self> {
        Self::decimal_of(DecimalType::Decimal32 { precision, scale })
    }

    /// Creates a Decimal64.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside 1..=18, or when a
    /// positive scale exceeds it.
    pub fn decimal64(precision: u8, scale: i8) -> Result<Self> {
        Self::decimal_of(DecimalType::Decimal64 { precision, scale })
    }

    /// Creates a Decimal128.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside 1..=38, or when a
    /// positive scale exceeds it.
    pub fn decimal128(precision: u8, scale: i8) -> Result<Self> {
        Self::decimal_of(DecimalType::Decimal128 { precision, scale })
    }

    /// Creates a Decimal256.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside 1..=76, or when a
    /// positive scale exceeds it.
    pub fn decimal256(precision: u8, scale: i8) -> Result<Self> {
        Self::decimal_of(DecimalType::Decimal256 { precision, scale })
    }

    /// Creates the most compact Arrow-compatible decimal for the requested
    /// precision and scale.
    ///
    /// Precisions through 9, 18, 38, and 76 use Decimal32, Decimal64,
    /// Decimal128, and Decimal256 respectively.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside 1..=76, or when a
    /// positive scale exceeds it.
    pub fn decimal(precision: u8, scale: i8) -> Result<Self> {
        Self::decimal_of(DecimalType::narrowest(precision, scale))
    }

    /// One checked decimal leaf, whichever width names it.
    ///
    /// The four constructors above are one rule with a width chosen for the
    /// caller, so the rule is written once here.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision is outside what the width holds,
    /// or when a positive scale exceeds it.
    pub fn decimal_of(family: DecimalType) -> Result<Self> {
        DataTypeValue::validate(&family)?;
        Ok(family.into())
    }

    /// The typed field's payload over any of the four widths, `None` for
    /// every other datatype.
    #[must_use]
    pub const fn decimal_type(&self) -> Option<DecimalType> {
        match *self {
            Self::Decimal32 { precision, scale } => {
                Some(DecimalType::Decimal32 { precision, scale })
            }
            Self::Decimal64 { precision, scale } => {
                Some(DecimalType::Decimal64 { precision, scale })
            }
            Self::Decimal128 { precision, scale } => {
                Some(DecimalType::Decimal128 { precision, scale })
            }
            Self::Decimal256 { precision, scale } => {
                Some(DecimalType::Decimal256 { precision, scale })
            }
            _ => None,
        }
    }
}

pub(crate) fn validate_decimal(
    kind: &'static str,
    precision: u8,
    scale: i8,
    maximum: u8,
) -> Result<()> {
    if precision == 0 || precision > maximum {
        return Err(invalid(
            kind,
            format_smolstr!("precision must be between 1 and {maximum}: {precision}"),
        ));
    }
    if scale > 0 && scale.unsigned_abs() > precision {
        return Err(invalid(
            kind,
            format_smolstr!("positive scale cannot exceed precision: {scale} > {precision}"),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------------
// Fixed-width decimal field markers.
// ------------------------------------------------------------------------

/// One fixed-point decimal: `decimal128(38, 18)` already applied, so a price
/// and a quantity add, multiply and compare as integers do.
mod fixed {
    use std::fmt;
    use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};
    use std::str::FromStr;

    use smol_str::format_smolstr;

    use super::Decimal128;
    use crate::i256;
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
    /// use yggdryl::Decimal18;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let px: Decimal18 = "82.5".parse()?;
    /// let qty = Decimal18::from_int(1_000);
    /// assert_eq!((px * qty).to_string(), "82500");
    /// assert_eq!((px / Decimal18::from_int(4)).to_string(), "20.625");
    /// assert_eq!(px + Decimal18::from_int(1), "83.5".parse()?);
    /// assert!(px > Decimal18::ZERO && -px < Decimal18::ZERO);
    /// // Exactly the column's value, and back.
    /// assert_eq!(px.units(), 82_500_000_000_000_000_000);
    /// assert_eq!(Decimal18::from_units(px.units()), Some(px));
    /// assert_eq!(px.to_f64(), 82.5);
    /// // Past thirty-eight digits there is no value, only an overflow.
    /// assert_eq!(Decimal18::MAX.checked_add(Decimal18::ONE), None);
    /// # Ok(())
    /// # }
    /// ```
    #[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Decimal18(i128);

    impl Decimal18 {
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
        /// use yggdryl::Decimal18;
        ///
        /// # fn main() -> yggdryl::Result<()> {
        /// assert_eq!(Decimal18::parse(" 1,250.50 ")?.to_string(), "1250.5");
        /// assert_eq!(Decimal18::parse("")?, Decimal18::ZERO);
        /// assert_eq!(Decimal18::parse(".5")?.to_string(), "0.5");
        /// assert_eq!(Decimal18::parse("2.5e3")?.to_string(), "2500");
        /// assert_eq!(Decimal18::parse("1E-2")?.to_string(), "0.01");
        /// assert_eq!(Decimal18::parse("0.1234567890123456789")?.to_string(), "0.123456789012345678");
        /// assert!(Decimal18::parse("1.2.3").is_err() && Decimal18::parse("NaN").is_err());
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
        /// what every [`Decimal18`] is.
        pub const DECIMAL: Self = Self::Decimal128 {
            precision: 38,
            scale: 18,
        };
    }

    impl fmt::Display for Decimal18 {
        /// The decimal text, with no trailing zero behind the point.
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let text = super::decimal_text(i256::from(self.0), Self::SCALE);
            let trimmed = match text.find('.') {
                Some(_) => text.trim_end_matches('0').trim_end_matches('.'),
                None => text.as_str(),
            };
            formatter.write_str(if trimmed.is_empty() { "0" } else { trimmed })
        }
    }

    impl fmt::Debug for Decimal18 {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "Decimal18({self})")
        }
    }

    impl FromStr for Decimal18 {
        type Err = Error;

        fn from_str(text: &str) -> Result<Self> {
            Self::parse(text)
        }
    }

    impl From<Decimal18> for Decimal128 {
        fn from(value: Decimal18) -> Self {
            value.into_decimal128()
        }
    }

    impl From<Decimal18> for Scalar {
        fn from(value: Decimal18) -> Self {
            Self::Decimal128(value.into_decimal128())
        }
    }

    impl From<i64> for Decimal18 {
        fn from(value: i64) -> Self {
            Self::from_int(value)
        }
    }

    macro_rules! operator {
        ($trait:ident, $method:ident, $assign:ident, $assign_method:ident, $checked:ident, $what:literal) => {
            impl $trait for Decimal18 {
                type Output = Self;

                fn $method(self, other: Self) -> Self {
                    self.$checked(other).unwrap_or_else(|| {
                        panic!(concat!("decimal ", $what, " overflows 38 digits"))
                    })
                }
            }

            impl $assign for Decimal18 {
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

    impl Neg for Decimal18 {
        type Output = Self;

        fn neg(self) -> Self {
            Self(-self.0)
        }
    }

    impl std::iter::Sum for Decimal18 {
        fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
            iter.fold(Self::ZERO, Add::add)
        }
    }
}

// ------------------------------------------------------------------------
// Decimal parameter grammar.
// ------------------------------------------------------------------------

impl Parser<'_> {
    pub(crate) fn parse_decimal_parameters(&mut self, default_precision: u8) -> Result<(u8, i8)> {
        let Some(close) = self.consume_opening() else {
            return Ok((default_precision, 0));
        };
        self.consume_label("precision");
        let position = self.current_position();
        let precision_value = self.parse_integer("decimal precision")?;
        let precision = u8::try_from(precision_value)
            .map_err(|_| self.error_at(position, "decimal precision must fit in u8"))?;
        let mut scale = 0_i8;
        if self.consume_separator() {
            self.consume_label("scale");
            let position = self.current_position();
            let scale_value = self.parse_integer("decimal scale")?;
            scale = i8::try_from(scale_value)
                .map_err(|_| self.error_at(position, "decimal scale must fit in i8"))?;
        }
        self.expect_symbol(close)?;
        Ok((precision, scale))
    }
}

// ------------------------------------------------------------------------
// Exact coefficient-and-scale decimals matching Arrow storage.
//
// `from_decimal` selects `D128` or `D256`. Scale remains part of the stored
// value, while equality, ordering, and hashing compare the represented number.
//
// ```
// use yggdryl::{i256, Scalar};
//
// let price = Scalar::from_decimal(i256::from_i128(1_050), 2);
//
// assert_eq!(price.as_decimal(), Some((i256::from_i128(1_050), 2)));
// assert_eq!(price, Scalar::from_decimal(i256::from_i128(105), 1));
// assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
// ```
// ------------------------------------------------------------------------

trait IntoI256 {
    fn into_i256(self) -> i256;
}

macro_rules! into_i256 {
    ($($native:ty),+ $(,)?) => {$(
        impl IntoI256 for $native {
            fn into_i256(self) -> i256 {
                i256::from_i128(self as i128)
            }
        }
    )+};
}

into_i256!(i32, i64, i128);

impl IntoI256 for i256 {
    fn into_i256(self) -> i256 {
        self
    }
}

macro_rules! decimal_leaf {
    ($name:ident, $coefficient:ty) => {
        #[doc = concat!("One exact `", stringify!($coefficient), "` coefficient and scale.")]
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        pub struct $name {
            coefficient: $coefficient,
            scale: i8,
        }

        impl $name {
            /// Construct an exact decimal representation.
            pub const fn new(coefficient: $coefficient, scale: i8) -> Self {
                Self { coefficient, scale }
            }

            /// Return the stored coefficient.
            pub const fn coefficient(&self) -> $coefficient {
                self.coefficient
            }

            /// Return the base-10 scale.
            pub const fn scale(&self) -> i8 {
                self.scale
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&decimal_text(self.coefficient.into_i256(), self.scale))
            }
        }
    };
}

decimal_leaf!(Decimal32, i32);
decimal_leaf!(Decimal64, i64);
decimal_leaf!(Decimal128, i128);
decimal_leaf!(Decimal256, i256);

// A decimal width is its own scalar family, exactly as `Boolean` is. The
// narrow widths also restate through their native coefficient; `Decimal256`
// writes that by hand below.
macro_rules! decimal_value {
    ($leaf:ident) => {
        impl Value for $leaf {
            fn dtype(&self) -> Result<DataType> {
                Scalar::$leaf(*self).dtype()
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$leaf(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
    ($leaf:ident, $native:ty) => {
        decimal_value!($leaf);

        impl DecimalValue for $leaf {
            fn coefficient(&self) -> i256 {
                self.coefficient().into_i256()
            }

            fn scale(&self) -> i8 {
                self.scale()
            }

            fn rescale(self, scale: i8) -> Result<Self> {
                let coefficient =
                    rescale_decimal(self.coefficient().into_i256(), self.scale(), scale)
                        .and_then(i256::as_i128)
                        .and_then(|value| <$native>::try_from(value).ok())
                        .ok_or(Error::InexactArithmetic {
                            operation: "rescale",
                            kind: stringify!($leaf),
                        })?;
                Ok(Self::new(coefficient, scale))
            }
        }
    };
}

decimal_value!(Decimal32, i32);
decimal_value!(Decimal64, i64);
decimal_value!(Decimal128, i128);
decimal_value!(Decimal256);

impl DecimalValue for Decimal256 {
    fn coefficient(&self) -> i256 {
        self.coefficient()
    }

    fn scale(&self) -> i8 {
        self.scale()
    }

    fn rescale(self, scale: i8) -> Result<Self> {
        rescale_decimal(self.coefficient(), self.scale(), scale)
            .map(|coefficient| Self::new(coefficient, scale))
            .ok_or(Error::InexactArithmetic {
                operation: "rescale",
                kind: "Decimal256",
            })
    }
}

impl Scalar {
    /// Build the narrowest exact decimal width that holds `unscaled`.
    pub fn from_decimal(unscaled: i256, scale: i8) -> Self {
        unscaled.as_i128().map_or_else(
            || Self::d256(unscaled, scale),
            |value| Self::d128(value, scale),
        )
    }

    /// Build an exact decimal from an unscaled integer and a scale.
    ///
    /// The value is `unscaled * 10^-scale`, so `Scalar::d128(1_050, 2)` is
    /// `10.50`. A negative scale multiplies instead, exactly as Arrow allows.
    pub const fn d128(unscaled: i128, scale: i8) -> Self {
        Self::Decimal128(Decimal128::new(unscaled, scale))
    }

    /// Build an exact decimal with a 256-bit coefficient.
    pub const fn d256(unscaled: i256, scale: i8) -> Self {
        Self::Decimal256(Decimal256::new(unscaled, scale))
    }

    /// Return the coefficient and scale when this is a 128-bit decimal.
    pub const fn as_d128(&self) -> Option<(i128, i8)> {
        match self {
            Self::Decimal128(value) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return the coefficient and scale when this is a 256-bit decimal.
    pub const fn as_d256(&self) -> Option<(i256, i8)> {
        match self {
            Self::Decimal256(value) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return this decimal's coefficient widened to 256 bits and its scale.
    pub fn as_decimal(&self) -> Option<(i256, i8)> {
        match self {
            Self::Decimal32(value) => Some((value.coefficient().into_i256(), value.scale())),
            Self::Decimal64(value) => Some((value.coefficient().into_i256(), value.scale())),
            Self::Decimal128(value) => Some((value.coefficient().into_i256(), value.scale())),
            Self::Decimal256(value) => Some((value.coefficient(), value.scale())),
            _ => None,
        }
    }

    /// Return whether this value is an exact decimal.
    pub const fn is_decimal(&self) -> bool {
        crate::DataTypeKind::Decimal.contains(self.id())
    }

    /// Return this decimal's unscaled integer at `scale`, when it is exact.
    ///
    /// A column declares its own scale, so a decimal written into one has to be
    /// restated at that scale. Restating to fewer fractional digits would throw
    /// digits away, so it answers `None` rather than rounding.
    pub fn decimal_unscaled_at(&self, scale: i8) -> Option<i128> {
        let (unscaled, current) = self.as_decimal()?;
        let unscaled = unscaled.as_i128()?;
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => scale_up_signed(unscaled, shift),
            // Removing digits is only exact when every digit removed is a zero.
            Ordering::Less => {
                let divisor = scale_up_signed(1, -shift)?;
                (unscaled % divisor == 0).then(|| unscaled / divisor)
            }
        }
    }

    /// Return this decimal's 256-bit coefficient at `scale`, when exact.
    pub fn decimal256_unscaled_at(&self, scale: i8) -> Option<i256> {
        let (unscaled, current) = self.as_decimal()?;
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => (0..shift).try_fold(unscaled, |held, _| held.checked_mul_ten()),
            Ordering::Less => (0..-shift).try_fold(unscaled, |held, _| held.divided_by_ten()),
        }
    }

    /// Render an exact decimal without passing through a float.
    pub fn into_decimal_utf8(&self) -> Option<String> {
        let (coefficient, scale) = self.as_decimal()?;
        Some(decimal_text(coefficient, scale))
    }
}

/// Render a coefficient and scale in ordinary decimal notation.
pub(crate) fn decimal_text(coefficient: i256, scale: i8) -> String {
    let encoded = coefficient.to_string();
    if scale == 0 {
        return encoded;
    }
    let (sign, digits) = encoded
        .strip_prefix('-')
        .map_or(("", encoded.as_str()), |digits| ("-", digits));
    if scale < 0 {
        return format!(
            "{sign}{digits}{}",
            "0".repeat(usize::from(scale.unsigned_abs()))
        );
    }
    let scale = usize::from(scale.unsigned_abs());
    if digits.len() > scale {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("{sign}0.{}{digits}", "0".repeat(scale - digits.len()))
    }
}

/// Strip the trailing zeros a decimal's coefficient carries.
///
/// Equal numbers share exactly one normal form, which is what lets `Hash` agree
/// with the numeric `Ord` below without either of them widening the coefficient.
pub(crate) fn normalize(unscaled: i256, scale: i8) -> (i256, i8) {
    if unscaled.is_zero() {
        return (i256::ZERO, 0);
    }
    let mut unscaled = unscaled;
    let mut scale = scale;
    while scale > i8::MIN {
        let Some(reduced) = unscaled.divided_by_ten() else {
            break;
        };
        unscaled = reduced;
        scale -= 1;
    }
    (unscaled, scale)
}

/// Compare two decimals by the number each one names.
pub(crate) fn compare(
    left_unscaled: i256,
    left_scale: i8,
    right_unscaled: i256,
    right_scale: i8,
) -> Ordering {
    let (left_unscaled, left_scale) = normalize(left_unscaled, left_scale);
    let (right_unscaled, right_scale) = normalize(right_unscaled, right_scale);
    if left_scale == right_scale {
        return left_unscaled.cmp(&right_unscaled);
    }
    let sign = left_unscaled
        .cmp(&i256::ZERO)
        .cmp(&right_unscaled.cmp(&i256::ZERO));
    if sign != Ordering::Equal {
        return sign;
    }

    // Same sign, different scale: bring the coefficient with fewer fractional
    // digits up to the other's scale and compare magnitudes. A product that no
    // longer fits is by that fact the larger magnitude, because the coefficient
    // it is compared against did fit.
    if left_scale < right_scale {
        scale_up(
            left_unscaled,
            i32::from(right_scale) - i32::from(left_scale),
        )
        .map_or_else(
            || {
                if left_unscaled.is_negative() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            },
            |left| left.cmp(&right_unscaled),
        )
    } else {
        scale_up(
            right_unscaled,
            i32::from(left_scale) - i32::from(right_scale),
        )
        .map_or_else(
            || {
                if right_unscaled.is_negative() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            },
            |right| left_unscaled.cmp(&right),
        )
    }
}

/// Multiply a magnitude by ten `digits` times, or report that it overflowed.
fn scale_up(unscaled: i256, digits: i32) -> Option<i256> {
    (0..digits).try_fold(unscaled, |held, _| held.checked_mul_ten())
}

/// Multiply a signed coefficient by ten `digits` times, or report the overflow.
fn scale_up_signed(unscaled: i128, digits: i32) -> Option<i128> {
    (0..digits).try_fold(unscaled, |unscaled, _| unscaled.checked_mul(10))
}

pub(crate) fn validate_decimal_value(
    value: &Scalar,
    precision: u8,
    width: u16,
) -> std::result::Result<(), ValidationFailure> {
    let Some(integer) = value.as_i128() else {
        return Err(expected("unscaled decimal integer", value));
    };
    let fits_width = match width {
        32 => i32::try_from(integer).is_ok(),
        64 => i64::try_from(integer).is_ok(),
        _ => true,
    };
    if !fits_width || decimal_digits(integer.unsigned_abs()) > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal value exceeds precision {precision} or physical width {width}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_decimal256_value(
    value: &Scalar,
    precision: u8,
    scale: i8,
) -> std::result::Result<(), ValidationFailure> {
    let Some(coefficient) = (if value.is_decimal() {
        value.decimal256_unscaled_at(scale)
    } else {
        value.as_i128().map(i256::from_i128)
    }) else {
        return Err(expected("d256", value));
    };
    let encoded = coefficient.to_string();
    let digits = encoded.trim_start_matches('-');
    if digits.len() > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal256 value exceeds precision {precision}"
        )));
    }
    Ok(())
}

fn decimal_digits(value: u128) -> usize {
    let mut digits = 1;
    let mut remaining = value / 10;
    while remaining > 0 {
        digits += 1;
        remaining /= 10;
    }
    digits
}

pub(crate) fn is_exact_number(value: &Scalar) -> bool {
    value.is_integer() || value.is_decimal()
}

pub(crate) fn decimal_value_parts(value: &Scalar) -> Option<(i256, i8)> {
    value.as_decimal()
}

pub(crate) fn exact_value_parts(value: &Scalar) -> Option<(i256, i8)> {
    decimal_value_parts(value).or_else(|| {
        value
            .as_i128()
            .map(|value| (i256::from_i128(value), 0))
            .or_else(|| value.as_u128().map(|value| (i256::from_u128(value), 0)))
    })
}

pub(crate) fn decimal_target(dtype: &DataType) -> Option<(bool, i8)> {
    match dtype {
        DataType::Decimal32 { scale, .. }
        | DataType::Decimal64 { scale, .. }
        | DataType::Decimal128 { scale, .. } => Some((false, *scale)),
        DataType::Decimal256 { scale, .. } => Some((true, *scale)),
        _ => None,
    }
}

pub(crate) const fn result_decimal_scale(operation: Arithmetic, left: i8, right: i8) -> Option<i8> {
    match operation {
        Arithmetic::Add | Arithmetic::Sub | Arithmetic::Rem => {
            Some(if left > right { left } else { right })
        }
        Arithmetic::Mul => left.checked_add(right),
        Arithmetic::Div => None,
    }
}

/// Select the smallest non-negative scale that represents an inferred exact
/// quotient. After reducing the coefficients, only factors of two and five in
/// the denominator can terminate in base ten. Input scales shift the required
/// power of ten; the selected result never keeps arbitrary padding zeros.
pub(crate) fn inferred_decimal_division_scale(
    left: &Scalar,
    left_scale: i8,
    right: &Scalar,
    right_scale: i8,
    wide: bool,
) -> Result<i8> {
    let (left_number, _) = exact_value_parts(left).ok_or_else(|| {
        invalid_binary(
            Arithmetic::Div,
            left,
            right,
            "left operand is not an exact number",
        )
    })?;
    let (right_number, _) = exact_value_parts(right).ok_or_else(|| {
        invalid_binary(
            Arithmetic::Div,
            left,
            right,
            "right operand is not an exact number",
        )
    })?;
    if right_number.is_zero() || left_number.is_zero() {
        return Ok(0);
    }

    let divisor = signed_gcd(left_number, right_number)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let numerator = left_number
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let mut denominator = right_number
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(Arithmetic::Div, wide))?;
    let mut twos = 0_i16;
    while let Some(reduced) = divide_exactly(denominator, 2) {
        denominator = reduced;
        twos += 1;
    }
    let mut fives = 0_i16;
    while let Some(reduced) = divide_exactly(denominator, 5) {
        denominator = reduced;
        fives += 1;
    }
    if denominator != i256::from_i128(1) && denominator != i256::from_i128(-1) {
        return Err(inexact_decimal_division(wide));
    }

    let required = twos.max(fives);
    let scale = (required + i16::from(left_scale) - i16::from(right_scale)).max(0);
    let (numerator, numerator_twos) = factor_power(numerator, 2);
    let (_, numerator_fives) = factor_power(numerator, 5);
    let trailing_zeroes =
        (numerator_twos + required - twos).min(numerator_fives + required - fives);
    let scale = (scale - trailing_zeroes).max(0);
    let maximum = if wide { 76 } else { 38 };
    if scale > maximum {
        return Err(decimal_overflow(Arithmetic::Div, wide));
    }
    i8::try_from(scale).map_err(|_| decimal_overflow(Arithmetic::Div, wide))
}

fn factor_power(mut value: i256, factor: i128) -> (i256, i16) {
    let mut count = 0;
    while let Some(reduced) = divide_exactly(value, factor) {
        value = reduced;
        count += 1;
    }
    (value, count)
}

pub(crate) fn decimal_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    wide: bool,
    target_scale: i8,
) -> Result<Scalar> {
    let (left_number, left_scale) = exact_value_parts(left).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "left operand is not an exact number",
        )
    })?;
    let (right_number, right_scale) = exact_value_parts(right).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "right operand is not an exact number",
        )
    })?;
    if right_number.is_zero() && matches!(operation, Arithmetic::Div | Arithmetic::Rem) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let held = match operation {
        Arithmetic::Add | Arithmetic::Sub | Arithmetic::Rem => {
            let left = rescale_decimal(left_number, left_scale, target_scale);
            let right = rescale_decimal(right_number, right_scale, target_scale);
            let (Some(left), Some(right)) = (left, right) else {
                return Err(decimal_overflow(operation, wide));
            };
            match operation {
                Arithmetic::Add => left.checked_add(right),
                Arithmetic::Sub => left.checked_sub(right),
                Arithmetic::Rem => left.checked_rem(right),
                _ => unreachable!(),
            }
        }
        Arithmetic::Mul => left_number.checked_mul(right_number).and_then(|held| {
            rescale_decimal(held, left_scale.checked_add(right_scale)?, target_scale)
        }),
        Arithmetic::Div => Some(exact_scaled_division(
            left_number,
            left_scale,
            right_number,
            right_scale,
            target_scale,
            wide,
        )?),
    }
    .ok_or_else(|| decimal_overflow(operation, wide))?;
    if wide {
        Ok(Scalar::d256(held, target_scale))
    } else {
        held.as_i128()
            .map(|held| Scalar::d128(held, target_scale))
            .ok_or_else(|| decimal_overflow(operation, false))
    }
}

/// Divide two scaled coefficients exactly without first multiplying either
/// full-width operand by a power of ten. Reducing first means `MAX / MAX`
/// reaches one instead of reporting overflow from an unnecessary intermediate.
fn exact_scaled_division(
    left: i256,
    left_scale: i8,
    right: i256,
    right_scale: i8,
    target_scale: i8,
    wide: bool,
) -> Result<i256> {
    let operation = Arithmetic::Div;
    let exponent = i16::from(target_scale) + i16::from(right_scale) - i16::from(left_scale);
    let divisor = signed_gcd(left, right).ok_or_else(|| decimal_overflow(operation, wide))?;
    let mut numerator = left
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(operation, wide))?;
    let mut denominator = right
        .checked_div(divisor)
        .ok_or_else(|| decimal_overflow(operation, wide))?;

    if exponent >= 0 {
        let places = exponent.unsigned_abs();
        let mut twos = 0;
        let mut fives = 0;
        while twos < places {
            let Some(reduced) = divide_exactly(denominator, 2) else {
                break;
            };
            denominator = reduced;
            twos += 1;
        }
        while fives < places {
            let Some(reduced) = divide_exactly(denominator, 5) else {
                break;
            };
            denominator = reduced;
            fives += 1;
        }
        numerator = apply_denominator_sign(numerator, denominator)
            .ok_or_else(|| inexact_decimal_division(wide))?;
        for _ in twos..places {
            numerator = numerator
                .checked_mul(i256::from_i128(2))
                .ok_or_else(|| decimal_overflow(operation, wide))?;
        }
        for _ in fives..places {
            numerator = numerator
                .checked_mul(i256::from_i128(5))
                .ok_or_else(|| decimal_overflow(operation, wide))?;
        }
        return Ok(numerator);
    }

    numerator = apply_denominator_sign(numerator, denominator)
        .ok_or_else(|| inexact_decimal_division(wide))?;
    for _ in 0..exponent.unsigned_abs() {
        numerator = numerator
            .divided_by_ten()
            .ok_or_else(|| inexact_decimal_division(wide))?;
    }
    Ok(numerator)
}

/// Euclid's algorithm can stay signed, which also handles the i256 minimum:
/// only the final common factor is normalized, and the minimum is left signed.
fn signed_gcd(mut left: i256, mut right: i256) -> Option<i256> {
    while !right.is_zero() {
        let remainder = left.checked_rem(right)?;
        left = right;
        right = remainder;
    }
    if left.is_negative() {
        left.checked_neg().or(Some(left))
    } else {
        Some(left)
    }
}

fn divide_exactly(value: i256, divisor: i128) -> Option<i256> {
    let divisor = i256::from_i128(divisor);
    value
        .checked_rem(divisor)
        .filter(|remainder| remainder.is_zero())
        .and_then(|_| value.checked_div(divisor))
}

fn apply_denominator_sign(numerator: i256, denominator: i256) -> Option<i256> {
    if denominator == i256::from_i128(1) {
        Some(numerator)
    } else if denominator == i256::from_i128(-1) {
        numerator.checked_neg()
    } else {
        None
    }
}

const fn inexact_decimal_division(wide: bool) -> Error {
    Error::InexactArithmetic {
        operation: Arithmetic::Div.name(),
        kind: if wide { "d256" } else { "d128" },
    }
}

fn rescale_decimal(value: i256, from: i8, to: i8) -> Option<i256> {
    match to.cmp(&from) {
        std::cmp::Ordering::Greater => {
            scale_decimal_up(value, u16::try_from(i16::from(to) - i16::from(from)).ok()?)
        }
        std::cmp::Ordering::Less => (0..u16::try_from(i16::from(from) - i16::from(to)).ok()?)
            .try_fold(value, |held, _| held.divided_by_ten()),
        std::cmp::Ordering::Equal => Some(value),
    }
}

fn scale_decimal_up(value: i256, places: u16) -> Option<i256> {
    (0..places).try_fold(value, |held, _| held.checked_mul_ten())
}

const fn decimal_overflow(operation: Arithmetic, wide: bool) -> Error {
    Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: if wide { "d256" } else { "d128" },
    }
}

/// Read a decimal coefficient out of its canonical spelling, at one scale.
///
/// The spelling is what a decimal prints plus a scientific exponent, and the
/// restatement is exact: a digit the declared scale cannot hold is refused
/// rather than rounded away, which is the same rule a decimal value already
/// carried between two scales.
pub(crate) fn decimal_from_text(
    text: &str,
    target_scale: i8,
) -> std::result::Result<i256, &'static str> {
    let text = text.trim();
    let exponent_at = text.find(['e', 'E']);
    let (mantissa, exponent) = exponent_at.map_or((text, 0_i32), |position| {
        let exponent = text[position + 1..].parse::<i32>().unwrap_or(i32::MIN);
        (&text[..position], exponent)
    });
    if exponent == i32::MIN
        || exponent_at.is_some_and(|position| text[position + 1..].contains(['e', 'E']))
    {
        return Err("invalid decimal exponent");
    }
    let (sign, mantissa) = match mantissa.as_bytes().first() {
        Some(b'-') => ("-", &mantissa[1..]),
        Some(b'+') => ("", &mantissa[1..]),
        _ => ("", mantissa),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.contains('.')
        || fraction.contains('.')
        || (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("invalid decimal digits");
    }
    let digits = format!("{sign}{whole}{fraction}");
    let mut coefficient =
        i256::from_str(&digits).map_err(|_| "decimal coefficient exceeds 256 bits")?;
    if coefficient == i256::ZERO {
        return Ok(coefficient);
    }
    let source_scale = i32::try_from(fraction.len())
        .map_err(|_| "decimal scale is too large")?
        .checked_sub(exponent)
        .ok_or("decimal scale is too large")?;
    let shift = i32::from(target_scale)
        .checked_sub(source_scale)
        .ok_or("decimal scale is too large")?;
    if shift >= 0 {
        for _ in 0..shift {
            coefficient = coefficient
                .checked_mul_ten()
                .ok_or("decimal coefficient exceeds 256 bits")?;
        }
    } else {
        for _ in 0..-shift {
            coefficient = coefficient
                .divided_by_ten()
                .ok_or("decimal has more fractional digits than the field allows")?;
        }
    }
    Ok(coefficient)
}

impl Scalar {
    /// Read a decimal out of its text spelling, at one datatype's scale.
    ///
    /// The counterpart of [`Scalar::from_temporal_text`] for the other family
    /// whose text carries a precision the storage may not hold: text that is
    /// not a decimal at all is a parse failure, and a decimal the declared
    /// scale cannot state exactly is read and then refused, because dropping a
    /// digit off a price is a value change and not a restatement.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for text that is not a decimal, and
    /// [`Error::InvalidRecord`] for one the declared scale or width cannot
    /// hold.
    pub(crate) fn from_decimal_text(dtype: &DataType, text: &str) -> Result<Self> {
        let scale = match dtype {
            DataType::Decimal32 { scale, .. }
            | DataType::Decimal64 { scale, .. }
            | DataType::Decimal128 { scale, .. }
            | DataType::Decimal256 { scale, .. } => *scale,
            other => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("{other} is not a decimal datatype"),
                });
            }
        };
        let coefficient = decimal_from_text(text, scale).map_err(|reason| {
            if reason.starts_with("invalid") {
                Error::Parse {
                    target: "decimal",
                    position: 0,
                    reason: SmolStr::new(reason),
                }
            } else {
                Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: SmolStr::new(reason),
                }
            }
        })?;
        if matches!(dtype, DataType::Decimal256 { .. }) {
            return Ok(Self::d256(coefficient, scale));
        }
        coefficient
            .as_i128()
            .map(|coefficient| Self::d128(coefficient, scale))
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static("decimal coefficient exceeds 128 bits"),
            })
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the four exact-decimal widths.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::validate_decimal;
    use crate::invalid;
    use crate::{DataType, Result};

    /// The Arrow storage one decimal datatype lays out.
    ///
    /// The variants are public, so a precision wider than the width holds can
    /// reach this boundary; each width states its own maximum here.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision or scale is outside what the width
    /// carries, or when the datatype belongs to another family.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        Ok(match dtype {
            DataType::Decimal32 { precision, scale } => {
                validate_decimal("Decimal32", *precision, *scale, 9)?;
                ArrowDataType::Decimal32(*precision, *scale)
            }
            DataType::Decimal64 { precision, scale } => {
                validate_decimal("Decimal64", *precision, *scale, 18)?;
                ArrowDataType::Decimal64(*precision, *scale)
            }
            DataType::Decimal128 { precision, scale } => {
                validate_decimal("Decimal128", *precision, *scale, 38)?;
                ArrowDataType::Decimal128(*precision, *scale)
            }
            DataType::Decimal256 { precision, scale } => {
                validate_decimal("Decimal256", *precision, *scale, 76)?;
                ArrowDataType::Decimal256(*precision, *scale)
            }
            other => {
                return Err(invalid(
                    "decimal",
                    format_smolstr!("expected a decimal datatype, got {other}"),
                ));
            }
        })
    }

    /// The decimal datatype one Arrow storage imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the precision or scale is outside what the width
    /// carries, or when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Decimal32(precision, scale) => DataType::decimal32(*precision, *scale),
            ArrowDataType::Decimal64(precision, scale) => DataType::decimal64(*precision, *scale),
            ArrowDataType::Decimal128(precision, scale) => DataType::decimal128(*precision, *scale),
            ArrowDataType::Decimal256(precision, scale) => DataType::decimal256(*precision, *scale),
            other => Err(invalid(
                "decimal",
                format_smolstr!("expected a decimal storage, got {other}"),
            )),
        }
    }
}

pub(crate) use arrow::{arrow_storage, from_arrow_storage};

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/decimal.rs` pins and a caller cannot reach.
    //!
    //! `Scalar::from_decimal_text` is crate-private: it is the one reader
    //! every decimal spelling goes through, and whether a digit the declared
    //! scale cannot hold is refused as a reading or as a parse failure is
    //! what decides whether a wider reader may try the same text. The
    //! forwarder changes no visibility.

    use crate::{DataType, Result, Scalar};

    /// Read `text` as a decimal at the scale `dtype` declares.
    pub fn from_decimal_text(dtype: &DataType, text: &str) -> Result<Scalar> {
        Scalar::from_decimal_text(dtype, text)
    }
}
