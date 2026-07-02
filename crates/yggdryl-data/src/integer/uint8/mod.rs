//! The `uint8` integer type: [`UInt8`], its nullable field [`UInt8Field`] and scalar
//! [`UInt8Scalar`].
//!
//! [`UInt8`] is Apache Arrow's `UInt8` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `u8`, stored little-endian in one byte, Arrow C Data
//! Interface format `"C"`.
//!
//! ```
//! use yggdryl_data::{DataType, UInt8, UInt8Field, UInt8Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt8.name(), "uint8");
//! assert_eq!(UInt8.arrow_format(), "C");
//! assert_eq!(UInt8.byte_width(), Some(1));
//! let bytes = UInt8.native_to_bytes(&42);
//! assert_eq!(UInt8.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = UInt8Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint8", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(UInt8Scalar::new(42).value(), Some(&42));
//! assert!(UInt8Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::UInt8;
pub use field::UInt8Field;
pub use scalar::UInt8Scalar;
