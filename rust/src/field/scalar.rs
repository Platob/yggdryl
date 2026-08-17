//! Null and Boolean field markers.

use super::typed::define_field_types;

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Null`].
    Null,
    "null",
    crate::DataType::Null
);
define_field_types!(
    /// A field whose datatype is [`crate::DataType::Boolean`].
    Boolean,
    "boolean",
    crate::DataType::Boolean
);
