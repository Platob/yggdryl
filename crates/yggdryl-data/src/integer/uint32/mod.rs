//! The `uint32` integer type: [`UInt32`], its nullable field [`UInt32Field`] and scalar
//! [`UInt32Scalar`].
//!
//! [`UInt32`] is Apache Arrow's `UInt32` — a fixed-width
//! [`Primitive`](crate::Primitive) whose native Rust type is `u32`, stored
//! little-endian in four bytes, Arrow C Data Interface format `"I"`.
//!
//! ```
//! use yggdryl_data::{DataType, UInt32, UInt32Field, UInt32Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt32.name(), "uint32");
//! assert_eq!(UInt32.arrow_format(), "I");
//! assert_eq!(UInt32.byte_width(), Some(4));
//! let bytes = UInt32.native_to_bytes(&42);
//! assert_eq!(UInt32.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = UInt32Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint32", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(UInt32Scalar::new(42).value(), Some(&42));
//! assert!(UInt32Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::UInt32;
pub use field::UInt32Field;
pub use scalar::UInt32Scalar;
