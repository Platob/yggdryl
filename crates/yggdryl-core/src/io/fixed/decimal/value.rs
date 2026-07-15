//! [`Decimal<B>`] — the **self-describing scaled-decimal value type** (`D32`/`D64`/`D128`/`D256`):
//! a coefficient integer plus its own scale, `value = coefficient × 10^-scale`. Unlike the
//! columnar [`DecimalScalar`](super::DecimalScalar) / [`DecimalSerie`](super::DecimalSerie) (whose
//! scale is fixed by the column's descriptor), each value carries its own scale, so mixed-scale
//! arithmetic works — `2.5 + 0.25` aligns and yields `2.75` — the way native decimals behave.
//!
//! Arithmetic is **checked**: every `checked_*` returns a guided [`DecimalError`] on overflow, and
//! the `+`/`-`/`*` operators panic with that same message (like Rust's integer operators overflow
//! in debug). Identity is **by value, over the normalized form**: `2.5 == 2.50`, they hash equal,
//! and [`serialize_bytes`](Decimal::serialize_bytes) writes that canonical form — so equal values
//! are byte-equal and hash equal (the crate's value-identity rule), while ordering is the true
//! numeric order.

use core::cmp::Ordering;
use core::marker::PhantomData;

use super::{DecimalBacking, DecimalCoeff, DecimalError};

/// The largest coefficient is 32 bytes (`d256`); a stack scratch of this size encodes one
/// coefficient with no allocation for the hasher and the byte codec.
const MAX_WIDTH: usize = 32;

