//! The **type converter** — casting a value / scalar / serie / buffer of one type into another.
//!
//! The entry points are the compile-time-generic `cast` methods on the fixed value types
//! ([`Scalar::cast`], [`Serie::cast`], [`Buffer::cast`]), which delegate to the [`Converter`]
//! trait defined here. A cast to the **same** type is a no-op that shares the backing buffer (no
//! data copy). Across the numeric family every pair is reachable directly (range-checked for
//! integers, precision-lossy for floats); every value also bridges to and from **UTF-8**
//! ([`Scalar::to_utf8`] / [`Utf8Scalar::parse_to`]) and **binary** ([`Scalar::to_binary`] /
//! [`BinaryScalar::read_to`]) — the two universal formats — so anything can reach anything.
//!
//! DESIGN: this first increment covers the numeric primitives (`u8`…`i128`, `f16`/`f32`/`f64`), the
//! null passthrough (a null casts to a null of the target), and the UTF-8 / binary bridges. The wide
//! byte-newtype integers (`u96`/`u128`/`u256`/`i96`/`i256`), the decimals, the temporal family, and
//! the fixed-size byte types are reached today through the UTF-8 / binary bridges; direct
//! [`Converter`] impls for them are a follow-up increment (they need a wider-than-`i128`
//! intermediate or a scale-aware path).

use core::any::TypeId;

use crate::io::fixed::{f16, Buffer, NativeType, Scalar, Serie};
use crate::io::var::{BinaryScalar, Utf8Scalar};

/// A guided cast failure — every variant names the offending value and the target type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastError {
    /// The value does not fit the target type's range (integer overflow / underflow).
    OutOfRange {
        /// The offending value, rendered.
        value: String,
        /// The target type name.
        to: &'static str,
    },
    /// A non-finite float (`NaN` / `±∞`) cannot become an integer.
    NotFinite {
        /// The target integer type name.
        to: &'static str,
    },
    /// The text could not be parsed as the target type.
    Parse {
        /// The offending text.
        text: String,
        /// The target type name.
        to: &'static str,
    },
    /// A binary value's byte width does not match the fixed width the target needs.
    WidthMismatch {
        /// The value's byte width.
        got: usize,
        /// The width the target type needs.
        need: usize,
        /// The target type name.
        to: &'static str,
    },
}

impl core::fmt::Display for CastError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfRange { value, to } => {
                write!(f, "value {value} is out of range for `{to}`")
            }
            Self::NotFinite { to } => {
                write!(
                    f,
                    "cannot cast a non-finite float (NaN/∞) to the integer type `{to}`"
                )
            }
            Self::Parse { text, to } => write!(f, "cannot parse {text:?} as `{to}`"),
            Self::WidthMismatch { got, need, to } => write!(
                f,
                "binary value is {got} bytes, but `{to}` needs exactly {need}"
            ),
        }
    }
}

impl std::error::Error for CastError {}

// -------------------------------------------------------------------------------------
// Numeric coercion capability — the i128 (integer) / f64 (float) intermediate.
// -------------------------------------------------------------------------------------

/// The numeric coercion capability of a primitive numeric [`NativeType`]: its exact `i128`
/// (integers) or `f64` (floats) bridge, so any numeric casts to any other through a common
/// intermediate. Integer targets are **range-checked**; float targets are precision-lossy.
pub trait NumericCast: NativeType {
    /// Whether this is a floating-point type.
    const IS_FLOAT: bool;

    /// This value as an `f64` (exact for the small integers, lossy for the wide ones and never
    /// for the floats).
    fn to_f64(self) -> f64;

    /// This value as an `i128` (exact for integers; a truncating cast for floats, unused on the
    /// float path).
    fn to_i128(self) -> i128;

    /// The value for `v`, or `None` if `v` is out of this type's range.
    fn try_from_i128(v: i128) -> Option<Self>;

    /// The value nearest `v` (rounding toward zero for integers), or `None` if `v` is non-finite
    /// or out of this type's range.
    fn from_f64(v: f64) -> Option<Self>;

    // ---- element-wise arithmetic kernels (the vectorized-op fast path) ---------------------
    //
    // The five arithmetic operators as per-type element kernels, so a `Serie<T: NumericCast>` op
    // loop stays one generic pass. Integers **wrap** (`wrapping_*`, like Arrow / numpy); floats use
    // the normal IEEE op. Division / remainder return `Option` — `None` only when an **integer**
    // divisor is zero (the caller writes a null), so the hot loop never panics; a float divides by
    // zero to IEEE `±∞` / `NaN` (always `Some`).

    /// `self + rhs` — wrapping for integers, IEEE for floats.
    fn add_wrapping(self, rhs: Self) -> Self;

