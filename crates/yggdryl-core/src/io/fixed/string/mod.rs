//! `io::fixed::string` — the **fixed-size UTF-8** type ([`FixedUtf8`]): every value is exactly
//! `N` bytes *and* valid UTF-8. The `FixedUtf8*` aliases + `&str` ergonomics over the shared
//! [`FixedSize*`](crate::io::fixed::FixedSizeType) generics.
//!
//! DESIGN: Arrow has no fixed-size UTF-8 type, so
//! [`to_arrow`](crate::io::DataType::to_arrow) maps this to `FixedSizeBinary(N)` — the closest
//! representation, losing the UTF-8 tag (the bytes still round-trip; only the schema tag is
//! coarser).

use core::str;

use crate::io::fixed::{
    FixedElement, FixedSizeField, FixedSizeScalar, FixedSizeSerie, FixedSizeType,
};
use crate::io::{DataTypeId, IoError};

/// A fixed-size **UTF-8 string** element — each value is validated to be valid UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedUtf8;

impl FixedElement for FixedUtf8 {
    const NAME: &'static str = "fixed_utf8";
    const TYPE_ID: DataTypeId = DataTypeId::FixedUtf8;

    fn validate(bytes: &[u8]) -> Result<(), IoError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|error| IoError::InvalidUtf8 {
                position: error.valid_up_to(),
            })
    }
}

/// Fixed-size-UTF-8 scalar ergonomics.
impl FixedSizeScalar<FixedUtf8> {
    /// A present scalar from a `&str` (always valid UTF-8); its width is `value.len()` bytes.
    pub fn of(value: &str) -> Self {
        Self::from_bytes_unchecked(value.as_bytes())
    }

    /// The value as `&str`, or `None` if null. Never allocates (known-valid UTF-8).
    pub fn as_str(&self) -> Option<&str> {
        self.value_bytes()
            .map(|bytes| str::from_utf8(bytes).unwrap_or_default())
    }
}

/// Fixed-size-UTF-8 column ergonomics.
impl FixedSizeSerie<FixedUtf8> {
    /// Element `index` as `&str` — zero-copy — or `None` if null or out of range.
    pub fn get_str(&self, index: usize) -> Option<&str> {
        self.get_bytes(index)
            .map(|bytes| str::from_utf8(bytes).unwrap_or_default())
    }
}

/// The descriptor of a fixed-size UTF-8 type — [`FixedSizeType<FixedUtf8>`](FixedSizeType).
pub type FixedUtf8Type = FixedSizeType<FixedUtf8>;
/// A named, nullable fixed-size UTF-8 column descriptor.
pub type FixedUtf8Field = FixedSizeField<FixedUtf8>;
/// One nullable fixed-size UTF-8 value.
pub type FixedUtf8Scalar = FixedSizeScalar<FixedUtf8>;
/// A nullable column of fixed-size UTF-8 values.
pub type FixedUtf8Serie = FixedSizeSerie<FixedUtf8>;
