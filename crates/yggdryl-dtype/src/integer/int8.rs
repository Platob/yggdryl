//! The [`Int8Type`] data type.
//!
//! Apache Arrow's `int8` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i8`, stored little-endian in 1 byte(s), Arrow C
//! Data Interface format `"c"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int8Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int8Type.name(), "int8");
//! assert_eq!(Int8Type.arrow_format(), "c");
//! assert_eq!(Int8Type.byte_width(), Some(1));
//! let bytes = Int8Type.native_to_bytes(&42);
//! assert_eq!(Int8Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int8Type, i8, "int8", "c", 1, Int8);
