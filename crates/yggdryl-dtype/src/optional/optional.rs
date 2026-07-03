//! The [`Optional`] base trait: the untyped surface of an optional data type.

use crate::{DataType, Logical, UnionType};

/// The untyped surface every optional data type carries: a logical value-or-null
/// type over a [`UnionType`] storage, exposing its value type.
///
/// It refines [`Logical<UnionType>`] — an optional is *stored* as the sparse
/// two-variant union between [`NullType`](crate::NullType) and the value type — and
/// is parameterised by the value data type `D` (rather than boxing it) so the
/// concrete type is preserved for zero-cost access, mirroring `yggdryl-field`'s
/// `Field` and `yggdryl-scalar`'s `Scalar`. A value type with a codec also gets the
/// typed [`TypedOptional`](crate::TypedOptional) layer.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, Optional, OptionalType};
///
/// let optional = OptionalType::new(Int64Type);
/// assert_eq!(optional.value_type().name(), "int64");
/// assert_eq!(optional.storage().name(), "union"); // from Logical
/// ```
pub trait Optional<D: DataType>: Logical<UnionType> {
    /// The value type this optional wraps.
    fn value_type(&self) -> &D;
}
