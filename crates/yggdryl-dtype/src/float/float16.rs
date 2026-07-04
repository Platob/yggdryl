//! The [`Float16Type`] data type.
//!
//! Apache Arrow's `float16` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is [`half::f16`], stored little-endian in 2 byte(s), Arrow C
//! Data Interface format `"e"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Float16Type, TypedDataType};
//! use yggdryl_dtype::half::f16;
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Float16Type.name(), "float16");
//! assert_eq!(Float16Type.arrow_format(), "e");
//! assert_eq!(Float16Type.byte_width(), Some(2));
//! let bytes = Float16Type.native_to_bytes(&f16::from_f32(1.5));
//! assert_eq!(Float16Type.native_from_bytes(&bytes).unwrap(), f16::from_f32(1.5));
//! ```

// Reuses the fixed-width little-endian primitive macro shared with the integer
// family: `half::f16`'s `to_le_bytes` / `from_le_bytes` make the codec identical.
crate::integer::int_data_type!(Float16Type, half::f16, "float16", "e", 2, Float16);
