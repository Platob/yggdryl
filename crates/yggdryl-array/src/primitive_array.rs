//! The fixed-width array container.

use core::hash::{Hash, Hasher};
use core::mem::size_of;

use arrow_buffer::{ArrowNativeType, Buffer, MutableBuffer, NullBuffer, ScalarBuffer};
use yggdryl_scalar::{Scalar, ScalarType};
use yggdryl_schema::PrimitiveType;

use crate::{Array, ArrayError};

/// A typed column of fixed-width values: a [`PrimitiveType`] plus its
/// natives in a refcounted `arrow-buffer` [`ScalarBuffer`] and an optional
/// [`NullBuffer`] validity bitmap, laid out exactly per the Arrow columnar
/// spec.
///
/// Slicing and per-element [`scalar`](PrimitiveArray::scalar) extraction are
/// zero-copy buffer slices holding a refcount on the parent buffer. Equality
/// and hashing are content-based over the valid elements' bytes, so an array
/// always equals itself (floats compare bit-wise) and null slots never leak
/// their padding.
///
/// ```
/// use yggdryl_array::{Array, PrimitiveArray};
/// use yggdryl_schema::Int32;
///
/// let column = PrimitiveArray::from_options(Int32, vec![Some(1), None, Some(3)]);
/// assert_eq!(column.value(2), Some(3));
///
/// let tail = column.slice(1, 2).unwrap(); // zero-copy
/// assert_eq!(tail.len(), 2);
/// assert_eq!(tail.value(0), None);
/// ```
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(
        try_from = "RawPrimitiveArray<T>",
        into = "RawPrimitiveArray<T>",
        bound(
            serialize = "T: serde::Serialize",
            deserialize = "T: serde::de::DeserializeOwned"
        )
    )
)]
pub struct PrimitiveArray<T: PrimitiveType<Native: ArrowNativeType>> {
    data_type: T,
    values: ScalarBuffer<T::Native>,
    validity: Option<NullBuffer>,
}

impl<T: PrimitiveType<Native: ArrowNativeType>> PrimitiveArray<T> {
    /// Builds the array from its parts, validating that the validity bitmap
    /// (if any) covers exactly the values.
    pub fn from_parts(
        data_type: T,
        values: ScalarBuffer<T::Native>,
        validity: Option<NullBuffer>,
    ) -> Result<Self, ArrayError> {
        if let Some(validity) = &validity {
            if validity.len() != values.len() {
                return Err(ArrayError::LengthMismatch {
                    values: values.len(),
                    validity: validity.len(),
                });
            }
        }
        Ok(Self {
            data_type,
            values,
            validity,
        })
    }

    /// Builds an all-valid array from native values over a fresh buffer.
    pub fn from_native(data_type: T, values: Vec<T::Native>) -> Self {
        Self {
            data_type,
            values: ScalarBuffer::from(values),
            validity: None,
        }
    }

    /// Builds the array from optional natives; `None`s become nulls (stored
    /// as zeroed slots, so the encoding is canonical), and an array with no
    /// nulls drops the bitmap entirely.
    pub fn from_options(data_type: T, values: Vec<Option<T::Native>>) -> Self {
        if values.iter().all(Option::is_some) {
            return Self::from_native(data_type, values.into_iter().flatten().collect());
        }
        let validity: NullBuffer = values.iter().map(Option::is_some).collect();
        let values: Vec<T::Native> = values
            .into_iter()
            .map(|value| value.unwrap_or_else(|| T::Native::usize_as(0)))
            .collect();
        Self {
            data_type,
            values: ScalarBuffer::from(values),
            validity: Some(validity),
        }
    }

    /// The native values, including the zeroed slots behind nulls.
    pub fn values(&self) -> &ScalarBuffer<T::Native> {
        &self.values
    }

    /// The native value at `index`; `None` when null or out of bounds.
    pub fn value(&self, index: usize) -> Option<T::Native> {
        match self.is_valid(index)? {
            true => Some(self.values[index]),
            false => None,
        }
    }

