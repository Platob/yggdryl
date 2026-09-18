//! Floating-point field markers.

use crate::types::typed::define_field_types;

define_field_types!(Float16Type, Float16, crate::DataType::Float16);
define_field_types!(Float32Type, Float32, crate::DataType::Float32);
define_field_types!(Float64Type, Float64, crate::DataType::Float64);
