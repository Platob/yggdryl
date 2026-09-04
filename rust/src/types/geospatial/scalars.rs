//! Geospatial typed scalar aliases.

use crate::types::typed::define_scalar_type;

define_scalar_type!(GeometryScalar, super::Geometry, "geometry");
define_scalar_type!(GeographyScalar, super::Geography, "geography");
