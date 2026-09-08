//! URL field marker and typed aliases.

use crate::TypedField;
use crate::types::typed::{define_field_types, define_scalar_type};

define_field_types!(UrlType, "url", crate::DataType::Url);
define_scalar_type!(UrlScalar, UrlType, "url", crate::DataType::Url);

/// A URL-typed field.
pub type UrlField = TypedField<UrlType>;
