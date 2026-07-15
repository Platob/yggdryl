//! `io::fixed::binary` — the **fixed-size opaque binary** type ([`FixedBinary`]): every value is
//! exactly `N` bytes (Arrow `FixedSizeBinary(N)`). The `FixedBinary*` aliases + ergonomics over
//! the shared [`FixedSize*`](crate::io::fixed::FixedSizeType) generics.

use crate::io::fixed::{
    FixedElement, FixedSizeField, FixedSizeScalar, FixedSizeSerie, FixedSizeType,
};
use crate::io::{DataTypeId, IoError};

/// A fixed-size **opaque binary** element — any bytes are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedBinary;

impl FixedElement for FixedBinary {
    const NAME: &'static str = "fixed_binary";
    const TYPE_ID: DataTypeId = DataTypeId::FixedBinary;

    fn validate(_bytes: &[u8]) -> Result<(), IoError> {
        Ok(())
    }
}

/// Fixed-size-binary scalar ergonomics.
impl FixedSizeScalar<FixedBinary> {
    /// A present scalar from raw bytes (any bytes are valid, so infallible); its width is
    /// `value.len()`.
    pub fn of(value: &[u8]) -> Self {
        Self::from_bytes_unchecked(value)
    }
}

/// The descriptor of a fixed-size binary type — [`FixedSizeType<FixedBinary>`](FixedSizeType).
pub type FixedBinaryType = FixedSizeType<FixedBinary>;
/// A named, nullable fixed-size binary column descriptor.
pub type FixedBinaryField = FixedSizeField<FixedBinary>;
/// One nullable fixed-size binary value.
pub type FixedBinaryScalar = FixedSizeScalar<FixedBinary>;
/// A nullable column of fixed-size binary values.
pub type FixedBinarySerie = FixedSizeSerie<FixedBinary>;
