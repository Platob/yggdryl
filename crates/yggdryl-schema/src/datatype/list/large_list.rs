//! The variable-size list data type with 64-bit offsets.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, Field, FieldRef, NestedType};

/// A variable-size list of `T` values with 64-bit offsets, mapping to Arrow
/// `LargeList` over the child field.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, LargeList, Utf8};
///
/// let item = Arc::new(Field::from_parts("item", Utf8, true, Default::default()));
/// let list = LargeList::from_parts(item);
/// assert_eq!(LargeList::from_arrow(&list.to_arrow()), Ok(list.clone()));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LargeList<T: DataType> {
    child: FieldRef<T>,
}

impl<T: DataType> LargeList<T> {
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

impl<T: DataType> DataType for LargeList<T> {
    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::LargeList(Arc::new(self.child.to_arrow()))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::LargeList(child) => {
                Ok(Self::from_parts(Arc::new(Field::from_arrow(child)?)))
            }
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "large_list",
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

impl<T: DataType> NestedType for LargeList<T> {
    fn num_children(&self) -> usize {
        1
    }
}

impl<T: DataType> fmt::Display for LargeList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "large_list<{}>", self.child)
    }
}
