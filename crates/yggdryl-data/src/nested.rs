//! The [`Nested`] category trait: a type composed of child fields.

use super::RawDataType;

/// A nested type composed of one or more child fields — e.g. `struct`, `list`, `map`.
///
/// [`child_count`](Nested::child_count) reports how many children the type has. Typed
/// child accessors — which must span children of differing data types — land with the
/// concrete nested types as the layer grows.
///
/// ```
/// use yggdryl_data::{arrow_schema, DataError, Nested, RawDataType};
///
/// // A struct of two int32 children, `a` and `b`.
/// #[derive(Debug)]
/// struct Pair;
///
/// impl Pair {
///     fn children() -> arrow_schema::Fields {
///         arrow_schema::Fields::from(vec![
///             arrow_schema::Field::new("a", arrow_schema::DataType::Int32, false),
///             arrow_schema::Field::new("b", arrow_schema::DataType::Int32, false),
///         ])
///     }
/// }
///
/// impl RawDataType for Pair {
///     fn name(&self) -> &str { "struct" }
///     fn arrow_format(&self) -> String { "+s".to_string() }
///     fn byte_width(&self) -> Option<usize> { None } // nested types have no fixed width
///     fn to_arrow(&self) -> arrow_schema::DataType {
///         arrow_schema::DataType::Struct(Pair::children())
///     }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         match data_type {
///             arrow_schema::DataType::Struct(fields) if *fields == Pair::children() => Ok(Pair),
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "Struct(a: Int32, b: Int32)".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// impl Nested for Pair {
///     fn child_count(&self) -> usize {
///         2
///     }
/// }
///
/// assert_eq!(Pair.child_count(), 2);
/// assert_eq!(Pair.byte_width(), None);
/// assert!(Pair::from_arrow(&Pair.to_arrow()).is_ok());
/// ```
pub trait Nested: RawDataType {
    /// The number of child fields this type contains.
    fn child_count(&self) -> usize;
}
