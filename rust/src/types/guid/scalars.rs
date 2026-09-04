//! GUID typed scalar alias.

use crate::types::typed::define_scalar_type;

define_scalar_type!(GuidScalar, super::Guid, "guid", crate::DataType::Guid);
