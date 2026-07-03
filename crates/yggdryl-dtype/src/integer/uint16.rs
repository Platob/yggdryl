//! The [`UInt16`] data type.
//!
//! Apache Arrow's `uint16` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u16`, stored little-endian in 2 byte(s), Arrow C
//! Data Interface format `"S"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt16, RawDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt16.name(), "uint16");
//! assert_eq!(UInt16.arrow_format(), "S");
//! assert_eq!(UInt16.byte_width(), Some(2));
//! let bytes = UInt16.native_to_bytes(&42);
//! assert_eq!(UInt16.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt16, u16, "uint16", "S", 2);
