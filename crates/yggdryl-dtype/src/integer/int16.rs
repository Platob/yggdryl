//! The [`Int16Type`] data type.
//!
//! Apache Arrow's `int16` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `i16`, stored little-endian in 2 byte(s), Arrow C
//! Data Interface format `"s"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int16Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int16Type.name(), "int16");
//! assert_eq!(Int16Type.arrow_format(), "s");
//! assert_eq!(Int16Type.byte_width(), Some(2));
//! let bytes = Int16Type.native_to_bytes(&42);
//! assert_eq!(Int16Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(Int16Type, i16, "int16", "s", 2, Int16);
