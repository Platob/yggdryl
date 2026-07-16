//! Logical **reshape coercions** (`to_struct` / `to_list` / `to_map`) on the erased column — the
//! nested-layer surface that lifts any column into a nested one, or returns it unchanged when it is
//! already the target (or no coercion rule applies).
//!
//! They live here (not beside the [`AnySerie`] trait at the `io` root) because they *produce* nested
//! types: keeping them in the `nested` module leaves the root trait free of a dependency on its
//! nested children. They are inherent methods on `dyn AnySerie` (like `get_by_path`), so no per-type
//! trait impl is needed — a single dispatch on the column's `type_id` picks the rule. The typed
//! `Serie<T>` twins live here for the same reason (the `fixed` family must not name its `nested`
//! sibling, but `nested` may name `fixed`).

use crate::io::fixed::{NativeType, Serie};
use crate::io::{boxed, AnySerie, DataTypeId, IoError};

use super::{ListSerie, MapSerie, StructSerie};

impl dyn AnySerie {
    /// This column **as a one-field struct** named `name` — row `i` becomes `{name: value_i}` (the
    /// length is preserved). If the column is **already a struct** it is returned unchanged (a clone).
    /// The logical `to_struct` coercion, and the primary binding surface.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::{boxed, AnySerie, DataTypeId};
    ///
    /// let col = boxed(Serie::from_values(&[1i32, 2, 3]));
    /// let st = col.to_struct("n");
    /// assert_eq!(st.type_id(), DataTypeId::Struct);
    /// assert_eq!(st.len(), 3);
    /// ```
    pub fn to_struct(&self, name: &str) -> Box<dyn AnySerie> {
        if self.type_id() == DataTypeId::Struct {
            return self.clone_box();
        }
        let mut child = self.clone_box();
        child.set_name(name);
        Box::new(
            StructSerie::from_series(vec![child])
                .expect("a single child column is always a valid struct"),
        )
    }

    /// This column **as a list of singletons** — row `i` becomes the single-element list `[value_i]`
    /// (offsets `0, 1, 2, …, n`; the child is this column). If the column is **already a list** it is
    /// returned unchanged (a clone). The logical `to_list` coercion.
    ///
    /// DESIGN: per-element singletons (not one list wrapping the whole column) preserve the row
    /// count, matching a scalar → `[scalar]` lift, so `to_list` composes with the row-wise surface.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::{boxed, AnySerie, DataTypeId};
    ///
    /// let col = boxed(Serie::from_values(&[1i32, 2, 3]));
    /// let list = col.to_list();
    /// assert_eq!(list.type_id(), DataTypeId::List);
    /// assert_eq!(list.len(), 3);
    /// ```
    pub fn to_list(&self) -> Box<dyn AnySerie> {
        if self.type_id() == DataTypeId::List {
            return self.clone_box();
        }
        let len = self.len();
        let mut child = self.clone_box();
        child.set_name("item");
        let offsets: Vec<i32> = (0..=len as i32).collect();
        Box::new(
            ListSerie::from_values(child, &offsets, None)
                .expect("singleton offsets cover the child exactly"),
        )
    }

    /// This column **as a map**, when a logical rule applies, else itself. If the column is **already
    /// a map** it is returned unchanged; if it is a **struct with exactly two columns** it becomes a
    /// map of one entry per row (column 0 = key, column 1 = value; offsets `0, 1, …, n`); any other
    /// column is returned **unchanged** (there is no logical map coercion for it).
    ///
    /// Fallible because the struct → map build enforces Arrow's "a map key is never null" invariant:
    /// a null-bearing key column is a guided [`Unsupported`](IoError::Unsupported) error.
    ///
    /// DESIGN: only a 2-column struct has an unambiguous `key → value` reading; every other shape has
    /// none, so it passes through unchanged rather than guessing one.
    pub fn to_map(&self) -> Result<Box<dyn AnySerie>, IoError> {
        if self.type_id() == DataTypeId::Map {
            return Ok(self.clone_box());
        }
        if let Some(st) = self.as_any().downcast_ref::<StructSerie>() {
            if st.num_columns() == 2 {
                let len = st.len();
                let mut keys = st
                    .column(0)
                    .expect("column 0 of a 2-column struct")
                    .clone_box();
                let mut values = st
                    .column(1)
                    .expect("column 1 of a 2-column struct")
                    .clone_box();
                keys.set_name("key");
                values.set_name("value");
                let offsets: Vec<i32> = (0..=len as i32).collect();
                return Ok(Box::new(MapSerie::from_entries(
                    keys, values, &offsets, None, false,
                )?));
            }
        }
        // No logical map coercion for this shape — return the column unchanged.
        Ok(self.clone_box())
    }
}

/// Typed reshape conveniences on the fixed primitive column — the leaf-typed twins of the erased
/// [`dyn AnySerie::to_struct`](AnySerie) / `to_list`, returning the concrete nested column. They
/// live in the nested layer (which may name the nested types) so the `fixed` family stays free of
/// any dependency on its `nested` sibling.
impl<T: NativeType> Serie<T> {
    /// This column as a one-field [`StructSerie`] named `name` (row `i` = `{name: value_i}`).
    pub fn to_struct(&self, name: &str) -> StructSerie {
        let mut child = boxed(self.clone());
        child.set_name(name);
        StructSerie::from_series(vec![child]).expect("a single child column is a valid struct")
    }

    /// This column as a singleton [`ListSerie`] (row `i` = `[value_i]`, offsets `0, 1, …, n`).
    pub fn to_list(&self) -> ListSerie {
        let len = self.len();
        let mut child = boxed(self.clone());
        child.set_name("item");
        let offsets: Vec<i32> = (0..=len as i32).collect();
        ListSerie::from_values(child, &offsets, None).expect("singleton offsets cover the child")
    }
}
