//! The [`BinaryType`] — variable-length bytes.

use std::hash::{Hash, Hasher};

use crate::data_type::DataType;
use crate::data_type_id::DataTypeId;
use crate::primitive_type::PrimitiveType;

/// The variable-length binary type — a string of bytes. The first concrete
/// [`DataType`], and a [`PrimitiveType`].
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId};
///
/// let dt = BinaryType::new();
/// assert_eq!(dt.type_id(), DataTypeId::Binary);
/// assert_eq!(dt.type_name(), "binary");
/// assert_eq!(BinaryType::new(), dt);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BinaryType;

impl BinaryType {
    /// A new binary type.
    pub fn new() -> Self {
        Self
    }
}

impl DataType for BinaryType {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Binary
    }

    fn type_name(&self) -> &str {
        "binary"
    }

    fn clone_box(&self) -> Box<dyn DataType> {
        Box::new(*self)
    }

    fn dyn_eq(&self, other: &dyn DataType) -> bool {
        // Parameterless: two binary types are equal when the discriminants match.
        other.type_id() == self.type_id()
    }

    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.type_id().hash(&mut state);
    }
}

impl PrimitiveType for BinaryType {}
