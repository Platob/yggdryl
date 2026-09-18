//! Media type field marker and typed aliases.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(MediaTypeType, MediaType, crate::DataType::MediaType);

/// A media-type-typed field.
pub type MediaTypeField = TypedField<MediaTypeType>;
