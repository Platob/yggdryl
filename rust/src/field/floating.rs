//! Floating-point field markers.

use super::typed::define_field_types;

define_field_types!(Float16, "float16", crate::DataType::Float16);
define_field_types!(Float32, "float32", crate::DataType::Float32);
define_field_types!(Float64, "float64", crate::DataType::Float64);
