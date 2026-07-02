//! The single-value container.

use core::hash::{Hash, Hasher};
use core::str;

use arrow_buffer::{ArrowNativeType, Buffer, MutableBuffer};
use yggdryl_schema::{BooleanType, PrimitiveType};

use crate::{BinaryScalarType, ScalarError, ScalarType, StringScalarType};

/// One typed value: a data type plus one element's value bytes in a
/// refcounted `arrow-buffer` [`Buffer`] (`None` = null), laid out per the
/// Arrow columnar spec.
///
/// A scalar extracted from a larger container is a zero-copy slice holding a
/// refcount on the parent buffer; a standalone scalar is the same type over a
/// fresh buffer. Equality and hashing are content-based: the data type, the
/// null flag and the value bytes.
///
/// ```
/// use yggdryl_scalar::Scalar;
/// use yggdryl_schema::{Int64Type, Utf8Type};
///
/// let count = Scalar::from_native(Int64Type, 42i64);
/// assert_eq!(count.as_native(), Some(42));
///
/// let name = Scalar::from_string(Utf8Type, "ygg");
/// assert_eq!(name.as_str(), Some("ygg"));
/// assert_eq!(Scalar::null(Utf8Type).as_str(), None);
/// ```
///
/// [`Buffer`]: arrow_buffer::Buffer
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(
        try_from = "RawScalar<T>",
        into = "RawScalar<T>",
        bound(
            serialize = "T: serde::Serialize",
            deserialize = "T: serde::de::DeserializeOwned"
        )
    )
)]
pub struct Scalar<T: ScalarType> {
    data_type: T,
    buffer: Option<Buffer>,
}

impl<T: ScalarType> Scalar<T> {
    /// Builds the scalar from its data type and optional value buffer
    /// (`None` = null), validating the bytes against the type's layout.
    pub fn from_parts(data_type: T, buffer: Option<Buffer>) -> Result<Self, ScalarError> {
        if let Some(buffer) = &buffer {
            data_type.validate_scalar_bytes(buffer.as_slice())?;
        }
        Ok(Self { data_type, buffer })
    }

    /// The null scalar of the given type.
    pub fn null(data_type: T) -> Self {
        Self {
            data_type,
            buffer: None,
        }
    }

    /// The scalar's data type.
    pub fn data_type(&self) -> &T {
        &self.data_type
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.buffer.is_none()
    }

    /// The value buffer, if the scalar is not null.
    pub fn buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    /// The value bytes, if the scalar is not null.
    pub fn value_bytes(&self) -> Option<&[u8]> {
        self.buffer.as_ref().map(Buffer::as_slice)
    }

    /// Encodes the scalar as `data type | null flag | value bytes`, the data
    /// type length-prefixed.
    pub fn to_bytes(&self) -> Vec<u8> {
        let data_type = self.data_type.to_bytes();
        let mut out = (data_type.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&data_type);
        match self.value_bytes() {
            Some(value) => {
                out.push(1);
                out.extend_from_slice(value);
            }
            None => out.push(0),
        }
        out
    }

    /// Deserializes the scalar from the encoding produced by
    /// [`to_bytes`](Scalar::to_bytes), validating fully.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
        crate::log_event!(trace, "Scalar::from_bytes len={}", bytes.len());
        let truncated = |actual: usize| ScalarError::InvalidBytes {
            message: format!(
                "truncated scalar encoding of {actual} bytes — re-encode with to_bytes"
            ),
        };
        let (len, rest) = bytes
            .split_first_chunk::<8>()
            .ok_or_else(|| truncated(bytes.len()))?;
        let len =
            usize::try_from(u64::from_le_bytes(*len)).map_err(|_| ScalarError::InvalidBytes {
                message: "length prefix does not fit this platform's usize".to_string(),
            })?;
        if rest.len() <= len {
            return Err(truncated(bytes.len()));
        }
        let (data_type, value) = rest.split_at(len);
        let data_type = T::from_bytes(data_type)?;
        match value {
            [] => Err(truncated(bytes.len())),
            [0] => Ok(Self::null(data_type)),
            [0, ..] => Err(ScalarError::InvalidBytes {
                message: format!("{} trailing bytes after a null scalar", value.len() - 1),
            }),
            [1, value @ ..] => Self::from_parts(data_type, Some(aligned_buffer(value))),
            [other, ..] => Err(ScalarError::InvalidBytes {
                message: format!("unknown null flag {other}, expected 0 or 1"),
            }),
        }
    }
}

