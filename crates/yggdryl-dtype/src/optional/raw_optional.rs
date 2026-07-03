//! The [`RawOptional`] base trait: the untyped surface of an optional data type.

use crate::{RawDataType, RawLogical, Union};

/// The untyped surface every optional data type carries: a logical value-or-null
/// type over a [`Union`] storage, exposing its value type.
///
/// It refines [`RawLogical<Union>`] — an optional is *stored* as the sparse
/// two-variant union between [`Null`](crate::Null) and the value type — and is
/// parameterised by the value data type `D` (rather than boxing it) so the
/// concrete type is preserved for zero-cost access, mirroring `yggdryl-field`'s
/// `RawField` and `yggdryl-scalar`'s `RawScalar`. A value type with a codec also
/// gets the typed [`TypedOptional`](crate::TypedOptional) layer.
///
/// ```
/// use yggdryl_dtype::{Int64, Optional, RawDataType, RawLogical, RawOptional};
///
/// let optional = Optional::new(Int64);
/// assert_eq!(optional.value_type().name(), "int64");
/// assert_eq!(optional.storage().name(), "union"); // from RawLogical
/// ```
pub trait RawOptional<D: RawDataType>: RawLogical<Union> {
    /// The value type this optional wraps.
    fn value_type(&self) -> &D;
}
