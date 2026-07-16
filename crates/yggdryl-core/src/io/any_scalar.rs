//! [`AnyScalar`] — a single, type-erased **cell value**: null, a leaf value's raw little-endian
//! bytes (tagged with its logical [`Field`](crate::io::fixed::Field)), a nested struct row, or — for
//! a list / map cell — the row's elements as an erased sub-[`Serie`](crate::io::AnySerie). It is what
//! an erased [`AnySerie::value`](crate::io::AnySerie::value) returns. Family-agnostic, so it lives at
//! the `io` root.

use super::fixed::Field;
use super::{AnySerie, DataTypeId, FieldType, NodePath, PathError, PathSegment};

/// One **type-erased value** — the cell of an erased [`AnySerie`](crate::io::AnySerie).
///
/// A leaf value carries its canonical little-endian bytes plus the logical
/// [`Field`](crate::io::fixed::Field) naming its type (a fixed value is `field.byte_width()` bytes; a
/// var value is its slice). A [`Struct`](AnyScalar::Struct) value is a whole nested row (its
/// per-field values). A [`List`](AnyScalar::List) / [`Map`](AnyScalar::Map) value holds *its own
/// elements* as an erased sub-column (`Box<dyn AnySerie>`) — a list scalar falls back on our
/// [`Serie`](crate::io::AnySerie), so it needs no dependency on a dedicated list column type. A
/// hashable value type — usable as a map/set key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyScalar {
    /// A null cell.
    Null,
    /// A present leaf value — its canonical little-endian bytes + the logical field naming its type.
    Leaf {
        /// The logical type of the value (id, width, and decimal params in metadata).
        field: Field,
        /// The value's canonical little-endian bytes.
        bytes: Vec<u8>,
    },
    /// A present nested struct value — its per-field cell values, in field order.
    Struct(Vec<AnyScalar>),
    /// A present **list** value — the row's item elements as an erased sub-column.
    List(Box<dyn AnySerie>),
    /// A present **map** value — the row's `key -> value` entries as an erased
    /// `StructSerie(key, value)` sub-column, plus whether the entries are sorted by key.
    Map {
        /// The `key -> value` entries as an erased struct sub-column.
        entries: Box<dyn AnySerie>,
        /// Whether the entries are sorted by key.
        keys_sorted: bool,
    },
}

// A manual `Hash` (not a derive): a `Box<dyn AnySerie>` is not `Hash`, so the `List`/`Map` variants
// hash over the sub-column's canonical bytes instead. This stays in lock-step with `PartialEq` — two
// erased columns that compare equal (`eq_any`) are byte-canonical and so serialize to equal bytes, so
// equal values hash equal. The other variants hash exactly as a derive would.
//
// DESIGN: hashing a list/map cell allocates its sub-column's frame once. A list value is a whole
// column, not a small scalar, so the "no per-op allocation" rule (which targets flat leaf values)
// does not apply here; keeping identity byte-canonical is the priority.
impl core::hash::Hash for AnyScalar {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Null => {}
            Self::Leaf { field, bytes } => {
                field.hash(state);
                bytes.hash(state);
            }
            Self::Struct(values) => values.hash(state),
            Self::List(serie) => serie.serialize_bytes().hash(state),
            Self::Map {
                entries,
                keys_sorted,
            } => {
                entries.serialize_bytes().hash(state);
                keys_sorted.hash(state);
            }
        }
    }
}

impl AnyScalar {
    /// A present leaf value from its logical field and canonical bytes.
    pub fn leaf(field: Field, bytes: Vec<u8>) -> Self {
        Self::Leaf { field, bytes }
    }

    /// A present struct value from its per-field cell values.
    pub fn struct_(values: Vec<AnyScalar>) -> Self {
        Self::Struct(values)
    }

    /// A present **list** value from the row's item elements as an erased sub-column.
    pub fn list(items: Box<dyn AnySerie>) -> Self {
        Self::List(items)
    }

    /// A present **map** value from the row's `key -> value` entries (an erased `StructSerie(key,
    /// value)` sub-column) and whether they are sorted by key.
    pub fn map(entries: Box<dyn AnySerie>, keys_sorted: bool) -> Self {
        Self::Map {
            entries,
            keys_sorted,
        }
    }

    /// The null value.
    pub fn null() -> Self {
        Self::Null
    }

    /// Whether the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Whether the value is present (non-null).
    pub fn is_valid(&self) -> bool {
        !self.is_null()
    }

    /// The value's element [`DataTypeId`], or `None` if null.
    pub fn type_id(&self) -> Option<DataTypeId> {
        match self {
            Self::Null => None,
            Self::Leaf { field, .. } => Some(FieldType::type_id(field)),
            Self::Struct(_) => Some(DataTypeId::Struct),
            Self::List(_) => Some(DataTypeId::List),
            Self::Map { .. } => Some(DataTypeId::Map),
        }
    }

    /// A present leaf value's raw bytes, or `None` if null or a nested value.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Leaf { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    /// A present struct value's per-field cell values, or `None`.
    pub fn as_struct(&self) -> Option<&[AnyScalar]> {
        match self {
            Self::Struct(values) => Some(values),
            _ => None,
        }
    }

    /// A present list value's item elements as an erased sub-column, or `None`.
    pub fn as_list(&self) -> Option<&(dyn AnySerie + 'static)> {
        match self {
            Self::List(items) => Some(items.as_ref()),
            _ => None,
        }
    }

