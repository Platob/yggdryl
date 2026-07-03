//! The typed [`TypedField`] trait: a [`Field`](super::Field) of a
//! [`TypedDataType<T>`](yggdryl_dtype::TypedDataType).

use super::Field;
use yggdryl_dtype::TypedDataType;

/// A [`Field<DT>`](super::Field) whose data type `DT` is a typed
/// [`TypedDataType<T>`](yggdryl_dtype::TypedDataType) — the field's values have
/// native Rust representation `T`.
///
/// The data type `DT` and native type `T` are explicit generic parameters (a typed
/// field has exactly one of each); `data_type` is inherited from
/// [`Field`](super::Field) and returns the `DT`. This keeps the surface aligned with
/// `yggdryl-scalar`'s `TypedScalar<DT, T>` and
/// [`TypedDataType<T>`](yggdryl_dtype::TypedDataType).
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataError, DataType, Int64Type};
/// use yggdryl_field::{arrow_schema, Field, TypedField};
///
/// #[derive(Debug)]
/// struct Column {
///     name: String,
///     data_type: Int64Type,
///     nullable: bool,
/// }
///
/// impl Field<Int64Type> for Column {
///     fn name(&self) -> &str {
///         &self.name
///     }
///     fn data_type(&self) -> &Int64Type {
///         &self.data_type
///     }
///     fn is_nullable(&self) -> bool {
///         self.nullable
///     }
///     fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
///         if let Some(extension) = field.metadata().get("ARROW:extension:name") {
///             return Err(DataError::IncompatibleArrowType {
///                 expected: "Int64Type".to_string(),
///                 got: format!("the extension type \"{extension}\""),
///             });
///         }
///         Ok(Column {
///             name: field.name().to_string(),
///             data_type: Int64Type::from_arrow(field.data_type())?,
///             nullable: field.is_nullable(),
///         })
///     }
/// }
///
/// impl TypedField<Int64Type, i64> for Column {}
///
/// let id = Column { name: "id".to_string(), data_type: Int64Type, nullable: false };
/// assert_eq!(id.name(), "id");
/// assert_eq!(id.data_type().name(), "int64");
/// ```
pub trait TypedField<DT: TypedDataType<T>, T>: Field<DT> {}