    /// Returns the `length` elements starting at `offset` as a zero-copy
    /// view: the slice holds a refcount on this array's buffers.
    pub fn slice(&self, offset: usize, length: usize) -> Result<Self, ArrayError> {
        if offset
            .checked_add(length)
            .is_none_or(|end| end > self.len())
        {
            return Err(ArrayError::SliceOutOfBounds {
                offset,
                length,
                len: self.len(),
            });
        }
        crate::log_event!(
            trace,
            "PrimitiveArray::slice offset={offset} length={length}"
        );
        Ok(Self {
            data_type: self.data_type.clone(),
            values: self.values.slice(offset, length),
            validity: self
                .validity
                .as_ref()
                .map(|validity| validity.slice(offset, length)),
        })
    }

    /// The element at `index` as a [`Scalar`] — a zero-copy slice of the
    /// values buffer; `None` when out of bounds.
    pub fn scalar(&self, index: usize) -> Option<Scalar<T>>
    where
        T: ScalarType,
    {
        match self.is_valid(index)? {
            false => Some(Scalar::null(self.data_type.clone())),
            true => {
                let size = size_of::<T::Native>();
                let buffer = self.values.inner().slice_with_length(index * size, size);
                // Sliced on the element grid of an element-aligned buffer, so
                // construction never fails.
                Scalar::from_parts(self.data_type.clone(), Some(buffer)).ok()
            }
        }
    }

    /// Encodes the array as `data type | length | validity | values`, the
    /// data type length-prefixed and the validity bit-packed behind a flag.
    pub fn to_bytes(&self) -> Vec<u8> {
        let data_type = self.data_type.to_bytes();
        let mut out = (data_type.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&data_type);
        out.extend_from_slice(&(self.len() as u64).to_le_bytes());
        match self.packed_validity() {
            Some(bits) => {
                out.push(1);
                out.extend_from_slice(&bits);
            }
            None => out.push(0),
        }
        out.extend_from_slice(self.values.inner().as_slice());
        out
    }

    /// Deserializes the array from the encoding produced by
    /// [`to_bytes`](PrimitiveArray::to_bytes), validating fully.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArrayError> {
        crate::log_event!(trace, "PrimitiveArray::from_bytes len={}", bytes.len());
        let truncated = || ArrayError::InvalidBytes {
            message: "truncated array encoding — re-encode with to_bytes".to_string(),
        };
        let take_u64 = |bytes: &mut &[u8]| -> Result<usize, ArrayError> {
            let (value, rest) = bytes.split_first_chunk::<8>().ok_or_else(truncated)?;
            *bytes = rest;
            usize::try_from(u64::from_le_bytes(*value)).map_err(|_| ArrayError::InvalidBytes {
                message: "length prefix does not fit this platform's usize".to_string(),
            })
        };

        let mut rest = bytes;
        let data_type_len = take_u64(&mut rest)?;
        if rest.len() < data_type_len {
            return Err(truncated());
        }
        let (data_type, mut rest) = rest.split_at(data_type_len);
        let data_type = T::from_bytes(data_type)?;
        let len = take_u64(&mut rest)?;
        let (flag, mut rest) = rest.split_first().ok_or_else(truncated)?;
        let validity = match flag {
            0 => None,
            1 => {
                let bits_len = len.div_ceil(8);
                if rest.len() < bits_len {
                    return Err(truncated());
                }
                let (bits, values) = rest.split_at(bits_len);
                rest = values;
                Some(NullBuffer::new(arrow_buffer::BooleanBuffer::new(
                    Buffer::from(bits.to_vec()),
                    0,
                    len,
                )))
            }
            other => {
                return Err(ArrayError::InvalidBytes {
                    message: format!("unknown validity flag {other}, expected 0 or 1"),
                })
            }
        };
        Self::from_raw_parts(data_type, len, rest, validity)
    }

    /// The validity bitmap bit-packed into whole bytes, `None` when every
    /// element is valid.
    fn packed_validity(&self) -> Option<Vec<u8>> {
        let validity = self.validity.as_ref()?;
        let mut bits = vec![0u8; validity.len().div_ceil(8)];
        for (index, valid) in validity.iter().enumerate() {
            if valid {
                bits[index / 8] |= 1 << (index % 8);
            }
        }
        Some(bits)
    }

    /// Rebuilds the array from decoded parts, copying the value bytes into a
    /// fresh element-aligned buffer and validating every length.
    fn from_raw_parts(
        data_type: T,
        len: usize,
        values: &[u8],
        validity: Option<NullBuffer>,
    ) -> Result<Self, ArrayError> {
        let expected =
            len.checked_mul(size_of::<T::Native>())
                .ok_or_else(|| ArrayError::InvalidBytes {
                    message: "array length overflows the value buffer size".to_string(),
                })?;
        if values.len() != expected {
            return Err(ArrayError::InvalidByteLength {
                expected,
                actual: values.len(),
            });
        }
        let mut buffer = MutableBuffer::new(values.len());
        buffer.extend_from_slice(values);
        Self::from_parts(
            data_type,
            ScalarBuffer::new(buffer.into(), 0, len),
            validity,
        )
    }

    /// The raw bytes of the element at `index`; padding behind nulls stays
    /// hidden from equality and hashing.
    fn element_bytes(&self, index: usize) -> &[u8] {
        let size = size_of::<T::Native>();
        &self.values.inner().as_slice()[index * size..(index + 1) * size]
    }
}

