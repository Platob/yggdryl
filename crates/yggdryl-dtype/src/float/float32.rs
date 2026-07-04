//! The [`Float32Type`] data type.
//!
//! Apache Arrow's `float32` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `f32`, stored little-endian in 4 byte(s), Arrow C
//! Data Interface format `"f"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Float32Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Float32Type.name(), "float32");
//! assert_eq!(Float32Type.arrow_format(), "f");
//! assert_eq!(Float32Type.byte_width(), Some(4));
//! let bytes = Float32Type.native_to_bytes(&1.5);
//! assert_eq!(Float32Type.native_from_bytes(&bytes).unwrap(), 1.5);
//! ```

// Reuses the fixed-width little-endian primitive macro shared with the integer
// family: `f32`'s `to_le_bytes` / `from_le_bytes` make the codec identical.
crate::integer::int_data_type!(Float32Type, f32, "float32", "f", 4, Float32);
