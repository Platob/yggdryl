//! The [`Int32Type`] data type.
//!
//! Apache Arrow's `int32` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i32`, stored little-endian in 4 byte(s), Arrow C
//! Data Interface format `"i"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int32Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int32Type.name(), "int32");
//! assert_eq!(Int32Type.arrow_format(), "i");
//! assert_eq!(Int32Type.byte_width(), Some(4));
//! let bytes = Int32Type.native_to_bytes(&42);
//! assert_eq!(Int32Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int32Type, i32, "int32", "i", 4, Int32);