    /// `self - rhs` — wrapping for integers, IEEE for floats.
    fn sub_wrapping(self, rhs: Self) -> Self;

    /// `self * rhs` — wrapping for integers, IEEE for floats.
    fn mul_wrapping(self, rhs: Self) -> Self;

    /// `self / rhs` — `None` when an **integer** `rhs` is zero (→ a null, no panic), else the
    /// wrapping quotient; a float always divides (IEEE `±∞` / `NaN`).
    fn div_checked(self, rhs: Self) -> Option<Self>;

    /// `self % rhs` — `None` when an **integer** `rhs` is zero (→ a null, no panic), else the
    /// wrapping remainder; a float always takes the IEEE remainder.
    fn rem_checked(self, rhs: Self) -> Option<Self>;
}

macro_rules! int_numeric {
    ($t:ty) => {
        impl NumericCast for $t {
            const IS_FLOAT: bool = false;
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn to_i128(self) -> i128 {
                self as i128
            }
            fn try_from_i128(v: i128) -> Option<Self> {
                <$t>::try_from(v).ok()
            }
            fn from_f64(v: f64) -> Option<Self> {
                if !v.is_finite() {
                    return None;
                }
                let t = v.trunc();
                // The bounds land inside `f64`'s exact-integer range for every type up to `i64`;
                // for `i128` the `as f64` bound is approximate, which only widens acceptance at the
                // extreme edge — `try_from_i128` on the truncated value is the exact gate below.
                (t >= <$t>::MIN as f64 && t <= <$t>::MAX as f64).then_some(t as $t)
            }
            fn add_wrapping(self, rhs: Self) -> Self {
                self.wrapping_add(rhs)
            }
            fn sub_wrapping(self, rhs: Self) -> Self {
                self.wrapping_sub(rhs)
            }
            fn mul_wrapping(self, rhs: Self) -> Self {
                self.wrapping_mul(rhs)
            }
            fn div_checked(self, rhs: Self) -> Option<Self> {
                // `wrapping_div` handles the lone signed `MIN / -1` overflow (wraps to `MIN`); the
                // zero divisor is the only case that must not reach it, so gate on it here.
                (rhs != 0).then(|| self.wrapping_div(rhs))
            }
            fn rem_checked(self, rhs: Self) -> Option<Self> {
                (rhs != 0).then(|| self.wrapping_rem(rhs))
            }
        }
    };
}
int_numeric!(u8);
int_numeric!(u16);
int_numeric!(u32);
int_numeric!(u64);
int_numeric!(i8);
int_numeric!(i16);
int_numeric!(i32);
int_numeric!(i64);
int_numeric!(i128);

macro_rules! float_numeric {
    ($t:ty) => {
        impl NumericCast for $t {
            const IS_FLOAT: bool = true;
            fn to_f64(self) -> f64 {
                self as f64
            }
            fn to_i128(self) -> i128 {
                self as i128
            }
            fn try_from_i128(v: i128) -> Option<Self> {
                Some(v as $t)
            }
            fn from_f64(v: f64) -> Option<Self> {
                Some(v as $t)
            }
            fn add_wrapping(self, rhs: Self) -> Self {
                self + rhs
            }
            fn sub_wrapping(self, rhs: Self) -> Self {
                self - rhs
            }
            fn mul_wrapping(self, rhs: Self) -> Self {
                self * rhs
            }
            fn div_checked(self, rhs: Self) -> Option<Self> {
                Some(self / rhs) // IEEE: division by zero is ±∞ / NaN, never a panic
            }
            fn rem_checked(self, rhs: Self) -> Option<Self> {
                Some(self % rhs)
            }
        }
    };
}
float_numeric!(f32);
float_numeric!(f64);

impl NumericCast for f16 {
    const IS_FLOAT: bool = true;
    fn to_f64(self) -> f64 {
        self.to_f64()
    }
    fn to_i128(self) -> i128 {
        self.to_f64() as i128
    }
    fn try_from_i128(v: i128) -> Option<Self> {
        Some(f16::from_f64(v as f64))
    }
    fn from_f64(v: f64) -> Option<Self> {
        Some(f16::from_f64(v))
    }
    // f16 has no native arithmetic; compute in f64 and round back — the same f64 bridge the casts use.
    fn add_wrapping(self, rhs: Self) -> Self {
        f16::from_f64(self.to_f64() + rhs.to_f64())
    }
    fn sub_wrapping(self, rhs: Self) -> Self {
        f16::from_f64(self.to_f64() - rhs.to_f64())
    }
    fn mul_wrapping(self, rhs: Self) -> Self {
        f16::from_f64(self.to_f64() * rhs.to_f64())
    }
    fn div_checked(self, rhs: Self) -> Option<Self> {
        Some(f16::from_f64(self.to_f64() / rhs.to_f64())) // IEEE (±∞ / NaN), never a panic
    }
    fn rem_checked(self, rhs: Self) -> Option<Self> {
        Some(f16::from_f64(self.to_f64() % rhs.to_f64()))
    }
}

