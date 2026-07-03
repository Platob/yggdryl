//! The [`UInt8`] data type.
//!
//! Apache Arrow's `uint8` — a fixed-width [`Primitive`](crate::Primitive) whose
//! native Rust type is `u8`, stored little-endian in 1 byte(s), Arrow C
//! Data Interface format `"C"`.
//!
//! ```
//! use yggdryl_dtype::{DataType, UInt8, RawDataType};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt8.name(), "uint8");
//! assert_eq!(UInt8.arrow_format(), "C");
//! assert_eq!(UInt8.byte_width(), Some(1));
//! let bytes = UInt8.native_to_bytes(&42);
//! assert_eq!(UInt8.native_from_bytes(&bytes).unwrap(), 42);
//! ```

crate::integer::int_data_type!(UInt8, u8, "uint8", "C", 1);
