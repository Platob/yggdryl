//! The [`FieldFactory`] trait: a typed data type builds its field.

use crate::TypedField;
use yggdryl_dtype::TypedDataType;

/// The generic field factory: a [`TypedDataType<T>`] that knows its concrete field
/// type and builds one.
///
/// The field layer builds on the data types, so the "data type → field" factory
/// lives here (implemented for every typed data type next to its field):
/// [`field`](FieldFactory::field) pairs a name and a nullability flag with the data
/// type. It is the counterpart of `yggdryl-scalar`'s `ScalarFactory` (data type →
/// scalar) and of [`TypedDataType::default_value`](yggdryl_dtype::TypedDataType) (data
/// type → value) — the typed data type is the model's generic factory hub.
///
/// The dynamic [`StructType`](yggdryl_dtype::StructType) and
/// [`UnionType`](yggdryl_dtype::UnionType), which are not typed data types, have no
/// factory; their fields ([`StructField`](crate::StructField),
/// [`UnionField`](crate::UnionField)) are constructed directly from a data type
/// instance.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_field::{Field, FieldFactory};
///
/// // The data type is the factory: it builds its field.
/// let id = Int64Type.field("id", false);
/// assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int64", false));
///
/// // Generic code builds a field from any typed data type.
/// fn column<T, D: FieldFactory<T>>(data_type: &D, name: &str) -> D::Field {
///     data_type.field(name, true)
/// }
/// assert!(column(&Int64Type, "score").is_nullable());
/// ```
pub trait FieldFactory<T>: TypedDataType<T> + Sized {
    /// The concrete field type of this data type.
    type Field: TypedField<Self, T>;

    /// Build the field of this data type named `name`, nullable or not.
    fn field(&self, name: impl Into<String>, nullable: bool) -> Self::Field;
}
