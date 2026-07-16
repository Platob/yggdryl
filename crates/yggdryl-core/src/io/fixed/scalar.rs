//! [`Scalar`] — a single, nullable value of a fixed-width `T` — and the [`FixedScalar`]
//! sub-trait of the root [`ScalarType`](crate::io::ScalarType).

use super::{Field, NativeType, PrimitiveType, Serie, TypedField};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, IOCursor, IoError, ScalarType};

/// The largest fixed-width primitive is 32 bytes (`u256`/`i256`); a stack scratch of this size
/// (de)serializes one value with no allocation. Every [`NativeType`]'s `WIDTH` is guarded at
/// compile time to fit this bound.
const MAX_WIDTH: usize = 32;

/// The **fixed-width scalar** sub-trait — a [`ScalarType`] over a [`NativeType`]. Its default
/// methods (the "pre-implementations") mutualize the serialized width so a concrete fixed
/// scalar supplies only its optional value.
pub trait FixedScalar: ScalarType {
    /// The native element type.
    type Native: NativeType;

    /// The value, or `None` if null.
    fn value(&self) -> Option<Self::Native>;

    /// The serialized byte width: one validity byte plus one value — mutualized default.
    fn serialized_width(&self) -> usize {
        1 + <Self::Native as NativeType>::WIDTH
    }
}

/// A single value of a fixed-width type `T`, possibly **null** — `Scalar<u8> = U8Scalar`.
/// Its wire form is one validity byte (`1` present / `0` null) followed by `T::WIDTH`
/// little-endian value bytes, read and written through the [`IOCursor`] abstraction, so a
/// scalar round-trips through any byte sink (a [`Bytes`](crate::io::Bytes), a file, …).
///
/// Its identity ([`PartialEq`]/[`Eq`]/[`Hash`]) is **bit-canonical**: two scalars are equal iff
/// their canonical little-endian value bytes are equal (and hashing streams those same bytes),
/// so the value type works as a map key and over a wire in every language. For the float types
/// (`f16`/`f32`/`f64`) this deliberately diverges from IEEE `==`: `NaN == NaN` when the bit
/// patterns match, and `+0.0 != -0.0` (different bytes) — the price of a total, hashable,
/// serialize-consistent identity, which the raw `f32`/`f64`/`f16` `==` cannot provide (they are
/// not `Eq`/`Hash`).
///
/// ```
/// use yggdryl_core::io::fixed::Scalar;
/// use yggdryl_core::io::{Bytes, IOCursor};
///
/// let value = Scalar::of(42i32);
/// let mut sink = Bytes::new();
/// value.write_to(&mut sink).unwrap();
///
/// sink.rewind();
/// assert_eq!(Scalar::<i32>::read_from(&mut sink).unwrap(), value);
/// ```
#[derive(Debug, Clone)]
pub struct Scalar<T: NativeType> {
    value: Option<T>,
    /// The value's own leaf [`Field`] descriptor — its name, declared nullability, and metadata.
    /// Excluded from value identity and the byte codec (only the value participates).
    field: Field,
}

/// Writes `value`'s canonical little-endian bytes into a stack scratch and returns the used
/// prefix length — the basis of the bit-canonical value identity (shared by `PartialEq`/`Hash`).
#[inline]
fn canonical_bytes<T: NativeType>(value: T, scratch: &mut [u8; MAX_WIDTH]) -> &[u8] {
    value.write_le(scratch);
    &scratch[..T::WIDTH]
}

impl<T: NativeType> PartialEq for Scalar<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self.value, other.value) {
            (Some(a), Some(b)) => {
                let (mut ab, mut bb) = ([0u8; MAX_WIDTH], [0u8; MAX_WIDTH]);
                canonical_bytes(a, &mut ab) == canonical_bytes(b, &mut bb)
            }
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: NativeType> Eq for Scalar<T> {}

impl<T: NativeType> core::hash::Hash for Scalar<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.value {
            Some(value) => {
                state.write_u8(1);
                let mut scratch = [0u8; MAX_WIDTH];
                state.write(canonical_bytes(value, &mut scratch));
            }
            None => state.write_u8(0),
        }
    }
}

impl<T: NativeType> Scalar<T> {
    /// A scalar from an optional value (`None` is null).
    pub fn new(value: Option<T>) -> Self {
        Self {
            value,
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        }
    }

