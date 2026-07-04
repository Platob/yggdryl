//! The [`AnyScalar`] type-erased scalar: the crate's own single-value holder.

use arrow_array::{Array, ArrayRef};

use crate::{
    Float32Scalar, Float64Scalar, Int16Scalar, Int32Scalar, Int64Scalar, Int8Scalar, Scalar,
    UInt16Scalar, UInt32Scalar, UInt64Scalar, UInt8Scalar,
};

/// A type-erased, possibly-null single value — the atomic counterpart of
/// [`AnySerie`](crate::AnySerie), the crate's **own value holder** behind a
/// [`RecordScalar`](crate::RecordScalar)'s fields (one `AnyScalar` per struct field).
///
/// It mirrors [`AnySerie`](crate::AnySerie) one value down: the fixed-width numeric
/// types are held *decomposed* as their concrete scalars (the integers [`Int8Scalar`]
/// … [`UInt64Scalar`] and the floats [`Float32Scalar`] / [`Float64Scalar`]) — a bare
/// native `Option`, read without an Arrow downcast — while any other type keeps its
/// one-element Arrow array zero-copy in the [`Arrow`](AnyScalar::Arrow) fallback. Both directions of the Arrow boundary are reference-count bumps, never
/// copies: [`from_arrow`](AnyScalar::from_arrow) *decomposes* a one-element array into
/// the matching scalar, and [`to_arrow_scalar`](AnyScalar::to_arrow_scalar)
/// *reconstitutes* the one-element array on demand.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_scalar::arrow_array::{self, Array};
/// use yggdryl_scalar::{AnyScalar, Int64Scalar, Scalar};
///
/// // A one-element int64 array decomposes into the concrete Int64Scalar...
/// let arrow: arrow_array::ArrayRef = Arc::new(arrow_array::Int64Array::from(vec![42]));
/// let scalar = AnyScalar::from_arrow(arrow.clone());
/// assert!(matches!(scalar, AnyScalar::Int64(_)));
/// assert!(!scalar.is_null());
///
/// // ...and reconstitutes the one-element array on demand, sharing the buffers.
/// assert_eq!(scalar.to_arrow_scalar().as_ref(), arrow.as_ref());
///
/// // A scalar is also built straight from a concrete scalar, no Arrow round trip.
/// assert_eq!(AnyScalar::from(Int64Scalar::new(42)), scalar);
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AnyScalar {
    /// An `int8` value, decomposed.
    Int8(Int8Scalar),
    /// An `int16` value, decomposed.
    Int16(Int16Scalar),
    /// An `int32` value, decomposed.
    Int32(Int32Scalar),
    /// An `int64` value, decomposed.
    Int64(Int64Scalar),
    /// A `uint8` value, decomposed.
    UInt8(UInt8Scalar),
    /// A `uint16` value, decomposed.
    UInt16(UInt16Scalar),
    /// A `uint32` value, decomposed.
    UInt32(UInt32Scalar),
    /// A `uint64` value, decomposed.
    UInt64(UInt64Scalar),
    /// A `float32` value, decomposed.
    Float32(Float32Scalar),
    /// A `float64` value, decomposed.
    Float64(Float64Scalar),
    /// Any other type, held as its one-element Arrow array zero-copy (decomposed
    /// variants are added as concrete scalars land).
    Arrow(ArrayRef),
}

/// Expands one arm per decomposed numeric variant, so every `match` below stays
/// exhaustive without repeating the ten widths by hand.
macro_rules! for_each_decomposed {
    ($self:expr, $scalar:ident => $body:expr, $arrow:ident => $fallback:expr) => {
        match $self {
            AnyScalar::Int8($scalar) => $body,
            AnyScalar::Int16($scalar) => $body,
            AnyScalar::Int32($scalar) => $body,
            AnyScalar::Int64($scalar) => $body,
            AnyScalar::UInt8($scalar) => $body,
            AnyScalar::UInt16($scalar) => $body,
            AnyScalar::UInt32($scalar) => $body,
            AnyScalar::UInt64($scalar) => $body,
            AnyScalar::Float32($scalar) => $body,
            AnyScalar::Float64($scalar) => $body,
            AnyScalar::Arrow($arrow) => $fallback,
        }
    };
}

