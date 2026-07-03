//! The typed [`TypedOptional`] trait: an [`Optional`](super::Optional) whose value
//! type has a codec.

use super::Optional;
use crate::TypedDataType;

/// An [`Optional`](super::Optional) whose value type is a typed
/// [`TypedDataType<T>`] — the optional's values have native Rust representation `T`.
///
/// The concrete value type is the associated [`ValueType`](TypedOptional::ValueType), so
/// an optional has exactly one; `value_type` is inherited from
/// [`Optional`](super::Optional) and returns it. It also carries the
/// [`TypedDataType<T>`] surface itself: the codec (and
/// [`default_value`](TypedDataType::default_value)) delegate to the value type.
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
pub trait TypedOptional<T>: Optional<Self::ValueType> + TypedDataType<T> {
    /// The concrete value type of this optional.
    type ValueType: TypedDataType<T>;
}
