//! The [`StructValue`] value.

use crate::AnyValue;

/// A struct value — an array of [`AnyValue`], one per field of its
/// [`StructType`](crate::StructType). Defaults to empty.
///
/// ```
/// use yggdryl_scalar::{AnyValue, StructValue};
///
/// let row = StructValue::new(vec![AnyValue::Int32(1), AnyValue::Null]);
/// assert_eq!(row.len(), 2);
/// assert_eq!(row.get(0), Some(&AnyValue::Int32(1)));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructValue(Vec<AnyValue>);

impl StructValue {
    /// A struct value from its ordered field values.
    pub fn new(values: Vec<AnyValue>) -> Self {
        Self(values)
    }

    /// The field values, in order.
    pub fn values(&self) -> &[AnyValue] {
        &self.0
    }

    /// The value at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&AnyValue> {
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
