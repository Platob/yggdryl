//! The [`Struct`] value.

use crate::value::Any;

/// A struct value — an array of [`Any`], one per field of its
/// [`StructType`](crate::StructType). Defaults to empty.
///
/// ```
/// use yggdryl_schema::{Any, Struct};
///
/// let row = Struct::new(vec![Any::Int32(1), Any::Null]);
/// assert_eq!(row.len(), 2);
/// assert_eq!(row.get(0), Some(&Any::Int32(1)));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Struct(Vec<Any>);

impl Struct {
    /// A struct value from its ordered field values.
    pub fn new(values: Vec<Any>) -> Self {
        Self(values)
    }

    /// The field values, in order.
    pub fn values(&self) -> &[Any] {
        &self.0
    }

    /// The value at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&Any> {
        self.0.get(index)
    }

    /// The number of field values.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the struct holds no field values.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
