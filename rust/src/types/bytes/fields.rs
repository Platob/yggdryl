//! The byte family's field marker.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(BytesType, "bytes", crate::DataType::Bytes(_));

/// A byte-typed field, whichever layout and bound it declares.
pub type BytesField = TypedField<BytesType>;
