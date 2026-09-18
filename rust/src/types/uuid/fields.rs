//! UUID field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(UuidType, Uuid, crate::DataType::Uuid);

/// A UUID-typed field.
pub type UuidField = TypedField<UuidType>;