impl AnyScalar {
    /// Decompose a **one-element** Arrow array into the matching scalar — the integer
    /// types read their element through the concrete scalar (a reference-count bump,
    /// no copy); any other type is held as the array itself, zero-copy.
    pub fn from_arrow(value: ArrayRef) -> Self {
        macro_rules! decompose {
            ($variant:ident, $scalar:ty) => {
                Self::$variant(
                    <$scalar>::from_arrow(value.as_ref())
                        .expect("the Arrow data type names its scalar type"),
                )
            };
        }
        use arrow_schema::DataType as A;
        match value.data_type() {
            A::Int8 => decompose!(Int8, Int8Scalar),
            A::Int16 => decompose!(Int16, Int16Scalar),
            A::Int32 => decompose!(Int32, Int32Scalar),
            A::Int64 => decompose!(Int64, Int64Scalar),
            A::UInt8 => decompose!(UInt8, UInt8Scalar),
            A::UInt16 => decompose!(UInt16, UInt16Scalar),
            A::UInt32 => decompose!(UInt32, UInt32Scalar),
            A::UInt64 => decompose!(UInt64, UInt64Scalar),
            A::Float32 => decompose!(Float32, Float32Scalar),
            A::Float64 => decompose!(Float64, Float64Scalar),
            _ => Self::Arrow(value),
        }
    }

    /// Reconstitute the one-element Arrow array on demand — the decomposed scalar
    /// rebuilds it, the fallback clones its handle (reference-count bumps only).
    pub fn to_arrow_scalar(&self) -> ArrayRef {
        for_each_decomposed!(self,
            scalar => scalar.to_arrow_scalar(),
            value => value.clone())
    }

    /// The Arrow data type of the value.
    pub fn data_type(&self) -> arrow_schema::DataType {
        self.to_arrow_scalar().data_type().clone()
    }

    /// Whether the value is null.
    pub fn is_null(&self) -> bool {
        // The *logical* null count, so an all-null `NullArray` (which carries no
        // physical null buffer) still reads as null.
        for_each_decomposed!(self,
            scalar => Scalar::is_null(scalar),
            value => Array::logical_null_count(value.as_ref()) > 0)
    }
}

impl PartialEq for AnyScalar {
    // Compared logically, like Arrow scalars: the decomposed fast path compares the
    // concrete scalars, mixed representations fall back to the one-element Arrow form
    // (so a decomposed value equals its zero-copy passthrough twin).
    fn eq(&self, other: &Self) -> bool {
        macro_rules! same {
            ($($variant:ident),+) => {
                match (self, other) {
                    $((AnyScalar::$variant(left), AnyScalar::$variant(right)) => left == right,)+
                    (left, right) => left.to_arrow_scalar().as_ref() == right.to_arrow_scalar().as_ref(),
                }
            };
        }
        same!(Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64)
    }
}

impl Eq for AnyScalar {}

impl From<ArrayRef> for AnyScalar {
    /// Decompose the one-element Arrow array (see [`from_arrow`](AnyScalar::from_arrow)).
    fn from(value: ArrayRef) -> Self {
        Self::from_arrow(value)
    }
}

macro_rules! from_concrete {
    ($($variant:ident, $scalar:ident);+ $(;)?) => {
        $(impl From<$scalar> for AnyScalar {
            /// Hold the concrete scalar directly — no Arrow round trip.
            fn from(scalar: $scalar) -> Self {
                Self::$variant(scalar)
            }
        })+
    };
}
from_concrete!(
    Int8, Int8Scalar; Int16, Int16Scalar; Int32, Int32Scalar; Int64, Int64Scalar;
    UInt8, UInt8Scalar; UInt16, UInt16Scalar; UInt32, UInt32Scalar; UInt64, UInt64Scalar;
    Float32, Float32Scalar; Float64, Float64Scalar;
);
