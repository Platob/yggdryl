//! **Nested** series — columns of other columns: [`StructSerie`] (records),
//! [`ListSerie<O>`] (variable-length lists) and [`MapSerie`] (key/value maps). Each
//! builds its child [`Serie`]s **recursively** through the [factory](crate::from_arrow),
//! so arbitrarily deep nesting (a list of structs of maps, …) resolves uniformly.
//!
//! - [`NestedSerie`] — the shared trait: child access by index ([`child`](NestedSerie::child)),
//!   by name ([`child_by_name`](NestedSerie::child_by_name), case-sensitive then
//!   case-insensitive) and by node path ([`child_path`](NestedSerie::child_path),
//!   `"a.b.c"`).

mod list_serie;
mod map_serie;
mod struct_serie;

pub use list_serie::ListSerie;
pub use map_serie::MapSerie;
pub use struct_serie::StructSerie;

use crate::path::{parse_path, Segment};
use crate::serie::{Serie, SerieRef};
use crate::SerieResult;

/// The shared interface of a nested column — access to its child column(s) by index,
/// name or node path. A concrete only supplies [`child_count`](NestedSerie::child_count)
/// and [`child`](NestedSerie::child); the name / path lookups derive from each child's
/// [`name`](Serie::name).
pub trait NestedSerie: Serie {
    /// The number of child columns (struct fields; `1` for a list; `2` for a map).
    fn child_count(&self) -> usize;

    /// The child column at `index`, or `None` if out of range.
    fn child(&self, index: usize) -> Option<SerieRef>;

    /// All child columns, in order.
    fn children(&self) -> Vec<SerieRef> {
        (0..self.child_count())
            .filter_map(|i| self.child(i))
            .collect()
    }

    /// The child named `name` **exactly** (case-sensitive), or `None`.
    fn child_named(&self, name: &str) -> Option<SerieRef> {
        (0..self.child_count())
            .filter_map(|i| self.child(i))
            .find(|c| c.name() == name)
    }

    /// The child named `name` **case-insensitively**, or `None`.
    fn child_named_ci(&self, name: &str) -> Option<SerieRef> {
        (0..self.child_count())
            .filter_map(|i| self.child(i))
            .find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// The child named `name` — an exact (case-sensitive) match, falling back to a
    /// case-insensitive one.
    fn child_by_name(&self, name: &str) -> Option<SerieRef> {
        self.child_named(name).or_else(|| self.child_named_ci(name))
    }

    /// Navigates a **node path** like `a.b.c` (or `["a.b"].c`, `tags.0`, …) into a
    /// descendant column. Each segment is matched against the current column's children;
    /// a wrapped (`[...]` / `"..."` / `'...'` / `` `...` ``) segment matches the name
    /// exactly, a bare numeric segment is a child index, and any other bare segment
    /// matches case-sensitively then case-insensitively. The path is **parsed first**,
    /// so a malformed one (unclosed wrapper, empty segment) is an `Err`; a well-formed
    /// path that does not resolve (a missing child, or a non-nested intermediate) is
    /// `Ok(None)`.
    fn child_path(&self, path: &str) -> SerieResult<Option<SerieRef>> {
        let mut segments = parse_path(path)?.into_iter();
        // First segment resolves against `self`; deeper ones against each nested child.
        let first = match segments.next() {
            Some(Segment::Index(index)) => self.child(index),
            Some(Segment::Exact(name)) => self.child_named(&name),
            Some(Segment::Loose(name)) => self.child_by_name(&name),
            None => return Ok(None),
        };
        let Some(mut current) = first else {
            return Ok(None);
        };
        for segment in segments {
            let Some(nested) = current.as_nested() else {
                return Ok(None);
            };
            match resolve(nested, &segment) {
                Some(next) => current = next,
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    }
}

/// Resolves one parsed path [`Segment`] against a nested column.
fn resolve(nested: &dyn NestedSerie, segment: &Segment) -> Option<SerieRef> {
    match segment {
        Segment::Index(index) => nested.child(*index),
        Segment::Exact(name) => nested.child_named(name),
        Segment::Loose(name) => nested.child_by_name(name),
    }
}
