//! [`DecimalScalar<B>`] — one nullable decimal value carried with its column `(precision, scale)`.
//! Its logical value is a [`Decimal<B>`](super::Decimal); identity is by that value (so `2.5` and
//! `2.50` scalars are equal), and it round-trips through any [`IOCursor`] byte sink.

use core::marker::PhantomData;

use super::{
    Decimal, DecimalBacking, DecimalCoeff, DecimalError, DecimalField, DecimalSerie, DecimalType,
};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, IOCursor, IoError, ScalarType};

/// A single, possibly-null decimal value of width `B`, precision `precision`, scale `scale`.
///
/// ```
/// use yggdryl_core::io::fixed::{D64, D64Scalar};
///
/// let s = D64Scalar::of(D64::new(12345, 2).unwrap()); // 123.45
/// assert_eq!(s.value().unwrap().to_string(), "123.45");
/// assert_eq!(s.scale(), 2);
/// assert!(D64Scalar::null(10, 2).is_null());
/// ```
pub struct DecimalScalar<B: DecimalBacking> {
    value: Option<B::Coeff>,
    /// The value's own [`DecimalField`] descriptor — its name, declared nullability, metadata, and
    /// the `(precision, scale)` dtype params. The `(precision, scale)` (folded through `value()`)
    /// join the coefficient in identity; the name / nullable / metadata are excluded.
    field: DecimalField<B>,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> DecimalScalar<B> {
    /// A present scalar from a value, taking the value's own scale as the column scale and a
    /// precision wide enough to hold it.
    pub fn of(value: Decimal<B>) -> Self {
        let scale = value.scale().max(0);
        let precision = (value.precision().max(scale as u32).max(1) as u8).min(B::MAX_PRECISION);
        Self::from_parts(Some(value.raw_coeff()), precision, scale)
    }

    /// A present scalar from `value`, re-expressed at `(precision, scale)` — a guided
    /// [`InexactRescale`](DecimalError::InexactRescale) if the value does not fit `scale` exactly,
    /// or [`PrecisionExceeded`](DecimalError::PrecisionExceeded) if it needs more than `precision`
    /// significant digits.
    pub fn with_precision_scale(
        value: Decimal<B>,
        precision: u8,
        scale: i8,
    ) -> Result<Self, DecimalError> {
        let rescaled = value.rescale(scale)?;
        if rescaled.precision() > precision as u32 {
            return Err(DecimalError::PrecisionExceeded {
                ty: B::NAME,
                precision: rescaled.precision(),
                max: precision,
            });
        }
        Ok(Self::from_parts(
            Some(rescaled.raw_coeff()),
            precision,
            scale,
        ))
    }

    /// The null scalar of the given `(precision, scale)`.
    pub fn null(precision: u8, scale: i8) -> Self {
        Self::from_parts(None, precision, scale)
    }

    /// A scalar from an already-fitted coefficient at `(precision, scale)` — the column's bridge to
    /// a scalar (kept crate-only; the coefficient is in range by construction).
    pub(crate) fn from_parts(value: Option<B::Coeff>, precision: u8, scale: i8) -> Self {
        Self {
            value,
            field: DecimalField::new("", precision, scale, false),
            _backing: PhantomData,
        }
    }

    /// The value, or `None` if null.
    pub fn value(&self) -> Option<Decimal<B>> {
        self.value
            .map(|coeff| Decimal::from_coeff(coeff, self.scale()))
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }

    /// The precision (from the held field).
    pub fn precision(&self) -> u8 {
        self.field.precision()
    }

    /// The scale (from the held field).
    pub fn scale(&self) -> i8 {
        self.field.scale()
    }

    field_accessors!();

    /// The erased [`AnyField`] this scalar contributes — its **held field** (name + metadata +
    /// precision/scale) with **effective** nullability `self.nullable() || self.is_null()`.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.is_null());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](DecimalScalar::field) but **consumes** the scalar.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.is_null();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// The typed descriptor.
    pub fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision(), self.scale())
    }

    /// This scalar **broadcast to a length-1 [`DecimalSerie`]** at its own `(precision, scale)` —
    /// the inverse of [`DecimalSerie::as_scalar`](DecimalSerie::as_scalar). Mirrors the fixed
    /// family's [`Scalar::to_serie`](crate::io::fixed::Scalar::to_serie); fallible only because the
    /// column re-expresses each value at its scale (the scalar's value already fits its own
    /// `(precision, scale)`, so it never fails in practice).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{D64, D64Scalar};
    ///
    /// let col = D64Scalar::of(D64::new(12345, 2).unwrap()).to_serie().unwrap(); // 123.45
    /// assert_eq!(col.len(), 1);
    /// assert_eq!(col.get(0).unwrap().to_string(), "123.45");
    /// ```
    pub fn to_serie(&self) -> Result<DecimalSerie<B>, DecimalError> {
        DecimalSerie::from_scalar(self.clone())
    }

    /// The serialized byte width: `[validity][precision][scale][coefficient]`.
    pub const fn serialized_len() -> usize {
        3 + B::WIDTH
    }

    /// Writes this scalar — `[validity: u8][precision: u8][scale: i8][coefficient: LE]` (the
    /// coefficient is zero when null) — advancing the sink's cursor.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let mut frame = [0u8; 3 + 32];
        let len = Self::serialized_len();
        frame[0] = u8::from(self.value.is_some());
        frame[1] = self.precision();
        frame[2] = self.scale() as u8;
        if let Some(coeff) = self.value {
            coeff.write_le(&mut frame[3..]);
        }
        sink.write_all(&frame[..len])
    }

    /// Reads a scalar written by [`write_to`](DecimalScalar::write_to), advancing the source cursor.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let mut frame = [0u8; 3 + 32];
        let len = Self::serialized_len();
        source.read_exact(&mut frame[..len])?;
        let present = frame[0] != 0;
        let precision = frame[1];
        let scale = frame[2] as i8;
        let value = present.then(|| B::Coeff::read_le(&frame[3..]));
        Ok(Self::from_parts(value, precision, scale))
    }

    /// This scalar's canonical bytes — the same `[validity][precision][scale][coefficient]` frame
    /// [`write_to`](DecimalScalar::write_to) produces, returned as an owned `Vec`. The exact inverse
    /// of [`deserialize_bytes`](DecimalScalar::deserialize_bytes), and the codec the Python / Node
    /// bindings expose (`serialize_bytes` / `serializeBytes`).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{D64, D64Scalar};
    ///
    /// let scalar = D64Scalar::of(D64::new(12345, 2).unwrap()); // 123.45
    /// assert_eq!(D64Scalar::deserialize_bytes(&scalar.serialize_bytes()).unwrap(), scalar);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::with_capacity(Self::serialized_len());
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a scalar from the bytes produced by
    /// [`serialize_bytes`](DecimalScalar::serialize_bytes), erroring on a truncated frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

