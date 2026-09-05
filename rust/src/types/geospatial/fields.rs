//! Geometry and geography field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(GeometryType, "geometry", crate::DataType::Geometry(_));
define_field_types!(GeographyType, "geography", crate::DataType::Geography(_));

/// A geometry-typed field.
pub type GeometryField = TypedField<GeometryType>;
/// A geography-typed field.
pub type GeographyField = TypedField<GeographyType>;
