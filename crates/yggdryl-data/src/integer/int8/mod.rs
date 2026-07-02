//! The `int8` integer type: [`Int8`], its nullable field [`Int8Field`] and scalar
//! [`Int8Scalar`].
//!
//! [`Int8`] is Apache Arrow's `Int8` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `i8`, stored little-endian in one byte, Arrow C Data
//! Interface format `"c"`.
//!
//! ```
//! use yggdryl_data::{DataType, Int8, Int8Field, Int8Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int8.name(), "int8");
//! assert_eq!(Int8.arrow_format(), "c");
//! assert_eq!(Int8.byte_width(), Some(1));
//! let bytes = Int8.native_to_bytes(&42);
//! assert_eq!(Int8.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = Int8Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int8", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(Int8Scalar::new(42).value(), Some(&42));
//! assert!(Int8Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Int8;
pub use field::Int8Field;
pub use scalar::Int8Scalar;
