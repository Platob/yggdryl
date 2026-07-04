//! The [`Float64Type`] data type.
//!
//! Apache Arrow's `float64` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `f64`, stored little-endian in 8 byte(s), Arrow C
//! Data Interface format `"g"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Float64Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Float64Type.name(), "float64");
//! assert_eq!(Float64Type.arrow_format(), "g");
//! assert_eq!(Float64Type.byte_width(), Some(8));
//! let bytes = Float64Type.native_to_bytes(&1.5);
//! assert_eq!(Float64Type.native_from_bytes(&bytes).unwrap(), 1.5);
//! ```

// Reuses the fixed-width little-endian primitive macro shared with the integer
// family: `f64`'s `to_le_bytes` / `from_le_bytes` make the codec identical.
crate::integer::int_data_type!(Float64Type, f64, "float64", "g", 8, Float64);
