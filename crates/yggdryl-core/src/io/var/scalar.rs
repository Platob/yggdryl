//! [`ByteScalar`] — a single, nullable variable-length value (`Utf8Scalar = ByteScalar<Utf8>`,
//! `BinaryScalar = ByteScalar<Binary>`).

use core::marker::PhantomData;

use super::{ByteField, ByteSerie, ByteType, VarElement};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, FieldType, IOCursor, IoError, ScalarType};

/// The **variable-length scalar** sub-trait — the sibling of
/// [`FixedScalar`](crate::io::fixed::FixedScalar) for a value whose bytes are not a fixed width
/// (a string, a blob). A concrete var scalar returns its bytes here.
pub trait VarScalar: ScalarType {
    /// The value's bytes, or `None` if null.
    fn value_bytes(&self) -> Option<&[u8]>;
}

/// A single variable-length value of kind `E`, possibly **null** — its bytes are validated for
/// the kind (a `Utf8` value is always valid UTF-8). Its wire form is one validity byte, then
/// (if present) a `u64` length and the bytes, read/written through the [`IOCursor`] abstraction.
///
/// ```
/// use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar};
/// use yggdryl_core::io::{Bytes, IOCursor};
///
/// let s = Utf8Scalar::of("héllo");
/// assert_eq!(s.as_str(), Some("héllo"));
///
/// let mut sink = Bytes::new();
/// s.write_to(&mut sink).unwrap();
/// sink.rewind();
/// assert_eq!(Utf8Scalar::read_from(&mut sink).unwrap(), s);
///
/// assert!(BinaryScalar::of(&[0xff, 0x00]).value_bytes().is_some());
/// ```
pub struct ByteScalar<E: VarElement> {
    value: Option<Box<[u8]>>,
    /// The value's own [`ByteField`] descriptor — its name, declared nullability, and metadata.
    /// Excluded from value identity and the byte codec (only the bytes participate).
    field: ByteField<E>,
    _element: PhantomData<E>,
}

impl<E: VarElement> ByteScalar<E> {
    /// A present scalar from raw `bytes`, validated for the kind (`InvalidUtf8` for bad UTF-8).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        E::validate(bytes)?;
        Ok(Self::from_bytes_unchecked(bytes))
    }

    /// A present scalar from raw `bytes` **without** validating them — the kind sub-modules use
    /// this for inputs known-valid by construction (e.g. a `&str` is always valid UTF-8).
    pub(crate) fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        Self {
            value: Some(bytes.into()),
            field: ByteField::new("", false),
            _element: PhantomData,
        }
    }

    /// A scalar from optional raw bytes (`None` is null), validated when present.
    pub fn new(value: Option<&[u8]>) -> Result<Self, IoError> {
        match value {
            Some(bytes) => Self::from_bytes(bytes),
            None => Ok(Self::null()),
        }
    }

    /// The null scalar.
    pub fn null() -> Self {
        Self {
            value: None,
            field: ByteField::new("", false),
            _element: PhantomData,
        }
    }

    /// The raw bytes, or `None` if null.
    pub fn value_bytes(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }

    /// The typed data type of this scalar.
    pub fn data_type(&self) -> ByteType<E> {
        ByteType::new()
    }

    field_accessors!();

    /// The erased [`AnyField`] this scalar contributes — its **held field** (name + metadata) with
    /// **effective** nullability `self.nullable() || self.is_null()`.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.is_null());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](ByteScalar::field) but **consumes** the scalar.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.is_null();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// A [`ByteField`] naming a column of this scalar's type with explicit nullability.
    pub fn typed_field(&self, name: &str, nullable: bool) -> ByteField<E> {
        ByteField::new(name, nullable)
    }

    /// This scalar **broadcast to a length-1 [`ByteSerie`]** — the inverse of
    /// [`ByteSerie::as_scalar`](ByteSerie::as_scalar). Mirrors the fixed family's
    /// [`Scalar::to_serie`](crate::io::fixed::Scalar::to_serie); it is fallible only because a var
    /// column validates its bytes for the kind on build (a present scalar's bytes are already valid,
    /// so it never fails in practice).
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Scalar;
    ///
    /// let col = Utf8Scalar::of("hi").to_serie().unwrap();
    /// assert_eq!(col.len(), 1);
    /// assert_eq!(col.get_str(0), Some("hi"));
    /// ```
    pub fn to_serie(&self) -> Result<ByteSerie<E>, IoError> {
        ByteSerie::from_scalar(self.clone())
    }

    /// Writes this scalar to `sink`: a validity byte, then (if present) a `u64` length and the
    /// bytes.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        match &self.value {
            Some(bytes) => {
                sink.write_all(&[1])?;
                sink.write_all(&(bytes.len() as u64).to_le_bytes())?;
                sink.write_all(bytes)
            }
            None => sink.write_all(&[0]),
        }
    }

    /// Reads a scalar written by [`write_to`](ByteScalar::write_to), validating a present value.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let mut validity = [0u8; 1];
        source.read_exact(&mut validity)?;
        if validity[0] == 0 {
            return Ok(Self::null());
        }
        // Bounded read: `len` is an untrusted frame value, so grow only as bytes arrive rather than
        // allocating the declared size up front (a hostile length errors cleanly, never aborts).
        let len = source.read_u64()? as usize;
        let bytes = source.read_exact_vec(len)?;
        Self::from_bytes(&bytes)
    }

    /// This scalar's canonical bytes — the same validity-byte-then-`[len][bytes]` frame
    /// [`write_to`](ByteScalar::write_to) produces, returned as an owned `Vec`. The exact inverse
    /// of [`deserialize_bytes`](ByteScalar::deserialize_bytes), and the codec the Python / Node
    /// bindings expose (`serialize_bytes` / `serializeBytes`).
    ///
    /// ```
    /// use yggdryl_core::io::var::Utf8Scalar;
    ///
    /// let value = Utf8Scalar::of("héllo");
    /// assert_eq!(Utf8Scalar::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a scalar from the bytes produced by
    /// [`serialize_bytes`](ByteScalar::serialize_bytes), validating a present value for the kind
    /// (`InvalidUtf8` for bad UTF-8) and erroring on a truncated frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

impl<E: VarElement> ScalarType for ByteScalar<E> {
    type Data = ByteType<E>;

    fn data_type(&self) -> ByteType<E> {
        ByteType::new()
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

impl<E: VarElement> VarScalar for ByteScalar<E> {
    fn value_bytes(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }
}

// The kind-specific ergonomics (`Utf8Scalar::of`/`as_str`, `BinaryScalar::of`) live with their
// markers in the `string` / `binary` sub-modules.

impl<E: VarElement> Clone for ByteScalar<E> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            field: self.field.clone(),
            _element: PhantomData,
        }
    }
}

impl<E: VarElement> PartialEq for ByteScalar<E> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<E: VarElement> Eq for ByteScalar<E> {}

impl<E: VarElement> core::hash::Hash for ByteScalar<E> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<E: VarElement> core::fmt::Debug for ByteScalar<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ByteScalar")
            .field("type", &E::NAME)
            .field("null", &self.is_null())
            .finish()
    }
}
