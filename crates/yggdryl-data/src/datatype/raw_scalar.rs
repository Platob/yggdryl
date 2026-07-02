//! The [`RawScalar`] base trait: a single, possibly-null value of a [`RawDataType`].

use super::RawDataType;

/// A single value of a data type, possibly null — the base trait mirroring an Apache
/// Arrow `Scalar`.
///
/// It carries its [`data_type`](RawScalar::data_type) of type `D`, reports whether it
/// [`is_null`](RawScalar::is_null), and exposes the native Rust
/// [`value`](RawScalar::value) (of the associated [`Value`](RawScalar::Value) type)
/// when non-null. Parameterising by `D` keeps the concrete type available for
/// zero-cost access; the associated `Value` names the in-memory representation a
/// concrete scalar holds.
///
/// ```
/// use yggdryl_data::{RawDataType, RawScalar};
///
/// struct Int32;
/// impl RawDataType for Int32 {
///     fn name(&self) -> &str { "int32" }
///     fn arrow_format(&self) -> String { "i".to_string() }
///     fn byte_width(&self) -> Option<usize> { Some(4) }
/// }
///
/// struct Int32Scalar {
///     data_type: Int32,
///     value: Option<i32>,
/// }
///
/// impl RawScalar<Int32> for Int32Scalar {
///     type Value = i32;
///     fn data_type(&self) -> &Int32 {
///         &self.data_type
///     }
///     fn is_null(&self) -> bool {
///         self.value.is_none()
///     }
///     fn value(&self) -> Option<&i32> {
///         self.value.as_ref()
///     }
/// }
///
/// let answer = Int32Scalar { data_type: Int32, value: Some(42) };
/// assert_eq!(answer.data_type().name(), "int32");
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
///
/// let missing = Int32Scalar { data_type: Int32, value: None };
/// assert!(missing.is_null());
/// assert_eq!(missing.value(), None);
/// ```
pub trait RawScalar<D: RawDataType> {
    /// The native Rust representation this scalar holds when non-null.
    type Value;

    /// The scalar's data type.
    fn data_type(&self) -> &D;

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool;

    /// The scalar's value, or `None` when it [`is_null`](RawScalar::is_null).
    fn value(&self) -> Option<&Self::Value>;
}
