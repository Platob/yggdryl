//! The [`UInt64Type`] data type.
//!
//! Apache Arrow's `uint64` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u64`, stored little-endian in 8 byte(s), Arrow C
//! Data Interface format `"L"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt64Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt64Type.name(), "uint64");
//! assert_eq!(UInt64Type.arrow_format(), "L");
//! assert_eq!(UInt64Type.byte_width(), Some(8));
//! let bytes = UInt64Type.native_to_bytes(&42);
//! assert_eq!(UInt64Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt64Type, u64, "uint64", "L", 8, UInt64);