/// A single scaled-decimal value of width `B`: a coefficient integer and a scale, together
/// denoting `coefficient × 10^-scale`. `Decimal<Dec128> = D128`.
///
/// ```
/// use yggdryl_core::io::fixed::D128;
///
/// let price = D128::new(12345, 2).unwrap(); // 123.45
/// let tax = D128::new(617, 2).unwrap();     //   6.17
/// assert_eq!((price + tax).to_string(), "129.62");
/// assert_eq!(D128::new(25, 1).unwrap(), D128::new(250, 2).unwrap()); // 2.5 == 2.50
/// ```
pub struct Decimal<B: DecimalBacking> {
    coeff: B::Coeff,
    scale: i8,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> Decimal<B> {
    /// The decimal `coefficient × 10^-scale`, or [`CoefficientOutOfRange`](DecimalError::CoefficientOutOfRange)
    /// if `coefficient` does not fit this width's integer.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::D64;
    /// assert_eq!(D64::new(12345, 3).unwrap().to_string(), "12.345");
    /// ```
    pub fn new(coefficient: i128, scale: i8) -> Result<Self, DecimalError> {
        let coeff =
            B::Coeff::from_i128(coefficient).ok_or(DecimalError::CoefficientOutOfRange {
                ty: B::NAME,
                max_precision: B::MAX_PRECISION,
            })?;
        Ok(Self::from_coeff(coeff, scale))
    }

    /// A decimal from an already-typed coefficient and scale — total (the coefficient is in range
    /// by construction). The constructor the columnar family uses when it already holds the raw
    /// coefficient.
    pub fn from_coeff(coeff: B::Coeff, scale: i8) -> Self {
        Self {
            coeff,
            scale,
            _backing: PhantomData,
        }
    }

    /// The integer `value` as a scale-0 decimal, or [`CoefficientOutOfRange`](DecimalError::CoefficientOutOfRange)
    /// if it does not fit.
    pub fn from_i128(value: i128) -> Result<Self, DecimalError> {
        Self::new(value, 0)
    }

    /// A decimal from a plain integer **coefficient string** (optional leading `-`) and a scale —
    /// the width-agnostic constructor the bindings use to carry a `d256` coefficient beyond `i128`
    /// as text. [`CoefficientOutOfRange`](DecimalError::CoefficientOutOfRange) if the coefficient
    /// does not fit the width, or [`ParseError`](DecimalError::ParseError) if it is not an integer.
    pub fn from_coeff_str(digits: &str, scale: i8) -> Result<Self, DecimalError> {
        let trimmed = digits.trim();
        if trimmed.is_empty()
            || !trimmed
                .strip_prefix(['-', '+'])
                .unwrap_or(trimmed)
                .bytes()
                .all(|b| b.is_ascii_digit())
            || trimmed
                .strip_prefix(['-', '+'])
                .unwrap_or(trimmed)
                .is_empty()
        {
            return Err(DecimalError::ParseError { ty: B::NAME });
        }
        let coeff = B::Coeff::parse_int(trimmed).ok_or(DecimalError::CoefficientOutOfRange {
            ty: B::NAME,
            max_precision: B::MAX_PRECISION,
        })?;
        Ok(Self::from_coeff(coeff, scale))
    }

    /// The maximum precision (significant digits) this width holds — `9`/`18`/`38`/`76`.
    pub const fn max_precision() -> u8 {
        B::MAX_PRECISION
    }

    /// The coefficient width in bits (`32`/`64`/`128`/`256`).
    pub const fn bit_width() -> u32 {
        (B::WIDTH as u32) * 8
    }

    /// The zero value (`0`, scale `0`).
    pub fn zero() -> Self {
        Self::from_coeff(B::Coeff::ZERO, 0)
    }

    /// The scale — the number of fractional digits (`value = coefficient × 10^-scale`).
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// The raw (unscaled) coefficient as an `i128`, or `None` if it exceeds `i128` (`d256` only).
    pub fn coefficient(&self) -> Option<i128> {
        self.coeff.to_i128()
    }

    /// The raw typed coefficient — the columnar family's bridge to the value type (kept crate-only
    /// so the `d256` coefficient's Arrow type never appears in a public signature).
    pub(crate) fn raw_coeff(&self) -> B::Coeff {
        self.coeff
    }

    /// The coefficient's canonical little-endian bytes (`B::WIDTH` of them).
    pub fn coefficient_le_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; B::WIDTH];
        self.coeff.write_le(&mut bytes);
        bytes
    }

    /// The raw (unscaled) coefficient as a decimal integer string (`"-12345"`) — the width-agnostic
    /// bridge the bindings use to carry a coefficient beyond `i128` (`d256`) to a native big
    /// integer. The inverse of [`from_coeff_str`](Decimal::from_coeff_str).
    pub fn coefficient_string(&self) -> String {
        format!("{}", self.coeff)
    }

    /// Whether the value is exactly zero.
    pub fn is_zero(&self) -> bool {
        self.coeff == B::Coeff::ZERO
    }

    /// Whether the value is strictly negative.
    pub fn is_negative(&self) -> bool {
        self.coeff.is_negative()
    }

    /// Whether the value is strictly positive.
    pub fn is_positive(&self) -> bool {
        !self.is_negative() && !self.is_zero()
    }

    /// The value's **precision** — the count of significant digits in the coefficient (`0` → `0`,
    /// `123.45` → `5`).
    pub fn precision(&self) -> u32 {
        self.coeff.digit_count()
    }

    /// The stable, lower-case type name (`"d32"` … `"d256"`).
    pub fn type_name(&self) -> &'static str {
        B::NAME
    }

    // ---- normalization & rescaling ---------------------------------------------------------

    /// The **normalized** form — the same value with trailing fractional zeros stripped (`2.50` →
    /// `2.5`, `0` → scale `0`). This is the canonical form used for equality, hashing, and the
    /// byte codec, so equal values share it exactly.
    pub fn normalized(&self) -> Self {
        let ten = B::Coeff::from_i128(10).expect("10 fits every width");
        let mut coeff = self.coeff;
        let mut scale = self.scale;
        while scale > 0 && coeff != B::Coeff::ZERO {
            match coeff.checked_rem(ten) {
                Some(r) if r == B::Coeff::ZERO => {
                    coeff = coeff
                        .checked_div(ten)
                        .expect("division by ten never overflows");
                    scale -= 1;
                }
                _ => break,
            }
        }
        Self::from_coeff(coeff, scale)
    }

    /// The coefficient raised to `target` scale (`coeff × 10^(target - self.scale)`), requiring
    /// `target >= self.scale`; `None` on overflow.
    fn raised_coeff(&self, target: i8) -> Option<B::Coeff> {
        let exp = (target as i32 - self.scale as i32).max(0) as u32;
        if exp == 0 {
            return Some(self.coeff);
        }
        self.coeff.checked_mul(B::Coeff::checked_pow10(exp)?)
    }

    /// This value re-expressed at `new_scale`, **exactly** — raising the scale is loss-free (bar
    /// overflow); lowering it errors [`InexactRescale`](DecimalError::InexactRescale) if any
    /// non-zero fractional digit would be dropped. Use [`round_to_scale`](Decimal::round_to_scale)
    /// / [`trunc_to_scale`](Decimal::trunc_to_scale) to opt into the loss.
    pub fn rescale(&self, new_scale: i8) -> Result<Self, DecimalError> {
        if new_scale >= self.scale {
            let coeff = self.raised_coeff(new_scale).ok_or(DecimalError::Overflow {
                ty: B::NAME,
                op: "rescale",
            })?;
            Ok(Self::from_coeff(coeff, new_scale))
        } else {
            let drop = (self.scale as i32 - new_scale as i32) as u32;
            let divisor = B::Coeff::checked_pow10(drop).ok_or(DecimalError::Overflow {
                ty: B::NAME,
                op: "rescale",
            })?;
            let remainder = self.coeff.checked_rem(divisor).unwrap_or(B::Coeff::ZERO);
            if remainder != B::Coeff::ZERO {
                return Err(DecimalError::InexactRescale {
                    ty: B::NAME,
                    from: self.scale,
                    to: new_scale,
                });
            }
            let coeff = self
                .coeff
                .checked_div(divisor)
                .expect("divisor is non-zero");
            Ok(Self::from_coeff(coeff, new_scale))
        }
    }

    /// This value at `new_scale`, **truncating** any dropped digits toward zero (no error unless
    /// raising the scale overflows).
    pub fn trunc_to_scale(&self, new_scale: i8) -> Result<Self, DecimalError> {
        if new_scale >= self.scale {
            return self.rescale(new_scale);
        }
        let drop = (self.scale as i32 - new_scale as i32) as u32;
        let divisor = B::Coeff::checked_pow10(drop).ok_or(DecimalError::Overflow {
            ty: B::NAME,
            op: "rescale",
        })?;
        let coeff = self
            .coeff
            .checked_div(divisor)
            .expect("divisor is non-zero");
        Ok(Self::from_coeff(coeff, new_scale))
    }

    /// This value at `new_scale`, **rounding** dropped digits half-away-from-zero (no error unless
    /// raising the scale overflows).
    pub fn round_to_scale(&self, new_scale: i8) -> Result<Self, DecimalError> {
        if new_scale >= self.scale {
            return self.rescale(new_scale);
        }
        let drop = (self.scale as i32 - new_scale as i32) as u32;
        let divisor = B::Coeff::checked_pow10(drop).ok_or(DecimalError::Overflow {
            ty: B::NAME,
            op: "rescale",
        })?;
        let quotient = self.coeff.checked_div(divisor).expect("non-zero divisor");
        let remainder = self.coeff.checked_rem(divisor).expect("non-zero divisor");
        // Round half-away-from-zero: bump the magnitude when |remainder| * 2 >= divisor.
        let rem_magnitude = if remainder.is_negative() {
            remainder.checked_neg().unwrap_or(remainder)
        } else {
            remainder
        };
        let two = B::Coeff::from_i128(2).unwrap();
        let bump = match rem_magnitude.checked_mul(two) {
            Some(twice) => twice >= divisor,
            None => true, // |2*remainder| overflowed => it was at least the divisor
        };
        let coeff = if bump {
            let one = if self.coeff.is_negative() {
                B::Coeff::from_i128(-1).unwrap()
            } else {
                B::Coeff::from_i128(1).unwrap()
            };
            quotient.checked_add(one).ok_or(DecimalError::Overflow {
                ty: B::NAME,
                op: "round",
            })?
        } else {
            quotient
        };
        Ok(Self::from_coeff(coeff, new_scale))
    }

    /// The integer part, truncated toward zero (scale `0` when there is a fractional part).
    pub fn trunc(&self) -> Self {
        if self.scale <= 0 {
            return *self;
        }
        // Dividing by 10^scale is always in range (never overflows), so this cannot fail.
        self.trunc_to_scale(0)
            .expect("truncating toward a smaller scale never overflows")
    }

    // ---- arithmetic ------------------------------------------------------------------------

    /// The aligned coefficients of `self` and `other` at their common (maximum) scale, or `None`
    /// on overflow while raising the lower-scale operand.
    fn aligned(&self, other: &Self) -> Option<(B::Coeff, B::Coeff, i8)> {
        if self.scale == other.scale {
            return Some((self.coeff, other.coeff, self.scale));
        }
        let scale = self.scale.max(other.scale);
        Some((self.raised_coeff(scale)?, other.raised_coeff(scale)?, scale))
    }

    /// `self + other` (scales aligned), or [`Overflow`](DecimalError::Overflow).
    pub fn checked_add(&self, other: &Self) -> Result<Self, DecimalError> {
        let (a, b, scale) = self.aligned(other).ok_or(overflow::<B>("add"))?;
        Ok(Self::from_coeff(
            a.checked_add(b).ok_or(overflow::<B>("add"))?,
            scale,
        ))
    }

    /// `self - other` (scales aligned), or [`Overflow`](DecimalError::Overflow).
    pub fn checked_sub(&self, other: &Self) -> Result<Self, DecimalError> {
        let (a, b, scale) = self.aligned(other).ok_or(overflow::<B>("sub"))?;
        Ok(Self::from_coeff(
            a.checked_sub(b).ok_or(overflow::<B>("sub"))?,
            scale,
        ))
    }

    /// `self × other` — the result scale is the **sum** of the operand scales — or
    /// [`Overflow`](DecimalError::Overflow).
    pub fn checked_mul(&self, other: &Self) -> Result<Self, DecimalError> {
        let scale = self.scale as i32 + other.scale as i32;
        if !(i8::MIN as i32..=i8::MAX as i32).contains(&scale) {
            return Err(overflow::<B>("mul"));
        }
        Ok(Self::from_coeff(
            self.coeff
                .checked_mul(other.coeff)
                .ok_or(overflow::<B>("mul"))?,
            scale as i8,
        ))
    }

    /// `-self`, or [`Overflow`](DecimalError::Overflow) (the two's-complement minimum has no
    /// negation in range).
    pub fn checked_neg(&self) -> Result<Self, DecimalError> {
        Ok(Self::from_coeff(
            self.coeff.checked_neg().ok_or(overflow::<B>("neg"))?,
            self.scale,
        ))
    }

    /// The absolute value, or [`Overflow`](DecimalError::Overflow) at the minimum.
    pub fn checked_abs(&self) -> Result<Self, DecimalError> {
        if self.is_negative() {
            self.checked_neg()
        } else {
            Ok(*self)
        }
    }

    /// `self % other` (scales aligned; truncating), or [`DivideByZero`](DecimalError::DivideByZero).
    pub fn checked_rem(&self, other: &Self) -> Result<Self, DecimalError> {
        let (a, b, scale) = self.aligned(other).ok_or(overflow::<B>("rem"))?;
        if b == B::Coeff::ZERO {
            return Err(DecimalError::DivideByZero { ty: B::NAME });
        }
        Ok(Self::from_coeff(
            a.checked_rem(b).ok_or(overflow::<B>("rem"))?,
            scale,
        ))
    }

    /// `self / other` at the given `result_scale` (truncating any further digits), or
    /// [`DivideByZero`](DecimalError::DivideByZero) / [`Overflow`](DecimalError::Overflow). Decimal
    /// division rarely terminates, so the caller states the scale it wants.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::D128;
    /// let a = D128::new(1, 0).unwrap();
    /// let b = D128::new(3, 0).unwrap();
    /// assert_eq!(a.checked_div(&b, 4).unwrap().to_string(), "0.3333");
    /// ```
    pub fn checked_div(&self, other: &Self, result_scale: i8) -> Result<Self, DecimalError> {
        if other.coeff == B::Coeff::ZERO {
            return Err(DecimalError::DivideByZero { ty: B::NAME });
        }
        // result = (coeff_a / coeff_b) * 10^(sb - sa + result_scale). Bring the exponent onto
        // whichever side keeps the integer division exact-to-scale.
        let exp = result_scale as i32 + other.scale as i32 - self.scale as i32;
        let coeff = if exp >= 0 {
            let scaled = self
                .coeff
                .checked_mul(B::Coeff::checked_pow10(exp as u32).ok_or(overflow::<B>("div"))?)
                .ok_or(overflow::<B>("div"))?;
            scaled
                .checked_div(other.coeff)
                .ok_or(overflow::<B>("div"))?
        } else {
            let divisor = other
                .coeff
                .checked_mul(B::Coeff::checked_pow10((-exp) as u32).ok_or(overflow::<B>("div"))?)
                .ok_or(overflow::<B>("div"))?;
            self.coeff
                .checked_div(divisor)
                .ok_or(overflow::<B>("div"))?
        };
        Ok(Self::from_coeff(coeff, result_scale))
    }

    // ---- conversions & numeric interop -----------------------------------------------------

    /// This value as an `f64` (lossy beyond `f64`'s 53-bit mantissa).
    pub fn to_f64(&self) -> f64 {
        self.coeff.to_f64() / 10f64.powi(self.scale as i32)
    }

    /// The decimal nearest `value` at the given `scale`, or [`NonFinite`](DecimalError::NonFinite)
    /// for `NaN`/`±inf` and [`CoefficientOutOfRange`](DecimalError::CoefficientOutOfRange) if the
    /// scaled value overflows the width. Lossy: `f64` cannot represent every decimal.
    pub fn from_f64(value: f64, scale: i8) -> Result<Self, DecimalError> {
        if !value.is_finite() {
            return Err(DecimalError::NonFinite { ty: B::NAME });
        }
        let scaled = (value * 10f64.powi(scale as i32)).round();
        // Bridge through `i128`. An `f64` magnitude beyond `i128` is already far past `f64`'s
        // exact-integer range (2^53), so treating it as out of range loses nothing real.
        if scaled.abs() < 1.7014118346046923e38 {
            Self::new(scaled as i128, scale)
        } else {
            Err(DecimalError::CoefficientOutOfRange {
                ty: B::NAME,
                max_precision: B::MAX_PRECISION,
            })
        }
    }

    /// This value as an exact `i128`, or [`NotInteger`](DecimalError::NotInteger) if it has a
    /// fractional part (use [`trunc`](Decimal::trunc) / [`round_to_scale`](Decimal::round_to_scale)
    /// first) / [`OutOfWidth`](DecimalError::OutOfWidth) if the integer exceeds `i128`.
    pub fn to_i128(&self) -> Result<i128, DecimalError> {
        let n = self.normalized();
        if n.scale > 0 {
            return Err(DecimalError::NotInteger {
                ty: B::NAME,
                scale: self.scale,
            });
        }
        let mut value = n.coeff.to_i128().ok_or(DecimalError::OutOfWidth {
            from: B::NAME,
            to: "i128",
        })?;
        for _ in 0..(-n.scale as i32) {
            value = value.checked_mul(10).ok_or(DecimalError::OutOfWidth {
                from: B::NAME,
                to: "i128",
            })?;
        }
        Ok(value)
    }

    /// This value re-typed to another decimal width `C`, same scale, or
    /// [`OutOfWidth`](DecimalError::OutOfWidth) if its magnitude does not fit `C`. Widening (e.g.
    /// `D32 → D128`) is always loss-free; narrowing can overflow.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{D32, Dec128};
    /// let small = D32::new(12345, 2).unwrap();       // 123.45 as d32
    /// let wide = small.cast::<Dec128>().unwrap();     // 123.45 as d128
    /// assert_eq!(wide.to_string(), "123.45");
    /// ```
    pub fn cast<C: DecimalBacking>(&self) -> Result<Decimal<C>, DecimalError> {
        // Bridge through the coefficient's decimal digits rather than `i128`, so a `d256`
        // coefficient beyond `i128` still casts (to `d256`, or to a narrower width when it fits).
        let coeff =
            C::Coeff::parse_int(&self.coefficient_string()).ok_or(DecimalError::OutOfWidth {
                from: B::NAME,
                to: C::NAME,
            })?;
        Ok(Decimal::<C>::from_coeff(coeff, self.scale))
    }

    // ---- byte codec ------------------------------------------------------------------------

    /// The serialized width of one value: one scale byte plus the coefficient (`1 + B::WIDTH`).
    pub const fn serialized_len() -> usize {
        1 + B::WIDTH
    }

    /// Writes the canonical `[scale: i8][coefficient: LE]` of the **normalized** value into `out`
    /// (which must be at least [`serialized_len`](Decimal::serialized_len) bytes) — no allocation.
    pub fn write_serialized(&self, out: &mut [u8]) {
        let n = self.normalized();
        out[0] = n.scale as u8;
        n.coeff.write_le(&mut out[1..]);
    }

    /// The canonical byte encoding — `[scale: i8][coefficient: little-endian]` of the normalized
    /// value. Equal values (`2.5` / `2.50`) encode identically; the inverse is
    /// [`deserialize_bytes`](Decimal::deserialize_bytes).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; Self::serialized_len()];
        self.write_serialized(&mut bytes);
        bytes
    }

    /// Reconstructs a value from [`serialize_bytes`](Decimal::serialize_bytes), or
    /// [`ParseError`](DecimalError::ParseError) if the slice is too short.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, DecimalError> {
        if bytes.len() < Self::serialized_len() {
            return Err(DecimalError::ParseError { ty: B::NAME });
        }
        let scale = bytes[0] as i8;
        let coeff = B::Coeff::read_le(&bytes[1..]);
        Ok(Self::from_coeff(coeff, scale))
    }
}

