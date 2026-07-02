//! The struct data type.

use core::fmt;
use std::sync::Arc;

use arrow_schema::DataType as ArrowDataType;

use crate::bytes::{put_len, Reader};
use crate::{
    AnyDataType, DataType, DataTypeError, DataTypeId, Field, NestedType, TypedField, TypedFieldRef,
};

/// A composite of named child fields, mapping to Arrow `Struct`. Children are
/// heterogeneous, so each is a field over the erased [`AnyDataType`].
///
/// ```
/// use std::sync::Arc;
/// use yggdryl_schema::{DataType, Field, Int32Type, StructType, TypedField, Utf8Type};
///
/// let person = StructType::from_parts(vec![
///     Arc::new(TypedField::from_parts("id", Int32Type.into(), false, Default::default())),
///     Arc::new(TypedField::from_parts("name", Utf8Type.into(), true, Default::default())),
/// ]);
/// assert_eq!(StructType::from_arrow(&person.to_arrow()), Ok(person.clone()));
/// assert_eq!(person.to_string(), "struct<id: int32, name: utf8?>");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StructType {
    fields: Vec<TypedFieldRef<AnyDataType>>,
}

impl StructType {
    /// Builds the struct type from its child fields; any list — including an
    /// empty one — is valid.
    pub fn from_parts(fields: Vec<TypedFieldRef<AnyDataType>>) -> Self {
        Self { fields }
    }

    /// The child fields, in declaration order.
    pub fn fields(&self) -> &[TypedFieldRef<AnyDataType>] {
        &self.fields
    }

    /// The child field at `index`, if any.
    pub fn field(&self, index: usize) -> Option<&TypedFieldRef<AnyDataType>> {
        self.fields.get(index)
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, fields: Option<Vec<TypedFieldRef<AnyDataType>>>) -> Self {
        Self::from_parts(fields.unwrap_or_else(|| self.fields.clone()))
    }

    /// Returns a copy with the child fields replaced.
    pub fn with_fields(&self, fields: Vec<TypedFieldRef<AnyDataType>>) -> Self {
        self.copy(Some(fields))
    }
}

impl DataType for StructType {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Struct(self.fields.iter().map(|field| field.to_arrow()).collect())
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Struct(fields) => Ok(Self::from_parts(
                fields
                    .iter()
                    .map(|field| TypedField::from_arrow(field).map(Arc::new))
                    .collect::<Result<_, _>>()?,
            )),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "struct",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![DataTypeId::Struct.to_u8()];
        put_len(&mut out, self.fields.len());
        for field in &self.fields {
            let bytes = field.to_bytes();
            put_len(&mut out, bytes.len());
            out.extend_from_slice(&bytes);
        }
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let mut reader = Reader::new(DataTypeId::Struct.strip_tag(bytes)?);
        let count = reader.take_len()?;
        let mut fields = Vec::new();
        for _ in 0..count {
            fields.push(Arc::new(TypedField::from_bytes(
                reader.take_len_prefixed()?,
            )?));
        }
        reader.finish()?;
        Ok(Self::from_parts(fields))
    }
}

impl NestedType for StructType {
    fn num_children(&self) -> usize {
        self.fields.len()
    }
}

impl fmt::Display for StructType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("struct<")?;
        for (index, field) in self.fields.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            field.fmt(f)?;
        }
        f.write_str(">")
    }
}
