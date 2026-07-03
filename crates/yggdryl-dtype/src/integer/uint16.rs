//! The [`UInt16Type`] data type.
//!
//! Apache Arrow's `uint16` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u16`, stored little-endian in 2 byte(s), Arrow C
//! Data Interface format `"S"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt16Type, TypedDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt16Type.name(), "uint16");
//! assert_eq!(UInt16Type.arrow_format(), "S");
//! assert_eq!(UInt16Type.byte_width(), Some(2));
//! let bytes = UInt16Type.native_to_bytes(&42);
//! assert_eq!(UInt16Type.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt16Type, u16, "uint16", "S", 2, UInt16);
