//! [`CastConverter<S, T>`] — numeric conversion between primitive types.

use core::marker::PhantomData;

use yggdryl_buffer::IoPrimitive;

use crate::{ConvertError, Converter, TypedConverter};

/// A C-style numeric cast from `S` to `T`, implemented for every ordered pair of the
/// ten native primitives (`i8` … `u64`, `f32`, `f64`).
///
/// Conversion uses Rust's `as` operator, so it is **total and allocation-free** (the
/// fastest path): widening is exact, and narrowing / float↔int follows the documented
/// `as` semantics (integer truncation, saturating float-to-int, nearest-or-even
/// int-to-float). Pair the two directions for a lossless round-trip only when `T` can
/// hold every `S` value.
///
/// ```
/// use yggdryl_converter::{CastConverter, TypedConverter};
///
/// let to_f64 = CastConverter::<i32, f64>::new();
/// assert_eq!(to_f64.encode(3_i32).unwrap(), 3.0_f64);
///
/// let narrow = CastConverter::<i64, u8>::new();
/// assert_eq!(narrow.encode(258_i64).unwrap(), 2_u8); // 258 & 0xFF
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct CastConverter<S, T> {
    _marker: PhantomData<(S, T)>,
}

impl<S, T> CastConverter<S, T> {
    /// Creates the numeric cast converter.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Casts one primitive value to another via `as` — the internal, per-pair machinery
/// behind [`CastConverter`]. Implemented for every ordered pair of native primitives.
pub(crate) trait NumCast<T> {
    /// The `self as T` cast.
    fn num_cast(self) -> T;
}

/// Reads `bytes` as packed little-endian `S` values, casts each to `T`, and returns
/// the packed little-endian `T` bytes.
fn cast_bytes<S, T>(bytes: &[u8]) -> Result<Vec<u8>, ConvertError>
where
    S: IoPrimitive + NumCast<T>,
    T: IoPrimitive,
{
    if !bytes.len().is_multiple_of(S::WIDTH) {
        return Err(ConvertError::InvalidByteLength {
            len: bytes.len(),
            width: S::WIDTH,
        });
    }
    let count = bytes.len() / S::WIDTH;
    let mut out = Vec::with_capacity(count * T::WIDTH);
    for chunk in bytes.chunks_exact(S::WIDTH) {
        S::from_le_slice(chunk).num_cast().write_le(&mut out);
    }
    Ok(out)
}

impl<S, T> Converter for CastConverter<S, T>
where
    S: IoPrimitive + NumCast<T>,
    T: IoPrimitive + NumCast<S>,
{
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        cast_bytes::<S, T>(bytes)
    }

    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        cast_bytes::<T, S>(bytes)
    }
}

impl<S, T> TypedConverter<S, T> for CastConverter<S, T>
where
    S: IoPrimitive + NumCast<T>,
    T: IoPrimitive + NumCast<S>,
{
    fn encode(&self, value: S) -> Result<T, ConvertError> {
        Ok(value.num_cast())
    }

    fn decode(&self, value: T) -> Result<S, ConvertError> {
        Ok(value.num_cast())
    }
}

/// Stamps out the full ordered-pair matrix of [`NumCast`] impls. The type list is
/// captured once as a `tt` group and threaded into each source row, so it can be
/// repeated at two nesting depths without a metavariable clash.
macro_rules! num_cast_matrix {
    ($($ty:ty),+ $(,)?) => {
        num_cast_matrix!(@rows [$($ty),+] $($ty),+);
    };
    (@rows $targets:tt $($s:ty),+) => {
        $( num_cast_matrix!(@row $s $targets); )+
    };
    (@row $s:ty [$($t:ty),+]) => {
        $(
            impl NumCast<$t> for $s {
                #[inline]
                #[allow(clippy::unnecessary_cast)] // the S == T diagonal is a no-op cast
                fn num_cast(self) -> $t {
                    self as $t
                }
            }
        )+
    };
}

num_cast_matrix!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);
