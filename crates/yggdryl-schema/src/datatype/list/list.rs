//! The variable-size list data type with 32-bit offsets.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Field, FieldRef, NestedType};

/// A variable-size list of `T` values with 32-bit offsets, mapping to Arrow
/// `List` over the child field.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, Int32, List};
///
/// let item = Arc::new(Field::from_parts("item", Int32, true, Default::default()));
/// let list = List::from_parts(item);
/// assert_eq!(List::from_arrow(&list.to_arrow()), Ok(list.clone()));
/// assert_eq!(list.to_string(), "list<item: int32?>");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct List<T: DataType> {
    child: FieldRef<T>,
}

impl<T: DataType> List<T> {
    /// Builds the list type from its child field.
    pub fn from_parts(child: FieldRef<T>) -> Self {
        Self { child }
    }

    /// The child field describing the list's values.
    pub fn child(&self) -> &FieldRef<T> {
        &self.child
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, child: Option<FieldRef<T>>) -> Self {
        Self::from_parts(child.unwrap_or_else(|| self.child.clone()))
    }

    /// Returns a copy with the child field replaced.
    pub fn with_child(&self, child: FieldRef<T>) -> Self {
        self.copy(Some(child))
    }
}

impl<T: DataType> DataType for List<T> {
    const TYPE_ID: DataTypeId = DataTypeId::List;

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::List(Arc::new(self.child.to_arrow()))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::List(child) => Ok(Self::from_parts(Arc::new(Field::from_arrow(child)?))),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "list",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.child.to_bytes()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        Ok(Self::from_parts(Arc::new(Field::from_bytes(bytes)?)))
    }
}

impl<T: DataType> NestedType for List<T> {
    fn num_children(&self) -> usize {
        1
    }
}

impl<T: DataType> fmt::Display for List<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list<{}>", self.child)
    }
}
