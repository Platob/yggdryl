//! Floating-point field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Float16, "float16", crate::DataType::Float16);
define_field_types!(Float32, "float32", crate::DataType::Float32);
define_field_types!(Float64, "float64", crate::DataType::Float64);

/// A Float16-typed field.
pub type Float16Field = TypedField<Float16>;
/// A Float32-typed field.
pub type Float32Field = TypedField<Float32>;
/// A Float64-typed field.
pub type Float64Field = TypedField<Float64>;