    /// A present (non-null) scalar.
    pub fn of(value: T) -> Self {
        Self::new(Some(value))
    }

    /// The null scalar.
    pub fn null() -> Self {
        Self::new(None)
    }

    /// The value, or `None` if null.
    pub fn value(&self) -> Option<T> {
        self.value
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }

    /// The typed data type of this scalar — a zero-cost `const` descriptor.
    pub const fn data_type(&self) -> PrimitiveType<T> {
        PrimitiveType::new()
    }

    field_accessors!();

    /// The erased [`AnyField`] this scalar contributes — its **held field** (name + metadata) with
    /// **effective** nullability `self.nullable() || self.is_null()` folded in (the scalar analogue
    /// of a serie's `nullable() || has_nulls()`).
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.is_null());
        AnyField::leaf(field)
    }

    /// Like [`field`](Scalar::field) but **consumes** the scalar, moving the held field's name and
    /// metadata into the result with no clone.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.is_null();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field)
    }

    /// A [`TypedField`] naming a column of this scalar's type with explicit nullability.
    pub fn typed_field(&self, name: &str, nullable: bool) -> TypedField<T> {
        TypedField::new(name, nullable)
    }

    /// This scalar **broadcast to a length-1 [`Serie`]** — the inverse of
    /// [`Serie::as_scalar`](Serie::as_scalar).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Scalar;
    ///
    /// let col = Scalar::of(7i32).to_serie();
    /// assert_eq!(col.len(), 1);
    /// assert_eq!(col.get(0), Some(7));
    /// ```
    pub fn to_serie(&self) -> Serie<T> {
        Serie::from_scalar(self.clone())
    }

    /// The serialized byte width: one validity byte plus one value.
    pub const fn serialized_width() -> usize {
        1 + T::WIDTH
    }

    /// Writes this scalar to `sink` — one validity byte then the value's little-endian bytes
    /// (zeros when null) — advancing its cursor.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let mut frame = [0u8; 1 + MAX_WIDTH];
        match self.value {
            Some(value) => {
                frame[0] = 1;
                value.write_le(&mut frame[1..]);
            }
            None => frame[0] = 0, // value bytes stay zero
        }
        sink.write_all(&frame[..Self::serialized_width()])
    }

    /// Reads a scalar written by [`write_to`](Scalar::write_to) from `source`, advancing its
    /// cursor. Errors ([`IoError::UnexpectedEof`]) if fewer than
    /// [`serialized_width`](Scalar::serialized_width) bytes remain.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let mut frame = [0u8; 1 + MAX_WIDTH];
        let frame = &mut frame[..Self::serialized_width()];
        source.read_exact(frame)?;
        let value = (frame[0] != 0).then(|| T::read_le(&frame[1..]));
        Ok(Self::new(value))
    }

    /// This scalar's canonical bytes — the same one-validity-byte-then-little-endian-value frame
    /// [`write_to`](Scalar::write_to) produces, returned as an owned `Vec`. The exact inverse of
    /// [`deserialize_bytes`](Scalar::deserialize_bytes), and the codec the Python / Node bindings
    /// expose (`serialize_bytes` / `serializeBytes`).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Scalar;
    ///
    /// let value = Scalar::of(-1234i32);
    /// assert_eq!(Scalar::<i32>::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);
    /// assert_eq!(Scalar::<i32>::null().serialize_bytes()[0], 0); // validity byte is 0 for null
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::with_capacity(Self::serialized_width());
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a scalar from the bytes produced by
    /// [`serialize_bytes`](Scalar::serialize_bytes), erroring
    /// ([`IoError::UnexpectedEof`]) if the frame is shorter than
    /// [`serialized_width`](Scalar::serialized_width).
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

impl<T: NativeType> From<T> for Scalar<T> {
    fn from(value: T) -> Self {
        Self::of(value)
    }
}

// The trait-hierarchy impls: `Scalar<T>` is the fixed implementation of `ScalarType`. Bodies
// read the fields directly (same module) so they never recurse into the inherent methods.
impl<T: NativeType> ScalarType for Scalar<T> {
    type Data = PrimitiveType<T>;

    fn data_type(&self) -> PrimitiveType<T> {
        PrimitiveType::new()
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

impl<T: NativeType> FixedScalar for Scalar<T> {
    type Native = T;

    fn value(&self) -> Option<T> {
        self.value
    }
}
