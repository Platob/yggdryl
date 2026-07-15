//! [`DecimalScalar<B>`] — one nullable decimal value carried with its column `(precision, scale)`.
//! Its logical value is a [`Decimal<B>`](super::Decimal); identity is by that value (so `2.5` and
//! `2.50` scalars are equal), and it round-trips through any [`IOCursor`] byte sink.

use core::marker::PhantomData;

use super::{Decimal, DecimalBacking, DecimalCoeff, DecimalError, DecimalType};
use crate::io::{IOCursor, IoError, ScalarType};

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
    precision: u8,
    scale: i8,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> DecimalScalar<B> {
    /// A present scalar from a value, taking the value's own scale as the column scale and a
    /// precision wide enough to hold it.
    pub fn of(value: Decimal<B>) -> Self {
        let scale = value.scale().max(0);
        let precision = (value.precision().max(scale as u32).max(1) as u8).min(B::MAX_PRECISION);
        Self {
            value: Some(value.raw_coeff()),
            precision,
            scale,
            _backing: PhantomData,
        }
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
        Ok(Self {
            value: Some(rescaled.raw_coeff()),
            precision,
            scale,
            _backing: PhantomData,
        })
    }

    /// The null scalar of the given `(precision, scale)`.
    pub fn null(precision: u8, scale: i8) -> Self {
        Self {
            value: None,
            precision,
            scale,
            _backing: PhantomData,
        }
    }

    /// A scalar from an already-fitted coefficient at `(precision, scale)` — the column's bridge to
    /// a scalar (kept crate-only; the coefficient is in range by construction).
    pub(crate) fn from_parts(value: Option<B::Coeff>, precision: u8, scale: i8) -> Self {
        Self {
            value,
            precision,
            scale,
            _backing: PhantomData,
        }
    }

    /// The value, or `None` if null.
    pub fn value(&self) -> Option<Decimal<B>> {
        self.value
            .map(|coeff| Decimal::from_coeff(coeff, self.scale))
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }

    /// The precision.
    pub fn precision(&self) -> u8 {
        self.precision
    }

    /// The scale.
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// The typed descriptor.
    pub fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision, self.scale)
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
        frame[1] = self.precision;
        frame[2] = self.scale as u8;
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
        Ok(Self {
            value,
            precision,
            scale,
            _backing: PhantomData,
        })
    }
}

impl<B: DecimalBacking> ScalarType for DecimalScalar<B> {
    type Data = DecimalType<B>;

    fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision, self.scale)
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
        *self
    }
}
impl<B: DecimalBacking> Copy for DecimalScalar<B> {}
impl<B: DecimalBacking> core::fmt::Debug for DecimalScalar<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecimalScalar")
            .field("type", &B::NAME)
            .field("precision", &self.precision)
            .field("scale", &self.scale)
            .field("value", &self.value().map(|v| v.to_string()))
            .finish()
    }
}
