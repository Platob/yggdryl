//! The [`AnySerie`] type-erased column: the crate's own array holder.

use arrow_array::{Array, ArrayRef};

use crate::{
    AnyScalar, Float16Serie, Float32Serie, Float64Serie, Int16Serie, Int32Serie, Int64Serie,
    Int8Serie, UInt16Serie, UInt32Serie, UInt64Serie, UInt8Serie,
};

/// A type-erased, buffer-backed column — the crate's **own array holder** behind
/// the nested scalars (the serie's items, the map's entries, the struct's columns).
///
/// The fixed-width numeric element types are held *decomposed* as the concrete
/// buffer-backed series (the integers [`Int8Serie`] … [`UInt64Serie`] and the floats
/// [`Float32Serie`] / [`Float64Serie`]) — the raw `ScalarBuffer` + `NullBuffer` pair,
/// leaner than an Arrow array handle and read without per-access downcasts — while any
/// other element type keeps its Arrow array zero-copy in the
/// [`Arrow`](AnySerie::Arrow) fallback. Both directions of the Arrow boundary are
/// reference-count bumps, never element copies: [`from_arrow`](AnySerie::from_arrow)
/// *decomposes* an Arrow array into the matching serie, and
/// [`to_arrow`](AnySerie::to_arrow) *reconstitutes* the Arrow array on demand
/// around the same shared buffers.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_scalar::arrow_array::{self, Array};
/// use yggdryl_scalar::{AnySerie, Int64Serie};
///
/// // An int64 Arrow array decomposes into the buffer-backed Int64Serie...
/// let arrow: arrow_array::ArrayRef = Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3]));
/// let serie = AnySerie::from_arrow(arrow.clone());
/// assert!(matches!(serie, AnySerie::Int64(_)));
/// assert_eq!(serie.len(), 3);
///
/// // ...and reconstitutes the Arrow array on demand, sharing the same buffers.
/// assert_eq!(serie.to_arrow().as_ref(), arrow.as_ref());
///
/// // A serie is also built straight from a concrete serie, no Arrow round trip.
/// assert_eq!(AnySerie::from(Int64Serie::from(vec![1, 2, 3])), serie);
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AnySerie {
    /// A column of `int8`, decomposed.
    Int8(Int8Serie),
    /// A column of `int16`, decomposed.
    Int16(Int16Serie),
    /// A column of `int32`, decomposed.
    Int32(Int32Serie),
    /// A column of `int64`, decomposed.
    Int64(Int64Serie),
    /// A column of `uint8`, decomposed.
    UInt8(UInt8Serie),
    /// A column of `uint16`, decomposed.
    UInt16(UInt16Serie),
    /// A column of `uint32`, decomposed.
    UInt32(UInt32Serie),
    /// A column of `uint64`, decomposed.
    UInt64(UInt64Serie),
    /// A column of `float16`, decomposed.
    Float16(Float16Serie),
    /// A column of `float32`, decomposed.
    Float32(Float32Serie),
    /// A column of `float64`, decomposed.
    Float64(Float64Serie),
    /// Any other element type, held as its Arrow array zero-copy (decomposed
    /// variants are added as concrete series land).
    Arrow(ArrayRef),
}

/// Expands one arm per decomposed numeric variant, so every `match` below stays
/// exhaustive without repeating the ten widths by hand.
macro_rules! for_each_decomposed {
    ($self:expr, $serie:ident => $body:expr, $arrow:ident => $fallback:expr) => {
        match $self {
            AnySerie::Int8($serie) => $body,
            AnySerie::Int16($serie) => $body,
            AnySerie::Int32($serie) => $body,
            AnySerie::Int64($serie) => $body,
            AnySerie::UInt8($serie) => $body,
            AnySerie::UInt16($serie) => $body,
            AnySerie::UInt32($serie) => $body,
            AnySerie::UInt64($serie) => $body,
            AnySerie::Float16($serie) => $body,
            AnySerie::Float32($serie) => $body,
            AnySerie::Float64($serie) => $body,
            AnySerie::Arrow($arrow) => $fallback,
        }
    };
}

