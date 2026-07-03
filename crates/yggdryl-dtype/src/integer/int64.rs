//! The [`Int64`] data type.
//!
//! Apache Arrow's `int64` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i64`, stored little-endian in 8 byte(s), Arrow C
//! Data Interface format `"l"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64, RawDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int64.name(), "int64");
//! assert_eq!(Int64.arrow_format(), "l");
//! assert_eq!(Int64.byte_width(), Some(8));
//! let bytes = Int64.native_to_bytes(&42);
//! assert_eq!(Int64.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int64, i64, "int64", "l", 8);
