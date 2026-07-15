//! [`Scalar`] — a single, nullable value of a fixed-width `T` — and the [`FixedScalar`]
//! sub-trait of the root [`ScalarType`](crate::io::ScalarType).

use super::{NativeType, PrimitiveType, Serie, TypedField};
use crate::io::{IOCursor, IoError, ScalarType};

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
#[derive(Debug, Clone, Copy)]
pub struct Scalar<T: NativeType> {
    value: Option<T>,
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
        Self { value }
    }

    /// A present (non-null) scalar.
    pub fn of(value: T) -> Self {
        Self { value: Some(value) }
    }

    /// The null scalar.
    pub fn null() -> Self {
        Self { value: None }
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

    /// A [`TypedField`] naming a column of this scalar's type.
    pub fn field(&self, name: &str, nullable: bool) -> TypedField<T> {
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
        Serie::from_scalar(*self)
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
        Ok(Self { value })
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
