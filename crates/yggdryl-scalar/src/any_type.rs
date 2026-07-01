//! The [`AnyType`] dynamic data type.

use yggdryl_schema::{DataType, DataTypeId};

use crate::{AnyValue, StructType};

/// A data type of any kind, resolved at run time — the dynamic counterpart of the
/// typed `DataType<T>` impls, used for the heterogeneous children of a [`StructType`].
/// It is a [`DataType`] over the dynamic [`AnyValue`]: either a primitive (by
/// [`DataTypeId`]) or a nested [`StructType`].
///
/// ```
/// use yggdryl_scalar::AnyType;
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let ty = AnyType::primitive(DataTypeId::Int32);
/// assert_eq!(ty.type_id(), DataTypeId::Int32);
/// assert_eq!(ty.type_name(), "int32");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnyType {
    /// A primitive type, identified by its discriminant.
    Primitive(DataTypeId),
    /// A nested struct type.
    Struct(StructType),
}

impl AnyType {
    /// A primitive type identified by `id`.
    pub fn primitive(id: DataTypeId) -> Self {
        Self::Primitive(id)
    }

    /// A nested struct type.
    pub fn struct_type(ty: StructType) -> Self {
        Self::Struct(ty)
    }
}

impl DataType<AnyValue> for AnyType {
    fn type_id(&self) -> DataTypeId {
        match self {
            AnyType::Primitive(id) => *id,
            AnyType::Struct(_) => DataTypeId::Struct,
        }
    }

    fn type_name(&self) -> &str {
        self.type_id().name()
    }
}