impl<B: DecimalBacking> ScalarType for DecimalScalar<B> {
    type Data = DecimalType<B>;

    fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision(), self.scale())
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

// Identity is by the logical value (like `Decimal`): two scalars are equal iff both null, or both
// present with equal decimal values (scale differences and all).
impl<B: DecimalBacking> PartialEq for DecimalScalar<B> {
    fn eq(&self, other: &Self) -> bool {
        match (self.value(), other.value()) {
            (Some(a), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        }
    }
}
impl<B: DecimalBacking> Eq for DecimalScalar<B> {}
impl<B: DecimalBacking> core::hash::Hash for DecimalScalar<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.value() {
            Some(value) => {
                state.write_u8(1);
                value.hash(state);
            }
            None => state.write_u8(0),
        }
    }
}
impl<B: DecimalBacking> Clone for DecimalScalar<B> {
    fn clone(&self) -> Self {
        Self {
            value: self.value,
            field: self.field.clone(),
            _backing: PhantomData,
        }
    }
}
impl<B: DecimalBacking> core::fmt::Debug for DecimalScalar<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecimalScalar")
            .field("type", &B::NAME)
            .field("precision", &self.precision())
            .field("scale", &self.scale())
            .field("value", &self.value().map(|v| v.to_string()))
            .finish()
    }
}
