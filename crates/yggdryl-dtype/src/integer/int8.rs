//! The [`Int8`] data type.
//!
//! Apache Arrow's `int8` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i8`, stored little-endian in 1 byte(s), Arrow C
//! Data Interface format `"c"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int8, RawDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int8.name(), "int8");
//! assert_eq!(Int8.arrow_format(), "c");
//! assert_eq!(Int8.byte_width(), Some(1));
//! let bytes = Int8.native_to_bytes(&42);
//! assert_eq!(Int8.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int8, i8, "int8", "c", 1);
