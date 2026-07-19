//! [`ListScalar`] — one **list element**: the child sub-range of a [`ListSerie`](super::ListSerie) at
//! a single index, materialized as an owned `Vec<`[`Value`]`>`, plus the element-level validity.

use crate::typed::nested::Value;

/// A single list element — `values[i]` is the `i`-th child of the sub-list (erased to a [`Value`]),
/// and `valid` is the element-level null flag (a **null** list is distinct from an **empty** one).
/// It owns its values, so it outlives the column it came from. `PartialEq` compares the whole list;
/// not `Eq` / `Hash`, because a [`Value`] can hold a float.
#[derive(Clone, Debug, PartialEq)]
pub struct ListScalar {
    values: Vec<Value>,
    valid: bool,
}

impl ListScalar {
    /// A list element from its child `values` and its element-level `valid` flag.
    pub fn new(values: Vec<Value>, valid: bool) -> Self {
        ListScalar { values, valid }
    }

    /// The child value at `index`, if present.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// The number of children in this sub-list.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the sub-list has no children (an empty — not necessarily null — list).
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Whether the **list element itself** is null (as opposed to a valid empty list).
    pub fn is_null(&self) -> bool {
        !self.valid
    }

    /// Whether the **list element itself** is valid (non-null).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// The child values in order (borrowed).
    pub fn values(&self) -> &[Value] {
        &self.values
    }
}
