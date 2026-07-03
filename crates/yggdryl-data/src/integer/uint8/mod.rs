//! The `uint8` integer type: [`UInt8Type`], its nullable field [`UInt8Field`] and scalar
//! [`UInt8`].
//!
//! [`UInt8Type`] is Apache Arrow's `UInt8Type` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `u8`, stored little-endian in one byte, Arrow C Data
//! Interface format `"C"`.
//!
//! ```
//! use yggdryl_data::{DataType, UInt8Type, UInt8Field, UInt8, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt8Type.name(), "uint8");
//! assert_eq!(UInt8Type.arrow_format(), "C");
//! assert_eq!(UInt8Type.byte_width(), Some(1));
//! let bytes = UInt8Type.native_to_bytes(&42);
//! assert_eq!(UInt8Type.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = UInt8Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint8", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(UInt8::new(42).value(), Some(&42));
//! assert!(UInt8::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::UInt8Type;
pub use field::UInt8Field;
pub use scalar::UInt8;
