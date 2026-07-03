//! The typed [`TypedOptional`] trait: an [`Optional`](super::Optional) whose value
//! type has a codec.

use super::Optional;
use crate::TypedDataType;

/// An [`Optional`](super::Optional) whose value type is a typed
/// [`TypedDataType<T>`] — the optional's values have native Rust representation `T`.
///
/// The concrete value type is [`Optional`](super::Optional)'s associated
/// [`ValueType`](super::Optional::ValueType), here refined to a
/// [`TypedDataType<T>`]; `value_type` is inherited from
/// [`Optional`](super::Optional). It also carries the [`TypedDataType<T>`] surface
/// itself: the codec (and [`default_value`](TypedDataType::default_value)) delegate
/// to the value type.
///
/// ```
/// use yggdryl_dtype::{Int64Type, OptionalType, TypedDataType, TypedOptional};
///
/// fn default_of<T, O: TypedOptional<T>>(optional: &O) -> T {
///     optional.default_value() // the value type's default
/// }
///
/// let optional = OptionalType::new(Int64Type);
/// assert_eq!(default_of(&optional), 0);
/// ```
pub trait TypedOptional<T>: Optional<ValueType: TypedDataType<T>> + TypedDataType<T> {}
