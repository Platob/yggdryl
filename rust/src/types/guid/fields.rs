//! GUID field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(GuidType, "guid", crate::DataType::Guid);

/// A GUID-typed field.
pub type GuidField = TypedField<GuidType>;
