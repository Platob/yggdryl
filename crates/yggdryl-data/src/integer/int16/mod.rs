//! The `int16` integer type: [`Int16`], its nullable field [`Int16Field`] and scalar
//! [`Int16Scalar`].
//!
//! [`Int16`] is Apache Arrow's `Int16` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `i16`, stored little-endian in two bytes, Arrow C Data
//! Interface format `"s"`.
//!
//! ```
//! use yggdryl_data::{DataType, Int16, Int16Field, Int16Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int16.name(), "int16");
//! assert_eq!(Int16.arrow_format(), "s");
//! assert_eq!(Int16.byte_width(), Some(2));
//! let bytes = Int16.native_to_bytes(&42);
//! assert_eq!(Int16.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = Int16Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int16", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(Int16Scalar::new(42).value(), Some(&42));
//! assert!(Int16Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Int16;
pub use field::Int16Field;
pub use scalar::Int16Scalar;
