//! The `int32` integer type: [`Int32`], its nullable field [`Int32Field`] and scalar
//! [`Int32Scalar`].
//!
//! [`Int32`] is Apache Arrow's `Int32` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `i32`, stored little-endian in four bytes, Arrow C Data
//! Interface format `"i"`.
//!
//! ```
//! use yggdryl_data::{DataType, Int32, Int32Field, Int32Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int32.name(), "int32");
//! assert_eq!(Int32.arrow_format(), "i");
//! assert_eq!(Int32.byte_width(), Some(4));
//! let bytes = Int32.native_to_bytes(&42);
//! assert_eq!(Int32.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = Int32Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int32", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(Int32Scalar::new(42).value(), Some(&42));
//! assert!(Int32Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Int32;
pub use field::Int32Field;
pub use scalar::Int32Scalar;