    /// A present map value's `key -> value` entries (as an erased sub-column) and its `keys_sorted`
    /// flag, or `None`.
    pub fn as_map(&self) -> Option<(&(dyn AnySerie + 'static), bool)> {
        match self {
            Self::Map {
                entries,
                keys_sorted,
            } => Some((entries.as_ref(), *keys_sorted)),
            _ => None,
        }
    }

    // ---- unified child access (symmetric with `AnySerie` / `AnyField`, one level of drill-down) --

    /// The number of **child values** one level down: a struct's per-field values, a list's elements,
    /// or a map's entries. A leaf / null value has none.
    ///
    /// DESIGN: a *value*'s children are its **data** one level in (elements / entries / field
    /// values), whereas a [column](crate::io::AnySerie)'s or [field](crate::io::AnyField)'s children
    /// are its **schema** structure (the item / key / value *types*). So a list value has one child
    /// per element (not one "item" child), and a map value one child per entry — the drill-down axis
    /// a caller walks a *value* along.
    pub fn num_children(&self) -> usize {
        match self {
            Self::Null | Self::Leaf { .. } => 0,
            Self::Struct(values) => values.len(),
            Self::List(items) => items.len(),
            Self::Map { entries, .. } => entries.len(),
        }
    }

    /// The child value at `index`, or `None` if out of range (a leaf / null has none) — a struct's
    /// `index`-th field value, a list's `index`-th element, or a map's `index`-th entry (a
    /// `struct{key, value}` value). Owned, because a list/map child is the erased sub-column's cell.
    pub fn child_scalar_at(&self, index: usize) -> Option<AnyScalar> {
        match self {
            Self::Null | Self::Leaf { .. } => None,
            Self::Struct(values) => values.get(index).cloned(),
            Self::List(items) => (index < items.len()).then(|| items.value(index)),
            Self::Map { entries, .. } => (index < entries.len()).then(|| entries.value(index)),
        }
    }

    /// The child value named `name`, or `None`. Only a [`Struct`](AnyScalar::Struct) value whose
    /// field values carry their schema name (a **named-leaf** child) resolves by name.
    ///
    /// DESIGN: the erased struct value is a *positional* `Vec<AnyScalar>` (Phase 1 keeps it
    /// name-free), so a struct value built by [`AnySerie::value`](crate::io::AnySerie::value) — whose
    /// leaf cells carry an empty name — resolves only by **index**; name resolution succeeds only for
    /// a value whose leaf children were constructed with their names. A list's elements and a map's
    /// entries are inherently unnamed, so they never resolve by name (use an index). Callers drilling
    /// a value therefore favour index segments; the serie/field surfaces carry the names.
    pub fn child_scalar_by(&self, name: &str) -> Option<AnyScalar> {
        match self {
            Self::Struct(values) => values.iter().find_map(|value| match value {
                Self::Leaf { field, .. } if field.name() == name => Some(value.clone()),
                _ => None,
            }),
            Self::Null | Self::Leaf { .. } | Self::List(_) | Self::Map { .. } => None,
        }
    }

    /// Resolves `path` against this value's nested data, returning the addressed **child value** — the
    /// data-drill-down symmetric with [`AnySerie::get_by_path`](crate::io::AnySerie::get_by_path). Each
    /// [`Name`](crate::io::PathSegment::Name) segment follows [`child_scalar_by`](AnyScalar::child_scalar_by),
    /// each [`Index`](crate::io::PathSegment::Index) segment follows
    /// [`child_scalar_at`](AnyScalar::child_scalar_at); the empty path returns a clone of this value.
    ///
    /// Because a value's children are its data (see [`num_children`](AnyScalar::num_children)), a path
    /// into a value indexes **elements / entries / field values** — e.g. `[0][2]` is "element 0 of
    /// field 0" — not the schema `[item]` a serie/field path walks.
    ///
    /// # Errors
    /// A [`PathError`] from [`NodePath::parse`](crate::io::NodePath::parse), or a
    /// [`PathError::NoChildNamed`] / [`PathError::ChildIndexOutOfRange`] naming the depth and the
    /// missing segment.
    pub fn get_by_path(&self, path: &str) -> Result<AnyScalar, PathError> {
        let parsed = NodePath::parse(path)?;
        let mut current = self.clone();
        for (depth, segment) in parsed.segments().iter().enumerate() {
            current = match segment {
                PathSegment::Name(name) => {
                    current
                        .child_scalar_by(name)
                        .ok_or_else(|| PathError::NoChildNamed {
                            depth,
                            name: name.clone(),
                            num_children: current.num_children(),
                        })?
                }
                PathSegment::Index(index) => current.child_scalar_at(*index).ok_or_else(|| {
                    PathError::ChildIndexOutOfRange {
                        depth,
                        index: *index,
                        num_children: current.num_children(),
                    }
                })?,
            };
        }
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::fixed::Serie;
    use crate::io::{boxed, AnyScalar};

    #[test]
    fn list_and_map_cells_report_their_type_and_are_hashable() {
        use std::collections::HashSet;

        let a = AnyScalar::list(boxed(Serie::from_values(&[1i32, 2, 3])));
        let b = AnyScalar::list(boxed(Serie::from_values(&[1i32, 2, 3])));
        let c = AnyScalar::list(boxed(Serie::from_values(&[9i32])));
        assert_eq!(a.type_id(), Some(DataTypeId::List));
        assert!(!a.is_null());
        assert_eq!(a, b); // equality compares the erased sub-Series
        assert_ne!(a, c);
        assert_eq!(a.as_list().unwrap().len(), 3);
        assert!(a.bytes().is_none() && a.as_struct().is_none());

        // Equal list cells hash equal -> usable as set/map keys.
        let set: HashSet<AnyScalar> = [a, b, c].into_iter().collect();
        assert_eq!(set.len(), 2);

        let map = AnyScalar::map(boxed(Serie::from_values(&[10i64, 20])), true);
        assert_eq!(map.type_id(), Some(DataTypeId::Map));
        let (entries, keys_sorted) = map.as_map().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(keys_sorted);
    }
}