impl AnySerie {
    /// Decompose an Arrow array into the matching serie — the integer types take
    /// their buffers apart (reference-count bumps, no element copies); any other
    /// type is held as the array itself, zero-copy.
    pub fn from_arrow(values: ArrayRef) -> Self {
        macro_rules! decompose {
            ($variant:ident, $serie:ty, $array:ty) => {
                Self::$variant(<$serie>::from(
                    values
                        .as_any()
                        .downcast_ref::<$array>()
                        .expect("the Arrow data type names its array type")
                        .clone(),
                ))
            };
        }
        use arrow_schema::DataType as A;
        match values.data_type() {
            A::Int8 => decompose!(Int8, Int8Serie, arrow_array::Int8Array),
            A::Int16 => decompose!(Int16, Int16Serie, arrow_array::Int16Array),
            A::Int32 => decompose!(Int32, Int32Serie, arrow_array::Int32Array),
            A::Int64 => decompose!(Int64, Int64Serie, arrow_array::Int64Array),
            A::UInt8 => decompose!(UInt8, UInt8Serie, arrow_array::UInt8Array),
            A::UInt16 => decompose!(UInt16, UInt16Serie, arrow_array::UInt16Array),
            A::UInt32 => decompose!(UInt32, UInt32Serie, arrow_array::UInt32Array),
            A::UInt64 => decompose!(UInt64, UInt64Serie, arrow_array::UInt64Array),
            A::Float16 => decompose!(Float16, Float16Serie, arrow_array::Float16Array),
            A::Float32 => decompose!(Float32, Float32Serie, arrow_array::Float32Array),
            A::Float64 => decompose!(Float64, Float64Serie, arrow_array::Float64Array),
            _ => Self::Arrow(values),
        }
    }

    /// Reconstitute the Arrow array on demand — the decomposed series reassemble
    /// around the same shared buffers (reference-count bumps, never element
    /// copies), the fallback clones its handle.
    pub fn to_arrow(&self) -> ArrayRef {
        for_each_decomposed!(self,
            serie => std::sync::Arc::new(serie.to_arrow_array()),
            values => values.clone())
    }

    /// The number of elements in the column.
    pub fn len(&self) -> usize {
        for_each_decomposed!(self, serie => serie.len(), values => Array::len(values.as_ref()))
    }

    /// Whether the column holds no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The Arrow data type of the elements.
    pub fn data_type(&self) -> arrow_schema::DataType {
        self.to_arrow().data_type().clone()
    }

    /// The zero-copy window of `length` elements starting at `offset` — sliced
    /// through the Arrow form (shared buffers) and decomposed back, so a slice of a
    /// decomposed serie stays decomposed.
    pub fn slice(&self, offset: usize, length: usize) -> Self {
        Self::from_arrow(self.to_arrow().slice(offset, length))
    }

    /// The element at `index` as a type-erased [`AnyScalar`], or `None` past the end
    /// — the integer columns read the element straight from their buffers (no Arrow
    /// round trip), any other column slices one element and decomposes it. This is
    /// the per-value bridge behind [`RecordScalar`](crate::RecordScalar) and the
    /// struct series' row access.
    pub fn get_scalar(&self, index: usize) -> Option<AnyScalar> {
        for_each_decomposed!(self,
            serie => serie.get_scalar_at(index).map(AnyScalar::from),
            values => (index < Array::len(values.as_ref()))
                .then(|| AnyScalar::from_arrow(Array::slice(values.as_ref(), index, 1))))
    }
}

impl PartialEq for AnySerie {
    // Compared logically, like Arrow arrays: the decomposed fast path compares the
    // concrete series, mixed representations fall back to the Arrow form (so a
    // decomposed column equals its zero-copy passthrough twin).
    fn eq(&self, other: &Self) -> bool {
        macro_rules! same {
            ($($variant:ident),+) => {
                match (self, other) {
                    $((AnySerie::$variant(left), AnySerie::$variant(right)) => left == right,)+
                    (left, right) => left.to_arrow().as_ref() == right.to_arrow().as_ref(),
                }
            };
        }
        same!(Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float16, Float32, Float64)
    }
}

impl Eq for AnySerie {}

impl From<ArrayRef> for AnySerie {
    /// Decompose the Arrow array (see [`from_arrow`](AnySerie::from_arrow)).
    fn from(values: ArrayRef) -> Self {
        Self::from_arrow(values)
    }
}

macro_rules! from_concrete {
    ($($variant:ident, $serie:ident);+ $(;)?) => {
        $(impl From<$serie> for AnySerie {
            /// Hold the concrete serie directly — no Arrow round trip.
            fn from(serie: $serie) -> Self {
                Self::$variant(serie)
            }
        })+
    };
}
from_concrete!(
    Int8, Int8Serie; Int16, Int16Serie; Int32, Int32Serie; Int64, Int64Serie;
    UInt8, UInt8Serie; UInt16, UInt16Serie; UInt32, UInt32Serie; UInt64, UInt64Serie;
    Float16, Float16Serie; Float32, Float32Serie; Float64, Float64Serie;
);
