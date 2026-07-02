//! The [`RawOptional`] base trait: the untyped surface of an optional data type.

use crate::{Logical, RawDataType, UnionType};

/// The untyped surface every optional data type carries: a logical value-or-null
/// type over a [`UnionType`] storage, exposing its value type.
///
/// It refines [`Logical`] with `Storage = UnionType` — an optional is *stored* as
/// the sparse two-variant union between [`Null`](crate::Null) and the value type —
/// and is parameterised by the value data type `D` (rather than boxing it) so the
/// concrete type is preserved for zero-cost access, mirroring
/// [`RawField`](crate::RawField) and [`RawScalar`](crate::RawScalar). A value type
/// with a codec also gets the typed [`Optional`](crate::Optional) layer.
///
/// ```
/// use yggdryl_data::{Int64, Logical, OptionalType, RawDataType, RawOptional};
///
/// let optional = OptionalType::new(Int64);
/// assert_eq!(optional.value_type().name(), "int64");
/// assert_eq!(optional.storage().name(), "union"); // from Logical
/// ```
pub trait RawOptional<D: RawDataType>: Logical<Storage = UnionType> {
    /// The value type this optional wraps.
    fn value_type(&self) -> &D;
}
