//! The [`BinaryType`] — variable-length bytes.

use crate::dtype::{DataType, DataTypeId, PrimitiveType};
use crate::nested_fields::NestedFields;

/// The variable-length binary type — a string of bytes. The first concrete
/// [`DataType`], and a [`PrimitiveType`].
///
/// ```
/// use yggdryl_schema::{BinaryType, DataType, DataTypeId};
///
/// let dt = BinaryType::new();
/// assert_eq!(dt.type_id(), DataTypeId::Binary);
/// assert_eq!(dt.type_name(), "binary");
/// assert!(dt.dyn_eq(&BinaryType::new()));
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BinaryType;

impl BinaryType {
    /// A new binary type.
    pub fn new() -> Self {
        Self
    }
}

// A primitive has no children — the empty `NestedFields` default is exactly right.
impl NestedFields for BinaryType {}

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
}

impl PrimitiveType for BinaryType {}