/// Builds an [`Overflow`](DecimalError::Overflow) for width `B` and operation `op`.
fn overflow<B: DecimalBacking>(op: &'static str) -> DecimalError {
    DecimalError::Overflow { ty: B::NAME, op }
}

// ------------------------------------------------------------------------------------------
// Value semantics: identity is by (normalized) value; ordering is true numeric order.
// ------------------------------------------------------------------------------------------

impl<B: DecimalBacking> Clone for Decimal<B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<B: DecimalBacking> Copy for Decimal<B> {}

impl<B: DecimalBacking> Default for Decimal<B> {
    fn default() -> Self {
        Self::zero()
    }
}

impl<B: DecimalBacking> Ord for Decimal<B> {
    /// True numeric order. Aligns scales with a checked multiply; when that multiply would
    /// overflow, the lower-scale operand's magnitude already exceeds the other's, so its sign
    /// decides the order — total and panic-free.
    fn cmp(&self, other: &Self) -> Ordering {
        if self.scale == other.scale {
            return self.coeff.cmp(&other.coeff);
        }
        let (small, large, swapped) = if self.scale <= other.scale {
            (self, other, false)
        } else {
            (other, self, true)
        };
        let ord = match small.raised_coeff(large.scale) {
            Some(scaled) => scaled.cmp(&large.coeff),
            // Raising `small` to `large`'s scale overflowed => |small| > |large|, so `small`'s
            // sign is the order of `small` vs `large`.
            None => small.coeff.cmp(&B::Coeff::ZERO),
        };
        if swapped {
            ord.reverse()
        } else {
            ord
        }
    }
}

