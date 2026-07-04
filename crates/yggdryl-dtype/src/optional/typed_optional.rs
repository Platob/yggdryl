//! The typed [`TypedOptional`] trait: an [`Optional`](super::Optional) whose value
//! type has a codec.

use super::Optional;
use crate::TypedDataType;

/// An [`Optional`](super::Optional) whose value type is a typed
/// [`TypedDataType<T>`] — the optional's values have native Rust representation `T`.
///
/// It names the concrete value type as the associated
/// [`ValueType`](TypedOptional::ValueType) (a [`TypedDataType<T>`]) so it is
/// preserved for zero-cost access. It also carries the [`TypedDataType<T>`] surface
/// itself: the codec (and [`default_value`](TypedDataType::default_value)) delegate
/// to the value type. The untyped [`Optional`](super::Optional) is implemented by
/// both the dynamic [`OptionalType`](crate::OptionalType) and the typed
/// [`TypedOptionalType<D>`](crate::TypedOptionalType); this typed layer is only the
/// latter.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, TypedDataType, TypedOptional, TypedOptionalType};
///
/// fn default_of<T, O: TypedOptional<T>>(optional: &O) -> T {
///     optional.default_value() // the value type's default
/// }
///
/// let optional = TypedOptionalType::new(Int64Type);
/// assert_eq!(optional.value_type().name(), "int64");
/// assert_eq!(default_of(&optional), 0);
/// ```
pub trait TypedOptional<T>: Optional + TypedDataType<T> {
    /// The value type this optional wraps.
    type ValueType: TypedDataType<T>;

    /// The value type this optional wraps.
    fn value_type(&self) -> &Self::ValueType;
}
