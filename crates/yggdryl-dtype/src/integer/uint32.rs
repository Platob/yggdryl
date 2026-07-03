//! The [`UInt32Type`] data type.
//!
//! Apache Arrow's `uint32` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u32`, stored little-endian in 4 byte(s), Arrow C
//! Data Interface format `"I"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt32Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt32Type.name(), "uint32");
//! assert_eq!(UInt32Type.arrow_format(), "I");
//! assert_eq!(UInt32Type.byte_width(), Some(4));
//! let bytes = UInt32Type.native_to_bytes(&42);
//! assert_eq!(UInt32Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt32Type, u32, "uint32", "I", 4, UInt32);