impl<B: DecimalBacking> PartialOrd for Decimal<B> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<B: DecimalBacking> PartialEq for Decimal<B> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<B: DecimalBacking> Eq for Decimal<B> {}

impl<B: DecimalBacking> core::hash::Hash for Decimal<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Hash the normalized form so equal values (2.5 / 2.50) hash equal — streamed into the
        // hasher through a stack scratch, no allocation.
        let n = self.normalized();
        state.write_i8(n.scale);
        let mut scratch = [0u8; MAX_WIDTH];
        n.coeff.write_le(&mut scratch);
        state.write(&scratch[..B::WIDTH]);
    }
}

impl<B: DecimalBacking> core::fmt::Debug for Decimal<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}({})", B::NAME, self)
    }
}

impl<B: DecimalBacking> core::fmt::Display for Decimal<B> {
    /// `[-]int[.frac]` — the coefficient digits with the decimal point placed `scale` from the
    /// right (padding with leading zeros); a non-positive scale prints an integer.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let digits = format!("{}", self.coeff);
        let negative = digits.starts_with('-');
        let magnitude = digits.strip_prefix('-').unwrap_or(&digits);
        if negative {
            f.write_str("-")?;
        }
        if self.scale <= 0 {
            f.write_str(magnitude)?;
            for _ in 0..(-self.scale as i32) {
                f.write_str("0")?;
            }
            return Ok(());
        }
        let scale = self.scale as usize;
        if magnitude.len() > scale {
            let point = magnitude.len() - scale;
            f.write_str(&magnitude[..point])?;
            f.write_str(".")?;
            f.write_str(&magnitude[point..])
        } else {
            f.write_str("0.")?;
            for _ in 0..(scale - magnitude.len()) {
                f.write_str("0")?;
            }
            f.write_str(magnitude)
        }
    }
}

