//! Geometry and geography field markers.

use super::typed::define_field_types;

define_field_types!(Geometry, "geometry", crate::DataType::Geometry(_));
define_field_types!(Geography, "geography", crate::DataType::Geography(_));
