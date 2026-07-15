//! `io::var::string` — the variable-length **UTF-8 string** kind: the [`Utf8`] marker (which
//! validates every value is valid UTF-8) and the `Utf8*` aliases + `&str` ergonomics over the
//! shared `Byte*` generics.
//!
//! DESIGN: only the `i32`-offset [`Utf8`] ships today. A `LargeUtf8` (`i64` offsets) is reserved
//! at [`DataTypeId::LargeUtf8`](crate::io::DataTypeId::LargeUtf8); adding it means giving
//! [`VarElement`] an offset-width axis (so `ByteSerie<E>` stores `Vec<E::Offset>`) — a clean
//! follow-up that reuses the one generic impl rather than forking it.

use core::str;

use super::{ByteField, ByteScalar, ByteSerie, ByteType, VarElement};
use crate::io::{DataTypeId, IoError};

/// A variable-length **UTF-8 string** element (`i32` offsets). Every value it stores is
/// validated to be valid UTF-8, so [`Utf8Scalar::as_str`] / [`Utf8Serie::get_str`] never
/// re-check and never allocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Utf8;

impl VarElement for Utf8 {
    const NAME: &'static str = "utf8";
    const TYPE_ID: DataTypeId = DataTypeId::Utf8;

    fn validate(bytes: &[u8]) -> Result<(), IoError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|error| IoError::InvalidUtf8 {
                position: error.valid_up_to(),
            })
    }

    #[cfg(feature = "arrow")]
    type Arrow = arrow_array::types::Utf8Type;
}

/// UTF-8 scalar ergonomics.
impl ByteScalar<Utf8> {
    /// A present scalar from a `&str` (always valid UTF-8, so infallible).
    pub fn of(value: &str) -> Self {
        Self::from_bytes_unchecked(value.as_bytes())
    }

    /// The value as `&str`, or `None` if null. Never allocates (the bytes are known-valid UTF-8).
    pub fn as_str(&self) -> Option<&str> {
        self.value_bytes()
            .map(|bytes| str::from_utf8(bytes).unwrap_or_default())
    }
}

/// UTF-8 column ergonomics.
impl ByteSerie<Utf8> {
    /// Appends one string (`None` is a null).
    pub fn push_str(&mut self, value: Option<&str>) {
        // A `&str` is always valid UTF-8, so this never errors.
        let _ = self.push_bytes(value.map(str::as_bytes));
    }

    /// A column from optional strings.
    pub fn from_strs(values: &[Option<&str>]) -> Self {
        let mut serie = Self::with_capacity(values.len());
        for &value in values {
            serie.push_str(value);
        }
        serie
    }

    /// Element `index` as `&str` — zero-copy — or `None` if null or out of range.
    pub fn get_str(&self, index: usize) -> Option<&str> {
        self.get_bytes(index)
            .map(|bytes| str::from_utf8(bytes).unwrap_or_default())
    }

    /// The elements as `Option<&str>`, in order.
    pub fn to_strs(&self) -> Vec<Option<&str>> {
        (0..self.len()).map(|i| self.get_str(i)).collect()
    }

    /// Overwrites element `index` with a string (`None` is a null) — the string ergonomic over
    /// [`set_bytes`](ByteSerie::set_bytes) (a length change rewrites the trailing offsets).
    /// Errors [`IndexOutOfBounds`](crate::io::IoError::IndexOutOfBounds) past the end.
    pub fn set_str(&mut self, index: usize, value: Option<&str>) -> Result<(), IoError> {
        self.set_bytes(index, value.map(str::as_bytes))
    }
}

/// The typed descriptor of the UTF-8 string type — [`ByteType<Utf8>`](ByteType).
pub type Utf8DataType = ByteType<Utf8>;
/// A named, nullable UTF-8 string column descriptor — [`ByteField<Utf8>`](ByteField).
pub type Utf8Field = ByteField<Utf8>;
/// One nullable UTF-8 string value — [`ByteScalar<Utf8>`](ByteScalar).
pub type Utf8Scalar = ByteScalar<Utf8>;
/// A nullable column of UTF-8 strings — [`ByteSerie<Utf8>`](ByteSerie).
pub type Utf8Serie = ByteSerie<Utf8>;
