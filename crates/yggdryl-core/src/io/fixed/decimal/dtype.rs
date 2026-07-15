//! [`DecimalType<B>`] — the **columnar decimal descriptor**: a decimal width `B` together with a
//! runtime `(precision, scale)`, mapping to Arrow's `Decimal{32,64,128,256}(precision, scale)`.
//!
//! Unlike the self-describing [`Decimal`](super::Decimal) value (whose scale rides each value), a
//! *column* fixes one `(precision, scale)` for every element — exactly Arrow's model — so the
//! descriptor carries them and the [`DecimalScalar`](super::DecimalScalar) /
//! [`DecimalSerie`](super::DecimalSerie) store only raw coefficients.

use core::marker::PhantomData;

use super::{DecimalBacking, DecimalError, DecimalField};
use crate::io::{DataType, DataTypeId, TypedDataType};

/// The descriptor of a decimal column of width `B`, precision `precision`, and scale `scale`
/// (`value = coefficient × 10^-scale`, at most `precision` significant digits).
pub struct DecimalType<B: DecimalBacking> {
    precision: u8,
    scale: i8,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> DecimalType<B> {
    /// A descriptor for `Decimal(precision, scale)`. **Clamps** `precision` into
    /// `1..=B::MAX_PRECISION` and `scale` into `0..=precision`, so the mapping to Arrow is always
    /// valid; use [`try_new`](DecimalType::try_new) to reject an out-of-range request instead.
    pub fn new(precision: u8, scale: i8) -> Self {
        let precision = precision.clamp(1, B::MAX_PRECISION);
        let scale = scale.clamp(0, precision as i8);
        Self {
            precision,
            scale,
            _backing: PhantomData,
        }
    }

    /// A descriptor for `Decimal(precision, scale)`, or a guided
    /// [`CoefficientOutOfRange`](DecimalError::CoefficientOutOfRange) if `precision` exceeds the
    /// width's maximum, or [`InexactRescale`](DecimalError::InexactRescale) if `scale` is negative
    /// or larger than `precision`.
    pub fn try_new(precision: u8, scale: i8) -> Result<Self, DecimalError> {
        if precision == 0 || precision > B::MAX_PRECISION {
            return Err(DecimalError::CoefficientOutOfRange {
                ty: B::NAME,
                max_precision: B::MAX_PRECISION,
            });
        }
        if scale < 0 || scale > precision as i8 {
            return Err(DecimalError::InexactRescale {
                ty: B::NAME,
                from: scale,
                to: precision as i8,
            });
        }
        Ok(Self {
            precision,
            scale,
            _backing: PhantomData,
        })
    }

    /// The precision (maximum significant digits).
    pub fn precision(&self) -> u8 {
        self.precision
    }

    /// The scale (fractional digits).
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// The width's maximum precision (`9`/`18`/`38`/`76`).
    pub const fn max_precision() -> u8 {
        B::MAX_PRECISION
    }

    /// A [`DecimalField`] naming a column of this type.
    pub fn field(&self, name: &str, nullable: bool) -> DecimalField<B> {
        DecimalField::new(name, self.precision, self.scale, nullable)
    }

    /// This descriptor as an [`arrow_schema::DataType`] — `Decimal{32,64,128,256}(precision, scale)`
    /// (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::DataType {
        B::TYPE_ID
            .to_arrow_decimal(self.precision, self.scale)
            .expect("a decimal id always maps to an Arrow Decimal")
    }

    /// Builds a descriptor from an [`arrow_schema::DataType`], or `None` if it is not the matching
    /// `Decimal` for this width (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(data_type: &arrow_schema::DataType) -> Option<Self> {
        use arrow_schema::DataType as A;
        let (precision, scale) = match (B::TYPE_ID, data_type) {
            (DataTypeId::D32, A::Decimal32(p, s))
            | (DataTypeId::D64, A::Decimal64(p, s))
            | (DataTypeId::D128, A::Decimal128(p, s))
            | (DataTypeId::D256, A::Decimal256(p, s)) => (*p, *s),
            _ => return None,
        };
        Some(Self::new(precision, scale))
    }
}

impl<B: DecimalBacking> DataType for DecimalType<B> {
    fn name(&self) -> &'static str {
        B::NAME
    }

    fn byte_width(&self) -> usize {
        B::WIDTH
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }

    // Override the centralized default: the erased `type_id().to_arrow(byte_width)` cannot know the
    // precision/scale, so the descriptor supplies them.
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        self.to_arrow()
    }
}

impl<B: DecimalBacking> TypedDataType for DecimalType<B> {
    /// The logical element is the self-describing [`Decimal`](super::Decimal) value.
    type Native = super::Decimal<B>;
}

// Value semantics (a plain, comparable, hashable descriptor).
impl<B: DecimalBacking> Clone for DecimalType<B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<B: DecimalBacking> Copy for DecimalType<B> {}
impl<B: DecimalBacking> PartialEq for DecimalType<B> {
    fn eq(&self, other: &Self) -> bool {
        self.precision == other.precision && self.scale == other.scale
    }
}
impl<B: DecimalBacking> Eq for DecimalType<B> {}
impl<B: DecimalBacking> core::hash::Hash for DecimalType<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        B::NAME.hash(state);
        self.precision.hash(state);
        self.scale.hash(state);
    }
}
impl<B: DecimalBacking> core::fmt::Debug for DecimalType<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

impl<B: DecimalBacking> core::fmt::Display for DecimalType<B> {
    /// The type signature `name(precision, scale)`, e.g. `d128(38, 18)`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}({}, {})", B::NAME, self.precision, self.scale)
    }
}
