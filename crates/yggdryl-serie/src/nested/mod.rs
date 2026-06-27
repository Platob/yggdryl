//! **Nested** series — columns of other columns: [`StructSerie`] (records),
//! [`ListSerie<O>`] (variable-length lists) and [`MapSerie`] (key/value maps). Each
//! builds its child [`Serie`]s **recursively** through the [factory](crate::from_arrow),
//! so arbitrarily deep nesting (a list of structs of maps, …) resolves uniformly.
//!
//! - [`NestedSerie`] — the shared trait: `child_count` / `child(index)` / `child_by_name`.

mod list_serie;
mod map_serie;
mod struct_serie;

pub use list_serie::ListSerie;
pub use map_serie::MapSerie;
pub use struct_serie::StructSerie;

use crate::serie::{Serie, SerieRef};

/// The shared interface of a nested column — access to its child column(s).
pub trait NestedSerie: Serie {
    /// The number of child columns (struct fields; `1` for a list; `2` for a map).
    fn child_count(&self) -> usize;

    /// The child column at `index`, or `None` if out of range.
    fn child(&self, index: usize) -> Option<SerieRef>;

    /// The child column named `name`, or `None`. Meaningful for [`StructSerie`] (field
    /// names) and [`MapSerie`] (`"key"` / `"value"`); `None` by default.
    fn child_by_name(&self, _name: &str) -> Option<SerieRef> {
        None
    }
}
