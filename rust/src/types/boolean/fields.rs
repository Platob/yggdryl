//! Null and Boolean field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Null`].
    NullType,
    "null",
    crate::DataType::Null
);

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Boolean`].
    BooleanType,
    "boolean",
    crate::DataType::Boolean
);

/// A null-typed field.
pub type NullField = TypedField<NullType>;
/// A Boolean-typed field.
pub type BooleanField = TypedField<BooleanType>;
