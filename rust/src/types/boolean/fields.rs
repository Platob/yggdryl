//! Null and Boolean field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    /// A field whose datatype is [`crate::DataType::Null`].
    Null,
    "null",
    crate::DataType::Null
);

/// A null-typed field.
pub type NullField = TypedField<Null>;
/// A Boolean-typed field.
pub type BooleanField = TypedField<Boolean>;
define_field_types!(
    /// A field whose datatype is [`crate::DataType::Boolean`].
    Boolean,
    "boolean",
    crate::DataType::Boolean
);