impl<T: PrimitiveType<Native: ArrowNativeType>> Array for PrimitiveArray<T> {
    type DataType = T;

    fn data_type(&self) -> &T {
        &self.data_type
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn validity(&self) -> Option<&NullBuffer> {
        self.validity.as_ref()
    }
}

// Equality and hashing are content-based per element: the null pattern plus
// the valid elements' bytes. Comparing bytes rather than natives keeps the
// `Eq` contract for floats (bit-wise, so NaN slots compare equal to
// themselves) and ignores whatever padding sits behind nulls.
impl<T: PrimitiveType<Native: ArrowNativeType>> PartialEq for PrimitiveArray<T> {
    fn eq(&self, other: &Self) -> bool {
        self.data_type == other.data_type
            && self.len() == other.len()
            && (0..self.len()).all(
                |index| match (self.is_valid(index), other.is_valid(index)) {
                    (Some(true), Some(true)) => {
                        self.element_bytes(index) == other.element_bytes(index)
                    }
                    (Some(false), Some(false)) => true,
                    _ => false,
                },
            )
    }
}

impl<T: PrimitiveType<Native: ArrowNativeType>> Eq for PrimitiveArray<T> {}

impl<T: PrimitiveType<Native: ArrowNativeType>> Hash for PrimitiveArray<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data_type.hash(state);
        self.len().hash(state);
        for index in 0..self.len() {
            match self.is_valid(index) {
                Some(true) => {
                    true.hash(state);
                    self.element_bytes(index).hash(state);
                }
                _ => false.hash(state),
            }
        }
    }
}

/// Mirror of the serialized parts, deserialized first so `try_from`
/// re-validates on the way in. The validity is bit-packed, matching
/// [`PrimitiveArray::to_bytes`].
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(bound(
    serialize = "T: serde::Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
struct RawPrimitiveArray<T: PrimitiveType<Native: ArrowNativeType>> {
    data_type: T,
    len: u64,
    values: Vec<u8>,
    validity: Option<Vec<u8>>,
}

#[cfg(feature = "serde")]
impl<T: PrimitiveType<Native: ArrowNativeType>> TryFrom<RawPrimitiveArray<T>>
    for PrimitiveArray<T>
{
    type Error = ArrayError;

    fn try_from(raw: RawPrimitiveArray<T>) -> Result<Self, Self::Error> {
        let len = usize::try_from(raw.len).map_err(|_| ArrayError::InvalidBytes {
            message: "array length does not fit this platform's usize".to_string(),
        })?;
        let validity = match raw.validity {
            None => None,
            Some(bits) => {
                if bits.len() != len.div_ceil(8) {
                    return Err(ArrayError::InvalidByteLength {
                        expected: len.div_ceil(8),
                        actual: bits.len(),
                    });
                }
                Some(NullBuffer::new(arrow_buffer::BooleanBuffer::new(
                    Buffer::from(bits),
                    0,
                    len,
                )))
            }
        };
        Self::from_raw_parts(raw.data_type, len, &raw.values, validity)
    }
}

#[cfg(feature = "serde")]
impl<T: PrimitiveType<Native: ArrowNativeType>> From<PrimitiveArray<T>> for RawPrimitiveArray<T> {
    fn from(array: PrimitiveArray<T>) -> Self {
        Self {
            len: array.len() as u64,
            values: array.values.inner().as_slice().to_vec(),
            validity: array.packed_validity(),
            data_type: array.data_type,
        }
    }
}
