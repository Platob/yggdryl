//! The [`FromScalar`] trait: native Rust targets readable out of any scalar.

use crate::Scalar;
use yggdryl_core::{ByteBuffer, ByteBufferSlice, RawIOBase};
use yggdryl_dtype::{DataError, DataType};

/// A native Rust type readable out of any scalar — the trait behind the generic
/// native accessors such as [`Serie::get_at`](crate::Serie::get_at).
///
/// The type parameter picks the target and the read redirects to the matching
/// `as_*` accessor, inheriting its exact-or-error contract: numbers convert
/// exactly or error, `bool` and `String` come from `as_bool` / `as_str`, and the
/// byte targets `Vec<u8>` / [`ByteBufferSlice`] come from `as_bytes` (the slice
/// wraps the bytes in a full-window `yggdryl-core` positioned-IO resource; the
/// borrowed bytes are copied once into the owned value).
///
/// ```
/// use yggdryl_scalar::{FromScalar, Int64Scalar, Scalar};
///
/// let answer = Int64Scalar::new(42);
/// assert_eq!(i32::from_scalar(&answer).unwrap(), 42);
/// assert_eq!(f64::from_scalar(&answer).unwrap(), 42.0);
/// assert!(String::from_scalar(&answer).is_err()); // an int64 has no str form
/// ```
pub trait FromScalar: Sized {
    /// Read this type out of `scalar` under the shared `as_*` contract.
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError>;
}

impl FromScalar for i8 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_i8()
    }
}

impl FromScalar for i16 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_i16()
    }
}

impl FromScalar for i32 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_i32()
    }
}

impl FromScalar for i64 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_i64()
    }
}

impl FromScalar for u8 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_u8()
    }
}

impl FromScalar for u16 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_u16()
    }
}

impl FromScalar for u32 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_u32()
    }
}

impl FromScalar for u64 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_u64()
    }
}

impl FromScalar for f32 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_f32()
    }
}

impl FromScalar for f64 {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_f64()
    }
}

impl FromScalar for bool {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        scalar.as_bool()
    }
}

impl FromScalar for String {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        Ok(scalar.as_str(None)?.into_owned())
    }
}

impl FromScalar for Vec<u8> {
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        Ok(scalar.as_bytes()?.to_vec())
    }
}

impl FromScalar for ByteBufferSlice {
    /// The bytes as a full-window `yggdryl-core` positioned-IO slice: the one
    /// copy is the borrow-to-owned move; every read after it is zero-copy.
    fn from_scalar<D: DataType, S: Scalar<D>>(scalar: &S) -> Result<Self, DataError> {
        let bytes = scalar.as_bytes()?;
        let length = bytes.len();
        Ok(ByteBuffer::from_bytes(bytes.to_vec()).slice(0, length))
    }
}
