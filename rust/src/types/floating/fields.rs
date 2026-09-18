//! Floating-point field markers.

use crate::types::typed::define_field_types;

define_field_types!(Float16Type, "float16", crate::DataType::Float16);
define_field_types!(Float32Type, "float32", crate::DataType::Float32);
define_field_types!(Float64Type, "float64", crate::DataType::Float64);
