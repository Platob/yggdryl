//! Null and Boolean field markers.

use crate::types::typed::define_field_types;

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Null`].
    NullType, Null,
    crate::DataType::Null
);

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Boolean`].
    BooleanType, Boolean,
    crate::DataType::Boolean
);
