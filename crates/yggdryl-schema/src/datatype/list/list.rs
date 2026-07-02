//! The variable-size list data type with 32-bit offsets.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataType, DataTypeError, DataTypeId, Field, NestedType, TypedField, TypedFieldRef};

/// A variable-size list of `T` values with 32-bit offsets, mapping to Arrow
/// `ListType` over the child field.
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, Int32Type, ListType, TypedField};
///
/// let item = Arc::new(TypedField::from_parts("item", Int32Type, true, Default::default()));
/// let list = ListType::from_parts(item);
/// assert_eq!(ListType::from_arrow(&list.to_arrow()), Ok(list.clone()));
/// assert_eq!(list.to_string(), "list<item: int32?>");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ListType<T: DataType> {
    child: TypedFieldRef<T>,
}

impl<T: DataType> ListType<T> {
    /// Builds the list type from its child field.
    pub fn from_parts(child: TypedFieldRef<T>) -> Self {
        Self { child }
    }

    /// The child field describing the list's values.
    pub fn child(&self) -> &TypedFieldRef<T> {
        &self.child
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, child: Option<TypedFieldRef<T>>) -> Self {
        Self::from_parts(child.unwrap_or_else(|| self.child.clone()))
    }

    /// Returns a copy with the child field replaced.
    pub fn with_child(&self, child: TypedFieldRef<T>) -> Self {
        self.copy(Some(child))
    }
}

impl<T: DataType> DataType for ListType<T> {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::List
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::List(Arc::new(self.child.to_arrow()))
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::List(child) => {
                Ok(Self::from_parts(Arc::new(TypedField::from_arrow(child)?)))
            }
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "list",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![DataTypeId::List.to_u8()];
        out.extend(self.child.to_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let payload = DataTypeId::List.strip_tag(bytes)?;
        Ok(Self::from_parts(Arc::new(TypedField::from_bytes(payload)?)))
    }
}

impl<T: DataType> NestedType for ListType<T> {
    fn num_children(&self) -> usize {
        1
    }
}

impl<T: DataType> fmt::Display for ListType<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "list<{}>", self.child)
    }
}
