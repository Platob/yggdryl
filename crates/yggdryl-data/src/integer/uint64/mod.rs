//! The `uint64` integer type: [`UInt64`], its nullable field [`UInt64Field`] and scalar
//! [`UInt64Scalar`].
//!
//! [`UInt64`] is Apache Arrow's `UInt64` — a fixed-width
//! [`Primitive`](crate::Primitive) whose native Rust type is `u64`, stored
//! little-endian in eight bytes, Arrow C Data Interface format `"L"`.
//!
//! ```
//! use yggdryl_data::{DataType, UInt64, UInt64Field, UInt64Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt64.name(), "uint64");
//! assert_eq!(UInt64.arrow_format(), "L");
//! assert_eq!(UInt64.byte_width(), Some(8));
//! let bytes = UInt64.native_to_bytes(&42);
//! assert_eq!(UInt64.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = UInt64Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint64", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(UInt64Scalar::new(42).value(), Some(&42));
//! assert!(UInt64Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::UInt64;
pub use field::UInt64Field;
pub use scalar::UInt64Scalar;
