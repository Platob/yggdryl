//! The [`Int64Type`] data type.
//!
//! Apache Arrow's `int64` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i64`, stored little-endian in 8 byte(s), Arrow C
//! Data Interface format `"l"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int64Type.name(), "int64");
//! assert_eq!(Int64Type.arrow_format(), "l");
//! assert_eq!(Int64Type.byte_width(), Some(8));
//! let bytes = Int64Type.native_to_bytes(&42);
//! assert_eq!(Int64Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int64Type, i64, "int64", "l", 8, Int64);
