//! The [`NestedSerie`] trait: easy child-serie access on the nested scalars.

use crate::AnySerie;

/// Child-serie access shared by every nested scalar — a value composed of child
/// columns hands them out as the crate's own [`AnySerie`] holders, zero-copy
/// (reference-count bumps, never element copies).
///
/// The children mirror the Arrow layout: a serie has one `"item"` child (its
/// elements), a map one `"entries"` child (the entries struct — with the `"key"` /
/// `"value"` projections reachable by name), and a struct / record one child per
/// field, by position and by field name. A null scalar has no child series
/// (`child_serie_at` answers `None`).
///
/// ```
/// use yggdryl_scalar::{Int64Scalar, NestedSerie, TypedSerie};
///
/// let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
/// assert_eq!(numbers.child_serie_count(), 1);
/// assert_eq!(numbers.child_serie_name_at(0).as_deref(), Some("item"));
/// assert_eq!(numbers.child_serie_at(0).unwrap().len(), 2);
/// assert_eq!(numbers.child_serie_by("item").unwrap().len(), 2);
/// assert!(numbers.child_serie_by("missing").is_none());
/// ```
pub trait NestedSerie {
    /// The number of child series this value carries.
    fn child_serie_count(&self) -> usize;

    /// The child serie at `index`, or `None` when the scalar is null or `index` is
    /// out of bounds. The handle shares the child's buffers — a reference-count
    /// bump, not a copy.
    fn child_serie_at(&self, index: usize) -> Option<AnySerie>;

    /// The name of the child at `index` (`"item"` for a serie, `"entries"` for a
    /// map, the field name for a struct), or `None` past the end.
    fn child_serie_name_at(&self, index: usize) -> Option<String>;

    /// The child serie named `name`, or `None` when the scalar is null or no child
    /// carries the name. Defaults to a scan over
    /// [`child_serie_name_at`](NestedSerie::child_serie_name_at); a type with
    /// derived projections (a map's `"key"` / `"value"`) overrides it.
    fn child_serie_by(&self, name: &str) -> Option<AnySerie> {
        (0..self.child_serie_count())
            .find(|&index| self.child_serie_name_at(index).as_deref() == Some(name))
            .and_then(|index| self.child_serie_at(index))
    }
}
