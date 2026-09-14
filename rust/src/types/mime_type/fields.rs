//! MIME type field marker and typed aliases.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(MimeTypeType, "mimetype", crate::DataType::MimeType);

/// A MIME-type-typed field.
pub type MimeTypeField = TypedField<MimeTypeType>;
