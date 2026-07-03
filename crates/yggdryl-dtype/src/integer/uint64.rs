//! The [`UInt64`] data type.
//!
//! Apache Arrow's `uint64` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u64`, stored little-endian in 8 byte(s), Arrow C
//! Data Interface format `"L"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt64, RawDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt64.name(), "uint64");
//! assert_eq!(UInt64.arrow_format(), "L");
//! assert_eq!(UInt64.byte_width(), Some(8));
//! let bytes = UInt64.native_to_bytes(&42);
//! assert_eq!(UInt64.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt64, u64, "uint64", "L", 8);
