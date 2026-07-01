//! The [`AnyType`] dynamic data type.

use yggdryl_schema::{DataType, DataTypeId};

use crate::{Any, StructType};

/// A data type of any kind, resolved at run time — the dynamic counterpart of the
/// typed `DataType<T>` impls, used for the heterogeneous children of a [`StructType`].
/// It is a [`DataType`] over the dynamic [`Any`]: either a primitive (by
/// [`DataTypeId`]) or a nested [`StructType`].
///
/// ```
/// use yggdryl_scalar::AnyType;
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// // Typed constructors redirect to the correct primitive…
/// let ty = AnyType::int32();
/// assert_eq!(ty.type_id(), DataTypeId::Int32);
/// assert_eq!(ty.type_name(), "int32");
/// // …and `DataTypeId` / `StructType` convert into an `AnyType` directly.
/// assert_eq!(AnyType::from(DataTypeId::Utf8), AnyType::utf8());
/// // Instance-of accessors read the type back.
/// assert_eq!(ty.as_primitive(), Some(DataTypeId::Int32));
/// assert!(ty.as_struct().is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnyType {
    /// A primitive type, identified by its discriminant.
    Primitive(DataTypeId),
    /// A nested struct type.
    Struct(StructType),
}

/// Generates the typed primitive constructors (e.g. [`AnyType::int64`]).
macro_rules! primitive_ctors {
    ($($method:ident => $id:ident),+ $(,)?) => {$(
        #[doc = concat!("The `", stringify!($id), "` primitive type.")]
        pub fn $method() -> Self {
            Self::Primitive(DataTypeId::$id)
        }
    )+};
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

    primitive_ctors! {
        null => Null,
        boolean => Boolean,
        int8 => Int8,
        int16 => Int16,
        int32 => Int32,
        int64 => Int64,
        int128 => Int128,
        int256 => Int256,
        uint8 => UInt8,
        uint16 => UInt16,
        uint32 => UInt32,
        uint64 => UInt64,
        uint128 => UInt128,
        uint256 => UInt256,
        utf8 => Utf8,
    }

    /// The primitive [`DataTypeId`] this type wraps, or `None` if it is a struct.
    pub fn as_primitive(&self) -> Option<DataTypeId> {
        match self {
            AnyType::Primitive(id) => Some(*id),
            AnyType::Struct(_) => None,
        }
    }

    /// Whether this is a struct type.
    pub fn is_struct(&self) -> bool {
        matches!(self, AnyType::Struct(_))
    }

    /// The nested [`StructType`], or `None` if this is a primitive.
    pub fn as_struct(&self) -> Option<&StructType> {
        match self {
            AnyType::Struct(ty) => Some(ty),
            AnyType::Primitive(_) => None,
        }
    }
}

impl From<DataTypeId> for AnyType {
    fn from(id: DataTypeId) -> Self {
        Self::Primitive(id)
    }
}

impl From<StructType> for AnyType {
    fn from(ty: StructType) -> Self {
        Self::Struct(ty)
    }
}

impl DataType<Any> for AnyType {
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
