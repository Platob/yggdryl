//! The [`ScalarFactory`] trait: a typed data type builds its scalar.

use crate::Scalar;
use yggdryl_dtype::TypedDataType;

/// The generic scalar factory: a [`TypedDataType<T>`] that knows its concrete scalar
/// type and builds one — from a native value, as null, or as the default.
///
/// The scalar layer builds on the data types, so the "data type → scalar" factory
/// lives here (implemented for every typed data type next to its scalar). It is the
/// counterpart of `yggdryl-field`'s `FieldFactory` (data type → field) and of
/// [`TypedDataType::default_value`](yggdryl_dtype::TypedDataType) (data type → value)
/// — the typed data type is the model's generic factory hub.
///
/// - [`scalar`](ScalarFactory::scalar) builds a scalar holding a native `value`.
/// - [`null_scalar`](ScalarFactory::null_scalar) builds the null scalar.
/// - [`default_scalar`](ScalarFactory::default_scalar) builds the type's default
///   scalar: a scalar holding
///   [`default_value`](yggdryl_dtype::TypedDataType::default_value), except where the
///   scalar itself models nullness (an optional's default scalar is its null
///   variant, matching the scalar's own `Default`).
///
/// The dynamic [`StructType`](yggdryl_dtype::StructType) and
/// [`UnionType`](yggdryl_dtype::UnionType), which are not typed data types, have no
/// factory; their scalars are constructed directly.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{Int64Type, SerieType, OptionalType};
/// use yggdryl_scalar::{Int64Scalar, Scalar, ScalarFactory};
///
/// // The data type is the factory: it builds scalars from values, null, or default.
/// assert_eq!(Int64Type.scalar(42), Int64Scalar::new(42));
/// assert!(Int64Type.null_scalar().is_null());
/// assert_eq!(Int64Type.default_scalar(), Int64Scalar::new(0));
///
/// // Sequences default to empty, not null; the optional's default is its null variant.
/// assert!(!SerieType::new(Int64Type).default_scalar().is_null());
/// assert!(OptionalType::new(Int64Type).default_scalar().is_null());
///
/// // Generic code builds a scalar from any typed data type.
/// fn one<T, D: ScalarFactory<T>>(data_type: &D, value: T) -> D::Scalar {
///     data_type.scalar(value)
/// }
/// assert_eq!(one(&Int64Type, 7), Int64Scalar::new(7));
/// ```
pub trait ScalarFactory<T>: TypedDataType<T> + Sized {
    /// The concrete scalar type of this data type.
    type Scalar: Scalar<DataType = Self>;

    /// Build a scalar of this data type holding the native `value`.
    fn scalar(&self, value: T) -> Self::Scalar;

    /// Build the null scalar of this data type.
    fn null_scalar(&self) -> Self::Scalar;

    /// Build this data type's default scalar — a scalar holding
    /// [`default_value`](yggdryl_dtype::TypedDataType::default_value), except where the
    /// scalar models nullness (an optional defaults to its null variant).
    fn default_scalar(&self) -> Self::Scalar;
}