impl<T: ScalarType + PrimitiveType> Scalar<T>
where
    T::Native: ArrowNativeType,
{
    /// Builds the scalar from a native value over a fresh buffer.
    pub fn from_native(data_type: T, value: T::Native) -> Self {
        Self {
            data_type,
            buffer: Some(Buffer::from_slice_ref(core::slice::from_ref(&value))),
        }
    }

    /// The native value, if the scalar is not null.
    pub fn as_native(&self) -> Option<T::Native> {
        // Constructors validate length and alignment, so the typed read is
        // exact and zero-copy.
        self.buffer
            .as_ref()
            .map(|buffer| buffer.typed_data::<T::Native>()[0])
    }

    /// The value as an `i64` — the native itself for [`Int64Type`]-natives,
    /// a checked conversion otherwise: widening always succeeds, and a value
    /// that does not fit returns `None` rather than truncating.
    ///
    /// [`Int64Type`]: yggdryl_schema::Int64Type
    pub fn as_i64(&self) -> Option<i64>
    where
        T::Native: TryInto<i64>,
    {
        self.as_native()?.try_into().ok()
    }

    /// The value as a `u64`, checked the same way as
    /// [`as_i64`](Scalar::as_i64) (a negative value returns `None`).
    pub fn as_u64(&self) -> Option<u64>
    where
        T::Native: TryInto<u64>,
    {
        self.as_native()?.try_into().ok()
    }

    /// The value as an `i128`, checked the same way as
    /// [`as_i64`](Scalar::as_i64) — every integer native up to
    /// [`Decimal128Type`](yggdryl_schema::Decimal128Type) widens losslessly.
    pub fn as_i128(&self) -> Option<i128>
    where
        T::Native: TryInto<i128>,
    {
        self.as_native()?.try_into().ok()
    }

    /// The value as an `f64`; offered only where the native widens without
    /// losing precision, so the read never lies.
    pub fn as_f64(&self) -> Option<f64>
    where
        T::Native: Into<f64>,
    {
        Some(self.as_native()?.into())
    }
}

impl Scalar<BooleanType> {
    /// Builds the scalar from a boolean; a detached element is one byte, the
    /// bit-packing of the Arrow spec applies to arrays.
    pub fn from_bool(value: bool) -> Self {
        Self {
            data_type: BooleanType,
            buffer: Some(Buffer::from_slice_ref([u8::from(value)])),
        }
    }

    /// The boolean value, if the scalar is not null.
    pub fn as_bool(&self) -> Option<bool> {
        self.value_bytes().map(|bytes| bytes[0] == 1)
    }
}

impl<T: StringScalarType> Scalar<T> {
    /// Builds the scalar from a string value over a fresh buffer.
    pub fn from_string(data_type: T, value: impl AsRef<str>) -> Self {
        Self {
            data_type,
            buffer: Some(Buffer::from(value.as_ref().as_bytes())),
        }
    }

    /// The string value, if the scalar is not null.
    pub fn as_str(&self) -> Option<&str> {
        // Constructors validate UTF-8, so the re-read is infallible.
        self.value_bytes()
            .map(|bytes| str::from_utf8(bytes).expect("validated UTF-8"))
    }
}

impl<T: BinaryScalarType> Scalar<T> {
    /// Builds the scalar from a byte value over a fresh buffer, validated
    /// against the type's layout (`FixedSizeBinaryType` checks the width).
    pub fn from_binary(data_type: T, value: impl AsRef<[u8]>) -> Result<Self, ScalarError> {
        Self::from_parts(data_type, Some(Buffer::from(value.as_ref())))
    }

    /// The byte value, if the scalar is not null.
    pub fn as_binary(&self) -> Option<&[u8]> {
        self.value_bytes()
    }
}

// Equality and hashing are content-based over the parts that define the
// value: the data type, the null flag and the value bytes. `Buffer`'s own
// comparisons already look at contents, but going through `value_bytes`
// keeps the two impls visibly consistent.
impl<T: ScalarType> PartialEq for Scalar<T> {
    fn eq(&self, other: &Self) -> bool {
        self.data_type == other.data_type && self.value_bytes() == other.value_bytes()
    }
}

impl<T: ScalarType> Eq for Scalar<T> {}

impl<T: ScalarType> Hash for Scalar<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data_type.hash(state);
        self.value_bytes().hash(state);
    }
}

/// Copies value bytes into a fresh 64-byte-aligned buffer so typed reads on
/// every fixed-width native stay valid.
fn aligned_buffer(bytes: &[u8]) -> Buffer {
    let mut buffer = MutableBuffer::new(bytes.len());
    buffer.extend_from_slice(bytes);
    buffer.into()
}

/// Mirror of the serialized parts, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "T: serde::Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
struct RawScalar<T: ScalarType> {
    data_type: T,
    value: Option<Vec<u8>>,
}

#[cfg(feature = "serde")]
impl<T: ScalarType> TryFrom<RawScalar<T>> for Scalar<T> {
    type Error = ScalarError;

    fn try_from(raw: RawScalar<T>) -> Result<Self, Self::Error> {
        Self::from_parts(raw.data_type, raw.value.as_deref().map(aligned_buffer))
    }
}

#[cfg(feature = "serde")]
impl<T: ScalarType> From<Scalar<T>> for RawScalar<T> {
    fn from(scalar: Scalar<T>) -> Self {
        Self {
            value: scalar.value_bytes().map(<[u8]>::to_vec),
            data_type: scalar.data_type,
        }
    }
}
