//! The typed [`TypedOptional`] trait: a [`RawOptional`](super::RawOptional) whose value
//! type has a codec.

use super::RawOptional;
use crate::DataType;

/// A [`RawOptional`](super::RawOptional) whose value type is a typed
/// [`DataType<T>`] — the optional's values have native Rust representation `T`.
///
/// The concrete value type is the associated [`ValueType`](TypedOptional::ValueType), so
/// an optional has exactly one; `value_type` is inherited from
/// [`RawOptional`](super::RawOptional) and returns it. It also carries the
/// [`DataType<T>`] surface itself: the codec (and
/// [`default_value`](DataType::default_value)) delegate to the value type, while
/// [`default_scalar`](DataType::default_scalar) is the null variant — the scalar
/// models nullness, so its default matches `Option::default`.
///
/// ```
/// use yggdryl_data::{DataType, Int64Type, TypedOptional, OptionalType, RawScalar};
///
/// fn default_of<T, O: TypedOptional<T>>(optional: &O) -> T {
///     optional.default_value() // the value type's default
/// }
///
/// let optional = OptionalType::new(Int64Type);
/// assert_eq!(default_of(&optional), 0);
/// assert!(optional.default_scalar().is_null()); // the null variant
/// ```
pub trait TypedOptional<T>: RawOptional<Self::ValueType> + DataType<T> {
    /// The concrete value type of this optional.
    type ValueType: DataType<T>;
}
