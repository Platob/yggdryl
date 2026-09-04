//! Geometry and geography field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Geometry, "geometry", crate::DataType::Geometry(_));
define_field_types!(Geography, "geography", crate::DataType::Geography(_));

/// A geometry-typed field.
pub type GeometryField = TypedField<Geometry>;
/// A geography-typed field.
pub type GeographyField = TypedField<Geography>;