/// Casts a numeric value `From` → `To`: exact `i128` when both are integers (range-checked), else
/// through `f64` (precision-lossy; a non-finite float cannot become an integer).
fn cast_numeric<From: NumericCast, To: NumericCast>(value: From) -> Result<To, CastError> {
    if From::IS_FLOAT || To::IS_FLOAT {
        let f = value.to_f64();
        if To::IS_FLOAT {
            return Ok(To::from_f64(f).expect("float targets accept any f64"));
        }
        // Float (or int) → integer target: reject non-finite, then range-check.
        if !f.is_finite() {
            return Err(CastError::NotFinite { to: To::NAME });
        }
        To::from_f64(f).ok_or_else(|| CastError::OutOfRange {
            value: f.trunc().to_string(),
            to: To::NAME,
        })
    } else {
        let i = value.to_i128();
        To::try_from_i128(i).ok_or_else(|| CastError::OutOfRange {
            value: i.to_string(),
            to: To::NAME,
        })
    }
}

// -------------------------------------------------------------------------------------
// The Converter trait — the cast contract (value / scalar / serie / buffer).
// -------------------------------------------------------------------------------------

/// The **cast contract** from `Self` to a target [`NativeType`] `To`, at four granularities:
/// a single [`cast_value`](Converter::cast_value), a nullable [`cast_scalar`](Converter::cast_scalar),
/// a column [`cast_serie`](Converter::cast_serie), and a raw [`cast_buffer`](Converter::cast_buffer).
/// The scalar / serie / buffer methods are mutualized defaults over `cast_value` (a null stays a
/// null). Implemented across the numeric family; the [`Scalar::cast`] / [`Serie::cast`] /
/// [`Buffer::cast`] inherent methods are the entry points and add the same-type fast path.
pub trait Converter<To: NativeType>: NativeType {
    /// Cast one value.
    fn cast_value(value: Self) -> Result<To, CastError>;

    /// Cast one nullable scalar (a null casts to a null of the target).
    fn cast_scalar(scalar: &Scalar<Self>) -> Result<Scalar<To>, CastError> {
        match scalar.value() {
            Some(value) => Self::cast_value(value).map(Scalar::of),
            None => Ok(Scalar::null()),
        }
    }

    /// Cast a whole column element-for-element (nulls preserved).
    fn cast_serie(serie: &Serie<Self>) -> Result<Serie<To>, CastError> {
        let mut out: Vec<Option<To>> = Vec::with_capacity(serie.len());
        for index in 0..serie.len() {
            out.push(match serie.get(index) {
                Some(value) => Some(Self::cast_value(value)?),
                None => None,
            });
        }
        Ok(Serie::from_options(&out))
    }

    /// Cast a raw (non-null) buffer.
    fn cast_buffer(buffer: &Buffer<Self>) -> Result<Buffer<To>, CastError> {
        let mut out = Buffer::with_capacity(buffer.count());
        for index in 0..buffer.count() {
            out.push(Self::cast_value(buffer.get(index).expect("index < count"))?);
        }
        Ok(out)
    }
}

// One blanket impl over the numeric family — no overlap (there is no other `Converter` impl), so
// the same-type case (`From == To`) rides the exact round-trip too.
impl<From: NumericCast, To: NumericCast> Converter<To> for From {
    fn cast_value(value: From) -> Result<To, CastError> {
        cast_numeric::<From, To>(value)
    }
}

// -------------------------------------------------------------------------------------
// Inherent `cast` entry points — with the same-type no-copy fast path.
// -------------------------------------------------------------------------------------

impl<T: NumericCast> Scalar<T> {
    /// This scalar cast to numeric type `U` — range-checked for an integer target, precision-lossy
    /// for a float. A null casts to a null of `U`.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Scalar;
    ///
    /// assert_eq!(Scalar::of(300i32).cast::<i64>().unwrap(), Scalar::of(300i64));
    /// assert!(Scalar::of(300i32).cast::<u8>().is_err());      // 300 > u8::MAX
    /// assert_eq!(Scalar::<i32>::null().cast::<f64>().unwrap(), Scalar::null());
    /// ```
    pub fn cast<U: NumericCast>(&self) -> Result<Scalar<U>, CastError> {
        <T as Converter<U>>::cast_scalar(self)
    }
}

