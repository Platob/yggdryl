//! The [`RawStruct`] base trait: the untyped surface of a struct data type.

use crate::Nested;
use arrow_schema::Fields;

/// The untyped surface every struct data type carries: its ordered, named child
/// fields.
///
/// It refines [`Nested`] (the children are the fields). The dynamic
/// [`StructType`](crate::StructType) implements it over arbitrary fields; a
/// statically-shaped struct also implements the typed [`Struct`](crate::Struct).
///
/// ```
/// use yggdryl_data::{arrow_schema, RawStruct, StructType};
///
/// let point = StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
/// ]));
/// assert_eq!(point.fields().len(), 1);
/// assert_eq!(point.fields()[0].name(), "x");
/// ```
pub trait RawStruct: Nested {
    /// The struct's ordered, named child fields.
    fn fields(&self) -> &Fields;
}
