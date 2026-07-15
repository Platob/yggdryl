//! [`Value`] — a single, type-erased **cell value**: null, a leaf value's raw little-endian bytes
//! (tagged with its logical [`Field`](crate::io::fixed::Field)), or a nested struct row. It is what
//! an erased [`Column::get`](super::Column::get) returns and what a
//! [`StructScalar`](super::StructScalar) row is built from.

use super::StructScalar;
use crate::io::fixed::Field as LeafField;
use crate::io::{DataTypeId, FieldType};

/// One **type-erased value** — the cell of an erased [`Column`](super::Column).
///
/// A leaf value carries its canonical little-endian bytes plus the logical
/// [`Field`](crate::io::fixed::Field) that names its type, so a caller can decode it (a fixed value
/// is `field.byte_width()` bytes; a var value is its slice). A [`Struct`](Value::Struct) value is a
/// whole nested row.
///
/// A hashable value type — usable as a map/set key, like the leaf scalars — with identity over the
/// canonical bytes (a null value is a distinct, data-less variant).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Value {
    /// A null cell.
    Null,
    /// A present leaf value — its canonical little-endian bytes + the logical field naming its type.
    Leaf {
        /// The logical type of the value (id, width, and decimal/temporal params in metadata).
        field: LeafField,
        /// The value's canonical little-endian bytes (a fixed value's slot, or a var value's slice).
        bytes: Vec<u8>,
    },
    /// A present nested struct value (a row).
    Struct(Box<StructScalar>),
}

impl Value {
    /// A present leaf value from its logical field and canonical bytes.
    pub fn leaf(field: LeafField, bytes: Vec<u8>) -> Self {
        Self::Leaf { field, bytes }
    }

    /// The null value.
    pub fn null() -> Self {
        Self::Null
    }

    /// Whether the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Whether the value is present (non-null).
    pub fn is_valid(&self) -> bool {
        !self.is_null()
    }

    /// The value's element [`DataTypeId`], or `None` if null.
    pub fn type_id(&self) -> Option<DataTypeId> {
        match self {
            Self::Null => None,
            Self::Leaf { field, .. } => Some(FieldType::type_id(field)),
            Self::Struct(_) => Some(DataTypeId::Struct),
        }
    }

    /// A present leaf value's raw bytes, or `None` if null or a struct.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Leaf { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    /// A present struct value's [`StructScalar`], or `None`.
    pub fn as_struct(&self) -> Option<&StructScalar> {
        match self {
            Self::Struct(scalar) => Some(scalar),
            _ => None,
        }
    }
}
