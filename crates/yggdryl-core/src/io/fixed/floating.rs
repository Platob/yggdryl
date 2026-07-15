//! `io::fixed::floating` — the fixed-width **IEEE-754 floating-point** primitives, `f16`
//! (half, via [`half::f16`]), `f32`, and `f64`. Each width is a handful of macro invocations
//! (below) over the generic `fixed` building blocks; all report
//! [`DataTypeCategory::Float`](crate::io::DataTypeCategory::Float), so `dt.is_floating()` drills
//! down without matching.

use half::f16;

use crate::io::fixed::{
    fixed_buffer, fixed_dtype, fixed_field, fixed_native, fixed_scalar, fixed_serie,
};

fixed_native!(f16, "f16", Float16Type, F16);
fixed_dtype!(F16DataType, f16);
fixed_field!(F16Field, f16);
fixed_scalar!(F16Scalar, f16);
fixed_serie!(F16Serie, f16);
fixed_buffer!(F16Buffer, f16);
fixed_native!(f32, "f32", Float32Type, F32);
fixed_dtype!(F32DataType, f32);
fixed_field!(F32Field, f32);
fixed_scalar!(F32Scalar, f32);
fixed_serie!(F32Serie, f32);
fixed_buffer!(F32Buffer, f32);
fixed_native!(f64, "f64", Float64Type, F64);
fixed_dtype!(F64DataType, f64);
fixed_field!(F64Field, f64);
fixed_scalar!(F64Scalar, f64);
fixed_serie!(F64Serie, f64);
fixed_buffer!(F64Buffer, f64);
