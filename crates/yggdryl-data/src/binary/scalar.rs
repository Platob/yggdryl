//! The [`BinaryScalar`] scalar of the [`Binary`](super::Binary) data type.

use super::Binary;
use crate::{DataError, RawScalar, Scalar};
use yggdryl_core::ByteBuffer;

/// A single, possibly-null `binary` value: a byte sequence held as a core
/// [`ByteBuffer`], so the value doubles as a positioned-IO resource.
///
/// The *scalar accessors* leverage the core IO layer: [`value`](RawScalar::value)
/// / [`as_bytes`](RawScalar::as_bytes) borrow the bytes directly (never copying),
/// [`io`](BinaryScalar::io) borrows the [`ByteBuffer`] for positioned
/// [`RawIOBase`](yggdryl_core::RawIOBase) reads, and
/// [`into_io`](BinaryScalar::into_io) moves it out to wrap in the core
/// [`RawIOCursor`](yggdryl_core::RawIOCursor) / [`RawIOSlice`](yggdryl_core::RawIOSlice)
/// adapters for streaming or windowed reads. [`as_str`](RawScalar::as_str) borrows
/// the bytes as `str` when they are valid UTF-8. Crossing the Arrow boundary
/// copies the bytes once between the Arrow buffer and the core resource — a
/// scalar holds one value, so the copy is a single sequence, never a column.
///
/// ```
/// use yggdryl_core::{RawIOBase, RawIOCursor, Whence};
/// use yggdryl_data::{BinaryScalar, RawDataType, RawScalar};
///
/// let blob = BinaryScalar::new(vec![1, 2, 3]);
/// assert!(!blob.is_null());
/// assert_eq!(blob.value(), Some(&[1, 2, 3][..]));
/// assert_eq!(blob.as_bytes().unwrap(), &[1, 2, 3][..]); // borrowed, never copied
/// assert_eq!(blob.data_type().name(), "binary");
///
/// // The value is a core IO resource: positioned reads, no copy.
/// let io = blob.io().unwrap();
/// assert_eq!(io.byte_size(), 3);
/// assert_eq!(io.pread_byte_one(1, Whence::Start)?, 2);
///
/// // Or move it into the core cursor adapter for streaming reads.
/// let cursor = RawIOCursor::new(blob.clone().into_io().unwrap());
/// assert_eq!(cursor.pread_byte_array(0, Whence::Start, 2)?, vec![1, 2]);
///
/// // UTF-8 bytes read back as a borrowed str; anything else is an actionable error.
/// assert_eq!(BinaryScalar::new(b"hi".to_vec()).as_str().unwrap(), "hi");
/// assert!(BinaryScalar::new(vec![0xFF]).as_str().is_err()); // not valid UTF-8
///
/// // The Arrow round trip is exact.
/// let arrow = blob.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(BinaryScalar::from_arrow(arrow.as_ref()).unwrap(), blob);
///
/// assert!(BinaryScalar::null().is_null());
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinaryScalar {
    data_type: Binary,
    value: Option<ByteBuffer>,
}

impl BinaryScalar {
    /// A `binary` scalar holding `value` (empty bytes are the empty value, not
    /// null).
    pub fn new(value: Vec<u8>) -> Self {
        Self {
            data_type: Binary,
            value: Some(ByteBuffer::from_bytes(value)),
        }
    }

    /// A null `binary` scalar.
    pub fn null() -> Self {
        Self {
            data_type: Binary,
            value: None,
        }
    }

    /// The value as the core positioned-IO resource, borrowed — every
    /// [`RawIOBase`](yggdryl_core::RawIOBase) read works on the borrow — or `None`
    /// when null.
    pub fn io(&self) -> Option<&ByteBuffer> {
        self.value.as_ref()
    }

    /// Consume the scalar, returning the value as the core IO resource (or `None`
    /// when null) — ready to wrap in a [`RawIOCursor`](yggdryl_core::RawIOCursor)
    /// for streaming reads or a [`RawIOSlice`](yggdryl_core::RawIOSlice) for a
    /// bounded byte window.
    pub fn into_io(self) -> Option<ByteBuffer> {
        self.value
    }
}

impl From<Vec<u8>> for BinaryScalar {
    /// A `binary` scalar holding `value`.
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl From<&[u8]> for BinaryScalar {
    /// A `binary` scalar holding a copy of `value`.
    fn from(value: &[u8]) -> Self {
        Self::new(value.to_vec())
    }
}

impl From<Option<Vec<u8>>> for BinaryScalar {
    /// A `binary` scalar holding `value`, or the null scalar for `None`.
    fn from(value: Option<Vec<u8>>) -> Self {
        match value {
            Some(value) => Self::new(value),
            None => Self::null(),
        }
    }
}

impl From<ByteBuffer> for BinaryScalar {
    /// A `binary` scalar taking over an existing core IO resource, moved — the
    /// inverse of [`into_io`](BinaryScalar::into_io).
    fn from(value: ByteBuffer) -> Self {
        Self {
            data_type: Binary,
            value: Some(value),
        }
    }
}

impl RawScalar<Binary> for BinaryScalar {
    type Value = [u8];

    fn data_type(&self) -> &Binary {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Option<&[u8]> {
        self.value.as_ref().map(ByteBuffer::as_bytes)
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        match &self.value {
            Some(value) => std::sync::Arc::new(arrow_array::BinaryArray::from_iter_values([
                value.as_bytes()
            ])),
            // Arrow arrays are immutable, so every null scalar shares one cached
            // one-null array; a clone is a reference-count bump.
            None => {
                static NULL: std::sync::OnceLock<arrow_array::ArrayRef> =
                    std::sync::OnceLock::new();
                NULL.get_or_init(|| std::sync::Arc::new(arrow_array::BinaryArray::new_null(1)))
                    .clone()
            }
        }
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::BinaryArray>()
            .ok_or_else(|| DataError::IncompatibleArrowType {
                expected: "Binary".to_string(),
                got: arrow_array::Array::data_type(array).to_string(),
            })?;
        Ok(if arrow_array::Array::is_null(array, 0) {
            Self::null()
        } else {
            Self::new(array.value(0).to_vec())
        })
    }

    // The native type answers directly, borrowed; UTF-8 bytes convert to str.
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        self.value
            .as_ref()
            .map(ByteBuffer::as_bytes)
            .ok_or(DataError::NullValue)
    }

    fn as_str(&self) -> Result<&str, DataError> {
        let value = self.value.as_ref().ok_or(DataError::NullValue)?;
        std::str::from_utf8(value.as_bytes()).map_err(|_| DataError::InexactConversion {
            value: format!(
                "{} byte(s) of non-UTF-8 data (as_bytes() reads them)",
                value.as_bytes().len()
            ),
            target: "str",
        })
    }
}

impl Scalar<[u8]> for BinaryScalar {
    type Type = Binary;
}