impl<T: NumericCast> Serie<T> {
    /// This column cast to numeric type `U` (nulls preserved). Casting to the **same** type shares
    /// the backing buffer with no data copy; a different type converts element-for-element.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    /// let wide = col.cast::<i64>().unwrap();
    /// assert_eq!(wide.to_options(), [Some(1i64), None, Some(3)]);
    /// ```
    pub fn cast<U: NumericCast>(&self) -> Result<Serie<U>, CastError> {
        if TypeId::of::<T>() == TypeId::of::<U>() {
            // SAFETY: `T` and `U` are the same type (equal `TypeId`), so `Serie<T>` and `Serie<U>`
            // are the identical type and layout. The clone shares the `Arc`-backed values buffer,
            // so this is a no-op cast with no data copy; `forget` avoids dropping the moved-out
            // source twice.
            let cloned = self.clone();
            let out = unsafe { core::mem::transmute_copy::<Serie<T>, Serie<U>>(&cloned) };
            core::mem::forget(cloned);
            return Ok(out);
        }
        <T as Converter<U>>::cast_serie(self)
    }
}

impl<T: NumericCast> Buffer<T> {
    /// This buffer cast to numeric type `U`. Casting to the **same** type shares the backing
    /// allocation with no data copy.
    pub fn cast<U: NumericCast>(&self) -> Result<Buffer<U>, CastError> {
        if TypeId::of::<T>() == TypeId::of::<U>() {
            // SAFETY: same type (equal `TypeId`) ⇒ same layout; the clone shares the allocation.
            let cloned = self.clone();
            let out = unsafe { core::mem::transmute_copy::<Buffer<T>, Buffer<U>>(&cloned) };
            core::mem::forget(cloned);
            return Ok(out);
        }
        <T as Converter<U>>::cast_buffer(self)
    }
}

// -------------------------------------------------------------------------------------
// The universal UTF-8 / binary bridges — anything reaches anything through a string or its bytes.
// -------------------------------------------------------------------------------------

impl<T: NativeType + core::fmt::Display> Scalar<T> {
    /// This scalar as a **UTF-8** scalar — the value's `Display` form (a null stays null). The
    /// universal "any → utf8" bridge.
    pub fn to_utf8(&self) -> Utf8Scalar {
        match self.value() {
            Some(value) => Utf8Scalar::of(&value.to_string()),
            None => Utf8Scalar::null(),
        }
    }
}

impl<T: NativeType> Scalar<T> {
    /// This scalar as a **binary** scalar — the value's canonical little-endian bytes (a null
    /// stays null). The universal "any → binary" bridge; reverse with [`BinaryScalar::read_to`].
    pub fn to_binary(&self) -> BinaryScalar {
        match self.value() {
            Some(value) => {
                let mut bytes = [0u8; 32];
                value.write_le(&mut bytes);
                BinaryScalar::of(&bytes[..T::WIDTH])
            }
            None => BinaryScalar::null(),
        }
    }
}

impl Utf8Scalar {
    /// Parse this UTF-8 scalar into a scalar of type `U` (a null stays null). The universal
    /// "utf8 → any" bridge.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Scalar;
    /// use yggdryl_core::io::var::Utf8Scalar;
    ///
    /// assert_eq!(Utf8Scalar::of("42").parse_to::<i32>().unwrap(), Scalar::of(42));
    /// assert!(Utf8Scalar::of("nope").parse_to::<i32>().is_err());
    /// ```
    pub fn parse_to<U>(&self) -> Result<Scalar<U>, CastError>
    where
        U: NativeType + core::str::FromStr,
    {
        match self.as_str() {
            Some(text) => text
                .trim()
                .parse::<U>()
                .map(Scalar::of)
                .map_err(|_| CastError::Parse {
                    text: text.to_string(),
                    to: U::NAME,
                }),
            None => Ok(Scalar::null()),
        }
    }
}

impl BinaryScalar {
    /// Read this binary scalar's little-endian bytes back into a scalar of type `U` (a null stays
    /// null). Errors [`WidthMismatch`](CastError::WidthMismatch) if the byte width is not `U`'s.
    /// The reverse of [`Scalar::to_binary`].
    pub fn read_to<U: NativeType>(&self) -> Result<Scalar<U>, CastError> {
        match self.value_bytes() {
            Some(bytes) if bytes.len() == U::WIDTH => Ok(Scalar::of(U::read_le(bytes))),
            Some(bytes) => Err(CastError::WidthMismatch {
                got: bytes.len(),
                need: U::WIDTH,
                to: U::NAME,
            }),
            None => Ok(Scalar::null()),
        }
    }
}
