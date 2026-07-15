//! [`AnyScalar`] — a single, type-erased **cell value**: null, a leaf value's raw little-endian
//! bytes (tagged with its logical [`Field`](crate::io::fixed::Field)), or a nested struct row. It is
//! what an erased [`AnySerie::value`](crate::io::AnySerie::value) returns. Family-agnostic, so it
//! lives at the `io` root.

use super::fixed::Field;
use super::{DataTypeId, FieldType};

/// One **type-erased value** — the cell of an erased [`AnySerie`](crate::io::AnySerie).
///
/// A leaf value carries its canonical little-endian bytes plus the logical
/// [`Field`](crate::io::fixed::Field) naming its type (a fixed value is `field.byte_width()` bytes; a
/// var value is its slice). A [`Struct`](AnyScalar::Struct) value is a whole nested row (its
/// per-field values). A hashable value type — usable as a map/set key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnyScalar {
    /// A null cell.
    Null,
    /// A present leaf value — its canonical little-endian bytes + the logical field naming its type.
    Leaf {
        /// The logical type of the value (id, width, and decimal params in metadata).
        field: Field,
        /// The value's canonical little-endian bytes.
        bytes: Vec<u8>,
    },
    /// A present nested struct value — its per-field cell values, in field order.
    Struct(Vec<AnyScalar>),
}

impl AnyScalar {
    /// A present leaf value from its logical field and canonical bytes.
    pub fn leaf(field: Field, bytes: Vec<u8>) -> Self {
        Self::Leaf { field, bytes }
    }

    /// A present struct value from its per-field cell values.
    pub fn struct_(values: Vec<AnyScalar>) -> Self {
        Self::Struct(values)
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

    /// A present struct value's per-field cell values, or `None`.
    pub fn as_struct(&self) -> Option<&[AnyScalar]> {
        match self {
            Self::Struct(values) => Some(values),
            _ => None,
        }
    }
}
