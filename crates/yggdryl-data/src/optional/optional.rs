//! The typed [`Optional`] trait: a [`RawOptional`](super::RawOptional) whose value
//! type has a codec.

use super::RawOptional;
use crate::DataType;

/// A [`RawOptional`](super::RawOptional) whose value type is a typed
/// [`DataType<T>`] — the optional's values have native Rust representation `T`.
///
/// The concrete value type is the associated [`ValueType`](Optional::ValueType), so
/// an optional has exactly one; `value_type` is inherited from
/// [`RawOptional`](super::RawOptional) and returns it. It also carries the
/// [`DataType<T>`] surface itself: the codec (and
/// [`default_value`](DataType::default_value)) delegate to the value type, while
/// [`default_scalar`](DataType::default_scalar) is the null variant — the scalar
/// models nullness, so its default matches `Option::default`.
///
/// ```
/// use yggdryl_data::{DataType, Int64, Optional, OptionalType, RawScalar};
///
/// fn default_of<T, O: Optional<T>>(optional: &O) -> T {
///     optional.default_value() // the value type's default
/// }
///
/// let optional = OptionalType::new(Int64);
/// assert_eq!(default_of(&optional), 0);
/// assert!(optional.default_scalar().is_null()); // the null variant
/// ```
pub trait Optional<T>: RawOptional<Self::ValueType> + DataType<T> {
    /// The concrete value type of this optional.
    type ValueType: DataType<T>;
}
