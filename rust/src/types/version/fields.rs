//! Version field marker and typed aliases.

use crate::TypedField;
use crate::types::typed::{define_field_types, define_scalar_type};

define_field_types!(VersionType, "version", crate::DataType::Version);
define_scalar_type!(
    VersionScalar,
    VersionType,
    "version",
    crate::DataType::Version
);

/// A version-typed field.
pub type VersionField = TypedField<VersionType>;
