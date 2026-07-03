//! The [`UInt8Type`] data type.
//!
//! Apache Arrow's `uint8` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u8`, stored little-endian in 1 byte(s), Arrow C
//! Data Interface format `"C"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt8Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt8Type.name(), "uint8");
//! assert_eq!(UInt8Type.arrow_format(), "C");
//! assert_eq!(UInt8Type.byte_width(), Some(1));
//! let bytes = UInt8Type.native_to_bytes(&42);
//! assert_eq!(UInt8Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt8Type, u8, "uint8", "C", 1, UInt8);
