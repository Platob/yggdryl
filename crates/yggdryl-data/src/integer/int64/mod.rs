//! The `int64` integer type: [`Int64`], its nullable field [`Int64Field`] and scalar
//! [`Int64Scalar`].
//!
//! [`Int64`] is Apache Arrow's `Int64` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `i64`, stored little-endian in eight bytes, Arrow C Data
//! Interface format `"l"`.
//!
//! ```
//! use yggdryl_data::{DataType, Int64, Int64Field, Int64Scalar, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int64.name(), "int64");
//! assert_eq!(Int64.arrow_format(), "l");
//! assert_eq!(Int64.byte_width(), Some(8));
//! let bytes = Int64.native_to_bytes(&42);
//! assert_eq!(Int64.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = Int64Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int64", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(Int64Scalar::new(42).value(), Some(&42));
//! assert!(Int64Scalar::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Int64;
pub use field::Int64Field;
pub use scalar::Int64Scalar;
