//! The [`Nested`] base trait: a type composed of child fields.

use super::DataType;

/// The untyped surface every nested type carries: a type composed of one or more
/// child fields — e.g. `struct`, `list`, `map`, `union`.
///
/// [`child_count`](Nested::child_count) reports how many children the type has;
/// the per-family traits ([`Serie`](crate::Serie), [`Map`](crate::Map),
/// [`Struct`](crate::Struct), [`Union`](crate::Union)) expose the children
/// themselves. A nested type whose values also have a native representation
/// implements the typed [`TypedNested`](crate::TypedNested).
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataError, DataType, Nested};
///
/// // A struct of two int32 children, `a` and `b`.
/// #[derive(Debug)]
/// struct PairType;
///
/// impl PairType {
///     fn children() -> arrow_schema::Fields {
///         arrow_schema::Fields::from(vec![
///             arrow_schema::Field::new("a", arrow_schema::DataType::Int32, false),
///             arrow_schema::Field::new("b", arrow_schema::DataType::Int32, false),
///         ])
///     }
/// }
///
/// impl DataType for PairType {
///     fn name(&self) -> &str { "struct" }
///     fn arrow_format(&self) -> String { "+s".to_string() }
///     fn byte_width(&self) -> Option<usize> { None } // nested types have no fixed width
///     fn to_arrow(&self) -> arrow_schema::DataType {
///         arrow_schema::DataType::Struct(PairType::children())
///     }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         match data_type {
///             arrow_schema::DataType::Struct(fields) if *fields == PairType::children() => Ok(PairType),
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "StructType(a: Int32Type, b: Int32Type)".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// impl Nested for PairType {
///     fn child_count(&self) -> usize {
///         2
///     }
/// }
///
/// assert_eq!(PairType.child_count(), 2);
/// assert_eq!(PairType.byte_width(), None);
/// assert!(PairType::from_arrow(&PairType.to_arrow()).is_ok());
/// ```
pub trait Nested: DataType {
    /// The number of child fields this type contains.
    fn child_count(&self) -> usize;
}
