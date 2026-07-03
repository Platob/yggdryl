//! The [`RawUnion`] base trait: the untyped surface of a union data type.

use crate::RawNested;
use arrow_schema::{UnionFields, UnionMode};

/// The untyped surface every union data type carries: its `(type id, child field)`
/// pairs and its mode — a value is exactly one of the child types, discriminated by
/// a type id.
///
/// It refines [`RawNested`] (the children are fields). The dynamic
/// [`Union`](crate::Union) implements it over arbitrary children; a
/// statically-shaped union also implements the typed [`TypedUnion`](crate::TypedUnion).
///
/// ```
/// use yggdryl_dtype::{Int64, RawUnion, Union};
///
/// let union = Union::optional(&Int64);
/// assert_eq!(union.fields().len(), 2);
/// assert_eq!(union.mode(), yggdryl_dtype::arrow_schema::UnionMode::Sparse);
/// ```
pub trait RawUnion: RawNested {
    /// The union's `(type id, child field)` pairs.
    fn fields(&self) -> &UnionFields;

    /// Whether the union is `Sparse` or `Dense`.
    fn mode(&self) -> UnionMode;
}
