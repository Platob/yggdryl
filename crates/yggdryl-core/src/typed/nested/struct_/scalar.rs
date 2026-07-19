//! [`StructScalar`] — one **struct row**: the erased [`Value`]s of a struct's children at a single
//! index, plus the row-level validity and the child names for
//! [`get_by_name`](StructScalar::get_by_name).

use crate::typed::nested::Value;

/// A single struct row — `values[i]` is child `i`'s element (erased to a [`Value`]), `valid` is the
/// row-level null flag, and `names[i]` is child `i`'s name (so a value can be read back by name).
/// `PartialEq` compares the whole row; not `Eq` / `Hash`, because a [`Value`] can hold a float.
#[derive(Clone, Debug, PartialEq)]
pub struct StructScalar {
    names: Vec<Box<str>>,
    values: Vec<Value>,
    valid: bool,
}

impl StructScalar {
    /// A row from its child `names`, child `values`, and row-level `valid` flag. `names` and `values`
    /// are parallel (`names[i]` labels `values[i]`).
    pub fn new(names: Vec<Box<str>>, values: Vec<Value>, valid: bool) -> Self {
        StructScalar {
            names,
            values,
            valid,
        }
    }

    /// The child value at `index`, if present.
    pub fn get(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// The first child value named `name`, if any.
    pub fn get_by_name(&self, name: &str) -> Option<&Value> {
        self.names
            .iter()
            .position(|child| child.as_ref() == name)
            .and_then(|index| self.values.get(index))
    }

    /// Whether the **row itself** is null.
    pub fn is_null(&self) -> bool {
        !self.valid
    }

    /// Whether the **row itself** is valid (non-null).
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// The number of child values.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the row has no children.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The child values in order (borrowed).
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// The name of child `index`, if present.
    pub fn name(&self, index: usize) -> Option<&str> {
        self.names.get(index).map(|name| name.as_ref())
    }
}
