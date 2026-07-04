//! The [`Serie`] base trait: the untyped surface of a serie data type.

use crate::Nested;
use arrow_schema::FieldRef;

/// The untyped surface every serie data type carries: a variable-length sequence of
/// one value type, exposing that value type's Arrow `"item"` child.
///
/// It refines [`Nested`] (the single child is the item field). The dynamic
/// [`SerieType`](crate::SerieType) implements it over an arbitrary value type; a
/// statically-typed serie also implements the typed [`TypedSerie`](crate::TypedSerie)
/// (via [`TypedSerieType<D>`](crate::TypedSerieType)), which adds the concrete
/// value-type accessor and the byte codec. This mirrors the dynamic
/// [`StructType`](crate::StructType) / [`Struct`](crate::Struct) split.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, Nested, Serie, SerieType};
///
/// let serie = SerieType::new(arrow_schema::DataType::Int64);
/// assert_eq!(serie.item_field().name(), "item");
/// assert_eq!(serie.child_count(), 1);
/// ```
pub trait Serie: Nested {
    /// The list's single Arrow child: the nullable `"item"` field of the value type.
    ///
    /// Returned by value (a reference-counted [`FieldRef`]), since a typed serie
    /// builds it from its value type rather than storing it — unlike
    /// [`Struct::fields`](crate::Struct::fields), which the dynamic struct stores and
    /// returns by reference.
    fn item_field(&self) -> FieldRef;
}
