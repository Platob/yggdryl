//! The [`RawNested`] base trait: a type composed of child fields.

use super::RawDataType;

/// The untyped surface every nested type carries: a type composed of one or more
/// child fields — e.g. `struct`, `list`, `map`, `union`.
///
/// [`child_count`](RawNested::child_count) reports how many children the type has;
/// the per-family traits ([`RawList`](crate::RawList), [`RawMap`](crate::RawMap),
/// [`RawStruct`](crate::RawStruct), [`RawUnion`](crate::RawUnion)) expose the
/// children themselves. A nested type whose values also have a native
/// representation implements the typed [`Nested`](crate::Nested).
///
/// ```
/// use yggdryl_data::{arrow_schema, DataError, RawDataType, RawNested};
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
///                 expected: "TypedStruct(a: Int32Type, b: Int32Type)".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// impl RawNested for Pair {
///     fn child_count(&self) -> usize {
///         2
///     }
/// }
///
/// assert_eq!(Pair.child_count(), 2);
/// assert_eq!(Pair.byte_width(), None);
/// assert!(Pair::from_arrow(&Pair.to_arrow()).is_ok());
/// ```
pub trait RawNested: RawDataType {
    /// The number of child fields this type contains.
    fn child_count(&self) -> usize;
}
