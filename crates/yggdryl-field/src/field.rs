//! The typed [`Field`] trait: a [`RawField`](super::RawField) of a
//! [`DataType<T>`](yggdryl_dtype::DataType).

use super::RawField;
use yggdryl_dtype::DataType;

/// A [`RawField`](super::RawField) whose data type is a typed
/// [`DataType<T>`](yggdryl_dtype::DataType) — the field's values have native Rust
/// representation `T`.
///
/// The concrete data type is the associated [`Type`](Field::Type), so a field has
/// exactly one; `data_type` is inherited from [`RawField`](super::RawField) and
/// returns it. Parameterising by the native type `T` (rather than the data type)
/// keeps the surface aligned with `yggdryl-scalar`'s `Scalar<T>` and
/// [`DataType<T>`](yggdryl_dtype::DataType).
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataError, Int64, RawDataType};
/// use yggdryl_field::{arrow_schema, Field, RawField};
///
/// #[derive(Debug)]
/// struct Column {
///     name: String,
///     data_type: Int64,
///     nullable: bool,
/// }
///
/// impl RawField<Int64> for Column {
///     fn name(&self) -> &str {
///         &self.name
///     }
///     fn data_type(&self) -> &Int64 {
///         &self.data_type
///     }
///     fn is_nullable(&self) -> bool {
///         self.nullable
///     }
///     fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
///         // An extension type is a different logical type riding on metadata.
///         if let Some(extension) = field.metadata().get("ARROW:extension:name") {
///             return Err(DataError::IncompatibleArrowType {
///                 expected: "Int64".to_string(),
///                 got: format!("the extension type \"{extension}\""),
///             });
///         }
///         Ok(Column {
///             name: field.name().to_string(),
///             data_type: Int64::from_arrow(field.data_type())?,
///             nullable: field.is_nullable(),
///         })
///     }
/// }
///
/// impl Field<i64> for Column {
///     type Type = Int64;
/// }
///
/// let id = Column { name: "id".to_string(), data_type: Int64, nullable: false };
/// assert_eq!(id.name(), "id");
/// assert_eq!(id.data_type().name(), "int64");
/// ```
pub trait Field<T>: RawField<Self::Type> {
    /// The concrete data type of this field.
    type Type: DataType<T>;
}
