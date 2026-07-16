//! Erased **child-column mutation** on the nested column — [`set_child_at`](AnySerie::set_child_at) /
//! [`set_child_by`](AnySerie::set_child_by) replace (or, for a struct, add-or-replace) one child
//! column of a struct / list / map in place. The uniform, binding-facing surface: the caller passes a
//! borrowed `&dyn AnySerie` child (matching how [`concat`](AnySerie::concat) takes a borrow) and the
//! setter clones it in.
//!
//! They live here (not beside the [`AnySerie`] trait at the `io` root) because they *name* the nested
//! column types to reach their `pub(crate)` swap primitives ([`StructSerie::replace_column`] /
//! [`StructSerie::set_or_add_column`], [`ListSerie::replace_item`], [`MapSerie::replace_keys`] /
//! [`MapSerie::replace_values`]) — keeping them in the `nested` module leaves the root trait free of a
//! dependency on its nested children. They are inherent methods on `dyn AnySerie` (like the
//! [`reshape`](super::reshape) coercions), so one dispatch on the column's `type_id` picks the family
//! with no per-type trait impl.
//!
//! DESIGN: the crate exposes **only** these two erased entry points, not a wide public per-type setter
//! surface (no public `set_column_at` / `add_column` / `remove_*`) — the concrete swap logic stays
//! `pub(crate)` so every mutation still passes through the length / non-null invariant guards. Because a
//! struct schema is *derived* from its columns, replacing or adding a column updates the derived
//! [`field`](AnySerie::field) automatically, and the whole column still round-trips through
//! serialize / deserialize.

use crate::io::{AnySerie, DataTypeId, IoError};

use super::{ListSerie, MapSerie, StructSerie};

impl dyn AnySerie {
    /// **Replaces** a nested child column at position `index`, in place — the erased, uniform child
    /// setter. Dispatches on the nested kind:
    ///
    /// - **struct** → replaces `columns[index]` (guided [`IndexOutOfBounds`](IoError::IndexOutOfBounds)
    ///   past the last column; the new child's `len()` must equal the struct's row count, else a guided
    ///   length error naming both). The slot's schema **name** is preserved (a positional replace
    ///   changes the column's type + data, not its name).
    /// - **list** → index `0` replaces the flattened item child (its `len()` must equal the current
    ///   flattened length `offsets[last]`); any other index is a guided error.
    /// - **map** → index `0` replaces the keys column (must stay non-null, length == entries), `1`
    ///   replaces the values column (length == entries); any other index is a guided error.
    ///
    /// A non-nested (**leaf**) column is a guided error (set a leaf cell with
    /// [`set_cell`](AnySerie::set_cell) / [`set_by_path`](AnySerie::set_by_path) instead). The `child`
    /// is borrowed and cloned in (matching [`concat`](AnySerie::concat)).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::var::Utf8Serie;
    /// use yggdryl_core::io::{boxed, AnySerie, DataTypeId};
    /// use yggdryl_core::io::nested::StructSerie;
    ///
    /// // struct<id: i64, name: utf8>; replace the `id` column (col 0) with an i32 column of the same rows.
    /// let mut table: Box<dyn AnySerie> = boxed(StructSerie::from_named(vec![
    ///     ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
    ///     ("name", boxed(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]))),
    /// ])
    /// .unwrap());
    /// let replacement = boxed(Serie::from_values(&[10i32, 20, 30]));
    /// table.set_child_at(0, replacement.as_ref()).unwrap();
    /// // The derived schema reflects the change: col 0 keeps its name but is now i32.
    /// let field = table.field("table");
    /// assert_eq!(field.child_field_at(0).unwrap().name(), "id");
    /// assert_eq!(field.child_field_at(0).unwrap().type_id(), DataTypeId::I32);
    /// ```
    pub fn set_child_at(&mut self, index: usize, child: &dyn AnySerie) -> Result<(), IoError> {
        match self.type_id() {
            DataTypeId::Struct => struct_mut(self).replace_column(index, child.clone_box()),
            DataTypeId::List => {
                if index != 0 {
                    return Err(list_child_index(index));
                }
                list_mut(self).replace_item(child.clone_box())
            }
            DataTypeId::Map => match index {
                0 => map_mut(self).replace_keys(child.clone_box()),
                1 => map_mut(self).replace_values(child.clone_box()),
                other => Err(map_child_index(other)),
            },
            other => Err(not_nested(other)),
        }
    }

