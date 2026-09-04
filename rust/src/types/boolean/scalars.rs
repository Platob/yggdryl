//! Null and Boolean typed scalar aliases.

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

impl From<()> for Scalar {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

define_scalar_type!(NullScalar, super::Null, "null", crate::DataType::Null);
define_scalar_type!(
    BooleanScalar,
    super::Boolean,
    "boolean",
    crate::DataType::Boolean
);
