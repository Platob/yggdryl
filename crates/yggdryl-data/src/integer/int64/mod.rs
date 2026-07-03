//! The `int64` integer type: [`Int64Type`], its nullable field [`Int64Field`] and scalar
//! [`Int64`].
//!
//! [`Int64Type`] is Apache Arrow's `Int64Type` — a fixed-width [`Primitive`](crate::Primitive)
//! whose native Rust type is `i64`, stored little-endian in eight bytes, Arrow C Data
//! Interface format `"l"`.
//!
//! ```
//! use yggdryl_data::{DataType, Int64Type, Int64Field, Int64, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(Int64Type.name(), "int64");
//! assert_eq!(Int64Type.arrow_format(), "l");
//! assert_eq!(Int64Type.byte_width(), Some(8));
//! let bytes = Int64Type.native_to_bytes(&42);
//! assert_eq!(Int64Type.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = Int64Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int64", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(Int64::new(42).value(), Some(&42));
//! assert!(Int64::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Int64Type;
pub use field::Int64Field;
pub use scalar::Int64;