    /// **Adds or replaces** a nested child column by `name`, in place — the erased, name-keyed child
    /// setter. Dispatches on the nested kind:
    ///
    /// - **struct** → dict-like add-or-replace: if a column has that `name` its type + data are
    ///   replaced, else the child is **added** as a new field carrying `name` (its `len()` must equal
    ///   the struct's row count — a field-less empty struct adopts the child's length).
    /// - **map** → `"key"` (or the key child's own name) replaces the keys column, `"value"` (or the
    ///   value child's own name) replaces the values column; any other name is a guided error naming
    ///   the two.
    /// - **list** → `"item"` (or the item child's own name) replaces the item child; any other name is
    ///   a guided error.
    ///
    /// A non-nested (**leaf**) column is a guided error. The `child` is borrowed and cloned in.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    /// use yggdryl_core::io::{boxed, AnySerie};
    /// use yggdryl_core::io::nested::StructSerie;
    ///
    /// // Add a brand-new `score` column to a one-field struct (dict-like add).
    /// let mut table: Box<dyn AnySerie> =
    ///     boxed(StructSerie::from_named(vec![("id", boxed(Serie::from_values(&[1i64, 2])))]).unwrap());
    /// table.set_child_by("score", boxed(Serie::from_values(&[9i32, 8])).as_ref()).unwrap();
    /// assert_eq!(table.num_children(), 2);
    /// assert_eq!(table.field("t").child_field_at(1).unwrap().name(), "score");
    /// ```
    pub fn set_child_by(&mut self, name: &str, child: &dyn AnySerie) -> Result<(), IoError> {
        match self.type_id() {
            DataTypeId::Struct => struct_mut(self).set_or_add_column(name, child.clone_box()),
            DataTypeId::List => {
                let list = list_mut(self);
                if name == list.values().name() || name == "item" {
                    list.replace_item(child.clone_box())
                } else {
                    Err(list_child_name(name))
                }
            }
            DataTypeId::Map => {
                let map = map_mut(self);
                if name == map.keys().name() || name == "key" {
                    map.replace_keys(child.clone_box())
                } else if name == map.values().name() || name == "value" {
                    map.replace_values(child.clone_box())
                } else {
                    Err(map_child_name(name))
                }
            }
            other => Err(not_nested(other)),
        }
    }
}

/// Recovers the concrete `&mut StructSerie` behind a column whose `type_id` is already `Struct`.
fn struct_mut(column: &mut dyn AnySerie) -> &mut StructSerie {
    column
        .as_any_mut()
        .downcast_mut::<StructSerie>()
        .expect("Struct type_id names a StructSerie")
}

/// Recovers the concrete `&mut ListSerie` behind a column whose `type_id` is already `List`.
fn list_mut(column: &mut dyn AnySerie) -> &mut ListSerie {
    column
        .as_any_mut()
        .downcast_mut::<ListSerie>()
        .expect("List type_id names a ListSerie")
}

/// Recovers the concrete `&mut MapSerie` behind a column whose `type_id` is already `Map`.
fn map_mut(column: &mut dyn AnySerie) -> &mut MapSerie {
    column
        .as_any_mut()
        .downcast_mut::<MapSerie>()
        .expect("Map type_id names a MapSerie")
}

/// The guided error for a child setter on a **leaf** column — a leaf has no child columns.
fn not_nested(id: DataTypeId) -> IoError {
    IoError::Unsupported {
        what: format!(
            "cannot set a child column on a {} leaf column; only a struct / list / map column has \
             child columns — overwrite a leaf cell with set_cell / set_by_path instead",
            id.name()
        ),
    }
}

/// The guided error for a **list** child index other than `0` (a list has one item child).
fn list_child_index(index: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a list column has a single child at index 0 (the flattened item child); got index \
             {index} — use set_child_at(0, ..) or set_child_by(\"item\", ..)"
        ),
    }
}

/// The guided error for a **list** child name other than the item child's.
fn list_child_name(name: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a list column has one child named \"item\" (the flattened item child); no child named \
             {name:?} — use set_child_by(\"item\", ..) or set_child_at(0, ..)"
        ),
    }
}

/// The guided error for a **map** child index other than `0` / `1`.
fn map_child_index(index: usize) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a map column has two child columns: index 0 is the keys column, index 1 is the values \
             column; got index {index}"
        ),
    }
}

/// The guided error for a **map** child name other than `"key"` / `"value"`.
fn map_child_name(name: &str) -> IoError {
    IoError::Unsupported {
        what: format!(
            "a map column has a \"key\" child and a \"value\" child; no child named {name:?} — use \
             set_child_by(\"key\", ..) or set_child_by(\"value\", ..)"
        ),
    }
}
