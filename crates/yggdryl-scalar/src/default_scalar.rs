//! The [`DefaultScalar`] trait: the scalar a data type defaults to.

use yggdryl_dtype::DataType;

/// A [`DataType<T>`] with a default [`Scalar`](DefaultScalar::Scalar) in this
/// layer — the scalar counterpart of
/// [`default_value`](yggdryl_dtype::DataType::default_value).
///
/// The scalar layer builds on the data types, never the other way around, so the
/// default *scalar* of a type lives here rather than on `DataType` itself: this
/// crate implements `DefaultScalar` for every `yggdryl-dtype` type whose scalar it
/// defines, next to that scalar. [`default_scalar`](DefaultScalar::default_scalar)
/// is a scalar holding [`default_value`](yggdryl_dtype::DataType::default_value),
/// except where the scalar itself models nullness (the optional defaults to its
/// null variant, matching the scalar's own `Default`).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{Int64 as Int64Type, List, Optional};
/// use yggdryl_scalar::{DefaultScalar, Int64, RawScalar};
///
/// // A value type's default scalar holds its default value.
/// assert_eq!(Int64Type.default_scalar(), Int64::new(0));
///
/// // Sequences default to empty, not null.
/// assert!(!List::new(Int64Type).default_scalar().is_null());
///
/// // The optional's scalar models nullness: its default is the null variant.
/// assert!(Optional::new(Int64Type).default_scalar().is_null());
/// ```
pub trait DefaultScalar<T>: DataType<T> {
    /// The scalar type this data type's defaults produce — conventionally a
    /// [`RawScalar`](crate::RawScalar) *of* this data type.
    type Scalar;

    /// The default [`Scalar`](DefaultScalar::Scalar) of this type: a scalar holding
    /// [`default_value`](yggdryl_dtype::DataType::default_value), except where the
    /// scalar itself models nullness (an optional's default scalar is its null
    /// variant, matching the scalar's own `Default`).
    fn default_scalar(&self) -> Self::Scalar;
}