impl<B: DecimalBacking> core::str::FromStr for Decimal<B> {
    type Err = DecimalError;

    /// Parses `[+-]?digits(.digits)?([eE][+-]?digits)?` — a plain literal (`"2.50"` → coefficient
    /// `250`, scale `2`) or scientific notation (`"1.5e3"` → `1500`, `"1.5e-2"` → `0.015`). The
    /// exponent form lets a Python `decimal.Decimal`'s string cross unchanged.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        let err = || DecimalError::ParseError { ty: B::NAME };
        let (negative, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        if rest.is_empty() {
            return Err(err());
        }
        // Split off an optional exponent (`e`/`E`), then the fraction.
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((mantissa, exp)) => (mantissa, exp.parse::<i32>().map_err(|_| err())?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = match mantissa.split_once('.') {
            Some((int_part, frac_part)) => (int_part, frac_part),
            None => (mantissa, ""),
        };
        if !int_part.bytes().all(|b| b.is_ascii_digit())
            || !frac_part.bytes().all(|b| b.is_ascii_digit())
            || (int_part.is_empty() && frac_part.is_empty())
        {
            return Err(err());
        }
        // `value = digits × 10^-(frac_len - exponent)`; the scale must fit an `i8`.
        let scale = frac_part.len() as i32 - exponent;
        if !(i8::MIN as i32..=i8::MAX as i32).contains(&scale) {
            return Err(err());
        }
        let mut digits = String::with_capacity(1 + int_part.len() + frac_part.len());
        if negative {
            digits.push('-');
        }
        digits.push_str(int_part);
        digits.push_str(frac_part);
        // "-" or "" would slip through with empty digits; guard it.
        if digits.trim_start_matches('-').is_empty() {
            return Err(err());
        }
        let coeff = B::Coeff::parse_int(&digits).ok_or(DecimalError::CoefficientOutOfRange {
            ty: B::NAME,
            max_precision: B::MAX_PRECISION,
        })?;
        Ok(Self::from_coeff(coeff, scale as i8))
    }
}

// The `+` / `-` / `*` / unary `-` operators panic on overflow with the guided message (like Rust's
// integer operators in debug); reach for the `checked_*` methods to handle overflow as a value.
impl<B: DecimalBacking> core::ops::Add for Decimal<B> {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        self.checked_add(&rhs).unwrap_or_else(|e| panic!("{e}"))
    }
}
impl<B: DecimalBacking> core::ops::Sub for Decimal<B> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self.checked_sub(&rhs).unwrap_or_else(|e| panic!("{e}"))
    }
}
impl<B: DecimalBacking> core::ops::Mul for Decimal<B> {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        self.checked_mul(&rhs).unwrap_or_else(|e| panic!("{e}"))
    }
}
impl<B: DecimalBacking> core::ops::Neg for Decimal<B> {
    type Output = Self;
    fn neg(self) -> Self {
        self.checked_neg().unwrap_or_else(|e| panic!("{e}"))
    }
}
